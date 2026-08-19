//! Stamps the release version into the daemon.
//!
//! `CARGO_PKG_VERSION` is this crate's own number, and nothing maintains it: the release version
//! lives in `version.properties`, which Gradle and cch-packager both read. So the about page,
//! which asks the daemon what version it is, reported 0.1.0 next to a module the root manager
//! listed as 0.2.0 — two numbers for one half of a matched pair.
//!
//! `CCH_RELEASE_VERSION` lets cch-packager pass the exact label it stamps into `module.prop`,
//! including the commit count and hash an off-tag build carries. Building the daemon on its own
//! falls back to the plain version in `version.properties`, which is still the right answer.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-env-changed=CCH_RELEASE_VERSION");

    let version = env::var("CCH_RELEASE_VERSION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .or_else(release_version)
        .unwrap_or_else(|| env::var("CARGO_PKG_VERSION").unwrap_or_default());

    println!("cargo:rustc-env=CCH_DAEMON_VERSION={version}");
}

/// `version=` from the workspace's `version.properties`.
fn release_version() -> Option<String> {
    let manifest = env::var("CARGO_MANIFEST_DIR").ok()?;
    let path = Path::new(&manifest)
        .join("..")
        .join("..")
        .join("version.properties");
    println!("cargo:rerun-if-changed={}", path.display());

    fs::read_to_string(&path)
        .ok()?
        .lines()
        .find_map(|line| line.trim().strip_prefix("version="))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
