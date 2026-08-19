use serde::{Deserialize, Serialize};

use crate::{Cursor, ParseError};

pub const AM_ANR_TAG: i32 = 30_008;
pub const AM_CRASH_TAG: i32 = 30_039;

const EVENT_TYPE_INT: u8 = 0;
const EVENT_TYPE_LONG: u8 = 1;
const EVENT_TYPE_STRING: u8 = 2;
const EVENT_TYPE_LIST: u8 = 3;
const EVENT_TYPE_FLOAT: u8 = 4;
const MAX_EVENT_DEPTH: usize = 16;
const MAX_EVENT_ELEMENTS: usize = 255;
const MAX_EVENT_STRING_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventValue {
    Int(i32),
    Long(i64),
    String(String),
    List(Vec<Self>),
    Float(f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub tag: i32,
    pub value: EventValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmCrashEvent {
    pub user_id: i32,
    pub pid: i32,
    pub process_name: String,
    pub flags: i32,
    pub exception_class: String,
    pub message: String,
    pub file: String,
    pub line: i32,
    /// Added in Android 14. Older entries omit the field and decode to false.
    pub recoverable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmAnrEvent {
    pub user_id: i32,
    pub pid: i32,
    pub process_name: String,
    pub flags: i32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivityEvent {
    Crash(AmCrashEvent),
    Anr(AmAnrEvent),
}

pub fn parse_event_payload(bytes: &[u8]) -> Result<EventRecord, ParseError> {
    let mut cursor = Cursor::new(bytes);
    let tag = cursor.read_i32_le("event tag")?;
    let value = parse_value(&mut cursor, 0)?;
    Ok(EventRecord { tag, value })
}

fn parse_value(cursor: &mut Cursor<'_>, depth: usize) -> Result<EventValue, ParseError> {
    if depth >= MAX_EVENT_DEPTH {
        return Err(ParseError::EventNestingTooDeep(MAX_EVENT_DEPTH));
    }
    match cursor.read_u8("event type")? {
        EVENT_TYPE_INT => Ok(EventValue::Int(cursor.read_i32_le("event int")?)),
        EVENT_TYPE_LONG => Ok(EventValue::Long(cursor.read_i64_le("event long")?)),
        EVENT_TYPE_STRING => {
            let length =
                usize::try_from(cursor.read_u32_le("event string length")?).map_err(|_| {
                    ParseError::IntegerOutOfRange {
                        field: "event string length",
                    }
                })?;
            if length > MAX_EVENT_STRING_BYTES {
                return Err(ParseError::LimitExceeded {
                    field: "event string",
                    limit: MAX_EVENT_STRING_BYTES,
                });
            }
            let bytes = cursor.read_bytes(length, "event string")?;
            let value = std::str::from_utf8(bytes)
                .map_err(|_| ParseError::InvalidUtf8 {
                    field: "event string",
                })?
                .to_owned();
            Ok(EventValue::String(value))
        }
        EVENT_TYPE_LIST => {
            let count = usize::from(cursor.read_u8("event list length")?);
            if count > MAX_EVENT_ELEMENTS {
                return Err(ParseError::LimitExceeded {
                    field: "event list",
                    limit: MAX_EVENT_ELEMENTS,
                });
            }
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(parse_value(cursor, depth + 1)?);
            }
            Ok(EventValue::List(values))
        }
        EVENT_TYPE_FLOAT => {
            let bits = cursor.read_u32_le("event float")?;
            Ok(EventValue::Float(f32::from_bits(bits)))
        }
        other => Err(ParseError::UnsupportedEventType(other)),
    }
}

pub fn parse_activity_event(record: EventRecord) -> Result<ActivityEvent, ParseError> {
    let EventValue::List(values) = record.value else {
        return Err(ParseError::WrongEventType {
            field: "activity event root",
        });
    };
    match record.tag {
        AM_CRASH_TAG => parse_am_crash(&values).map(ActivityEvent::Crash),
        AM_ANR_TAG => parse_am_anr(&values).map(ActivityEvent::Anr),
        tag => Err(ParseError::UnexpectedTag(tag)),
    }
}

fn parse_am_crash(values: &[EventValue]) -> Result<AmCrashEvent, ParseError> {
    if !(values.len() == 8 || values.len() == 9) {
        return Err(ParseError::WrongValueCount {
            expected: "8 or 9",
            actual: values.len(),
        });
    }
    Ok(AmCrashEvent {
        user_id: int_at(values, 0, "user_id")?,
        pid: int_at(values, 1, "pid")?,
        process_name: string_at(values, 2, "process_name")?,
        flags: int_at(values, 3, "flags")?,
        exception_class: string_at(values, 4, "exception_class")?,
        message: string_at(values, 5, "message")?,
        file: string_at(values, 6, "file")?,
        line: int_at(values, 7, "line")?,
        recoverable: if values.len() == 9 {
            int_at(values, 8, "recoverable")? != 0
        } else {
            false
        },
    })
}

fn parse_am_anr(values: &[EventValue]) -> Result<AmAnrEvent, ParseError> {
    if values.len() != 5 {
        return Err(ParseError::WrongValueCount {
            expected: "5",
            actual: values.len(),
        });
    }
    Ok(AmAnrEvent {
        user_id: int_at(values, 0, "user_id")?,
        pid: int_at(values, 1, "pid")?,
        process_name: string_at(values, 2, "process_name")?,
        flags: int_at(values, 3, "flags")?,
        reason: string_at(values, 4, "reason")?,
    })
}

fn int_at(values: &[EventValue], index: usize, field: &'static str) -> Result<i32, ParseError> {
    match values.get(index) {
        Some(EventValue::Int(value)) => Ok(*value),
        Some(EventValue::Long(value)) => {
            i32::try_from(*value).map_err(|_| ParseError::IntegerOutOfRange { field })
        }
        _ => Err(ParseError::WrongEventType { field }),
    }
}

fn string_at(
    values: &[EventValue],
    index: usize,
    field: &'static str,
) -> Result<String, ParseError> {
    match values.get(index) {
        Some(EventValue::String(value)) => Ok(value.clone()),
        _ => Err(ParseError::WrongEventType { field }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(value: i32, output: &mut Vec<u8>) {
        output.push(EVENT_TYPE_INT);
        output.extend_from_slice(&value.to_le_bytes());
    }

    fn string(value: &str, output: &mut Vec<u8>) {
        output.push(EVENT_TYPE_STRING);
        output.extend_from_slice(&(value.len() as u32).to_le_bytes());
        output.extend_from_slice(value.as_bytes());
    }

    #[test]
    fn parses_android_14_am_crash() {
        let mut bytes = AM_CRASH_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, 9]);
        int(10, &mut bytes);
        int(4242, &mut bytes);
        string("com.example:worker", &mut bytes);
        int(0, &mut bytes);
        string("java.lang.IllegalStateException", &mut bytes);
        string("broken", &mut bytes);
        string("Worker.kt", &mut bytes);
        int(91, &mut bytes);
        int(1, &mut bytes);

        let parsed = parse_activity_event(parse_event_payload(&bytes).unwrap()).unwrap();
        assert_eq!(
            parsed,
            ActivityEvent::Crash(AmCrashEvent {
                user_id: 10,
                pid: 4242,
                process_name: "com.example:worker".to_owned(),
                flags: 0,
                exception_class: "java.lang.IllegalStateException".to_owned(),
                message: "broken".to_owned(),
                file: "Worker.kt".to_owned(),
                line: 91,
                recoverable: true,
            })
        );
    }

    #[test]
    fn parses_stable_five_field_am_anr() {
        let mut bytes = AM_ANR_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, 5]);
        int(0, &mut bytes);
        int(99, &mut bytes);
        string("com.example", &mut bytes);
        int(1, &mut bytes);
        string("Input dispatching timed out", &mut bytes);

        let parsed = parse_activity_event(parse_event_payload(&bytes).unwrap()).unwrap();
        assert!(matches!(parsed, ActivityEvent::Anr(event) if event.pid == 99));
    }

    #[test]
    fn rejects_truncated_string() {
        let mut bytes = 1_i32.to_le_bytes().to_vec();
        bytes.push(EVENT_TYPE_STRING);
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(b"short");
        assert!(matches!(
            parse_event_payload(&bytes),
            Err(ParseError::LengthOutOfBounds { .. })
        ));
    }
}
