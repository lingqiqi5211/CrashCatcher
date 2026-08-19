use serde::{Deserialize, Serialize};

use crate::ParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLogEntry<'a> {
    pub priority: u8,
    pub tag: &'a str,
    pub message: &'a str,
}

impl<'a> TextLogEntry<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, ParseError> {
        let (&priority, rest) = payload
            .split_first()
            .ok_or(ParseError::Truncated { field: "priority" })?;
        let tag_end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(ParseError::MissingCrashField("tag terminator"))?;
        let tag = std::str::from_utf8(&rest[..tag_end])
            .map_err(|_| ParseError::InvalidUtf8 { field: "log tag" })?;
        let message_bytes = &rest[tag_end + 1..];
        let message_end = message_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(message_bytes.len());
        let message = std::str::from_utf8(&message_bytes[..message_end]).map_err(|_| {
            ParseError::InvalidUtf8 {
                field: "log message",
            }
        })?;
        Ok(Self {
            priority,
            tag,
            message,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaFrame {
    pub class_name: String,
    pub method_name: String,
    pub file_name: Option<String>,
    pub line: Option<u32>,
}

impl JavaFrame {
    #[must_use]
    pub fn normalized(&self) -> String {
        format!("{}.{}", self.class_name, self.method_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashBufferReport {
    pub thread_name: String,
    pub process_name: String,
    pub pid: i32,
    pub exception_class: String,
    pub exception_message: String,
    pub frames: Vec<JavaFrame>,
    pub raw: String,
}

pub fn parse_crash_buffer(message: &str) -> Result<CrashBufferReport, ParseError> {
    let normalized = message.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let first = lines
        .find(|line| line.trim_start().starts_with("FATAL EXCEPTION:"))
        .ok_or(ParseError::MissingCrashField("FATAL EXCEPTION header"))?;
    let thread_name = first
        .trim_start()
        .strip_prefix("FATAL EXCEPTION:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ParseError::MissingCrashField("thread name"))?
        .to_owned();

    let process_line = lines
        .find(|line| line.trim_start().starts_with("Process:"))
        .ok_or(ParseError::MissingCrashField("process line"))?;
    let (process_name, pid) = parse_process_line(process_line.trim())?;

    let remaining: Vec<&str> = lines.collect();
    let exception_line = remaining
        .iter()
        .map(|line| line.trim())
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("at ")
                && !line.starts_with("Caused by:")
                && !line.starts_with("Suppressed:")
                && !line.starts_with("...")
        })
        .ok_or(ParseError::MissingCrashField("exception line"))?;
    let (exception_class, exception_message) = exception_line.split_once(':').map_or_else(
        || ((*exception_line).to_owned(), String::new()),
        |(class, detail)| (class.trim().to_owned(), detail.trim().to_owned()),
    );
    let frames = remaining
        .iter()
        .filter_map(|line| parse_java_frame(line.trim()))
        .collect();

    Ok(CrashBufferReport {
        thread_name,
        process_name,
        pid,
        exception_class,
        exception_message,
        frames,
        raw: normalized,
    })
}

fn parse_process_line(line: &str) -> Result<(String, i32), ParseError> {
    let content = line
        .strip_prefix("Process:")
        .ok_or(ParseError::MissingCrashField("process prefix"))?
        .trim();
    let (name, pid_text) = content
        .rsplit_once(", PID:")
        .ok_or(ParseError::MissingCrashField("PID"))?;
    let pid = pid_text
        .trim()
        .parse::<i32>()
        .map_err(|_| ParseError::IntegerOutOfRange { field: "PID" })?;
    Ok((name.trim().to_owned(), pid))
}

fn parse_java_frame(line: &str) -> Option<JavaFrame> {
    let body = line.strip_prefix("at ")?.trim();
    let open = body.rfind('(')?;
    let close = body.strip_suffix(')')?;
    let method_path = body[..open].trim();
    let location = &close[open + 1..];
    let method_separator = method_path.rfind('.')?;
    let class_name = method_path[..method_separator].to_owned();
    let method_name = method_path[method_separator + 1..].to_owned();
    let (file_name, line) = if matches!(location, "Native Method" | "Unknown Source") {
        (None, None)
    } else if let Some((file, line_text)) = location.rsplit_once(':') {
        (Some(file.to_owned()), line_text.parse::<u32>().ok())
    } else {
        (Some(location.to_owned()), None)
    };
    Some(JavaFrame {
        class_name,
        method_name,
        file_name,
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runtime_init_crash_message() {
        let input = "FATAL EXCEPTION: DefaultDispatcher-worker-1\n\
                     Process: com.example:sync, PID: 4321\n\
                     java.lang.IllegalStateException: cache is closed\n\
                     \tat com.example.Cache.read(Cache.kt:44)\n\
                     \tat java.util.List.get(List.java:10)";
        let report = parse_crash_buffer(input).unwrap();
        assert_eq!(report.thread_name, "DefaultDispatcher-worker-1");
        assert_eq!(report.process_name, "com.example:sync");
        assert_eq!(report.exception_class, "java.lang.IllegalStateException");
        assert_eq!(report.frames[0].normalized(), "com.example.Cache.read");
        assert_eq!(report.frames[0].line, Some(44));
    }

    #[test]
    fn parses_priority_tag_and_message() {
        let payload = b"\x06AndroidRuntime\0FATAL EXCEPTION: main\0";
        let entry = TextLogEntry::parse(payload).unwrap();
        assert_eq!(entry.priority, 6);
        assert_eq!(entry.tag, "AndroidRuntime");
        assert_eq!(entry.message, "FATAL EXCEPTION: main");
    }
}
