use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex, RwLock,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use cch_auth::{AuthError, AuthenticatedManager, Authenticator, ManagerPin};
use cch_config::{ConfigDocument, ConfigStore, MuteScope, NotifyMode};
use cch_model::{CrashKind, CrashRecord, PayloadCodec, RecordId};
use cch_packages::{PackageIndex, is_safe_package_name, is_safe_settings_key};
use cch_settings::{AndroidSettings, DialogTakeoverStatus as SettingsTakeoverStatus};
use cch_store::{Inserted, Store};
use cch_wire::{
    AppConfigResult, AppEntry, BridgeAction, BridgeFacts, CollectorHealth, CollectorSource,
    DialogTakeoverResult, DialogTakeoverStatus, ErrorCode, Event, ExportFormat, ExportRedaction,
    GlobalConfigResult, MAX_PAYLOAD_CHUNK_BYTES, ModuleStatus, MuteResult, NotificationAction,
    NotificationSpec, PROTOCOL_VERSION, PackageIndexFacts, PayloadChunk, PayloadOpened, Request,
    RequestEnvelope, Response, ResponseEnvelope, RuntimeFacts, WireError,
};

use tracing::warn;

use crate::{BridgeBroker, load_package_index};

const EVENT_QUEUE_CAPACITY: usize = 256;
/// Smallest gap between two package-index reloads.
const PACKAGE_RELOAD_INTERVAL_MS: i64 = 5_000;
/// The crash alert activity, in the *relative* form `am -n` accepts.
///
/// Measured, not assumed: with the fully qualified class name this ROM's `am` answers
/// "Activity class … does not exist" even though PackageManager lists the activity — it
/// matches against the abbreviated name it stores. The leading dot is what works.
const MANAGER_DETAIL_COMPONENT: &str =
    "io.github.lingqiqi5211.crashcatcher/.ui.detail.CrashDetailActivity";

pub trait DialogSettings: Send + Sync {
    fn status(&self) -> Result<SettingsTakeoverStatus, String>;
    fn set_enabled(&self, enabled: bool) -> Result<SettingsTakeoverStatus, String>;
    fn dropbox_tag_enabled(&self, tag: &str) -> Result<bool, String>;
}

/// Runtime control over how much the daemon writes to its log.
///
/// A trait rather than a `tracing` handle in the core: the handle's type carries the whole
/// subscriber stack as generics, and this way the host tests — where no subscriber is installed
/// at all — can pass something inert.
pub trait LogLevelControl: Send + Sync {
    fn set_debug(&self, debug: bool);
}

/// Facts about this process, supplied by whoever starts it.
///
/// Grouped rather than added to an already long constructor, and kept out of the core's own
/// discovery: the daemon runs as a child of `service.sh`, which is what actually knows where the
/// state lives, and a test wants to point it somewhere else.
pub struct DaemonRuntime {
    pub state_dir: std::path::PathBuf,
    pub android_sdk: u32,
    pub log_control: Arc<dyn LogLevelControl>,
}

pub struct RuntimeDialogSettings {
    inner: AndroidSettings,
}

impl RuntimeDialogSettings {
    #[must_use]
    pub fn new(android_sdk: u32) -> Self {
        Self {
            inner: AndroidSettings::new(android_sdk),
        }
    }
}

impl DialogSettings for RuntimeDialogSettings {
    fn status(&self) -> Result<SettingsTakeoverStatus, String> {
        self.inner
            .dialog_takeover_status()
            .map_err(|error| error.to_string())
    }

    fn set_enabled(&self, enabled: bool) -> Result<SettingsTakeoverStatus, String> {
        self.inner
            .set_dialog_takeover(enabled)
            .map_err(|error| error.to_string())
    }

    fn dropbox_tag_enabled(&self, tag: &str) -> Result<bool, String> {
        self.inner
            .dropbox_tag_enabled(tag)
            .map_err(|error| error.to_string())
    }
}

pub struct DaemonCore {
    store: Arc<Store>,
    config_store: Mutex<ConfigStore>,
    packages: RwLock<PackageIndex>,
    dialog_settings: Arc<dyn DialogSettings>,
    bridge: Arc<BridgeBroker>,
    started_at: Instant,
    collectors: Mutex<BTreeMap<String, CollectorHealth>>,
    events: EventBus,
    payload_handles: Mutex<HashMap<u64, RecordId>>,
    next_payload_handle: Mutex<u64>,
    volatile_mutes: Mutex<HashMap<String, MuteScope>>,
    /// When the package index was last reloaded, for [`Self::begin_package_reload`].
    last_package_reload_ms: Mutex<i64>,
    state_dir: std::path::PathBuf,
    android_sdk: u32,
    log_control: Arc<dyn LogLevelControl>,
}

impl DaemonCore {
    #[must_use]
    pub fn new(
        store: Arc<Store>,
        config_store: ConfigStore,
        packages: PackageIndex,
        dialog_settings: Arc<dyn DialogSettings>,
        bridge: Arc<BridgeBroker>,
        runtime: DaemonRuntime,
    ) -> Arc<Self> {
        let collectors = CollectorSource::ALL
            .into_iter()
            .map(|source| {
                (
                    collector_key(source),
                    CollectorHealth {
                        source,
                        enabled: true,
                        ever_received: false,
                        last_received_ms: None,
                        detail: None,
                    },
                )
            })
            .collect();
        Arc::new(Self {
            store,
            config_store: Mutex::new(config_store),
            packages: RwLock::new(packages),
            dialog_settings,
            bridge,
            started_at: Instant::now(),
            collectors: Mutex::new(collectors),
            events: EventBus::default(),
            payload_handles: Mutex::new(HashMap::new()),
            next_payload_handle: Mutex::new(1),
            volatile_mutes: Mutex::new(HashMap::new()),
            // Zero, not "now": the index was just built, but a manager installed
            // while the daemon was down still needs the very first connection to be
            // able to trigger a reload.
            last_package_reload_ms: Mutex::new(0),
            state_dir: runtime.state_dir,
            android_sdk: runtime.android_sdk,
            log_control: runtime.log_control,
        })
    }

    /// Puts the stored logging level into effect, for start-up.
    ///
    /// The switch is persisted, so a daemon that restarts while someone is reproducing something
    /// has to come back at the level they chose rather than reverting to info.
    pub fn apply_log_level(&self) -> Result<(), WireError> {
        let config = self.load_config()?;
        self.log_control.set_debug(config.global.debug_logging);
        Ok(())
    }

    #[must_use]
    pub fn bridge(&self) -> &Arc<BridgeBroker> {
        &self.bridge
    }

    pub fn subscribe(&self) -> Receiver<Event> {
        self.events.subscribe()
    }

    /// Whether the package index's system flags came from PackageManager; see
    /// [`PackageIndex::system_flags_known`].
    #[must_use]
    pub fn package_flags_known(&self) -> bool {
        self.packages
            .read()
            .is_ok_and(|packages| packages.system_flags_known())
    }

