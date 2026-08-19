//! Joins PackageManager's UID index with its authoritative APK code paths.

#![forbid(unsafe_code)]
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const ANDROID_UID_USER_RANGE: u32 = 100_000;

/// Whether an APK path is one of the read-only partitions the platform ships on.
///
/// Only a fallback for when PackageManager could not be asked directly, and a poor one: the
/// set of partitions keeps growing. `/system_ext` — where a current Android puts Settings,
/// Telecom and most of the priv-apps — was missing from an earlier version of this list, so
/// those crashes were filed as ordinary apps and showed up with "record system apps" off.
#[must_use]
pub fn looks_like_system_path(path: Option<&Path>) -> bool {
    let Some(path) = path else {
        return false;
    };
    let path = path.to_string_lossy();
    [
        "/system/",
        "/system_ext/",
        "/product/",
        "/vendor/",
        "/odm/",
        "/oem/",
        "/apex/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub uid: u32,
    pub debuggable: bool,
    pub data_dir: PathBuf,
    pub seinfo: String,
    pub gids: Vec<u32>,
    pub profileable_from_shell: Option<bool>,
    pub version_code: Option<i64>,
    pub code_path: Option<PathBuf>,
    /// Whether PackageManager considers this a system package.
    ///
    /// Taken from `cmd package list packages -s` where that could be read, since it is
    /// `FLAG_SYSTEM` itself rather than a guess about it. See [`looks_like_system_path`] for
    /// the fallback and why guessing was not good enough.
    pub is_system: bool,
}
impl PackageEntry {
    #[must_use]
    pub const fn user_id(&self) -> u32 {
        self.uid / ANDROID_UID_USER_RANGE
    }
    #[must_use]
    pub const fn app_id(&self) -> u32 {
        self.uid % ANDROID_UID_USER_RANGE
    }
    #[must_use]
    pub fn base_apk_path(&self) -> Option<PathBuf> {
        let p = self.code_path.as_ref()?;
        if p.extension().is_some_and(|x| x == "apk") {
            Some(p.clone())
        } else {
            Some(PathBuf::from(format!(
                "{}/base.apk",
                p.to_string_lossy().trim_end_matches('/')
            )))
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct PackageIndex {
    entries: Vec<PackageEntry>,
    by_uid: HashMap<u32, Vec<usize>>,
    by_name: HashMap<String, usize>,
    system_flags_known: bool,
}
impl PackageIndex {
    /// Joins `packages.list` with code paths taken from `packages.xml`.
    ///
    /// Only usable where that file is still text. Since Android 12 the platform
    /// writes it as Android Binary XML, so prefer [`Self::build`] with
    /// [`parse_pm_code_paths`].
    pub fn parse(list: &str, xml: &str) -> Result<Self, PackageError> {
        Self::build(list, &parse_code_paths(xml.as_bytes())?, &HashSet::new())
    }

    /// Joins `packages.list` with an already-resolved name → code path map.
    ///
    /// Split from the parsing so the code paths can come from whichever source is
    /// actually readable on the running device; a package missing from
    /// [`code_paths`] simply has no path rather than failing the whole index.
    ///
    /// `system_packages` is PackageManager's own answer to "which of these are system
    /// packages". An empty set means it could not be asked, and each entry falls back to
    /// [`looks_like_system_path`].
    pub fn build(
        list: &str,
        code_paths: &HashMap<String, PathBuf>,
        system_packages: &HashSet<String>,
    ) -> Result<Self, PackageError> {
        let mut entries = vec![];
        for (i, line) in list.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut e = parse_line(line, i + 1)?;
            e.code_path = code_paths.get(&e.name).cloned();
            e.is_system = if system_packages.is_empty() {
                looks_like_system_path(e.code_path.as_deref())
            } else {
                system_packages.contains(&e.name)
            };
            entries.push(e)
        }
        let mut index = Self {
            entries,
            by_uid: HashMap::new(),
            by_name: HashMap::new(),
            system_flags_known: !system_packages.is_empty(),
        };
        for (i, e) in index.entries.iter().enumerate() {
            index.by_uid.entry(e.uid).or_default().push(i);
            index.by_name.insert(e.name.clone(), i);
        }
        Ok(index)
    }
    pub fn by_uid(&self, uid: u32) -> impl Iterator<Item = &PackageEntry> {
        self.by_uid
            .get(&uid)
            .into_iter()
            .flatten()
            .filter_map(|i| self.entries.get(*i))
    }
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&PackageEntry> {
        self.by_name.get(name).and_then(|i| self.entries.get(*i))
    }
    #[must_use]
    pub fn entries(&self) -> &[PackageEntry] {
        &self.entries
    }
    /// Whether the index holds nothing, which means "not loaded" rather than "no packages".
    ///
    /// Callers that treat a lookup miss as proof a package is not installed have to check
    /// this first: an unreadable `packages.list` also produces misses, for everything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether [`PackageEntry::is_system`] came from PackageManager rather than from the path.
    ///
    /// False means the index was built before PackageManager was answering — which is the
    /// normal case for a daemon started from a boot script, and worth retrying for, since the
    /// path fallback has nothing to work with either when `cmd package` was the source of the
    /// paths too.
    #[must_use]
    pub fn system_flags_known(&self) -> bool {
        self.system_flags_known
    }

    /// Takes `previous`'s system flags for the packages it knew, if this index has none.
    ///
    /// A rebuild happens for reasons that have nothing to do with the flags — the manager was
    /// reinstalled, so its APK moved and authentication needs the new path — and it can land at a
    /// moment when `cmd package` is unavailable again. Replacing outright would then throw away a
    /// completed answer and leave every app looking third-party until the next reboot, because
    /// the retry that completed it has already finished.
    ///
    /// Packages `previous` did not have keep whatever their path implied: they were installed
    /// since, which is exactly the case a rebuild exists to pick up.
    pub fn inherit_system_flags(&mut self, previous: &Self) {
        if self.system_flags_known || !previous.system_flags_known {
            return;
        }
        for entry in &mut self.entries {
            if let Some(known) = previous
                .by_name
                .get(&entry.name)
                .and_then(|i| previous.entries.get(*i))
            {
                entry.is_system = known.is_system;
            }
        }
        self.system_flags_known = true;
    }
}

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("packages.list line {line}: expected at least six fields")]
    TooFewFields { line: usize },
    #[error("packages.list line {line}: invalid {field}")]
    InvalidField { line: usize, field: &'static str },
    #[error("packages.xml is malformed: {0}")]
    InvalidXml(String),
}
fn parse_line(line: &str, n: usize) -> Result<PackageEntry, PackageError> {
    let f: Vec<&str> = line.split_ascii_whitespace().collect();
    if f.len() < 6 {
        return Err(PackageError::TooFewFields { line: n });
    }
    let name = f[0];
    if name.is_empty() || name.contains('/') {
        return Err(PackageError::InvalidField {
            line: n,
            field: "package name",
        });
    }
    let gids = if f[5] == "none" {
        vec![]
    } else {
        f[5].split(',')
            .map(|v| field(v, n, "gids"))
            .collect::<Result<_, _>>()?
    };
    Ok(PackageEntry {
        name: name.into(),
        uid: field(f[1], n, "uid")?,
        debuggable: flag(f[2], n, "debuggable")?,
        data_dir: f[3].into(),
        seinfo: f[4].into(),
        gids,
        profileable_from_shell: f
            .get(6)
            .map(|v| flag(v, n, "profileable_from_shell"))
            .transpose()?,
        version_code: f.get(7).map(|v| field(v, n, "version_code")).transpose()?,
        code_path: None,
        // Neither is in `packages.list`; both are filled in by `build`.
        is_system: false,
    })
}
fn field<T: std::str::FromStr>(
    v: &str,
    line: usize,
    name: &'static str,
) -> Result<T, PackageError> {
    v.parse()
        .map_err(|_| PackageError::InvalidField { line, field: name })
}
fn flag(v: &str, line: usize, name: &'static str) -> Result<bool, PackageError> {
    match v {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(PackageError::InvalidField { line, field: name }),
    }
}
fn parse_code_paths<R: BufRead>(input: R) -> Result<HashMap<String, PathBuf>, PackageError> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut buf = vec![];
    let mut out = HashMap::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"package" => {
                let mut name = None;
                let mut path = None;
                for a in e.attributes().with_checks(true) {
                    let a = a.map_err(|x| PackageError::InvalidXml(x.to_string()))?;
                    let value = || {
                        a.decode_and_unescape_value(reader.decoder())
                            .map(|v| v.into_owned())
                            .map_err(|x| PackageError::InvalidXml(x.to_string()))
                    };
                    match a.key.as_ref() {
                        b"name" => name = Some(value()?),
                        b"codePath" => path = Some(PathBuf::from(value()?)),
                        _ => {}
                    }
                }
                if let (Some(n), Some(p)) = (name, path)
                    && android_path_is_absolute(&p)
                {
                    out.insert(n, p);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(PackageError::InvalidXml(e.to_string())),
        }
        buf.clear()
    }
    Ok(out)
}
/// Parses `cmd package list packages -f` into a name → APK path map.
///
/// This is the code-path source to prefer. `/data/system/packages.xml` has been
/// Android Binary XML since Android 12, so reading it as text fails outright — and
/// decoding ABX would mean depending on a private serialisation format, exactly the
/// kind of coupling this project exists to avoid. `cmd package` is a public CLI whose
/// output shape has been stable for years, costs one spawn for every package, and
/// reports system packages at their real partition path instead of the staging
/// `codePath`, which is what the system-app test actually wants.
///
/// Each line is `package:<apk path>=<package name>`. The name is taken from the *last*
/// `=` because installed APK directories are base64 and routinely contain `==`, while
/// a package name never contains `=`.
#[must_use]
pub fn parse_pm_code_paths(output: &str) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    for line in output.lines() {
        let Some(body) = line.trim().strip_prefix("package:") else {
            continue;
        };
        // Lines without the trailing name appear when the caller forgot `-f`; there is
        // nothing to key on, so they are skipped rather than guessed at.
        let Some((path, name)) = body.rsplit_once('=') else {
            continue;
        };
        let path = PathBuf::from(path);
        if is_safe_package_name(name) && android_path_is_absolute(&path) {
            paths.insert(name.to_owned(), path);
        }
    }
    paths
}

