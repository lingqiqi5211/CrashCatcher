//! Fail-closed manager-socket authentication.

#![deny(unsafe_op_in_unsafe_fn)]
use cch_apk_sig::{ApkSigError, CertificateDigest};
use cch_packages::{PackageIndex, android_path_is_absolute};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerPin(CertificateDigest);
impl ManagerPin {
    pub fn parse(value: &str) -> Result<Self, AuthError> {
        let value = value.trim();
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AuthError::InvalidPin(
                "pin must be exactly 64 hexadecimal characters",
            ));
        }
        let mut digest = [0; 32];
        for (i, slot) in digest.iter_mut().enumerate() {
            *slot = u8::from_str_radix(
                value
                    .get(i * 2..i * 2 + 2)
                    .ok_or(AuthError::InvalidPin("pin byte is truncated"))?,
                16,
            )
            .map_err(|_| AuthError::InvalidPin("invalid hexadecimal"))?;
        }
        if digest.iter().all(|b| *b == 0) {
            return Err(AuthError::InvalidPin("all-zero pins are forbidden"));
        }
        Ok(Self(digest))
    }
    pub fn load(path: &Path) -> Result<Self, AuthError> {
        let m = fs::symlink_metadata(path).map_err(|source| AuthError::PinIo {
            path: path.into(),
            source,
        })?;
        if !m.file_type().is_file() {
            return Err(AuthError::InvalidPin("pin path must be a regular file"));
        }
        validate_metadata(&m)?;
        if m.len() > 256 {
            return Err(AuthError::InvalidPin("pin file is unexpectedly large"));
        }
        Self::parse(
            &fs::read_to_string(path).map_err(|source| AuthError::PinIo {
                path: path.into(),
                source,
            })?,
        )
    }
    #[must_use]
    pub const fn digest(&self) -> &CertificateDigest {
        &self.0
    }
}
#[cfg(any(target_os = "android", target_os = "linux"))]
fn validate_metadata(m: &fs::Metadata) -> Result<(), AuthError> {
    use std::os::unix::fs::MetadataExt;
    if m.uid() != 0 || m.gid() != 0 {
        return Err(AuthError::InvalidPin("pin file must be owned by root:root"));
    }
    if m.mode() & 0o022 != 0 {
        return Err(AuthError::InvalidPin(
            "pin file must not be group/world writable",
        ));
    }
    Ok(())
}
#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn validate_metadata(_: &fs::Metadata) -> Result<(), AuthError> {
    Ok(())
}