    pub fn replace_packages(&self, packages: PackageIndex) -> Result<(), WireError> {
        *self
            .packages
            .write()
            .map_err(|_| WireError::internal("package index lock poisoned"))? = packages;
        Ok(())
    }

    /// Installs a rebuilt index without losing what the current one already established.
    ///
    /// The reload behind this exists to pick up a moved APK path, and can happen while
    /// `cmd package` is unavailable — replacing outright would then undo the completed system
    /// flags and leave every app looking third-party until the next reboot.
    fn install_packages(&self, mut packages: PackageIndex) -> Result<(), WireError> {
        {
            // Scoped so the read guard is gone before `replace_packages` takes the write lock;
            // holding both on one thread would deadlock.
            let current = self
                .packages
                .read()
                .map_err(|_| WireError::internal("package index lock poisoned"))?;
            packages.inherit_system_flags(&current);
        }
        self.replace_packages(packages)
    }

    /// Authenticates a peer uid against the pinned manager certificate.
    ///
    /// Retries once against a freshly loaded package index when the first failure is
    /// one a stale index explains. The index is built at start-up, but an APK path is
    /// not stable: reinstalling or updating the manager moves it to a new randomised
    /// directory under `/data/app`, so a long-running daemon would keep rejecting the
    /// real manager — reporting "not connected" — until something restarted it. That
    /// is precisely what happens after every manager update, so it cannot be left to
    /// a reboot.
    pub fn authenticate_uid(
        &self,
        uid: u32,
        pin: &ManagerPin,
    ) -> Result<AuthenticatedManager, WireError> {
        let first = match self.authenticate_against_current_index(uid, pin)? {
            Ok(manager) => return Ok(manager),
            Err(error) => error,
        };

        if !first.may_be_stale_package_index() || !self.begin_package_reload() {
            return Err(unauthorized(&first));
        }

        match load_package_index() {
            Ok(packages) => self.install_packages(packages)?,
            Err(error) => {
                // Report the authentication failure, not the reload failure: the
                // caller asked to be let in, and the reload was our idea.
                warn!(%error, "package index reload failed");
                return Err(unauthorized(&first));
            }
        }

        self.authenticate_against_current_index(uid, pin)?
            .map_err(|second| unauthorized(&second))
    }

    fn authenticate_against_current_index(
        &self,
        uid: u32,
        pin: &ManagerPin,
    ) -> Result<Result<AuthenticatedManager, AuthError>, WireError> {
        let packages = self
            .packages
            .read()
            .map_err(|_| WireError::internal("package index lock poisoned"))?;
        Ok(Authenticator::new(&packages, pin).authenticate_uid(uid))
    }

    /// Rate-limits index reloads, and reports whether this caller may do one.
    ///
    /// Without this, any app on the device could make the daemon enumerate every
    /// installed package on demand simply by reconnecting in a loop. One reload per
    /// [`PACKAGE_RELOAD_INTERVAL_MS`] is far more often than installs actually happen
    /// and cheap enough to be uninteresting as an amplifier.
    fn begin_package_reload(&self) -> bool {
        let now = now_ms();
        let Ok(mut last) = self.last_package_reload_ms.lock() else {
            return false;
        };
        if now.saturating_sub(*last) < PACKAGE_RELOAD_INTERVAL_MS {
            return false;
        }
        *last = now;
        true
    }

    pub fn dispatch(&self, envelope: RequestEnvelope) -> ResponseEnvelope {
        let seq = envelope.seq;
        match self.handle_request(envelope.request) {
            Ok(response) => ResponseEnvelope::ok(seq, response),
            Err(error) => ResponseEnvelope::err(seq, error),
        }
    }

    pub fn ingest(&self, mut record: CrashRecord) -> Result<Option<Inserted>, WireError> {
        self.enrich_record(&mut record)?;
        let config = self.load_config()?;
        if !captures_kind(&config, record.kind)
            || !config.should_record(
                &record.package_name,
                record.is_system_app,
                record.is_main_process(),
                record.self_handled,
            )
        {
            return Ok(None);
        }

        let inserted = self
            .store
            .insert(&record, config.global.retention)
            .map_err(|error| error.to_wire())?;
        self.store
            .sweep(now_ms(), config.global.retention)
            .map_err(|error| error.to_wire())?;

        self.events.broadcast(Event::CrashRecorded {
            record: inserted.record.clone(),
            group: inserted.group.clone(),
            is_new_group: inserted.is_new_group,
        });

        if config.should_notify(&record.package_name, record.is_foreground)
            && !self.is_muted(&record.package_name)
        {
            self.notify(
                &record,
                &inserted,
                config.effective_notify_mode(&record.package_name),
            );
        }
        Ok(Some(inserted))
    }

    pub fn mark_collector_received(&self, source: CollectorSource, at_ms: i64) {
        if let Ok(mut collectors) = self.collectors.lock()
            && let Some(health) = collectors.get_mut(&collector_key(source))
        {
            health.ever_received = true;
            health.last_received_ms = Some(at_ms);
            health.detail = None;
        }
    }

    pub fn mark_collector_error(&self, source: CollectorSource, detail: impl Into<String>) {
        if let Ok(mut collectors) = self.collectors.lock()
            && let Some(health) = collectors.get_mut(&collector_key(source))
        {
            health.detail = Some(detail.into());
        }
    }

    pub fn clear_collector_error(&self, source: CollectorSource) {
        if let Ok(mut collectors) = self.collectors.lock()
            && let Some(health) = collectors.get_mut(&collector_key(source))
        {
            health.detail = None;
        }
    }

    pub fn was_source_ingested(&self, key: &str) -> Result<bool, WireError> {
        self.store
            .was_ingested(key)
            .map_err(|error| error.to_wire())
    }

    pub fn mark_source_ingested(&self, key: &str) -> Result<bool, WireError> {
        self.store
            .mark_ingested(key, now_ms())
            .map_err(|error| error.to_wire())
    }

    /// Drops every mute. Called at start-up, which is what `UntilRestart` means.
    pub fn clear_volatile_mutes(&self) -> Result<(), WireError> {
        self.clear_mutes(|_| true).map(|_| ())
    }

    /// Drops the mutes meant to last until the screen was unlocked, returning how many went.
    ///
    /// Driven by `ScreenEvent` from the events collector. Without it the scope never expired:
    /// the only thing that ever cleared a mute was a daemon restart, so "until unlock" was
    /// "until reboot" and the app stayed silent indefinitely.
    pub fn clear_unlock_mutes(&self) -> Result<usize, WireError> {
        self.clear_mutes(|scope| scope == MuteScope::UntilUnlock)
    }

