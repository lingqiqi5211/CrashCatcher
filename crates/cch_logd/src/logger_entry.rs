use crate::{Cursor, MAX_LOG_PAYLOAD_BYTES, ParseError};

/// Header metadata plus the payload selected via the entry's own `hdr_size`.
/// This works for the historical logger entry layouts as well as v4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggerEntry<'a> {
    pub pid: i32,
    pub tid: u32,
    pub seconds: u32,
    pub nanoseconds: u32,
    pub log_id: Option<u32>,
    pub uid: Option<u32>,
    pub payload: &'a [u8],
}

pub fn parse_logger_entry(bytes: &[u8]) -> Result<LoggerEntry<'_>, ParseError> {
    let mut cursor = Cursor::new(bytes);
    let payload_length = usize::from(cursor.read_u16_le("logger payload length")?);
    if payload_length > MAX_LOG_PAYLOAD_BYTES {
        return Err(ParseError::LimitExceeded {
            field: "logger payload",
            limit: MAX_LOG_PAYLOAD_BYTES,
        });
    }
    let header_size = usize::from(cursor.read_u16_le("logger header size")?);
    if header_size < 20 || header_size > bytes.len() {
        return Err(ParseError::InvalidHeaderSize(header_size));
    }
    let pid = cursor.read_i32_le("logger pid")?;
    let tid = cursor.read_u32_le("logger tid")?;
    let seconds = cursor.read_u32_le("logger seconds")?;
    let nanoseconds = cursor.read_u32_le("logger nanoseconds")?;

    let log_id = if header_size >= 24 {
        Some(cursor.read_u32_le("logger id")?)
    } else {
        None
    };
    let uid = if header_size >= 28 {
        Some(cursor.read_u32_le("logger uid")?)
    } else {
        None
    };

    let end = header_size
        .checked_add(payload_length)
        .ok_or(ParseError::IntegerOutOfRange {
            field: "logger entry length",
        })?;
    if end > bytes.len() {
        return Err(ParseError::LengthOutOfBounds {
            field: "logger payload",
            length: payload_length,
            remaining: bytes.len().saturating_sub(header_size),
        });
    }
    Ok(LoggerEntry {
        pid,
        tid,
        seconds,
        nanoseconds,
        log_id,
        uid,
        payload: &bytes[header_size..end],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_v4_header_size_instead_of_a_fixed_offset() {
        let payload = b"payload";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&28_u16.to_le_bytes());
        bytes.extend_from_slice(&123_i32.to_le_bytes());
        bytes.extend_from_slice(&456_u32.to_le_bytes());
        bytes.extend_from_slice(&7_u32.to_le_bytes());
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&5_u32.to_le_bytes());
        bytes.extend_from_slice(&10_123_u32.to_le_bytes());
        bytes.extend_from_slice(payload);

        let entry = parse_logger_entry(&bytes).unwrap();
        assert_eq!(entry.payload, payload);
        assert_eq!(entry.log_id, Some(5));
        assert_eq!(entry.uid, Some(10_123));
    }
}
