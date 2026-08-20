use serde::{Deserialize, Serialize};

use crate::{Cursor, ParseError};

pub const AM_ANR_TAG: i32 = 30_008;
pub const AM_CRASH_TAG: i32 = 30_039;
pub const WM_SET_KEYGUARD_SHOWN_TAG: i32 = 30_067;
pub const SCREEN_TOGGLED_TAG: i32 = 70_000;

const EVENT_TYPE_INT: u8 = 0;
const EVENT_TYPE_LONG: u8 = 1;
const EVENT_TYPE_STRING: u8 = 2;
const EVENT_TYPE_LIST: u8 = 3;
const EVENT_TYPE_FLOAT: u8 = 4;
const MAX_EVENT_DEPTH: usize = 16;
const MAX_EVENT_ELEMENTS: usize = 255;
const MAX_EVENT_STRING_BYTES: usize = 64 * 1024;
/// Android user ids stay small even on devices with work profiles and cloned apps.
const MAX_PLAUSIBLE_USER_ID: i32 = 999;

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

/// Something in the events buffer that ends a "muted until unlock" window.
///
/// Read from the events buffer because the privileged bridge cannot deliver it:
/// `registerReceiver` on a bare `app_process`'s reflected system context returns without
/// registering anything — the process never attached an `ApplicationThread` to the activity
/// manager, so `dumpsys activity broadcasts` lists no receiver for it and `ACTION_USER_PRESENT`
/// has nowhere to arrive. The events buffer is already open for crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenEvent {
    /// Keyguard stopped showing, so the user is past the lock screen.
    Unlocked,
    /// The display went off; reaching the device again has to pass the lock screen.
    ScreenOff,
}

/// Recognises the two events above, and `None` for everything else.
///
/// Screen-on and keyguard-appeared deliberately return `None`: waking a locked phone is not
/// the user getting back to it.
pub fn parse_screen_event(record: &EventRecord) -> Option<ScreenEvent> {
    match record.tag {
        // `(screen_state|1|5)` — a bare int, not a list, because it is a single field.
        SCREEN_TOGGLED_TAG => match record.value {
            EventValue::Int(0) => Some(ScreenEvent::ScreenOff),
            _ => None,
        },
        WM_SET_KEYGUARD_SHOWN_TAG => keyguard_showing(&record.value)
            .and_then(|showing| (!showing).then_some(ScreenEvent::Unlocked)),
        _ => None,
    }
}

