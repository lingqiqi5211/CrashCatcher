//! Where the daemon gets its uid → package → APK path mapping.
//!
//! Two sources, both public and both plain text:
//!
//! - `/data/system/packages.list` for the uid index. Authoritative and stable.
//! - `cmd package list packages -f` for APK paths.
//!
//! Deliberately *not* `/data/system/packages.xml`. Since Android 12 the platform
//! writes that file as Android Binary XML, so reading it as text fails outright with
//! "stream did not contain valid UTF-8" — which took this daemon into a crash loop on
//! a HyperOS/Android 16 device. Decoding ABX would put a private serialisation format
//! on the critical path, the class of dependency this project exists to avoid.

use std::{collections::HashMap, fs, path::PathBuf, process::Command};

use cch_packages::{PackageError, PackageIndex, parse_pm_code_paths};
use thiserror::Error;

const PACKAGES_LIST: &str = "/data/system/packages.list";

/// Builds a fresh package index from the live system.
pub fn load_package_index() -> Result<PackageIndex, PackageIndexError> {
    let list = fs::read_to_string(PACKAGES_LIST).map_err(|source| PackageIndexError::List {
        path: PathBuf::from(PACKAGES_LIST),
        source,
    })?;
    PackageIndex::build(&list, &read_code_paths()).map_err(PackageIndexError::Parse)
}

/// Resolves every package's APK path, or nothing if PackageManager cannot be asked.
///
/// An empty map is not fatal: the paths only feed the manager's signature check and
/// the system-app flag, so losing them degrades those two things, whereas refusing to
/// start loses crash collection entirely. Staying up also means the manager can
/// connect and *say* that something is wrong.
fn read_code_paths() -> HashMap<String, PathBuf> {
    Command::new("cmd")
        .args(["package", "list", "packages", "-f"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| parse_pm_code_paths(&output))
        .unwrap_or_default()
}

#[derive(Debug, Error)]
pub enum PackageIndexError {
    #[error("failed to read {path}: {source}")]
    List {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("package index failed: {0}")]
    Parse(#[source] PackageError),
}