    /// Forgets the mutes matching `matches`, in memory, in the store *and* in the config.
    ///
    /// The config is the one that was missing, and it is the one the user sees: the app's
    /// settings screen reads `AppConfig.mute`, and `SetAppConfig` re-applies it — so a mute
    /// left behind there shows as still active and comes back the next time anything about
    /// that app is changed.
    ///
    /// Driven from the config rather than from memory, because at start-up memory is empty:
    /// a scope that only ever expires on restart cannot be cleared by looking at the map it
    /// was just wiped from. Memory is folded in as well so a mute set since the last write
    /// is not missed.
    fn clear_mutes(&self, matches: impl Fn(MuteScope) -> bool) -> Result<usize, WireError> {
        let mut cleared: Vec<String> = self
            .load_config()?
            .apps
            .iter()
            .filter(|(_, config)| matches(config.mute))
            .map(|(package, _)| package.clone())
            .collect();

        {
            let mut mutes = self
                .volatile_mutes
                .lock()
                .map_err(|_| WireError::internal("mute lock poisoned"))?;
            for (package, scope) in mutes.clone() {
                if matches(scope) {
                    mutes.remove(&package);
                    if !cleared.contains(&package) {
                        cleared.push(package);
                    }
                }
            }
        }

        if cleared.is_empty() {
            return Ok(0);
        }

        for package in &cleared {
            self.store
                .set_package_mute(package, None)
                .map_err(|error| error.to_wire())?;
        }
        self.update_config(|document| {
            for package in &cleared {
                let mut config = document.app(package);
                config.mute = MuteScope::None;
                if config.is_default() {
                    document.apps.remove(package);
                } else {
                    document.apps.insert(package.clone(), config);
                }
            }
        })?;
        self.events.broadcast(Event::ConfigChanged);
        Ok(cleared.len())
    }