/// Whether `name` is safe to put in a command line as a package name.
///
/// Deliberately narrow — an Android package name only needs these characters, and this guards
/// the arguments the daemon hands to `am` and `cmd package`.
#[must_use]
pub fn is_safe_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_'))
}

/// Whether `name` is safe to use as a settings key for whatever crashed.
///
/// Looser than [`is_safe_package_name`] because not everything that crashes is an app: a
/// tombstone names its process, so per-app settings for a platform binary are keyed by
/// `/vendor/bin/hw/…`. Held to the package rules, every one of those was rejected as an invalid
/// request — the settings screen for one could not even read its own config back, let alone
/// ignore or mute it.
///
/// Still a whitelist, just a wider one: these characters cover every process name the platform
/// actually produces — a path, an `@1.0-service` HAL, a `:remote` subprocess, a bare
/// `system_server` — while leaving out every shell metacharacter. Nothing on this path reaches a
/// shell today (the command-line callers keep [`is_safe_package_name`], and even they go through
/// `Command::args` rather than `sh -c`), and a whitelist is what keeps that true if one ever
/// does. `..` is refused for the same reason: nothing builds a path out of these yet.
#[must_use]
pub fn is_safe_settings_key(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.contains("..")
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'@' | b'+' | b'-')
        })
}
#[must_use]
pub fn android_path_is_absolute(path: &Path) -> bool {
    path.to_string_lossy().starts_with('/')
}
#[must_use]
pub fn code_path_is_under_data_app(path: &Path) -> bool {
    path.to_string_lossy().starts_with("/data/app/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_indexes() {
        let list = "com.example 10123 0 /data/user/0/com.example default 3003,9997 1 42\n";
        let xml =
            r#"<packages><package name="com.example" codePath="/data/app/token"/></packages>"#;
        let i = PackageIndex::parse(list, xml).unwrap();
        let e = i.by_name("com.example").unwrap();
        assert_eq!(
            e.base_apk_path().as_deref(),
            Some(Path::new("/data/app/token/base.apk"))
        );
        assert_eq!(e.gids, vec![3003, 9997]);
    }
    #[test]
    fn rejects_bad() {
        assert!(matches!(
            PackageIndex::parse("com.x 1 7 /data/x se none", "<packages/>"),
            Err(PackageError::InvalidField {
                field: "debuggable",
                ..
            })
        ));
        assert!(!is_safe_package_name("../x"));
    }

    /// The settings key has to accept what a tombstone actually reports, since that is what the
    /// per-app screen for a platform process is keyed by.
    #[test]
    fn a_settings_key_accepts_a_process_path_but_not_a_traversal() {
        for accepted in [
            "com.example.app",
            "com.example.app:remote",
            "/vendor/bin/hw/android.hardware.audio.service_64",
            "./bluetooth_audio_provider_session_pcm192_probe",
            "surfaceflinger",
        ] {
            assert!(is_safe_settings_key(accepted), "{accepted}");
        }
        for refused in [
            "",
            "../etc/passwd",
            "/vendor/../data/x",
            "name with spaces",
            "quote'injection",
            "quote\"injection",
            "line\nbreak",
            // Shell metacharacters, refused by construction rather than by enumeration.
            "x;rm -rf /",
            "x|y",
            "x&y",
            "x$(y)",
            "x`y`",
            "x>y",
            "x*",
        ] {
            assert!(!is_safe_settings_key(refused), "{refused:?}");
        }
        // A package name that reaches a command line stays on the strict rules.
        assert!(!is_safe_package_name(
            "/vendor/bin/hw/android.hardware.audio.service_64"
        ));
    }

    #[test]
    fn reads_pm_code_paths() {
        // The installed path deliberately carries base64 `==` runs, which is what
        // splitting on the first `=` used to get wrong.
        let output = "\
package:/data/app/~~4V6pR6-Cdw36LKFIT-CaUg==/com.example-f5SjhMZZVhasY5z-rKQACA==/base.apk=com.example
package:/system/priv-app/Settings/Settings.apk=com.android.settings
package:/data/app/broken/base.apk
package:not-a-package-line
package:relative/path.apk=com.relative
";
        let paths = parse_pm_code_paths(output);

        assert_eq!(
            paths.get("com.example").map(PathBuf::as_path),
            Some(Path::new(
                "/data/app/~~4V6pR6-Cdw36LKFIT-CaUg==/com.example-f5SjhMZZVhasY5z-rKQACA==/base.apk"
            ))
        );
        assert_eq!(
            paths.get("com.android.settings").map(PathBuf::as_path),
            Some(Path::new("/system/priv-app/Settings/Settings.apk"))
        );
        // A relative path is not something PackageManager reports; refusing it keeps a
        // malformed line from becoming a path the signature check would then resolve
        // against the daemon's working directory.
        assert!(!paths.contains_key("com.relative"));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn system_partition_path_survives_the_join() {
        let list = "com.android.settings 1000 0 /data/user/0/com.android.settings default none\n";
        let paths = parse_pm_code_paths(
            "package:/system/priv-app/Settings/Settings.apk=com.android.settings\n",
        );

        let index = PackageIndex::build(list, &paths, &HashSet::new()).unwrap();
        let entry = index.by_name("com.android.settings").unwrap();

        // Already an .apk, so it must be used as-is rather than gaining `/base.apk`.
        assert_eq!(
            entry.base_apk_path().as_deref(),
            Some(Path::new("/system/priv-app/Settings/Settings.apk"))
        );
    }

    #[test]
    fn package_manager_decides_which_packages_are_system() {
        let list = "com.android.settings 1000 0 /data/user/0/com.android.settings default none\n\
                    com.example.app 10123 0 /data/user/0/com.example.app default none\n";
        // Deliberately no code paths: the answer must not depend on them.
        let system = HashSet::from(["com.android.settings".to_owned()]);

        let index = PackageIndex::build(list, &HashMap::new(), &system).unwrap();

        assert!(index.by_name("com.android.settings").unwrap().is_system);
        assert!(!index.by_name("com.example.app").unwrap().is_system);
    }

    /// The regression: Settings lives on `/system_ext` on a current release, and the partition
    /// list it was matched against did not have that prefix.
    #[test]
    fn the_path_fallback_covers_the_partitions_a_current_android_uses() {
        for path in [
            "/system/priv-app/Telecom/Telecom.apk",
            "/system_ext/priv-app/Settings/Settings.apk",
            "/product/app/Something.apk",
            "/vendor/app/Something.apk",
            "/apex/com.android.bt/app/Bluetooth/Bluetooth.apk",
        ] {
            assert!(
                looks_like_system_path(Some(Path::new(path))),
                "{path} is on a platform partition"
            );
        }
        assert!(!looks_like_system_path(Some(Path::new(
            "/data/app/~~a==/com.example.app-b==/base.apk"
        ))));
        assert!(!looks_like_system_path(None));
    }

    /// A rebuild at a moment when PackageManager is unavailable again must not throw away the
    /// flags a previous one established.
    #[test]
    fn a_rebuild_without_package_manager_inherits_the_known_flags() {
        let list = "com.android.settings 1000 0 /data/user/0/com.android.settings default none\n\
                    com.example.app 10123 0 /data/user/0/com.example.app default none\n";
        let complete = PackageIndex::build(
            list,
            &HashMap::new(),
            &HashSet::from(["com.android.settings".to_owned()]),
        )
        .unwrap();

        // The same device, rebuilt while `cmd package` answers nothing: no paths, no flags.
        let mut blind = PackageIndex::build(list, &HashMap::new(), &HashSet::new()).unwrap();
        assert!(!blind.by_name("com.android.settings").unwrap().is_system);

        blind.inherit_system_flags(&complete);

        assert!(blind.system_flags_known());
        assert!(blind.by_name("com.android.settings").unwrap().is_system);
        assert!(!blind.by_name("com.example.app").unwrap().is_system);
    }

    #[test]
    fn inheriting_never_overwrites_a_fresh_answer() {
        let list = "com.android.settings 1000 0 /data/user/0/com.android.settings default none\n";
        // Stale: it used to be a system app and is not one now, however unlikely.
        let stale = PackageIndex::build(
            list,
            &HashMap::new(),
            &HashSet::from(["com.android.settings".to_owned()]),
        )
        .unwrap();
        let mut fresh =
            PackageIndex::build(list, &HashMap::new(), &HashSet::from(["other".to_owned()]))
                .unwrap();

        fresh.inherit_system_flags(&stale);

        assert!(
            !fresh.by_name("com.android.settings").unwrap().is_system,
            "PackageManager's current answer wins"
        );
    }

    /// Without PackageManager's answer the path is all there is, and it is better than nothing.
    #[test]
    fn an_empty_system_set_falls_back_to_the_path() {
        let list = "com.android.settings 1000 0 /data/user/0/com.android.settings default none\n";
        let paths = parse_pm_code_paths(
            "package:/system_ext/priv-app/Settings/Settings.apk=com.android.settings\n",
        );

        let index = PackageIndex::build(list, &paths, &HashSet::new()).unwrap();

        assert!(index.by_name("com.android.settings").unwrap().is_system);
    }
}
