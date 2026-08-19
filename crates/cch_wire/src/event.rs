use serde::{Deserialize, Serialize};

use crate::{
    WireError,
    dto::{GroupSummary, ModuleStatus, RecordSummary},
};

/// Wire tags of the events this build understands.
///
/// Used to tell "an event from a newer daemon" apart from "a corrupt frame", so
/// only the latter is treated as a problem.
const KNOWN_EVENTS: &[&str] = &[
    "crash_recorded",
    "config_changed",
    "module_status_changed",
    "dropped",
];

/// A one-way push from the daemon on the subscribe lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    CrashRecorded {
        record: RecordSummary,
        group: GroupSummary,
        /// First time this fingerprint has been seen, so the list gains a row
        /// rather than incrementing one.
        is_new_group: bool,
    },
    /// Configuration changed — including by another client — so re-read it.
    ConfigChanged,
    ModuleStatusChanged(ModuleStatus),
    /// Events were coalesced away because the client could not keep up.
    ///
    /// Reported rather than dropped silently: a list that quietly misses rows
    /// during a crash storm is worse than one that says it did.
    Dropped {
        count: u64,
        since_ms: i64,
    },
}

impl Event {
    /// Parses an event, skipping kinds this build does not know.
    ///
    /// `Ok(None)` means "a newer daemon sent something newer" — the client must
    /// ignore it and keep the connection. Only a genuinely malformed frame is an
    /// error, because dropping the subscription on an unknown tag would make every
    /// future protocol addition a breaking change for old clients.
    pub fn parse_lenient(json: &str) -> Result<Option<Self>, WireError> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|source| WireError::malformed_frame(format!("event is not JSON: {source}")))?;

        let tag = value
            .get("event")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| WireError::malformed_frame("event frame has no `event` tag"))?
            .to_owned();

        if !KNOWN_EVENTS.contains(&tag.as_str()) {
            return Ok(None);
        }

        serde_json::from_value(value).map(Some).map_err(|source| {
            WireError::malformed_frame(format!("event {tag} did not parse: {source}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn events_are_tagged_by_event() {
        let json = serde_json::to_string(&Event::ConfigChanged).expect("serializes");
        assert_eq!(json, r#"{"event":"config_changed"}"#);
    }

    #[test]
    fn known_events_round_trip() {
        let event = Event::Dropped {
            count: 12,
            since_ms: 1_755_440_000_000,
        };
        let json = serde_json::to_string(&event).expect("serializes");
        assert_eq!(Event::parse_lenient(&json).expect("parses"), Some(event));
    }

    #[test]
    fn an_event_from_a_newer_daemon_is_skipped_not_fatal() {
        let parsed = Event::parse_lenient(r#"{"event":"invented_later","payload":{"x":1}}"#)
            .expect("an unknown event must not be an error");
        assert_eq!(parsed, None);
    }

    #[test]
    fn a_known_event_with_extra_fields_still_parses() {
        let parsed = Event::parse_lenient(
            r#"{"event":"dropped","count":3,"since_ms":1,"added_later":true}"#,
        )
        .expect("unknown fields are ignored");
        assert_eq!(
            parsed,
            Some(Event::Dropped {
                count: 3,
                since_ms: 1
            })
        );
    }

    #[test]
    fn malformed_frames_are_reported() {
        assert_eq!(
            Event::parse_lenient("not json")
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::MalformedFrame
        );
        assert_eq!(
            Event::parse_lenient(r#"{"no_tag":true}"#)
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::MalformedFrame
        );
    }

    #[test]
    fn a_known_event_missing_required_fields_is_an_error() {
        // Distinct from the unknown-tag case: we know this event and it is broken.
        assert_eq!(
            Event::parse_lenient(r#"{"event":"dropped"}"#)
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::MalformedFrame
        );
    }
}
