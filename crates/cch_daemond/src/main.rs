use std::{env, fs, io, path::PathBuf, process::Command, sync::Arc};

use cch_auth::ManagerPin;
use cch_bridge_supervisor::{BridgeProcessSpec, BridgeSupervisor, RestartPolicy};
use cch_config::{CONFIG_FILE_NAME, ConfigStore};
use cch_daemond::{
    BridgeBroker, DaemonCore, DaemonServers, PackageIndexError, RuntimeDialogSettings, ServerError,
    load_package_index, start_collectors,
};
use cch_store::Store;
use cch_wire::BRIDGE_SOCKET_NAME;
use thiserror::Error;
use tracing::info;

const MANAGER_PACKAGE: &str = "io.github.lingqiqi5211.crashcatcher";

fn main() -> Result<(), MainError> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let arguments = Arguments::parse(env::args().skip(1))?;
    fs::create_dir_all(&arguments.state_dir)?;
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
    );
    core.clear_volatile_mutes().map_err(MainError::Wire)?;

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
