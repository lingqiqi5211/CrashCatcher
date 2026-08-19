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
use cch_packages::{PackageIndex, is_safe_package_name};
use cch_settings::{AndroidSettings, DialogTakeoverStatus as SettingsTakeoverStatus};
use cch_store::{Inserted, Store};
use cch_wire::{
    AppConfigResult, AppEntry, BridgeAction, CollectorHealth, CollectorSource,
    DialogTakeoverResult, DialogTakeoverStatus, ErrorCode, Event, ExportFormat, ExportRedaction,
    GlobalConfigResult, MAX_PAYLOAD_CHUNK_BYTES, ModuleStatus, MuteResult, NotificationAction,
    NotificationSpec, PROTOCOL_VERSION, PayloadChunk, PayloadOpened, Request, RequestEnvelope,
    Response, ResponseEnvelope, WireError,
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
}

impl DaemonCore {
    #[must_use]
    pub fn new(
        store: Arc<Store>,
        config_store: ConfigStore,
        packages: PackageIndex,
        dialog_settings: Arc<dyn DialogSettings>,
        bridge: Arc<BridgeBroker>,
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
        })
    }

    #[must_use]
    pub fn bridge(&self) -> &Arc<BridgeBroker> {
        &self.bridge
    }

    pub fn subscribe(&self) -> Receiver<Event> {
        self.events.subscribe()
    }

    pub fn replace_packages(&self, packages: PackageIndex) -> Result<(), WireError> {
        *self
            .packages
            .write()
            .map_err(|_| WireError::internal("package index lock poisoned"))? = packages;
        Ok(())
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
            Ok(packages) => self.replace_packages(packages)?,
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

    pub fn clear_volatile_mutes(&self) -> Result<(), WireError> {
        self.volatile_mutes
            .lock()
            .map_err(|_| WireError::internal("mute lock poisoned"))?
            .clear();
        self.store
            .clear_all_mutes()
            .map(|_| ())
            .map_err(|error| error.to_wire())
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
                    daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
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
                self.events.broadcast(Event::ConfigChanged);
                Ok(Response::GlobalConfig {
                    result: Box::new(GlobalConfigResult {
                        adjusted: requested != stored.global,
                        config: stored.global,
                    }),
                })
            }
            Request::GetAppConfig { package_name } => {
                validate_package(&package_name)?;
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
                validate_package(&package_name)?;
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
                query,
                limit,
            } => self.list_apps(include_system_apps, query.as_deref(), limit),
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
                validate_package(&package_name)?;
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
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: PROTOCOL_VERSION,
            uptime_ms,
            collectors,
            bridge_connected: self.bridge.is_connected(),
            dialog_takeover,
            storage: self
                .store
                .storage_status()
                .map_err(|error| error.to_wire())?,
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
        query: Option<&str>,
        limit: u32,
    ) -> Result<Response, WireError> {
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let config = self.load_config()?;
        let apps = self
            .store
            .package_rollups(include_system_apps, limit)
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

    fn enrich_record(&self, record: &mut CrashRecord) -> Result<(), WireError> {
        if let Some(package) = self
            .bridge
            .cached_package(&record.package_name, record.user_id)
        {
            record.app_version_name = package.version_name;
            record.app_version_code = package.version_code;
            record.is_system_app = package.is_system_app;
            return Ok(());
        }
        let packages = self
            .packages
            .read()
            .map_err(|_| WireError::internal("package index lock poisoned"))?;
        if let Some(package) = packages.by_name(&record.package_name) {
            record.user_id = i32::try_from(package.user_id()).unwrap_or(i32::MAX);
            record.app_version_code = record.app_version_code.or(package.version_code);
            record.is_system_app = package.code_path.as_ref().is_some_and(|path| {
                let path = path.to_string_lossy();
                path.starts_with("/system/")
                    || path.starts_with("/product/")
                    || path.starts_with("/vendor/")
                    || path.starts_with("/apex/")
            });
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

fn validate_package(package_name: &str) -> Result<(), WireError> {
    if is_safe_package_name(package_name) {
        Ok(())
    } else {
        Err(WireError::invalid_request("invalid package name"))
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
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open_in_memory(directory.path()).expect("store"));
        let core = DaemonCore::new(
            store,
            ConfigStore::new(directory.path().join("config.json")),
            PackageIndex::default(),
            Arc::new(FakeSettings),
            BridgeBroker::new(),
        );
        (directory, core)
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
}
