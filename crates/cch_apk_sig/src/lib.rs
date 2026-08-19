//! Extracts signer certificate digests from installed v2/v3-signed APKs.

#![forbid(unsafe_code)]
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use thiserror::Error;

const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const MAGIC: &[u8; 16] = b"APK Sig Block 42";
const V2: u32 = 0x7109_871a;
const V3: u32 = 0xf053_68c0;
const V31: u32 = 0x1b93_ad61;
const MAX_BLOCK: usize = 64 * 1024 * 1024;
const MAX_SIGNERS: usize = 32;
const MAX_CERT: usize = 4 * 1024 * 1024;
pub type CertificateDigest = [u8; 32];

#[derive(Debug, Error)]
pub enum ApkSigError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("APK has no valid ZIP end-of-central-directory record")]
    MissingEocd,
    #[error("ZIP64 APKs are not supported for authentication")]
    Zip64Unsupported,
    #[error("ZIP central directory bounds are inconsistent")]
    InvalidCentralDirectory,
    #[error("APK has no v2/v3 signing block")]
    MissingSigningBlock,
    #[error("APK signing block is malformed: {0}")]
    Malformed(&'static str),
    #[error("APK signing block exceeds the {0}-byte safety limit")]
    SigningBlockTooLarge(usize),
    #[error("APK signer contains no certificate")]
    MissingCertificate,
}

