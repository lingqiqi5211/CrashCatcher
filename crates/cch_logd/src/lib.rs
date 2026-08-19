//! Host-testable parsers for Android's binary events log and crash-buffer text.

#![deny(unsafe_op_in_unsafe_fn)]

mod crash;
mod event;
mod logger_entry;
#[cfg(target_os = "android")]
mod reader;

pub use crash::{CrashBufferReport, JavaFrame, TextLogEntry, parse_crash_buffer};
pub use event::{
    AM_ANR_TAG, AM_CRASH_TAG, ActivityEvent, AmAnrEvent, AmCrashEvent, EventRecord, EventValue,
    SCREEN_TOGGLED_TAG, ScreenEvent, WM_SET_KEYGUARD_SHOWN_TAG, parse_activity_event,
    parse_event_payload, parse_screen_event,
};
pub use logger_entry::{LoggerEntry, parse_logger_entry};
#[cfg(target_os = "android")]
pub use reader::{AndroidLogReader, LogBuffer, LogReaderError};

use thiserror::Error;

pub const MAX_LOG_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("input ended while reading {field}")]
    Truncated { field: &'static str },
    #[error("{field} length {length} exceeds the remaining {remaining} bytes")]
    LengthOutOfBounds {
        field: &'static str,
        length: usize,
        remaining: usize,
    },
    #[error("{field} exceeds the {limit}-byte safety limit")]
    LimitExceeded { field: &'static str, limit: usize },
    #[error("unsupported event value type {0}")]
    UnsupportedEventType(u8),
    #[error("event nesting is deeper than {0}")]
    EventNestingTooDeep(usize),
    #[error("invalid UTF-8 in {field}")]
    InvalidUtf8 { field: &'static str },
    #[error("unexpected event tag {0}")]
    UnexpectedTag(i32),
    #[error("{field} has the wrong event type")]
    WrongEventType { field: &'static str },
    #[error("{field} is outside the supported integer range")]
    IntegerOutOfRange { field: &'static str },
    #[error("event has {actual} values, expected {expected}")]
    WrongValueCount {
        expected: &'static str,
        actual: usize,
    },
    #[error("logger header size {0} is invalid")]
    InvalidHeaderSize(usize),
    #[error("crash-buffer message is missing {0}")]
    MissingCrashField(&'static str),
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }
    pub(crate) fn read_u8(&mut self, field: &'static str) -> Result<u8, ParseError> {
        let byte = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(ParseError::Truncated { field })?;
        self.position += 1;
        Ok(byte)
    }
    pub(crate) fn read_bytes(
        &mut self,
        length: usize,
        field: &'static str,
    ) -> Result<&'a [u8], ParseError> {
        if length > self.remaining() {
            return Err(ParseError::LengthOutOfBounds {
                field,
                length,
                remaining: self.remaining(),
            });
        }
        let start = self.position;
        self.position += length;
        Ok(&self.bytes[start..self.position])
    }
    pub(crate) fn read_u16_le(&mut self, field: &'static str) -> Result<u16, ParseError> {
        let bytes = self.read_bytes(2, field)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    pub(crate) fn read_i32_le(&mut self, field: &'static str) -> Result<i32, ParseError> {
        let bytes = self.read_bytes(4, field)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    pub(crate) fn read_u32_le(&mut self, field: &'static str) -> Result<u32, ParseError> {
        let bytes = self.read_bytes(4, field)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    pub(crate) fn read_i64_le(&mut self, field: &'static str) -> Result<i64, ParseError> {
        let b = self.read_bytes(8, field)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
}
