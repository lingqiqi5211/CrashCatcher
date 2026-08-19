//! Configuration: the model, partial-update patches, and the JSON document store.
//!
//! The daemon is the single source of truth for configuration. The manager app
//! never touches `SharedPreferences` for anything that affects collection — it
//! reads and writes through RPC — so there is exactly one place a setting can
//! live and exactly one place it can be wrong.

#![forbid(unsafe_code)]

mod model;
mod patch;
mod store;

pub use model::{
    AppConfig, ConfigDocument, GlobalConfig, MuteScope, NotifyMode, RetentionPolicy,
    CONFIG_SCHEMA_VERSION,
};
pub use patch::{AppConfigPatch, GlobalConfigPatch, RetentionPatch};
pub use store::{ConfigError, ConfigStore};

/// File name of the config document inside the persistent directory.
pub const CONFIG_FILE_NAME: &str = "config.json";