/// Reads `keyguardShowing`, whose position moved between releases.
///
/// Android 11 logs `(keyguardShowing),(aodShowing),(Reason)`, 12 inserts
/// `(keyguardGoingAway)`, and 14 both prepends `(Display Id)` and adds `(occluded)`. Every
/// shape ends with the reason string, so the field count is the only thing that tells them
/// apart; an unrecognised one gives up rather than guessing, which leaves the mute in place.
fn keyguard_showing(value: &EventValue) -> Option<bool> {
    let EventValue::List(values) = value else {
        return None;
    };
    let index = match values.len() {
        3 | 4 => 0,
        6 => 1,
        _ => return None,
    };
    match values.get(index)? {
        EventValue::Int(showing) => Some(*showing != 0),
        _ => None,
    }
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
    let (user_id, pid) = activity_user_and_pid(values)?;
    Ok(AmCrashEvent {
        user_id,
        pid,
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
    let (user_id, pid) = activity_user_and_pid(values)?;
    Ok(AmAnrEvent {
        user_id,
        pid,
        process_name: string_at(values, 2, "process_name")?,
        flags: int_at(values, 3, "flags")?,
        reason: string_at(values, 4, "reason")?,
    })
}

/// Reads the first two `am_crash` / `am_anr` fields across platform variants.
///
/// AOSP's event-log tag still declares `(User, PID)`, but some Android 16 builds emit
/// `(PID, User)` while keeping that stale declaration. Treat the unambiguous large/small
/// pair as swapped and retain the documented order for ambiguous values. Without this,
/// the event fragment carries pid 0, cannot join the crash-buffer fragment, and the same
/// crash is stored and announced twice after the full merge window.
fn activity_user_and_pid(values: &[EventValue]) -> Result<(i32, i32), ParseError> {
    let first = int_at(values, 0, "user_or_pid")?;
    let second = int_at(values, 1, "pid_or_user")?;
    if first > MAX_PLAUSIBLE_USER_ID && (0..=MAX_PLAUSIBLE_USER_ID).contains(&second) {
        Ok((second, first))
    } else {
        Ok((first, second))
    }
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
    fn parses_pid_first_android_16_am_crash() {
        let mut bytes = AM_CRASH_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, 9]);
        int(4242, &mut bytes);
        int(10, &mut bytes);
        string("com.example:worker", &mut bytes);
        int(0, &mut bytes);
        string("java.lang.IllegalStateException", &mut bytes);
        string("broken", &mut bytes);
        string("Worker.kt", &mut bytes);
        int(91, &mut bytes);
        int(0, &mut bytes);

        let parsed = parse_activity_event(parse_event_payload(&bytes).unwrap()).unwrap();
        assert!(matches!(
            parsed,
            ActivityEvent::Crash(AmCrashEvent {
                user_id: 10,
                pid: 4242,
                ..
            })
        ));
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
    fn parses_pid_first_android_16_am_anr() {
        let mut bytes = AM_ANR_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, 5]);
        int(4242, &mut bytes);
        int(10, &mut bytes);
        string("com.example", &mut bytes);
        int(1, &mut bytes);
        string("Input dispatching timed out", &mut bytes);

        let parsed = parse_activity_event(parse_event_payload(&bytes).unwrap()).unwrap();
        assert!(matches!(
            parsed,
            ActivityEvent::Anr(AmAnrEvent {
                user_id: 10,
                pid: 4242,
                ..
            })
        ));
    }

    /// The six-field shape as logged by the device this was traced on:
    /// `wm_set_keyguard_shown: [0,0,0,1,0,setKeyguardShown]` right after a real unlock.
    fn keyguard_shown(fields: &[i32]) -> EventRecord {
        let mut bytes = WM_SET_KEYGUARD_SHOWN_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, (fields.len() + 1) as u8]);
        for field in fields {
            int(*field, &mut bytes);
        }
        string("setKeyguardShown", &mut bytes);
        parse_event_payload(&bytes).unwrap()
    }

    fn screen_toggled(state: i32) -> EventRecord {
        let mut bytes = SCREEN_TOGGLED_TAG.to_le_bytes().to_vec();
        int(state, &mut bytes);
        parse_event_payload(&bytes).unwrap()
    }

    #[test]
    fn keyguard_going_away_is_an_unlock_on_every_field_layout() {
        // Android 14+: the display id shifts `keyguardShowing` to index 1.
        assert_eq!(
            parse_screen_event(&keyguard_shown(&[0, 0, 0, 1, 0])),
            Some(ScreenEvent::Unlocked)
        );
        // Android 12: keyguardShowing, aodShowing, keyguardGoingAway.
        assert_eq!(
            parse_screen_event(&keyguard_shown(&[0, 0, 1])),
            Some(ScreenEvent::Unlocked)
        );
        // Android 11: keyguardShowing, aodShowing.
        assert_eq!(
            parse_screen_event(&keyguard_shown(&[0, 0])),
            Some(ScreenEvent::Unlocked)
        );
    }

    #[test]
    fn keyguard_appearing_does_not_end_a_mute() {
        // The same unlock also logs showing=1 twice while the lock screen is still up.
        assert_eq!(parse_screen_event(&keyguard_shown(&[0, 1, 1, 0, 0])), None);
        assert_eq!(parse_screen_event(&keyguard_shown(&[0, 1, 0, 1, 0])), None);
    }

    #[test]
    fn an_unknown_field_layout_leaves_the_mute_alone() {
        assert_eq!(
            parse_screen_event(&keyguard_shown(&[0, 0, 0, 0, 0, 0, 0])),
            None
        );
    }

    #[test]
    fn only_screen_off_ends_a_mute() {
        assert_eq!(
            parse_screen_event(&screen_toggled(0)),
            Some(ScreenEvent::ScreenOff)
        );
        assert_eq!(parse_screen_event(&screen_toggled(1)), None);
    }

    #[test]
    fn a_crash_is_not_a_screen_event() {
        let mut bytes = AM_ANR_TAG.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[EVENT_TYPE_LIST, 5]);
        int(0, &mut bytes);
        int(99, &mut bytes);
        string("com.example", &mut bytes);
        int(1, &mut bytes);
        string("stuck", &mut bytes);
        assert_eq!(
            parse_screen_event(&parse_event_payload(&bytes).unwrap()),
            None
        );
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
