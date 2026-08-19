//! Joins PackageManager's UID index with its authoritative APK code paths.

#![forbid(unsafe_code)]
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::BufRead,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const ANDROID_UID_USER_RANGE: u32 = 100_000;
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
}
impl PackageIndex {
    /// Joins `packages.list` with code paths taken from `packages.xml`.
    ///
    /// Only usable where that file is still text. Since Android 12 the platform
    /// writes it as Android Binary XML, so prefer [`Self::build`] with
    /// [`parse_pm_code_paths`].
    pub fn parse(list: &str, xml: &str) -> Result<Self, PackageError> {
        Self::build(list, &parse_code_paths(xml.as_bytes())?)
    }

    /// Joins `packages.list` with an already-resolved name → code path map.
    ///
    /// Split from the parsing so the code paths can come from whichever source is
    /// actually readable on the running device; a package missing from
    /// [`code_paths`] simply has no path rather than failing the whole index.
    pub fn build(list: &str, code_paths: &HashMap<String, PathBuf>) -> Result<Self, PackageError> {
        let mut entries = vec![];
        for (i, line) in list.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut e = parse_line(line, i + 1)?;
            e.code_path = code_paths.get(&e.name).cloned();
            entries.push(e)
        }
        let mut index = Self {
            entries,
            by_uid: HashMap::new(),
            by_name: HashMap::new(),
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

#[must_use]
pub fn is_safe_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_'))
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

        let index = PackageIndex::build(list, &paths).unwrap();
        let entry = index.by_name("com.android.settings").unwrap();

        // Already an .apk, so it must be used as-is rather than gaining `/base.apk`.
        assert_eq!(
            entry.base_apk_path().as_deref(),
            Some(Path::new("/system/priv-app/Settings/Settings.apk"))
        );
    }
}
