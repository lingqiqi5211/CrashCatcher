//! The daemon's log file, rotated by size.
//!
//! Written here rather than left to `service.sh`'s `>>` redirect, because a redirect cannot be
//! rotated: renaming the file leaves this process's descriptor pointing at the same inode, so it
//! keeps writing to the file that was moved aside and the new one stays empty. Owning the
//! descriptor is what makes rotation possible at all.
//!
//! stderr stays redirected by the launcher. A panic or a failed start never reaches `tracing`,
//! and that output is the whole record of a daemon that could not get going.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

/// Size at which the current file is rotated.
pub const MAX_LOG_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// How many rotated files are kept, `daemon.log.1` through `.8`.
pub const MAX_LOG_FILES: usize = 8;

/// The live log file name; rotated copies get a `.N` suffix.
pub const LOG_FILE_NAME: &str = "daemon.log";

pub struct RollingLog {
    directory: PathBuf,
    max_bytes: u64,
    max_files: usize,
    file: File,
    written: u64,
    /// Whether the last byte written was a newline.
    ///
    /// A formatted record reaches `write` in several calls, so the size check alone would cut a
    /// line in half and leave its remainder at the top of the next file, parsing as neither.
    at_line_start: bool,
}

impl RollingLog {
    pub fn open(directory: impl Into<PathBuf>) -> io::Result<Self> {
        Self::with_limits(directory, MAX_LOG_FILE_BYTES, MAX_LOG_FILES)
    }

    pub fn with_limits(
        directory: impl Into<PathBuf>,
        max_bytes: u64,
        max_files: usize,
    ) -> io::Result<Self> {
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        let path = directory.join(LOG_FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            directory,
            max_bytes,
            max_files: max_files.max(1),
            file,
            written,
            at_line_start: true,
        })
    }

    /// Moves `daemon.log` to `.1`, shifting the rest up and dropping the oldest.
    fn rotate(&mut self) -> io::Result<()> {
        let live = self.directory.join(LOG_FILE_NAME);
        let numbered = |index: usize| self.directory.join(format!("{LOG_FILE_NAME}.{index}"));

        let _ = fs::remove_file(numbered(self.max_files));
        for index in (1..self.max_files).rev() {
            let from = numbered(index);
            if from.is_file() {
                let _ = fs::rename(&from, numbered(index + 1));
            }
        }
        let _ = fs::rename(&live, numbered(1));

        self.file = OpenOptions::new().create(true).append(true).open(&live)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for RollingLog {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.written >= self.max_bytes && self.at_line_start {
            self.rotate()?;
        }
        let written = self.file.write(buffer)?;
        self.written += written as u64;
        if written > 0 {
            self.at_line_start = buffer[..written].ends_with(b"\n");
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(sink: &mut RollingLog, text: &str) {
        writeln!(sink, "{text}").expect("write");
        sink.flush().expect("flush");
    }

    #[test]
    fn rotates_once_the_limit_is_passed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut sink = RollingLog::with_limits(directory.path(), 32, 3).expect("open");

        for index in 0..12 {
            line(&mut sink, &format!("entry {index} padding padding"));
        }

        let live = directory.path().join(LOG_FILE_NAME);
        assert!(live.is_file());
        assert!(directory.path().join("daemon.log.1").is_file());
        assert!(
            !directory.path().join("daemon.log.4").exists(),
            "only max_files copies are kept"
        );
    }

    #[test]
    fn the_newest_lines_are_in_the_live_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut sink = RollingLog::with_limits(directory.path(), 32, 3).expect("open");

        line(&mut sink, "old line here padding padding");
        line(&mut sink, "newest line");

        let live = fs::read_to_string(directory.path().join(LOG_FILE_NAME)).expect("read");
        assert!(live.contains("newest line"));
    }

    #[test]
    fn an_existing_file_is_appended_to_rather_than_truncated() {
        let directory = tempfile::tempdir().expect("tempdir");
        {
            let mut sink = RollingLog::with_limits(directory.path(), 4096, 3).expect("open");
            line(&mut sink, "from the previous run");
        }

        let mut sink = RollingLog::with_limits(directory.path(), 4096, 3).expect("reopen");
        line(&mut sink, "from this run");

        let live = fs::read_to_string(directory.path().join(LOG_FILE_NAME)).expect("read");
        assert!(live.contains("from the previous run"));
        assert!(live.contains("from this run"));
    }

    /// A rotation renames files, and a stale descriptor would keep writing to the moved one.
    #[test]
    fn writing_continues_into_the_new_file_after_a_rotation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut sink = RollingLog::with_limits(directory.path(), 16, 3).expect("open");

        line(&mut sink, "first entry over the limit");
        line(&mut sink, "after rotation");

        let live = fs::read_to_string(directory.path().join(LOG_FILE_NAME)).expect("read");
        assert!(live.contains("after rotation"));
        assert!(!live.contains("first entry"));
    }

    /// The size check alone would rotate between the message and its newline, which is how the
    /// tail of one record ends up at the head of the next file.
    #[test]
    fn a_record_is_never_split_across_two_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut sink = RollingLog::with_limits(directory.path(), 8, 3).expect("open");

        for index in 0..5 {
            line(&mut sink, &format!("record {index} with a tail"));
        }

        for name in [LOG_FILE_NAME, "daemon.log.1", "daemon.log.2"] {
            let path = directory.path().join(name);
            if !path.is_file() {
                continue;
            }
            let body = fs::read_to_string(&path).expect("read");
            for line in body.lines().filter(|line| !line.is_empty()) {
                assert!(
                    line.starts_with("record ") && line.ends_with(" with a tail"),
                    "{name} holds a partial record: {line:?}"
                );
            }
        }
    }
}