    /// Opens a payload for descriptor passing, *and* registers a chunk handle for it.
    ///
    /// The handle is not redundant. Descriptor passing over `SCM_RIGHTS` can fail on
    /// the receiving side for reasons the daemon cannot see or predict — Android's
    /// `LocalSocket` truncating the control message, or SELinux refusing the app
    /// access to the received memfd, both of which happen on HyperOS. The client then
    /// has a response that claims a descriptor it does not have. Handing back a handle
    /// as well means it can fall back to chunked reads immediately instead of failing
    /// the whole page, and costs one unused map entry when the fast path works.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn open_payload_fd(
        &self,
        id: &RecordId,
    ) -> Result<(std::os::fd::OwnedFd, PayloadOpened), WireError> {
        let detail = self.store.get_record(id).map_err(|error| error.to_wire())?;
        let total_bytes = self
            .store
            .payload_text_bytes(id)
            .map_err(|error| error.to_wire())?;
        let fd = self
            .store
            .open_payload_fd(id)
            .map_err(|error| error.to_wire())?;
        let handle = self.register_payload_handle(id.clone())?;
        Ok((
            fd,
            PayloadOpened {
                total_bytes,
                state: detail.record.payload_state,
                codec_on_disk: PayloadCodec::Raw,
                fd_attached: true,
                handle: Some(handle),
            },
        ))
    }

    /// Reserves a chunk-read handle for a record.
    fn register_payload_handle(&self, id: RecordId) -> Result<u64, WireError> {
        let handle = {
            let mut next = self
                .next_payload_handle
                .lock()
                .map_err(|_| WireError::internal("payload handle counter poisoned"))?;
            let handle = *next;
            *next = next.saturating_add(1);
            handle
        };
        self.payload_handles
            .lock()
            .map_err(|_| WireError::internal("payload handle lock poisoned"))?
            .insert(handle, id);
        Ok(handle)
    }

    fn handle_request(&self, request: Request) -> Result<Response, WireError> {
        match request {
            Request::Handshake {
                protocol_version,
                client_version: _,
            } => {
                if protocol_version != PROTOCOL_VERSION {
                    return Err(WireError::new(
                        ErrorCode::VersionMismatch,
                        format!(
                            "client protocol {protocol_version}, daemon protocol {PROTOCOL_VERSION}"
                        ),
                    ));
                }
                Ok(Response::Handshake {
                    protocol_version: PROTOCOL_VERSION,
                    daemon_version: env!("CCH_DAEMON_VERSION").to_owned(),
                })
            }
            Request::ModuleStatus => Ok(Response::ModuleStatus {
                status: Box::new(self.module_status()?),
            }),
            Request::ListGroups { page } => self
                .store
                .list_groups(&page)
                .map(|page| Response::Groups { page })
                .map_err(|error| error.to_wire()),
            Request::ListRecords { group_id, page } => self
                .store
                .list_records(&group_id, &page)
                .map(|page| Response::Records { page })
                .map_err(|error| error.to_wire()),
            // One group by id, for a detail page opened without the list that
            // produced it — a notification tap, or a restored back stack. Fetching
            // the containing page and filtering it client-side would need a cursor
            // the caller does not have.
            Request::GetGroup { group_id } => self
                .store
                .get_group(&group_id)
                .map(|group| Response::Group {
                    group: Box::new(group),
                })
                .map_err(|error| error.to_wire()),
            Request::GetRecord { id } => self
                .store
                .get_record(&id)
                .map(|detail| Response::Record {
                    detail: Box::new(detail),
                })
                .map_err(|error| error.to_wire()),
            Request::OpenPayload { id } => self.open_payload_fallback(id),
            Request::ReadPayload {
                handle,
                offset,
                len,
            } => self.read_payload(handle, offset, len),
            Request::ClosePayload { handle } => {
                self.payload_handles
                    .lock()
                    .map_err(|_| WireError::internal("payload handle lock poisoned"))?
                    .remove(&handle);
                Ok(Response::Closed)
            }
            Request::ExportRecords {
                ids,
                format,
                redaction,
            } => self.export_records(&ids, format, redaction),
            Request::DeleteRecords { target } => {
                let (removed_records, removed_groups) = self
                    .store
                    .delete(&target)
                    .map_err(|error| error.to_wire())?;
                Ok(Response::Deleted {
                    removed_records,
                    removed_groups,
                })
            }
            Request::GetGlobalConfig => {
                let config = self.load_config()?.global;
                Ok(Response::GlobalConfig {
                    result: Box::new(GlobalConfigResult {
                        config,
                        adjusted: false,
                    }),
                })
            }
            Request::SetGlobalConfig { patch } => {
                let before = self.load_config()?.global;
                let requested = patch.apply(&before);
                let stored = self.update_config(|document| {
                    document.global = patch.apply(&document.global);
                })?;
                // Takes effect on this process immediately: the point of the switch is to
                // capture something that is happening now, and asking for a restart first would
                // lose whatever prompted it.
                self.log_control.set_debug(stored.global.debug_logging);
                self.events.broadcast(Event::ConfigChanged);
                Ok(Response::GlobalConfig {
                    result: Box::new(GlobalConfigResult {
                        adjusted: requested != stored.global,
                        config: stored.global,
                    }),
                })
            }
            Request::GetAppConfig { package_name } => {
                validate_settings_key(&package_name)?;
                Ok(Response::AppConfig {
                    result: AppConfigResult {
                        config: self.load_config()?.app(&package_name),
                        package_name,
                    },
                })
            }
            Request::SetAppConfig {
                package_name,
                patch,
            } => {
                validate_settings_key(&package_name)?;
                let stored = self.update_config(|document| {
                    let updated = patch.apply(&document.app(&package_name));
                    if updated.is_default() {
                        document.apps.remove(&package_name);
                    } else {
                        document.apps.insert(package_name.clone(), updated);
                    }
                })?;
                let config = stored.app(&package_name);
                self.apply_mute(&package_name, config.mute)?;
                self.events.broadcast(Event::ConfigChanged);
                Ok(Response::AppConfig {
                    result: AppConfigResult {
                        package_name,
                        config,
                    },
                })
            }
            Request::ListApps {
                include_system_apps,
                include_system_processes,
                query,
                limit,
            } => self.list_apps(
                include_system_apps,
                include_system_processes,
                query.as_deref(),
                limit,
            ),
            Request::Stats {
                time_from_ms,
                time_to_ms,
                bucket_ms,
            } => {
                let mut stats = self
                    .store
                    .stats(time_from_ms, time_to_ms, bucket_ms)
                    .map_err(|error| error.to_wire())?;
                stats.installed_app_count = self
                    .packages
                    .read()
                    .map_err(|_| WireError::internal("package index lock poisoned"))?
                    .entries()
                    .len() as u64;
                Ok(Response::Stats {
                    stats: Box::new(stats),
                })
            }
            Request::ReadRuntimeLog { name, max_bytes } => {
                let budget = if max_bytes == 0 {
                    crate::diagnostics::DEFAULT_LOG_BYTES
                } else {
                    max_bytes
                };
                let log =
                    crate::diagnostics::read_runtime_log(&self.state_dir, name.as_deref(), budget);
                Ok(Response::RuntimeLog {
                    name: log.name,
                    text: log.text,
                    truncated: log.truncated,
                    total_bytes: log.total_bytes,
                    files: log.files,
                })
            }
            Request::ReopenApp {
                package_name,
                user_id,
            } => {
                validate_package(&package_name)?;
                // Same reason as the crash alert: the bridge cannot start activities, so
                // this runs `am` as root instead of going through it.
                Ok(Response::Reopened {
                    launched: start_launcher_activity(&package_name, user_id),
                })
            }
            Request::DismissNotification { record_id } => Ok(Response::NotificationDismissed {
                // Not an error when the bridge is away: the notification it would have
                // taken down cannot exist either, so the caller has what it asked for.
                dismissed: self.bridge.cancel_notification(record_id).unwrap_or(false),
            }),
            Request::MuteApp {
                package_name,
                scope,
            } => {
                validate_settings_key(&package_name)?;
                self.apply_mute(&package_name, scope)?;
                self.update_config(|document| {
                    let mut config = document.app(&package_name);
                    config.mute = scope;
                    if config.is_default() {
                        document.apps.remove(&package_name);
                    } else {
                        document.apps.insert(package_name.clone(), config);
                    }
                })?;
                self.events.broadcast(Event::ConfigChanged);
                Ok(Response::Muted {
                    result: MuteResult {
                        package_name,
                        scope,
                        muted_until_ms: (scope != MuteScope::None).then_some(i64::MAX),
                    },
                })
            }
            Request::SetDialogTakeover { enabled } => {
                let raw = self
                    .dialog_settings
                    .set_enabled(enabled)
                    .map_err(WireError::unavailable)?;
                self.update_config(|document| {
                    document.global.takeover_system_dialog = enabled;
                })?;
                self.events.broadcast(Event::ConfigChanged);
                Ok(Response::DialogTakeover {
                    result: DialogTakeoverResult {
                        status: wire_dialog_status(&raw, None),
                    },
                })
            }
        }
    }

    fn module_status(&self) -> Result<ModuleStatus, WireError> {
        let config = self.load_config()?;
        let mut collectors: Vec<_> = self
            .collectors
            .lock()
            .map_err(|_| WireError::internal("collector health lock poisoned"))?
            .values()
            .cloned()
            .collect();
        for collector in &mut collectors {
            collector.enabled = collector_enabled(&config, collector.source);
        }
        if let Some(dropbox) = collectors
            .iter_mut()
            .find(|health| health.source == CollectorSource::Dropbox)
        {
            for tag in ["data_app_crash", "data_app_anr", "data_app_native_crash"] {
                if matches!(self.dialog_settings.dropbox_tag_enabled(tag), Ok(false)) {
                    dropbox.detail = Some(format!("dropbox:{tag} is disabled"));
                    break;
                }
            }
        }

        let dialog_takeover = match self.dialog_settings.status() {
            Ok(status) => wire_dialog_status(&status, None),
            Err(error) => DialogTakeoverStatus {
                requested: config.global.takeover_system_dialog,
                effective: false,
                anr_show_background_conflict: false,
                unsupported_reason: Some(error),
            },
        };
        let uptime_ms = i64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(i64::MAX);
        Ok(ModuleStatus {
            daemon_version: env!("CCH_DAEMON_VERSION").to_owned(),
            protocol_version: PROTOCOL_VERSION,
            uptime_ms,
            collectors,
            bridge_connected: self.bridge.is_connected(),
            dialog_takeover,
            storage: self
                .store
                .storage_status()
                .map_err(|error| error.to_wire())?,
            runtime: self.runtime_facts(&config)?,
        })
    }

    /// The facts a diagnostics page needs that nothing else reports.
    fn runtime_facts(&self, config: &ConfigDocument) -> Result<RuntimeFacts, WireError> {
        let packages = self
            .packages
            .read()
            .map_err(|_| WireError::internal("package index lock poisoned"))?;
        let hello = self.bridge.hello();
        let active_mutes = self
            .volatile_mutes
            .lock()
            .map(|mutes| mutes.len())
            .unwrap_or(0)
            .max(
                config
                    .apps
                    .values()
                    .filter(|app| app.mute != MuteScope::None)
                    .count(),
            );

        Ok(RuntimeFacts {
            pid: std::process::id(),
            // Compile-time: this is the binary that is running, not what the device prefers.
            abi: std::env::consts::ARCH.to_owned(),
            android_sdk: self.android_sdk,
            selinux: read_selinux_mode(),
            store_schema_version: cch_store::SCHEMA_VERSION,
            debug_logging: config.global.debug_logging,
            package_index: PackageIndexFacts {
                package_count: u32::try_from(packages.entries().len()).unwrap_or(u32::MAX),
                system_flags_known: packages.system_flags_known(),
            },
            bridge: BridgeFacts {
                connected: self.bridge.is_connected(),
                version: hello.as_ref().map(|hello| hello.bridge_version.clone()),
                android_sdk: hello.as_ref().map(|hello| hello.android_sdk),
            },
            active_mutes: u32::try_from(active_mutes).unwrap_or(u32::MAX),
        })
    }

    fn open_payload_fallback(&self, id: RecordId) -> Result<Response, WireError> {
        let detail = self
            .store
            .get_record(&id)
            .map_err(|error| error.to_wire())?;
        let total_bytes = self
            .store
            .payload_text_bytes(&id)
            .map_err(|error| error.to_wire())?;
        let handle = self.register_payload_handle(id)?;
        Ok(Response::PayloadOpened {
            payload: PayloadOpened {
                total_bytes,
                state: detail.record.payload_state,
                codec_on_disk: PayloadCodec::Raw,
                fd_attached: false,
                handle: Some(handle),
            },
        })
    }

    fn read_payload(&self, handle: u64, offset: u64, len: u32) -> Result<Response, WireError> {
        if len == 0 {
            return Err(WireError::invalid_request(
                "payload chunk length must be non-zero",
            ));
        }
        let id = self
            .payload_handles
            .lock()
            .map_err(|_| WireError::internal("payload handle lock poisoned"))?
            .get(&handle)
            .cloned()
            .ok_or_else(|| WireError::not_found(format!("unknown payload handle {handle}")))?;
        let len = len.min(MAX_PAYLOAD_CHUNK_BYTES);
        let (text, next_offset, eof) = self
            .store
            .read_payload_chunk(&id, offset, len)
            .map_err(|error| error.to_wire())?;
        Ok(Response::PayloadChunk {
            chunk: PayloadChunk {
                offset,
                text,
                next_offset,
                eof,
            },
        })
    }

    fn export_records(
        &self,
        ids: &[RecordId],
        format: ExportFormat,
        redaction: ExportRedaction,
    ) -> Result<Response, WireError> {
        if ids.is_empty() {
            return Err(WireError::invalid_request(
                "export requires at least one record",
            ));
        }
        let mut entries = Vec::with_capacity(ids.len());
        for id in ids {
            let mut detail = self.store.get_record(id).map_err(|error| error.to_wire())?;
            let payload = self
                .store
                .read_payload_text(id)
                .unwrap_or_else(|error| format!("[payload unavailable: {error}]"));
            if redaction.hide_package_name {
                detail.group.package_name = "<redacted>".to_owned();
                detail.group.process_name = "<redacted>".to_owned();
            }
            entries.push((detail, payload));
        }

        let text = match format {
            ExportFormat::Json => {
                let values: Vec<_> = entries
                    .iter()
                    .map(|(detail, payload)| {
                        serde_json::json!({ "detail": detail, "payload": payload })
                    })
                    .collect();
                serde_json::to_string_pretty(&values)
                    .map_err(|error| WireError::internal(error.to_string()))?
            }
            ExportFormat::Text => entries
                .into_iter()
                .map(|(detail, payload)| {
                    format!(
                        "{} | {:?} | {}\n{}\n",
                        detail.group.package_name,
                        detail.group.kind,
                        detail.record.happened_at_ms,
                        payload
                    )
                })
                .collect::<Vec<_>>()
                .join("\n---\n"),
        };
        Ok(Response::Export { text })
    }

    fn list_apps(
        &self,
        include_system_apps: bool,
        include_system_processes: bool,
        query: Option<&str>,
        limit: u32,
    ) -> Result<Response, WireError> {
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let config = self.load_config()?;
        let apps = self
            .store
            .package_rollups(include_system_apps, include_system_processes, limit)
            .map_err(|error| error.to_wire())?
            .into_iter()
            .filter(|rollup| {
                query.is_none_or(|needle| {
                    rollup
                        .package_name
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
                })
            })
            .map(|rollup| {
                let label = self
                    .bridge
                    .cached_package(&rollup.package_name, rollup.user_id)
                    .and_then(|package| package.label);
                AppEntry {
                    config: config.app(&rollup.package_name),
                    package_name: rollup.package_name,
                    label,
                    user_id: rollup.user_id,
                    is_system_app: rollup.is_system_app,
                    package_installed: rollup.package_installed,
                    group_count: rollup.group_count,
                    occurrence: rollup.occurrence,
                    last_seen_ms: Some(rollup.last_seen_ms),
                }
            })
            .collect();
        Ok(Response::Apps { apps })
    }

    fn load_config(&self) -> Result<ConfigDocument, WireError> {
        self.config_store
            .lock()
            .map_err(|_| WireError::internal("config store lock poisoned"))?
            .load()
            .map_err(|error| WireError::internal(error.to_string()))
    }

    fn update_config<F>(&self, update: F) -> Result<ConfigDocument, WireError>
    where
        F: FnOnce(&mut ConfigDocument),
    {
        self.config_store
            .lock()
            .map_err(|_| WireError::internal("config store lock poisoned"))?
            .update(update)
            .map_err(|error| WireError::internal(error.to_string()))
    }

    fn apply_mute(&self, package_name: &str, scope: MuteScope) -> Result<(), WireError> {
        let mut mutes = self
            .volatile_mutes
            .lock()
            .map_err(|_| WireError::internal("mute lock poisoned"))?;
        if scope == MuteScope::None {
            mutes.remove(package_name);
        } else {
            mutes.insert(package_name.to_owned(), scope);
        }
        self.store
            .set_package_mute(package_name, (scope != MuteScope::None).then_some(i64::MAX))
            .map(|_| ())
            .map_err(|error| error.to_wire())
    }

    fn is_muted(&self, package_name: &str) -> bool {
        self.volatile_mutes
            .lock()
            .is_ok_and(|mutes| mutes.contains_key(package_name))
    }

    /// Fills in what only the daemon can know: version, install location, and whether the
    /// crashing thing is an app at all.
    ///
    /// The last one is why a lookup miss is not ignored. A tombstone names its process, so a
    /// platform binary arrives with `package_name` set to `/vendor/bin/hw/…` or `surfaceflinger`;
    /// leaving `is_system_app` at its default let those straight past `include_system_apps`,
    /// which is what made the setting look like it did nothing.
    fn enrich_record(&self, record: &mut CrashRecord) -> Result<(), WireError> {
        if let Some(package) = self
            .bridge
            .cached_package(&record.package_name, record.user_id)
        {
            record.app_version_name = package.version_name;
            record.app_version_code = package.version_code;
            record.is_system_app = package.is_system_app;
            record.package_installed = true;
            return Ok(());
        }
        let packages = self
            .packages
            .read()
            .map_err(|_| WireError::internal("package index lock poisoned"))?;
        if let Some(package) = packages.by_name(&record.package_name) {
            record.user_id = i32::try_from(package.user_id()).unwrap_or(i32::MAX);
            record.app_version_code = record.app_version_code.or(package.version_code);
        }
        if let Some(origin) = classify_package(&packages, &record.package_name) {
            record.is_system_app = origin.is_system_app;
            record.package_installed = origin.package_installed;
        }
        Ok(())
    }

    /// Posts the user-facing alert for a freshly recorded crash.
    ///
    /// The wording lives here rather than in the manager because the daemon is what is
    /// running when a crash happens — the manager may never have been opened. The strings
    /// are Chinese to match the app, which ships one locale; if that ever changes they
    /// have to move to the manager and be pushed down at connect time, since neither the
    /// daemon nor the DEX bridge has access to Android resources.
    ///
    /// The title carries the package name only as a fallback: the bridge is a privileged
    /// Java process that can resolve the launcher label, and does, because
    /// `io.github.example.app` is not how a user knows which app just died.
    fn notify(&self, record: &CrashRecord, inserted: &Inserted, mode: NotifyMode) {
        let title = format!("{} 已崩溃", record.package_name);
        let body = record
            .summary
            .text
            .clone()
            .or_else(|| record.summary.class_name.clone())
            .unwrap_or_else(|| format!("{:?}", record.kind));
        match mode {
            NotifyMode::Nothing => {}
            NotifyMode::Dialog => {
                start_alert_activity(inserted.record.id.as_str(), record.user_id);
            }
            NotifyMode::Notification | NotifyMode::Toast => {
                let notification = NotificationSpec {
                    record_id: inserted.record.id.clone(),
                    package_name: record.package_name.clone(),
                    user_id: record.user_id,
                    title,
                    body,
                    actions: vec![
                        NotificationAction {
                            title: "查看详情".to_owned(),
                            action: BridgeAction::OpenDetails,
                        },
                        NotificationAction {
                            title: "重新打开".to_owned(),
                            action: BridgeAction::ReopenApp,
                        },
                        NotificationAction {
                            title: "静音".to_owned(),
                            action: BridgeAction::MuteUntilUnlock,
                        },
                    ],
                };
                let _ = self.bridge.post_notification(notification);
            }
        }
    }
}

