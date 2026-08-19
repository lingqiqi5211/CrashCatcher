//! Narrow adapter around Android's `settings` command.

#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use std::{io, process::Command};
use thiserror::Error;
const HIDE: &str = "hide_error_dialogs";
const ANR_BG: &str = "anr_show_background";
const MIN_SDK: u32 = 28;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
pub trait SettingsCommand {
    fn run(&self, args: &[&str]) -> Result<CommandOutput, io::Error>;
}
#[derive(Debug, Clone)]
pub struct ProcessSettingsCommand {
    executable: String,
}
impl Default for ProcessSettingsCommand {
    fn default() -> Self {
        Self {
            executable: "settings".into(),
        }
    }
}
impl SettingsCommand for ProcessSettingsCommand {
    fn run(&self, args: &[&str]) -> Result<CommandOutput, io::Error> {
        let o = Command::new(&self.executable).args(args).output()?;
        Ok(CommandOutput {
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogTakeoverStatus {
    pub supported: bool,
    pub requested: bool,
    pub effective: bool,
    pub anr_show_background: bool,
    pub anr_override_needs_clear: bool,
}
pub struct AndroidSettings<C = ProcessSettingsCommand> {
    sdk: u32,
    command: C,
}
impl AndroidSettings<ProcessSettingsCommand> {
    #[must_use]
    pub fn new(sdk: u32) -> Self {
        Self {
            sdk,
            command: ProcessSettingsCommand::default(),
        }
    }
}
impl<C: SettingsCommand> AndroidSettings<C> {
    #[must_use]
    pub const fn with_command(sdk: u32, command: C) -> Self {
        Self { sdk, command }
    }
    pub fn dialog_takeover_status(&self) -> Result<DialogTakeoverStatus, SettingsError> {
        let supported = self.sdk >= MIN_SDK;
        let requested = self.get_i64("global", HIDE)?.unwrap_or(0) != 0;
        let bg = self.get_i64("secure", ANR_BG)?.unwrap_or(0) != 0;
        Ok(DialogTakeoverStatus {
            supported,
            requested,
            effective: supported && requested && !bg,
            anr_show_background: bg,
            anr_override_needs_clear: requested && bg,
        })
    }
    pub fn set_dialog_takeover(
        &self,
        enabled: bool,
    ) -> Result<DialogTakeoverStatus, SettingsError> {
        if enabled && self.sdk < MIN_SDK {
            return Err(SettingsError::UnsupportedSdk(self.sdk));
        }
        if enabled {
            self.put_i64("secure", ANR_BG, 0)?
        }
        self.put_i64("global", HIDE, i64::from(enabled))?;
        self.dialog_takeover_status()
    }
    pub fn dropbox_tag_enabled(&self, tag: &str) -> Result<bool, SettingsError> {
        validate_tag(tag)?;
        let v = self.get("global", &format!("dropbox:{tag}"))?;
        Ok(!v
            .as_deref()
            .is_some_and(|x| x.eq_ignore_ascii_case("disabled")))
    }
    pub fn get(&self, namespace: &str, key: &str) -> Result<Option<String>, SettingsError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let o = self.exec(&["get", namespace, key])?;
        let v = o.stdout.trim();
        if v.is_empty() || v.eq_ignore_ascii_case("null") {
            Ok(None)
        } else {
            Ok(Some(v.into()))
        }
    }
    fn get_i64(&self, n: &str, k: &str) -> Result<Option<i64>, SettingsError> {
        self.get(n, k)?
            .map(|v| {
                v.parse().map_err(|_| SettingsError::InvalidValue {
                    namespace: n.into(),
                    key: k.into(),
                    value: v,
                })
            })
            .transpose()
    }
    fn put_i64(&self, n: &str, k: &str, v: i64) -> Result<(), SettingsError> {
        validate_namespace(n)?;
        validate_key(k)?;
        self.exec(&["put", n, k, &v.to_string()])?;
        Ok(())
    }
    fn exec(&self, args: &[&str]) -> Result<CommandOutput, SettingsError> {
        let o = self.command.run(args).map_err(SettingsError::CommandIo)?;
        if o.success {
            Ok(o)
        } else {
            Err(SettingsError::CommandFailed {
                args: args.iter().map(|v| (*v).into()).collect(),
                stderr: o.stderr.trim().into(),
            })
        }
    }
}
fn validate_namespace(v: &str) -> Result<(), SettingsError> {
    if matches!(v, "global" | "secure") {
        Ok(())
    } else {
        Err(SettingsError::InvalidName("namespace"))
    }
}
fn validate_key(v: &str) -> Result<(), SettingsError> {
    if !v.is_empty()
        && v.len() <= 160
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.'))
    {
        Ok(())
    } else {
        Err(SettingsError::InvalidName("key"))
    }
}
fn validate_tag(v: &str) -> Result<(), SettingsError> {
    if !v.is_empty() && v.len() <= 100 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Ok(())
    } else {
        Err(SettingsError::InvalidName("dropbox tag"))
    }
}
#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("system dialog takeover requires Android 9+, SDK is {0}")]
    UnsupportedSdk(u32),
    #[error("invalid settings {0}")]
    InvalidName(&'static str),
    #[error("settings {namespace} {key} returned non-integer {value:?}")]
    InvalidValue {
        namespace: String,
        key: String,
        value: String,
    },
    #[error("failed to execute settings: {0}")]
    CommandIo(#[source] io::Error),
    #[error("settings command {args:?} failed: {stderr}")]
    CommandFailed { args: Vec<String>, stderr: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};
    #[derive(Default)]
    struct Fake {
        v: RefCell<HashMap<(String, String), String>>,
    }
    impl Fake {
        fn set(&self, n: &str, k: &str, v: &str) {
            self.v.borrow_mut().insert((n.into(), k.into()), v.into());
        }
    }
    impl SettingsCommand for Fake {
        fn run(&self, a: &[&str]) -> Result<CommandOutput, io::Error> {
            let mut v = self.v.borrow_mut();
            let stdout = match a {
                ["get", n, k] => v
                    .get(&(n.to_string(), k.to_string()))
                    .cloned()
                    .unwrap_or_else(|| "null".into()),
                ["put", n, k, x] => {
                    v.insert((n.to_string(), k.to_string()), x.to_string());
                    String::new()
                }
                _ => return Err(io::Error::other("bad fake")),
            };
            Ok(CommandOutput {
                success: true,
                stdout,
                stderr: String::new(),
            })
        }
    }
    #[test]
    fn takeover_clears_override() {
        let f = Fake::default();
        f.set("secure", ANR_BG, "1");
        let s = AndroidSettings::with_command(35, f);
        let status = s.set_dialog_takeover(true).unwrap();
        assert!(status.effective);
        assert!(!status.anr_show_background);
    }
    #[test]
    fn old_android_refuses() {
        assert!(matches!(
            AndroidSettings::with_command(27, Fake::default()).set_dialog_takeover(true),
            Err(SettingsError::UnsupportedSdk(27))
        ));
    }
    #[test]
    fn dropbox_defaults_enabled() {
        let s = AndroidSettings::with_command(35, Fake::default());
        assert!(s.dropbox_tag_enabled("data_app_crash").unwrap());
        s.command
            .set("global", "dropbox:data_app_crash", "disabled");
        assert!(!s.dropbox_tag_enabled("data_app_crash").unwrap());
    }
}
