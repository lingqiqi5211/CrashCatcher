//! CrashCatcher's root daemon.

#![deny(unsafe_op_in_unsafe_fn)]

mod bridge_broker;
mod collectors;
mod core;
mod diagnostics;
mod packages;
mod server;
mod transport;

pub use bridge_broker::BridgeBroker;
pub use collectors::{CollectorRuntime, start_collectors};
pub use core::{
    DaemonCore, DaemonRuntime, DialogSettings, LogLevelControl, RuntimeDialogSettings, now_ms,
};
pub use diagnostics::{DEFAULT_LOG_BYTES, MAX_LOG_BYTES};
pub use packages::{PackageIndexError, load_package_index};
pub use server::{DaemonServers, ServerError};
pub use transport::{read_json_frame, write_json_frame};