/// SELinux's current mode, or `unknown` where it cannot be read.
///
/// Worth reporting because several things here behave differently under enforcing — descriptor
/// passing to the manager most visibly — and "it works on my permissive device" is otherwise
/// indistinguishable from "it works".
fn read_selinux_mode() -> String {
    match std::fs::read_to_string("/sys/fs/selinux/enforce") {
        Ok(value) => match value.trim() {
            "1" => "enforcing".to_owned(),
            "0" => "permissive".to_owned(),
            other => format!("unknown({other})"),
        },
        Err(_) => "unknown".to_owned(),
    }
}

/// Where a crash came from, as far as the package index can tell.
struct PackageOrigin {
    is_system_app: bool,
    package_installed: bool,
}

/// Decides whether a name belongs to an app, a system app, or no app at all.
///
/// `None` means "do not touch what is already there". That is the answer whenever the index
/// is empty, which means it could not be read rather than that the device has no packages: a
/// miss would otherwise reclassify every app on the device as a platform process, and with
/// `include_system_apps` off — the default — that silently drops all crash recording.
fn classify_package(packages: &PackageIndex, name: &str) -> Option<PackageOrigin> {
    if packages.is_empty() {
        return None;
    }
    match packages.by_name(name) {
        Some(package) => Some(PackageOrigin {
            is_system_app: package.is_system,
            package_installed: true,
        }),
        // No such package. A tombstone names its process, so this is how a platform binary
        // arrives — `/vendor/bin/hw/…`, `surfaceflinger` — and it belongs to the platform.
        None => Some(PackageOrigin {
            is_system_app: true,
            package_installed: false,
        }),
    }
}

