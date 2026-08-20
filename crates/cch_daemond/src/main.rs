use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use cch_auth::ManagerPin;
use cch_bridge_supervisor::{BridgeProcessSpec, BridgeSupervisor, RestartPolicy};
use cch_config::{CONFIG_FILE_NAME, ConfigStore};
use cch_daemond::{
    BridgeBroker, DaemonCore, DaemonRuntime, DaemonServers, LogLevelControl, PackageIndexError,
    RollingLog, RuntimeDialogSettings, ServerError, load_package_index, start_collectors,
};
use cch_store::Store;
use cch_wire::BRIDGE_SOCKET_NAME;
use thiserror::Error;
use tracing::{info, warn};
use tracing_subscriber::{
    filter::LevelFilter, layer::SubscriberExt, registry::Registry, reload, util::SubscriberInitExt,
};

const MANAGER_PACKAGE: &str = "io.github.lingqiqi5211.crashcatcher";

fn main() -> Result<(), MainError> {
    // Arguments first: the log file lives under the state directory, so there is nowhere to
    // write until they are parsed. A failure before this reaches stderr, which the launcher
    // redirects.
    let arguments = Arguments::parse(env::args().skip(1))?;
    fs::create_dir_all(&arguments.state_dir)?;
    let log_control = install_logging(&arguments.state_dir);
    let store = Arc::new(Store::open(arguments.state_dir.join("store"))?);
    let config_store = ConfigStore::new(arguments.state_dir.join(CONFIG_FILE_NAME));
    let packages = load_package_index()?;
    let pin = ManagerPin::load(&arguments.manager_pin)?;
    let android_sdk = arguments.android_sdk.unwrap_or_else(read_android_sdk);
    let bridge = BridgeBroker::new();
    let core = DaemonCore::new(
        store,
        config_store,
        packages,
        Arc::new(RuntimeDialogSettings::new(android_sdk)),
        bridge,
        DaemonRuntime {
            state_dir: arguments.state_dir.clone(),
            android_sdk,
            log_control,
        },
    );
    if let Err(error) = core.apply_log_level() {
        warn!(%error, "could not apply the stored logging level");
    }
    core.clear_volatile_mutes().map_err(MainError::Wire)?;
    complete_package_index(Arc::clone(&core));

    let servers = DaemonServers::start(Arc::clone(&core), pin)?;
    let bridge_spec = BridgeProcessSpec::android_defaults(
        arguments.module_dir.join("dex/cch_bridge.dex"),
        BRIDGE_SOCKET_NAME,
        MANAGER_PACKAGE,
    );
    let _bridge_supervisor = BridgeSupervisor::start(bridge_spec, RestartPolicy::default())?;
    let _collectors = start_collectors(core);

    if let Some(parent) = arguments.ready_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.ready_file, b"ready\n")?;
    info!(sdk = android_sdk, "crashcatcher daemon ready");
    servers.wait()?;
    Ok(())
}

/// Installs the log subscriber and returns the handle that can change its level later.
///
/// A reload layer, not a fixed filter: the debug switch exists to capture something that is
/// happening now, and a level that only applied on restart would lose it.
///
/// Falls back to stdout if the log file cannot be opened, which keeps the launcher's redirect as
/// the last resort rather than losing the output entirely.
fn install_logging(state_dir: &Path) -> Arc<dyn LogLevelControl> {
    let (filter, handle) = reload::Layer::new(LevelFilter::INFO);
    let registry = tracing_subscriber::registry().with(filter);

    match RollingLog::open(state_dir.join("logs")) {
        Ok(sink) => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    // `Mutex<W>` is what tracing-subscriber accepts as a shared writer, and the
                    // lock is also what keeps two threads' records from interleaving mid-line.
                    .with_writer(Mutex::new(sink)),
            )
            .init(),
        Err(error) => {
            registry
                .with(tracing_subscriber::fmt::layer().with_ansi(false))
                .init();
            warn!(%error, "could not open the log file; logging to stdout");
        }
    }

    Arc::new(ReloadableLevel(handle))
}

struct ReloadableLevel(reload::Handle<LevelFilter, Registry>);