pub trait CertificateSource {
    fn certificate_sha256(&self, path: &Path) -> Result<Vec<CertificateDigest>, String>;
}
#[derive(Debug, Clone, Copy, Default)]
pub struct InstalledApkCertificates;
impl CertificateSource for InstalledApkCertificates {
    fn certificate_sha256(&self, path: &Path) -> Result<Vec<CertificateDigest>, String> {
        cch_apk_sig::certificate_sha256(path).map_err(|e| e.to_string())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedManager {
    pub uid: u32,
    pub package_name: String,
    pub apk_path: PathBuf,
}
pub struct Authenticator<'a, S = InstalledApkCertificates> {
    packages: &'a PackageIndex,
    pin: &'a ManagerPin,
    certificates: S,
}
impl<'a> Authenticator<'a, InstalledApkCertificates> {
    #[must_use]
    pub const fn new(packages: &'a PackageIndex, pin: &'a ManagerPin) -> Self {
        Self {
            packages,
            pin,
            certificates: InstalledApkCertificates,
        }
    }
}
impl<'a, S: CertificateSource> Authenticator<'a, S> {
    #[must_use]
    pub const fn with_source(
        packages: &'a PackageIndex,
        pin: &'a ManagerPin,
        certificates: S,
    ) -> Self {
        Self {
            packages,
            pin,
            certificates,
        }
    }
    pub fn authenticate_uid(&self, uid: u32) -> Result<AuthenticatedManager, AuthError> {
        let candidates: Vec<_> = self.packages.by_uid(uid).collect();
        if candidates.is_empty() {
            return Err(AuthError::UnknownUid(uid));
        }
        let mut had_path = false;
        let mut last_error = None;
        for package in candidates {
            let Some(path) = package.base_apk_path() else {
                continue;
            };
            if !android_path_is_absolute(&path) {
                continue;
            }
            had_path = true;
            match self.certificates.certificate_sha256(&path) {
                Ok(digests) => {
                    if digests
                        .iter()
                        .any(|d| constant_time_eq(d, self.pin.digest()))
                    {
                        return Ok(AuthenticatedManager {
                            uid,
                            package_name: package.name.clone(),
                            apk_path: path,
                        });
                    }
                }
                Err(e) => last_error = Some(e),
            }
        }
        if !had_path {
            Err(AuthError::MissingApkPath(uid))
        } else if let Some(e) = last_error {
            Err(AuthError::CertificateRead(e))
        } else {
            Err(AuthError::SignatureMismatch(uid))
        }
    }
}
impl AuthError {
    /// Whether a stale package index is a plausible cause of this failure.
    ///
    /// The index is built once at start-up, but APK paths are not stable: every
    /// reinstall or update moves the manager into a freshly randomised directory
    /// under `/data/app`, and a first-time install adds a uid the daemon has never
    /// seen. Both leave a running daemon unable to locate an APK it should accept,
    /// which is worth one reload and retry.
    ///
    /// [`Self::SignatureMismatch`] is deliberately excluded. There the APK *was*
    /// read and its certificate simply is not the pinned one — reloading cannot
    /// change that, and retrying on it would let any unauthorised app on the device
    /// make the daemon re-enumerate every package by reconnecting in a loop.
    #[must_use]
    pub const fn may_be_stale_package_index(&self) -> bool {
        matches!(
            self,
            Self::UnknownUid(_) | Self::MissingApkPath(_) | Self::CertificateRead(_)
        )
    }
}

fn constant_time_eq(a: &CertificateDigest, b: &CertificateDigest) -> bool {
    a.iter().zip(b).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}
#[cfg(any(target_os = "android", target_os = "linux"))]
pub type PeerSocketHandle = std::os::fd::RawFd;
#[cfg(not(any(target_os = "android", target_os = "linux")))]
pub type PeerSocketHandle = usize;
#[cfg(any(target_os = "android", target_os = "linux"))]
pub fn peer_credentials(fd: PeerSocketHandle) -> Result<PeerCredentials, AuthError> {
    let mut c = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = libc::socklen_t::try_from(std::mem::size_of::<libc::ucred>())
        .map_err(|_| AuthError::PeerCredentials(io::Error::other("ucred size overflow")))?;
    // SAFETY: c is writable initialized storage; len is its exact size; getsockopt retains no pointer and fd remains open.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(c).cast(),
            std::ptr::addr_of_mut!(len),
        )
    };
    if result != 0 {
        return Err(AuthError::PeerCredentials(io::Error::last_os_error()));
    }
    Ok(PeerCredentials {
        pid: c.pid,
        uid: c.uid,
        gid: c.gid,
    })
}
#[cfg(not(any(target_os = "android", target_os = "linux")))]
pub fn peer_credentials(_: PeerSocketHandle) -> Result<PeerCredentials, AuthError> {
    Err(AuthError::UnsupportedPlatform)
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid manager signing pin: {0}")]
    InvalidPin(&'static str),
    #[error("failed to read pin {path}: {source}")]
    PinIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("no installed package owns peer uid {0}")]
    UnknownUid(u32),
    #[error("packages for uid {0} have no authoritative APK path")]
    MissingApkPath(u32),
    #[error("peer uid {0} does not match the pinned signing certificate")]
    SignatureMismatch(u32),
    #[error("failed to inspect installed APK certificate: {0}")]
    CertificateRead(String),
    #[error("SO_PEERCRED failed: {0}")]
    PeerCredentials(#[source] io::Error),
    #[error("SO_PEERCRED is unsupported on this host")]
    UnsupportedPlatform,
}
impl From<ApkSigError> for AuthError {
    fn from(v: ApkSigError) -> Self {
        Self::CertificateRead(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    struct Fake {
        values: HashMap<PathBuf, Vec<CertificateDigest>>,
    }
    impl CertificateSource for Fake {
        fn certificate_sha256(&self, p: &Path) -> Result<Vec<CertificateDigest>, String> {
            self.values.get(p).cloned().ok_or_else(|| "missing".into())
        }
    }
    #[test]
    fn pins_are_strict() {
        assert!(ManagerPin::parse(&"ab".repeat(32)).is_ok());
        assert!(ManagerPin::parse(&"00".repeat(32)).is_err());
        assert!(ManagerPin::parse("abc").is_err());
    }
    #[test]
    fn authenticates_pinned_uid() {
        let packages = PackageIndex::parse(
            "com.manager 10123 0 /data/user/0/com.manager default none 0 1",
            r#"<packages><package name="com.manager" codePath="/data/app/manager"/></packages>"#,
        )
        .unwrap();
        let pin = ManagerPin::parse(&"ab".repeat(32)).unwrap();
        let source = Fake {
            values: HashMap::from([(
                PathBuf::from("/data/app/manager/base.apk"),
                vec![[0xab; 32]],
            )]),
        };
        let auth = Authenticator::with_source(&packages, &pin, source);
        assert_eq!(
            auth.authenticate_uid(10123).unwrap().package_name,
            "com.manager"
        );
        assert!(matches!(
            auth.authenticate_uid(9),
            Err(AuthError::UnknownUid(9))
        ));
    }
}