fn captures_kind(config: &ConfigDocument, kind: CrashKind) -> bool {
    match kind {
        CrashKind::JavaException => config.global.capture_java,
        CrashKind::Anr => config.global.capture_anr,
        CrashKind::NativeCrash => config.global.capture_native,
        CrashKind::Wtf => config.global.capture_wtf,
    }
}

fn collector_enabled(config: &ConfigDocument, source: CollectorSource) -> bool {
    match source {
        CollectorSource::Events | CollectorSource::Dropbox => {
            config.global.capture_java
                || config.global.capture_anr
                || config.global.capture_native
                || config.global.capture_wtf
        }
        CollectorSource::CrashBuffer => config.global.capture_java,
        CollectorSource::Tombstone => config.global.capture_native,
        CollectorSource::AnrFile => config.global.capture_anr,
    }
}

fn collector_key(source: CollectorSource) -> String {
    format!("{source:?}")
}

/// For requests that put the name on a command line — starting an activity.
fn validate_package(package_name: &str) -> Result<(), WireError> {
    if is_safe_package_name(package_name) {
        Ok(())
    } else {
        Err(WireError::invalid_request("invalid package name"))
    }
}

/// For requests that only key settings by the name.
///
/// Separate because not everything that crashes is an app, and per-app settings have to work for
/// the rest: held to the package rules, a platform process's settings screen could not read its
/// own config back, and ignoring or muting one silently failed as an invalid request.
fn validate_settings_key(package_name: &str) -> Result<(), WireError> {
    if is_safe_settings_key(package_name) {
        Ok(())
    } else {
        Err(WireError::invalid_request("invalid settings key"))
    }
}

fn wire_dialog_status(
    status: &SettingsTakeoverStatus,
    error: Option<String>,
) -> DialogTakeoverStatus {
    DialogTakeoverStatus {
        requested: status.requested,
        effective: status.effective,
        anr_show_background_conflict: status.anr_override_needs_clear,
        unsupported_reason: if status.supported {
            error
        } else {
            Some("system dialog takeover requires Android 9 or newer".to_owned())
        },
    }
}

#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[derive(Default)]
struct EventBus {
    subscribers: Mutex<Vec<SyncSender<Event>>>,
}

impl EventBus {
    fn subscribe(&self) -> Receiver<Event> {
        let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    fn broadcast(&self, event: Event) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| match subscriber.try_send(event.clone()) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            });
        }
    }
}

fn unauthorized(error: &AuthError) -> WireError {
    WireError::new(ErrorCode::Unauthorized, error.to_string())
}

/// Starts a package's launcher activity, for the reopen action.
///
/// Resolved by intent rather than by component name so this does not have to ask
/// PackageManager which activity is the launcher one — `am` already does that.
/// Starts an app's launcher activity, for 重新打开.
///
/// The component is resolved first, then started by name. The obvious form —
/// `am start -a MAIN -c LAUNCHER -p <package>` — does not work here: `-p` is a *filter*
/// applied to an implicit intent, not a target, and this ROM answers it with
///
///   Error: Activity not started, unable to resolve Intent { act=…MAIN cat=[…LAUNCHER] pkg=… }
///
/// so the button reported success at the UI and did nothing on screen.
fn start_launcher_activity(package_name: &str, user_id: i32) -> bool {
    let Some(component) = resolve_launcher_component(package_name, user_id) else {
        warn!(package_name, user_id, "no launcher activity to reopen");
        return false;
    };
    run_am(&["start", "--user", &user_id.to_string(), "-n", &component])
}

