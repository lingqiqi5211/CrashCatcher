use std::{
    fs, io,
    path::{Path, PathBuf},
};

use tracing::warn;

use crate::ConfigDocument;

/// Failures reading or writing the config document.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config io failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("config at {path} is not valid JSON: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Reads and writes the single JSON config document.
///
/// A whole-file document rather than a table: it is small, always read in full,
/// and being human-readable and diffable is worth more here than partial-write
/// support. Writes go through a temp file and a rename so a reader never sees a
/// half-written config, and a power loss cannot leave the daemon unbootable.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the document, treating a missing file as "all defaults".
    ///
    /// First run is not an error: the packaged module ships no config and the
    /// daemon writes one on demand.
    pub fn load(&self) -> Result<ConfigDocument, ConfigError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(ConfigDocument::default());
            }
            Err(source) => {
                return Err(ConfigError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        serde_json::from_str::<ConfigDocument>(&text)
            .map(ConfigDocument::normalized)
            .map_err(|source| ConfigError::Parse {
                path: self.path.clone(),
                source,
            })
    }

    /// Loads the document, surviving a corrupt file.
    ///
    /// Moves the unreadable file aside instead of overwriting it — the user's
    /// settings may be recoverable by hand, and silently discarding them is worse
    /// than one warning line. Refusing to boot would be worse still: the config is
    /// not a trust boundary, so this path fails open.
    #[must_use]
    pub fn load_or_recover(&self) -> ConfigDocument {
        match self.load() {
            Ok(document) => document,
            Err(error) => {
                warn!(
                    path = %self.path.display(),
                    %error,
                    "config unreadable; moving it aside and continuing with defaults"
                );
                self.quarantine();
                ConfigDocument::default()
            }
        }
    }

    fn quarantine(&self) {
        let mut target = self.path.clone();
        let name = self.path.file_name().map_or_else(
            || "config.json".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        target.set_file_name(format!("{name}.corrupt"));
        if let Err(error) = fs::rename(&self.path, &target) {
            warn!(
                from = %self.path.display(),
                to = %target.display(),
                %error,
                "could not preserve the corrupt config"
            );
        }
    }

    /// Writes the document atomically.
    pub fn save(&self, document: &ConfigDocument) -> Result<(), ConfigError> {
        let normalized = document.clone().normalized();
        let mut json = serde_json::to_string_pretty(&normalized).map_err(ConfigError::Serialize)?;
        json.push('\n');

        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let temp = self.temp_path();
        write_then_sync(&temp, json.as_bytes())?;
        // Rename over the live file: readers see either the old or the new bytes,
        // never a mix.
        fs::rename(&temp, &self.path).map_err(|source| ConfigError::Io {
            path: self.path.clone(),
            source,
        })
    }

    /// Loads, mutates, saves, and hands back the stored result.
    ///
    /// Returning the post-save document matters for the RPC contract: the client
    /// sees the value that was actually persisted after clamping, not the value it
    /// asked for.
    pub fn update<F>(&self, mutate: F) -> Result<ConfigDocument, ConfigError>
    where
        F: FnOnce(&mut ConfigDocument),
    {
        let mut document = self.load_or_recover();
        mutate(&mut document);
        let document = document.normalized();
        self.save(&document)?;
        Ok(document)
    }

    fn temp_path(&self) -> PathBuf {
        let mut temp = self.path.clone();
        let name = self.path.file_name().map_or_else(
            || "config.json".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        temp.set_file_name(format!(".{name}.tmp"));
        temp
    }
}

fn write_then_sync(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    use std::io::Write;

    let mut file = fs::File::create(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    // Without this the rename can land before the bytes do, leaving an empty
    // config after an unclean shutdown.
    file.sync_all().map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    drop(file);

    restrict_permissions(path);
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    // The config lives under /data/adb and is root's business only.
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        warn!(path = %path.display(), %error, "could not restrict config permissions");
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, GlobalConfig, NotifyMode, RetentionPolicy};

    fn store_in(dir: &tempfile::TempDir) -> ConfigStore {
        ConfigStore::new(dir.path().join("config.json"))
    }

    #[test]
    fn a_missing_file_loads_defaults_without_creating_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        assert_eq!(store.load().expect("loads"), ConfigDocument::default());
        assert!(!store.path().exists(), "reading must not write");
    }

    #[test]
    fn documents_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);

        let mut document = ConfigDocument::default();
        document.global.takeover_system_dialog = true;
        document.global.notify_mode = NotifyMode::Toast;
        document.apps.insert(
            "com.example.app".to_owned(),
            AppConfig {
                ignore: true,
                ..AppConfig::default()
            },
        );

        store.save(&document).expect("saves");
        assert_eq!(store.load().expect("loads"), document);
    }

    #[test]
    fn saving_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        store.save(&ConfigDocument::default()).expect("saves");

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "config.json")
            .collect();
        assert!(leftovers.is_empty(), "unexpected files: {leftovers:?}");
    }

    #[test]
    fn saving_twice_overwrites_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        store.save(&ConfigDocument::default()).expect("first save");

        let mut second = ConfigDocument::default();
        second.global.enabled = false;
        store.save(&second).expect("second save");
        assert!(!store.load().expect("loads").global.enabled);
    }

    #[test]
    fn saved_values_are_clamped_on_the_way_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);

        let document = ConfigDocument {
            global: GlobalConfig {
                retention: RetentionPolicy {
                    retention_days: 0,
                    ..RetentionPolicy::default()
                },
                ..GlobalConfig::default()
            },
            ..ConfigDocument::default()
        };
        store.save(&document).expect("saves");

        assert_eq!(
            store.load().expect("loads").global.retention.retention_days,
            RetentionPolicy::MIN_RETENTION_DAYS
        );
    }

    #[test]
    fn update_returns_what_was_actually_persisted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);

        let returned = store
            .update(|document| {
                document.global.retention.max_records_total = 1;
            })
            .expect("updates");

        // The caller asked for 1 and gets told the clamped truth.
        assert_eq!(
            returned.global.retention.max_records_total,
            RetentionPolicy::MIN_RECORDS_TOTAL
        );
        assert_eq!(store.load().expect("loads"), returned);
    }

    #[test]
    fn a_corrupt_file_is_preserved_and_defaults_are_used() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        fs::write(store.path(), b"{ this is not json").expect("write garbage");

        assert!(store.load().is_err(), "load must report the corruption");
        assert_eq!(store.load_or_recover(), ConfigDocument::default());

        let quarantined = dir.path().join("config.json.corrupt");
        assert!(quarantined.exists(), "the user's file must be kept");
        assert!(!store.path().exists());
    }

    #[test]
    fn recovery_can_then_write_a_fresh_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(&dir);
        fs::write(store.path(), b"garbage").expect("write garbage");

        let recovered = store
            .update(|document| document.global.enabled = false)
            .expect("update after recovery");
        assert!(!recovered.global.enabled);
        assert_eq!(store.load().expect("loads"), recovered);
    }

    #[test]
    fn parent_directories_are_created_on_demand() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ConfigStore::new(dir.path().join("nested/deeper/config.json"));
        store.save(&ConfigDocument::default()).expect("saves");
        assert!(store.path().exists());
    }
}
