//! Runtime reader for Android's stable liblog logger-list API.

use std::{ffi::c_void, io, ptr::NonNull};

use libloading::Library;
use thiserror::Error;

use crate::{LoggerEntry, ParseError, parse_logger_entry};

const LOGGER_ENTRY_MAX_BYTES: usize = 64 * 1024;
const ANDROID_LOG_RDONLY: i32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum LogBuffer {
    Events = 2,
    Crash = 4,
}

type LoggerListAlloc = unsafe extern "C" fn(i32, u32, i32) -> *mut c_void;
type LoggerOpen = unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void;
type LoggerListRead = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
type LoggerListFree = unsafe extern "C" fn(*mut c_void);

#[repr(C, align(8))]
struct RawLogMessage {
    bytes: [u8; LOGGER_ENTRY_MAX_BYTES],
}

pub struct AndroidLogReader {
    _library: Library,
    logger_list: NonNull<c_void>,
    read: LoggerListRead,
    free: LoggerListFree,
    buffer: RawLogMessage,
}

impl AndroidLogReader {
    pub fn open(buffer: LogBuffer) -> Result<Self, LogReaderError> {
        // SAFETY: liblog is part of Android's platform ABI and remains loaded for
        // the whole lifetime of every copied function pointer below.
        let library = unsafe { Library::new("liblog.so") }
            .map_err(|source| LogReaderError::Load(source.to_string()))?;
        // SAFETY: symbol names and signatures match log/log_read.h.
        let alloc = unsafe { library.get::<LoggerListAlloc>(b"android_logger_list_alloc\0") }
            .map_err(|source| LogReaderError::Load(source.to_string()))?;
        let alloc = *alloc;
        // SAFETY: symbol names and signatures match log/log_read.h.
        let open = unsafe { library.get::<LoggerOpen>(b"android_logger_open\0") }
            .map_err(|source| LogReaderError::Load(source.to_string()))?;
        let open = *open;
        // SAFETY: symbol names and signatures match log/log_read.h.
        let read = unsafe { library.get::<LoggerListRead>(b"android_logger_list_read\0") }
            .map_err(|source| LogReaderError::Load(source.to_string()))?;
        let read = *read;
        // SAFETY: symbol names and signatures match log/log_read.h.
        let free = unsafe { library.get::<LoggerListFree>(b"android_logger_list_free\0") }
            .map_err(|source| LogReaderError::Load(source.to_string()))?;
        let free = *free;

        // SAFETY: this allocator takes values only and returns an owned opaque list.
        let logger_list = NonNull::new(unsafe { alloc(ANDROID_LOG_RDONLY, 0, 0) })
            .ok_or(LogReaderError::Allocation)?;
        // SAFETY: the list is valid and the buffer id is a documented log_id_t.
        if unsafe { open(logger_list.as_ptr(), buffer as i32) }.is_null() {
            // SAFETY: allocation succeeded and ownership has not moved.
            unsafe { free(logger_list.as_ptr()) };
            return Err(LogReaderError::Open(io::Error::last_os_error()));
        }

        Ok(Self {
            _library: library,
            logger_list,
            read,
            free,
            buffer: RawLogMessage {
                bytes: [0; LOGGER_ENTRY_MAX_BYTES],
            },
        })
    }

    pub fn read_entry(&mut self) -> Result<LoggerEntry<'_>, LogReaderError> {
        // SAFETY: both pointers remain valid for the duration of the call and the
        // destination is larger than Android's log_msg storage.
        let length = unsafe {
            (self.read)(
                self.logger_list.as_ptr(),
                self.buffer.bytes.as_mut_ptr().cast::<c_void>(),
            )
        };
        if length < 0 {
            return Err(LogReaderError::Read(io::Error::from_raw_os_error(-length)));
        }
        let length = usize::try_from(length).map_err(|_| LogReaderError::Length)?;
        if length > self.buffer.bytes.len() {
            return Err(LogReaderError::Length);
        }
        parse_logger_entry(&self.buffer.bytes[..length]).map_err(LogReaderError::Parse)
    }
}

impl Drop for AndroidLogReader {
    fn drop(&mut self) {
        // SAFETY: the pointer was allocated by liblog and is freed exactly once.
        unsafe { (self.free)(self.logger_list.as_ptr()) };
    }
}

#[derive(Debug, Error)]
pub enum LogReaderError {
    #[error("failed to load liblog reader API: {0}")]
    Load(String),
    #[error("android_logger_list_alloc returned null")]
    Allocation,
    #[error("failed to open Android log buffer: {0}")]
    Open(#[source] io::Error),
    #[error("failed to read Android log buffer: {0}")]
    Read(#[source] io::Error),
    #[error("liblog returned an invalid entry length")]
    Length,
    #[error("invalid logger entry: {0}")]
    Parse(#[from] ParseError),
}