/// The `package/activity` a launcher tap would start, or None if the app has no launcher
/// entry — a service-only or hidden package, which is a normal thing to have crashed.
///
/// `resolve-activity --brief` prints the resolve dump first and the component last, in the
/// relative `pkg/.Class` form `am -n` accepts on this ROM.
fn resolve_launcher_component(package_name: &str, user_id: i32) -> Option<String> {
    let output = std::process::Command::new("cmd")
        .args([
            "package",
            "resolve-activity",
            "--brief",
            "--user",
            &user_id.to_string(),
            package_name,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .rfind(|line| line.contains('/') && !line.contains(char::is_whitespace))
        .map(str::to_owned)
}

/// Shows the crash alert, for `NotifyMode::Dialog`.
///
/// Started with `am` from this process rather than through the privileged bridge. The
/// bridge is a bare `app_process`: it has a `Context`, which is enough for
/// `NotificationManager`, but no registered application record — so ActivityManager
/// rejects an activity start from it with
///
///   Unable to find app for caller …IApplicationThread$Stub$Proxy (pid=-1) when starting
///
/// and 弹窗 mode appeared to do nothing at all. This process is root, which `am` is happy
/// to start activities for, including into another user.
///
/// Failure is deliberately only logged. The record is already stored by this point, so a
/// missing alert costs the notification, not the crash.
fn start_alert_activity(record_id: &str, user_id: i32) {
    run_am(&[
        "start",
        "--user",
        &user_id.to_string(),
        // The action the activity's own filter declares. An explicit component with a
        // non-matching action is refused on this ROM even though the component is right.
        "-a",
        "io.github.lingqiqi5211.crashcatcher.BRIDGE_ACTION",
        "-n",
        MANAGER_DETAIL_COMPONENT,
        "--es",
        "record_id",
        record_id,
    ]);
}

/// Runs `am` and reports whether it succeeded.
///
/// Failure is logged, never propagated: by the time either caller runs, the crash is
/// already stored, so a refused activity costs the alert rather than the record.
fn run_am(arguments: &[&str]) -> bool {
    match std::process::Command::new("am")
        .args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) => {
            warn!(%status, ?arguments, "am refused to start the activity");
            false
        }
        Err(error) => {
            warn!(%error, "could not run am");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cch_config::ConfigStore;
    use cch_model::{CrashSummary, Fingerprint, PayloadSource, SourceMask};

    struct FakeSettings;

    impl DialogSettings for FakeSettings {
        fn status(&self) -> Result<SettingsTakeoverStatus, String> {
            Ok(SettingsTakeoverStatus {
                supported: true,
                requested: false,
                effective: false,
                anr_show_background: false,
                anr_override_needs_clear: false,
            })
        }

        fn set_enabled(&self, enabled: bool) -> Result<SettingsTakeoverStatus, String> {
            Ok(SettingsTakeoverStatus {
                supported: true,
                requested: enabled,
                effective: enabled,
                anr_show_background: false,
                anr_override_needs_clear: false,
            })
        }

        fn dropbox_tag_enabled(&self, _tag: &str) -> Result<bool, String> {
            Ok(true)
        }
    }

    fn test_core() -> (tempfile::TempDir, Arc<DaemonCore>) {
        core_with_packages(PackageIndex::default())
    }

    fn core_with_packages(packages: PackageIndex) -> (tempfile::TempDir, Arc<DaemonCore>) {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open_in_memory(directory.path()).expect("store"));
        let core = DaemonCore::new(
            store,
            ConfigStore::new(directory.path().join("config.json")),
            packages,
            Arc::new(FakeSettings),
            BridgeBroker::new(),
            test_runtime(directory.path()),
        );
        (directory, core)
    }

    /// Records what it was asked for, so a test can check the switch actually reached something.
    #[derive(Default)]
    struct RecordedLevel(Mutex<Vec<bool>>);

    impl LogLevelControl for RecordedLevel {
        fn set_debug(&self, debug: bool) {
            if let Ok(mut calls) = self.0.lock() {
                calls.push(debug);
            }
        }
    }

    fn test_runtime(state_dir: &std::path::Path) -> DaemonRuntime {
        DaemonRuntime {
            state_dir: state_dir.to_path_buf(),
            android_sdk: 34,
            log_control: Arc::new(RecordedLevel::default()),
        }
    }

    /// An index holding one ordinary app, in `packages.list` shape.
    fn one_app_index() -> PackageIndex {
        PackageIndex::build(
            "com.example 10123 0 /data/user/0/com.example default none 0 1",
            &Default::default(),
            &Default::default(),
        )
        .expect("index")
    }

    fn record_named(package: &str, process: &str) -> CrashRecord {
        CrashRecord {
            kind: CrashKind::NativeCrash,
            package_name: package.to_owned(),
            process_name: process.to_owned(),
            user_id: 0,
            pid: 42,
            // Now, not a fixed epoch: `ingest` sweeps on the retention window straight after
            // inserting, so a record dated 1970 is gone before anything can read it back.
            happened_at_ms: now_ms(),
            app_version_name: None,
            app_version_code: None,
            is_system_app: false,
            package_installed: true,
            is_foreground: None,
            self_handled: false,
            dropped_count: None,
            sources: SourceMask::TOMBSTONE,
            summary: CrashSummary::new(Some("SIGSEGV".to_owned()), None),
            fingerprint: Fingerprint::from_raw_frames(CrashKind::NativeCrash, "SIGSEGV", &[]),
            payload: PayloadSource::Inline(b"boom".to_vec()),
        }
    }

    #[test]
    fn handshake_refuses_a_different_protocol() {
        let (_directory, core) = test_core();
        let response = core.dispatch(RequestEnvelope {
            seq: 7,
            request: Request::Handshake {
                protocol_version: PROTOCOL_VERSION + 1,
                client_version: "test".to_owned(),
            },
        });
        assert!(matches!(
            response.err,
            Some(WireError {
                code: ErrorCode::VersionMismatch,
                ..
            })
        ));
    }

    #[test]
    fn ingest_broadcasts_and_persists() {
        let (_directory, core) = test_core();
        let events = core.subscribe();
        let record = CrashRecord {
            kind: CrashKind::JavaException,
            package_name: "com.example".to_owned(),
            process_name: "com.example".to_owned(),
            user_id: 0,
            pid: 42,
            happened_at_ms: 1_000,
            app_version_name: None,
            app_version_code: None,
            is_system_app: false,
            package_installed: true,
            is_foreground: Some(true),
            self_handled: false,
            dropped_count: None,
            sources: SourceMask::EVENTS,
            summary: CrashSummary::new(Some("java.lang.Error".to_owned()), None),
            fingerprint: Fingerprint::from_raw_frames(
                CrashKind::JavaException,
                "java.lang.Error",
                &["com.example.Main.fail".to_owned()],
            ),
            payload: PayloadSource::Inline(b"boom".to_vec()),
        };
        let inserted = core.ingest(record).expect("ingest").expect("stored");
        assert_eq!(inserted.group.occurrence, 1);
        assert!(matches!(events.try_recv(), Ok(Event::CrashRecorded { .. })));
    }

    /// A tombstone names its process, so platform binaries arrive with a path where a package
    /// name belongs. They used to be stored as ordinary apps — nothing resolved them, so
    /// `is_system_app` stayed false and they went straight past `include_system_apps`, which is
    /// what made that setting look like it did nothing.
    #[test]
    fn a_platform_process_is_not_recorded_while_system_apps_are_off() {
        let (_directory, core) = core_with_packages(one_app_index());
        let stored = core
            .ingest(record_named(
                "/vendor/bin/hw/android.hardware.audio.service_64",
                "/vendor/bin/hw/android.hardware.audio.service_64",
            ))
            .expect("ingest");
        assert!(stored.is_none(), "a platform process is not an app");
    }

    #[test]
    fn a_platform_process_is_recorded_as_one_when_system_apps_are_on() {
        let (_directory, core) = core_with_packages(one_app_index());
        core.update_config(|document| document.global.include_system_apps = true)
            .expect("enables system apps");

        let inserted = core
            .ingest(record_named("surfaceflinger", "surfaceflinger"))
            .expect("ingest")
            .expect("stored");
        assert!(!inserted.group.package_installed, "no package to install");
        assert!(inserted.group.is_system_app, "belongs to the platform");
    }

    #[test]
    fn an_installed_app_is_still_an_app() {
        let (_directory, core) = core_with_packages(one_app_index());
        let inserted = core
            .ingest(record_named("com.example", "com.example"))
            .expect("ingest")
            .expect("stored");
        assert!(inserted.group.package_installed);
        assert!(!inserted.group.is_system_app, "not under /system");
    }

    /// The debug switch has to reach the running process. Persisting it and waiting for a
    /// restart would lose whatever the user turned it on to capture.
    #[test]
    fn turning_on_debug_logging_takes_effect_immediately() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open_in_memory(directory.path()).expect("store"));
        let recorded = Arc::new(RecordedLevel::default());
        let core = DaemonCore::new(
            store,
            ConfigStore::new(directory.path().join("config.json")),
            PackageIndex::default(),
            Arc::new(FakeSettings),
            BridgeBroker::new(),
            DaemonRuntime {
                state_dir: directory.path().to_path_buf(),
                android_sdk: 34,
                log_control: Arc::clone(&recorded) as Arc<dyn LogLevelControl>,
            },
        );

        let response = core.dispatch(RequestEnvelope {
            seq: 1,
            request: Request::SetGlobalConfig {
                patch: cch_config::GlobalConfigPatch {
                    debug_logging: Some(true),
                    ..Default::default()
                },
            },
        });
        assert!(response.err.is_none(), "{:?}", response.err);
        assert_eq!(recorded.0.lock().expect("calls").as_slice(), &[true]);

        // And a restart has to come back at the level that was chosen, not at info.
        core.apply_log_level().expect("applies");
        assert_eq!(recorded.0.lock().expect("calls").as_slice(), &[true, true]);
    }

    /// The status is what a diagnostics page reads; these are the fields that say why something
    /// is not working rather than that it is not.
    #[test]
    fn the_status_reports_what_a_diagnosis_needs() {
        let (_directory, core) = core_with_packages(one_app_index());

        let response = core.dispatch(RequestEnvelope {
            seq: 1,
            request: Request::ModuleStatus,
        });
        let Some(cch_wire::Response::ModuleStatus { status }) = response.ok else {
            panic!("expected a status, got {:?}", response);
        };

        assert_eq!(status.runtime.android_sdk, 34);
        assert_eq!(status.runtime.pid, std::process::id());
        assert_eq!(status.runtime.package_index.package_count, 1);
        assert!(
            !status.runtime.package_index.system_flags_known,
            "built without PackageManager, and the page has to be able to say so"
        );
        assert!(!status.runtime.bridge.connected);
        assert_eq!(
            status.runtime.store_schema_version,
            cch_store::SCHEMA_VERSION
        );
        assert!(!status.runtime.debug_logging);
    }

    #[test]
    fn a_recovered_collector_clears_only_its_error() {
        let (_directory, core) = test_core();
        core.mark_collector_error(CollectorSource::Events, "stream closed");
        core.mark_collector_error(CollectorSource::CrashBuffer, "stream closed");

        core.clear_collector_error(CollectorSource::Events);

        let status = core.module_status().expect("module status");
        let events = status
            .collectors
            .iter()
            .find(|health| health.source == CollectorSource::Events)
            .expect("events health");
        let crash = status
            .collectors
            .iter()
            .find(|health| health.source == CollectorSource::CrashBuffer)
            .expect("crash health");
        assert_eq!(events.detail, None);
        assert_eq!(crash.detail.as_deref(), Some("stream closed"));
        assert!(!events.ever_received);
    }

    /// Per-app settings have to work for a platform process too — it is the thing most worth
    /// silencing. Keyed by its path, every one of these used to come back as an invalid request,
    /// so the settings screen could not read its config, and ignoring or muting did nothing.
    #[test]
    fn a_platform_process_can_be_configured_but_not_launched() {
        let (_directory, core) = test_core();
        let process = "/vendor/bin/hw/android.hardware.audio.service_64".to_owned();

        let response = core.dispatch(RequestEnvelope {
            seq: 1,
            request: Request::GetAppConfig {
                package_name: process.clone(),
            },
        });
        assert!(response.err.is_none(), "{:?}", response.err);

        let response = core.dispatch(RequestEnvelope {
            seq: 2,
            request: Request::MuteApp {
                package_name: process.clone(),
                scope: MuteScope::UntilUnlock,
            },
        });
        assert!(response.err.is_none(), "{:?}", response.err);

        // Starting an activity puts the name on a command line, so that one stays strict — and
        // there is no launcher activity for a HAL anyway.
        let response = core.dispatch(RequestEnvelope {
            seq: 3,
            request: Request::ReopenApp {
                package_name: process,
                user_id: 0,
            },
        });
        assert!(matches!(
            response.err,
            Some(WireError {
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
    }

    /// The guard against the failure mode that would be far worse than over-recording: an
    /// unreadable `packages.list` misses on every lookup, and treating that as "none of these
    /// are apps" would drop every crash on the device while the setting is off.
    #[test]
    fn an_unloaded_package_index_records_everything_rather_than_nothing() {
        let (_directory, core) = core_with_packages(PackageIndex::default());
        let inserted = core
            .ingest(record_named("com.example", "com.example"))
            .expect("ingest")
            .expect("stored");
        assert!(inserted.group.package_installed);
    }
}