pub fn certificate_sha256(path: &Path) -> Result<Vec<CertificateDigest>, ApkSigError> {
    let mut f = File::open(path).map_err(|source| ApkSigError::Io {
        path: path.into(),
        source,
    })?;
    certificate_sha256_from_reader(&mut f).map_err(|e| match e {
        ApkSigError::Io { source, .. } => ApkSigError::Io {
            path: path.into(),
            source,
        },
        x => x,
    })
}
pub fn certificate_sha256_from_reader<R: Read + Seek>(
    r: &mut R,
) -> Result<Vec<CertificateDigest>, ApkSigError> {
    let len = r.seek(SeekFrom::End(0)).map_err(ioerr)?;
    let e = find_eocd(r, len)?;
    let central = u64::from(e.offset);
    if central.checked_add(u64::from(e.size)) != Some(e.absolute) || central > len {
        return Err(ApkSigError::InvalidCentralDirectory);
    }
    let block = read_block(r, central)?;
    let pairs = parse_pairs(&block)?;
    let scheme = [V31, V3, V2]
        .into_iter()
        .find_map(|id| {
            pairs
                .iter()
                .find(|(found, _)| *found == id)
                .map(|(_, v)| *v)
        })
        .ok_or(ApkSigError::MissingSigningBlock)?;
    parse_certificates(scheme)
}
struct Eocd {
    absolute: u64,
    size: u32,
    offset: u32,
}
fn find_eocd<R: Read + Seek>(r: &mut R, len: u64) -> Result<Eocd, ApkSigError> {
    let start = len.saturating_sub(65_557);
    r.seek(SeekFrom::Start(start)).map_err(ioerr)?;
    let mut tail = vec![];
    r.read_to_end(&mut tail).map_err(ioerr)?;
    if tail.len() < 22 {
        return Err(ApkSigError::MissingEocd);
    }
    for p in (0..=tail.len() - 22).rev() {
        if tail[p..p + 4] != EOCD {
            continue;
        }
        let comment = usize::from(u16le(&tail[p + 20..p + 22])?);
        if p + 22 + comment != tail.len() {
            continue;
        }
        let size = u32le(&tail[p + 12..p + 16])?;
        let offset = u32le(&tail[p + 16..p + 20])?;
        if size == u32::MAX || offset == u32::MAX {
            return Err(ApkSigError::Zip64Unsupported);
        }
        return Ok(Eocd {
            absolute: start + u64::try_from(p).unwrap_or(u64::MAX),
            size,
            offset,
        });
    }
    Err(ApkSigError::MissingEocd)
}
fn read_block<R: Read + Seek>(r: &mut R, central: u64) -> Result<Vec<u8>, ApkSigError> {
    if central < 24 {
        return Err(ApkSigError::MissingSigningBlock);
    }
    r.seek(SeekFrom::Start(central - 24)).map_err(ioerr)?;
    let mut footer = [0; 24];
    r.read_exact(&mut footer).map_err(ioerr)?;
    if &footer[8..] != MAGIC {
        return Err(ApkSigError::MissingSigningBlock);
    }
    let size = u64le(&footer[..8])?;
    let total = size
        .checked_add(8)
        .ok_or(ApkSigError::Malformed("block size overflow"))?;
    let total = usize::try_from(total).map_err(|_| ApkSigError::SigningBlockTooLarge(MAX_BLOCK))?;
    if total > MAX_BLOCK {
        return Err(ApkSigError::SigningBlockTooLarge(MAX_BLOCK));
    }
    let start = central
        .checked_sub(total as u64)
        .ok_or(ApkSigError::Malformed("block starts before file"))?;
    r.seek(SeekFrom::Start(start)).map_err(ioerr)?;
    let mut block = vec![0; total];
    r.read_exact(&mut block).map_err(ioerr)?;
    if u64le(&block[..8])? != size || &block[block.len() - 16..] != MAGIC {
        return Err(ApkSigError::Malformed("size fields or magic disagree"));
    }
    Ok(block)
}
fn parse_pairs(block: &[u8]) -> Result<Vec<(u32, &[u8])>, ApkSigError> {
    if block.len() < 32 {
        return Err(ApkSigError::Malformed("short block"));
    }
    let end = block.len() - 24;
    let mut c = Cursor::new(&block[8..end]);
    let mut out = vec![];
    while c.remaining() > 0 {
        let len = usize::try_from(c.u64()?)
            .map_err(|_| ApkSigError::Malformed("pair length overflow"))?;
        if len < 4 {
            return Err(ApkSigError::Malformed("pair shorter than ID"));
        }
        let pair = c.read(len)?;
        out.push((u32le(&pair[..4])?, &pair[4..]));
    }
    Ok(out)
}
fn parse_certificates(value: &[u8]) -> Result<Vec<CertificateDigest>, ApkSigError> {
    let mut root = Cursor::new(value);
    let mut signers = Cursor::new(root.lp()?);
    let mut out = vec![];
    while signers.remaining() > 0 {
        if out.len() >= MAX_SIGNERS {
            return Err(ApkSigError::Malformed("too many signers"));
        }
        let signer = signers.lp()?;
        let cert = first_cert(signer)?;
        out.push(Sha256::digest(cert).into());
    }
    if out.is_empty() {
        Err(ApkSigError::MissingCertificate)
    } else {
        Ok(out)
    }
}
fn first_cert(signer: &[u8]) -> Result<&[u8], ApkSigError> {
    let mut s = Cursor::new(signer);
    let mut signed = Cursor::new(s.lp()?);
    let _digests = signed.lp()?;
    let mut certs = Cursor::new(signed.lp()?);
    let cert = certs.lp()?;
    if cert.is_empty() {
        return Err(ApkSigError::MissingCertificate);
    }
    if cert.len() > MAX_CERT {
        return Err(ApkSigError::Malformed("certificate too large"));
    }
    Ok(cert)
}
struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Cursor<'a> {
    const fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
    fn read(&mut self, n: usize) -> Result<&'a [u8], ApkSigError> {
        if n > self.remaining() {
            return Err(ApkSigError::Malformed("truncated length-prefixed field"));
        }
        let p = self.p;
        self.p += n;
        Ok(&self.b[p..self.p])
    }
    fn u64(&mut self) -> Result<u64, ApkSigError> {
        u64le(self.read(8)?)
    }
    fn lp(&mut self) -> Result<&'a [u8], ApkSigError> {
        let n = usize::try_from(u32le(self.read(4)?)?)
            .map_err(|_| ApkSigError::Malformed("length overflow"))?;
        self.read(n)
    }
}
fn u16le(b: &[u8]) -> Result<u16, ApkSigError> {
    Ok(u16::from_le_bytes(
        b.try_into()
            .map_err(|_| ApkSigError::Malformed("truncated u16"))?,
    ))
}
fn u32le(b: &[u8]) -> Result<u32, ApkSigError> {
    Ok(u32::from_le_bytes(
        b.try_into()
            .map_err(|_| ApkSigError::Malformed("truncated u32"))?,
    ))
}
fn u64le(b: &[u8]) -> Result<u64, ApkSigError> {
    Ok(u64::from_le_bytes(
        b.try_into()
            .map_err(|_| ApkSigError::Malformed("truncated u64"))?,
    ))
}
fn ioerr(source: io::Error) -> ApkSigError {
    ApkSigError::Io {
        path: PathBuf::new(),
        source,
    }
}
#[must_use]
pub fn digest_hex(d: &CertificateDigest) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in d {
        out.push(char::from(H[usize::from(b >> 4)]));
        out.push(char::from(H[usize::from(b & 15)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn lp(v: &[u8]) -> Vec<u8> {
        let mut o = (v.len() as u32).to_le_bytes().to_vec();
        o.extend_from_slice(v);
        o
    }
    fn fake(cert: &[u8]) -> Vec<u8> {
        let certs = lp(cert);
        let mut signed = lp(&[]);
        signed.extend(lp(&certs));
        signed.extend(lp(&[]));
        let mut signer = lp(&signed);
        signer.extend(lp(&[]));
        signer.extend(lp(&[]));
        let scheme = lp(&lp(&signer));
        let mut pair = ((4 + scheme.len()) as u64).to_le_bytes().to_vec();
        pair.extend(V3.to_le_bytes());
        pair.extend(scheme);
        let size = (pair.len() + 24) as u64;
        let mut block = size.to_le_bytes().to_vec();
        block.extend(pair);
        block.extend(size.to_le_bytes());
        block.extend(MAGIC);
        let mut apk = b"prefix".to_vec();
        apk.extend(block);
        let offset = apk.len() as u32;
        let central = [0x50, 0x4b, 1, 2];
        apk.extend(central);
        apk.extend(EOCD);
        apk.extend([0; 8]);
        apk.extend((central.len() as u32).to_le_bytes());
        apk.extend(offset.to_le_bytes());
        apk.extend(0u16.to_le_bytes());
        apk
    }
    #[test]
    fn extracts_cert() {
        let cert = b"DER";
        let d = certificate_sha256_from_reader(&mut Cursor::new(fake(cert))).unwrap();
        let expected: CertificateDigest = Sha256::digest(cert).into();
        assert_eq!(d, vec![expected]);
        assert_eq!(digest_hex(&expected).len(), 64);
    }
}