impl LogLevelControl for ReloadableLevel {
    fn set_debug(&self, debug: bool) {
        let level = if debug {
            LevelFilter::DEBUG
        } else {
            LevelFilter::INFO
        };
        // Failure here means the subscriber is gone, which cannot happen while this process is
        // running — and a log level is not worth taking the daemon down over either way.
        if self.0.modify(|current| *current = level).is_err() {
            warn!("could not change the log level");
        }
    }
}

/// Rebuilds the package index once PackageManager is answering.
///
/// The daemon starts from the module's boot script, well before `system_server` is up, so the
/// first index is built from `packages.list` alone: `cmd package` fails, leaving no APK paths
/// and no system flags. Every app then looks like a third-party one, which is what let platform
/// crashes into a list filtered to exclude them — and without this it would stay that way until
/// something else happened to reload the index.
///
/// On its own thread, and giving up after [`INDEX_RETRY_LIMIT`] tries: this is an improvement
/// on the first index, not a prerequisite for collecting.
fn complete_package_index(core: Arc<DaemonCore>) {
    if core.package_flags_known() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("ct-package-index".to_owned())
        .spawn(move || {
            for _ in 0..INDEX_RETRY_LIMIT {
                std::thread::sleep(INDEX_RETRY_INTERVAL);
                let Ok(index) = load_package_index() else {
                    continue;
                };
                if !index.system_flags_known() {
                    continue;
                }
                let packages = index.entries().len();
                if let Err(error) = core.replace_packages(index) {
                    warn!(%error, "could not install the completed package index");
                    return;
                }
                info!(packages, "package index completed from PackageManager");
                return;
            }
            warn!("PackageManager never answered; system apps stay identified by APK path only");
        });
    if let Err(error) = spawned {
        warn!(%error, "could not start the package-index retry");
    }
}

/// Long enough that the retries span a slow boot, short enough to be done before a user opens
/// the manager: 60 × 2s covers two minutes.
const INDEX_RETRY_LIMIT: u32 = 60;
const INDEX_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug)]
struct Arguments {
    state_dir: PathBuf,
    module_dir: PathBuf,
    manager_pin: PathBuf,
    ready_file: PathBuf,
    android_sdk: Option<u32>,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, MainError> {
        let mut state_dir = None;
        let mut module_dir = None;
        let mut manager_pin = None;
        let mut ready_file = None;
        let mut android_sdk = None;
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| MainError::Arguments(format!("missing value after {argument}")))?;
            match argument.as_str() {
                "--state-dir" => state_dir = Some(PathBuf::from(value)),
                "--module-dir" => module_dir = Some(PathBuf::from(value)),
                "--manager-pin" => manager_pin = Some(PathBuf::from(value)),
                "--ready-file" => ready_file = Some(PathBuf::from(value)),
                "--android-sdk" => {
                    android_sdk = Some(value.parse().map_err(|_| {
                        MainError::Arguments("--android-sdk must be an integer".to_owned())
                    })?)
                }
                _ => return Err(MainError::Arguments(format!("unknown argument {argument}"))),
            }
        }
        let state_dir =
            state_dir.ok_or_else(|| MainError::Arguments("--state-dir is required".to_owned()))?;
        let module_dir = module_dir
            .ok_or_else(|| MainError::Arguments("--module-dir is required".to_owned()))?;
        let manager_pin = manager_pin
            .ok_or_else(|| MainError::Arguments("--manager-pin is required".to_owned()))?;
        let ready_file = ready_file.unwrap_or_else(|| state_dir.join("runtime/ready"));
        Ok(Self {
            state_dir,
            module_dir,
            manager_pin,
            ready_file,
            android_sdk,
        })
    }
}

fn read_android_sdk() -> u32 {
    Command::new("getprop")
        .arg("ro.build.version.sdk")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Error)]
enum MainError {
    #[error("invalid arguments: {0}")]
    Arguments(String),
    #[error("io failed: {0}")]
    Io(#[from] io::Error),
    #[error("storage failed: {0}")]
    Store(#[from] cch_store::StoreError),
    #[error("{0}")]
    Packages(#[from] PackageIndexError),
    #[error("manager authentication setup failed: {0}")]
    Auth(#[from] cch_auth::AuthError),
    #[error("bridge supervisor failed: {0}")]
    Bridge(#[from] cch_bridge_supervisor::SupervisorError),
    #[error("server failed: {0}")]
    Server(#[from] ServerError),
    #[error("daemon setup failed: {0}")]
    Wire(cch_wire::WireError),
}
