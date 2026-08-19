//! Cross-language wire vectors.
//!
//! The literals here are the same ones the Kotlin client asserts, and both are
//! copied from `docs/wire-vectors.md`. Changing the protocol should turn *both*
//! suites red; if only one goes red, the two sides have drifted.
//!
//! Comparison is on the parsed JSON tree with null-valued keys stripped, because on
//! this wire null and absent mean the same thing (`Option` + `serde(default)` on one
//! side, `explicitNulls = false` on the other). The single documented exception —
//! `AppConfigPatch::notify_mode` — is asserted on its exact shape instead.

#![allow(clippy::expect_used)]

use cch_config::{AppConfigPatch, MuteScope, NotifyMode};
use cch_model::CrashKind;
use cch_wire::{
    ChannelKind, CrashFilter, Cursor, CursorAnchor, Event, PageRequest, Request, RequestEnvelope,
    Response, ResponseEnvelope, SortKey, WireError,
};
use serde_json::{Value, json};

/// Removes null-valued keys so "absent" and "null" compare equal.
fn normalize(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            map.retain(|_, entry| !entry.is_null());
            let normalized = map
                .iter()
                .map(|(key, entry)| (key.clone(), normalize(entry.clone())))
                .collect();
            Value::Object(normalized)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| normalize(item.clone())).collect())
        }
        _ => value,
    }
}

#[track_caller]
fn assert_serializes_to<T: serde::Serialize>(value: &T, expected: Value) {
    let actual = serde_json::to_value(value).expect("serializes");
    assert_eq!(normalize(actual), normalize(expected));
}

#[test]
fn channel_hello_vectors() {
    assert_serializes_to(&ChannelKind::Control, json!({"kind": "control"}));
    assert_serializes_to(&ChannelKind::Subscribe, json!({"kind": "subscribe"}));
}

#[test]
fn list_groups_request_vector() {
    let envelope = RequestEnvelope {
        seq: 7,
        request: Request::ListGroups {
            page: PageRequest {
                filter: CrashFilter {
                    packages: vec!["com.example.app".to_owned()],
                    kinds: vec![CrashKind::Anr],
                    ..CrashFilter::default()
                },
                sort: SortKey::OccurrenceDesc,
                cursor: None,
                limit: 25,
            },
        },
    };

    assert_serializes_to(
        &envelope,
        json!({
            "seq": 7,
            "request": {
                "method": "list_groups",
                "page": {
                    "filter": {
                        "packages": ["com.example.app"],
                        "kinds": ["anr"],
                        "user_ids": [],
                        "include_system_apps": false,
                        "only_main_process": false,
                        "only_self_handled": false
                    },
                    "sort": "occurrence_desc",
                    "limit": 25
                }
            }
        }),
    );
}

#[test]
fn delete_records_flattens_its_target_next_to_the_method() {
    let envelope = RequestEnvelope {
        seq: 8,
        request: Request::DeleteRecords {
            target: cch_wire::DeleteTarget::Group {
                group_id: "0123456789abcdef0123456789abcdef".to_owned(),
            },
        },
    };

    assert_serializes_to(
        &envelope,
        json!({
            "seq": 8,
            "request": {
                "method": "delete_records",
                "target": "group",
                "group_id": "0123456789abcdef0123456789abcdef"
            }
        }),
    );
}

#[test]
fn the_three_notify_mode_patch_states_have_distinct_shapes() {
    // This is the one place where null and absent mean different things, so the
    // exact shape is asserted rather than normalized.
    let unchanged = serde_json::to_value(AppConfigPatch::default()).expect("serializes");
    assert_eq!(
        unchanged.get("notify_mode"),
        None,
        "an untouched patch must omit the key, not send null"
    );

    let follow_global = AppConfigPatch {
        notify_mode: Some(None),
        ..AppConfigPatch::default()
    };
    assert_eq!(
        serde_json::to_value(follow_global)
            .expect("serializes")
            .get("notify_mode"),
        Some(&Value::Null),
        "clearing the override must send an explicit null"
    );

    let set_to = AppConfigPatch {
        notify_mode: Some(Some(NotifyMode::Toast)),
        ..AppConfigPatch::default()
    };
    assert_eq!(
        serde_json::to_value(set_to)
            .expect("serializes")
            .get("notify_mode"),
        Some(&Value::String("toast".to_owned()))
    );
}

#[test]
fn set_app_config_request_vectors() {
    let request = |patch: AppConfigPatch, seq: u64| RequestEnvelope {
        seq,
        request: Request::SetAppConfig {
            package_name: "com.example.app".to_owned(),
            patch,
        },
    };

    assert_serializes_to(
        &request(AppConfigPatch::default(), 9),
        json!({
            "seq": 9,
            "request": {
                "method": "set_app_config",
                "package_name": "com.example.app",
                "patch": {}
            }
        }),
    );

    // Normalizing would erase the meaningful null here, so compare it directly.
    let cleared = serde_json::to_value(request(
        AppConfigPatch {
            notify_mode: Some(None),
            ..AppConfigPatch::default()
        },
        10,
    ))
    .expect("serializes");
    assert_eq!(cleared["request"]["patch"]["notify_mode"], Value::Null);

    assert_serializes_to(
        &request(
            AppConfigPatch {
                notify_mode: Some(Some(NotifyMode::Toast)),
                ..AppConfigPatch::default()
            },
            11,
        ),
        json!({
            "seq": 11,
            "request": {
                "method": "set_app_config",
                "package_name": "com.example.app",
                "patch": {"notify_mode": "toast"}
            }
        }),
    );
}

#[test]
fn mute_request_vector() {
    assert_serializes_to(
        &RequestEnvelope {
            seq: 13,
            request: Request::MuteApp {
                package_name: "com.example.app".to_owned(),
                scope: MuteScope::UntilUnlock,
            },
        },
        json!({
            "seq": 13,
            "request": {
                "method": "mute_app",
                "package_name": "com.example.app",
                "scope": "until_unlock"
            }
        }),
    );
}

#[test]
fn response_envelope_vectors() {
    assert_serializes_to(
        &ResponseEnvelope::ok(
            8,
            Response::Deleted {
                removed_records: 3,
                removed_groups: 1,
            },
        ),
        json!({
            "seq": 8,
            "ok": {"response": "deleted", "removed_records": 3, "removed_groups": 1}
        }),
    );

    assert_serializes_to(
        &ResponseEnvelope::err(
            8,
            WireError::cursor_invalidated(
                "cursor was issued for LastSeenDesc but the request sorts by PackageAsc",
            ),
        ),
        json!({
            "seq": 8,
            "err": {
                "code": "cursor_invalidated",
                "message": "cursor was issued for LastSeenDesc but the request sorts by PackageAsc"
            }
        }),
    );

    assert_serializes_to(
        &ResponseEnvelope::ok(12, Response::Apps { apps: Vec::new() }),
        json!({"seq": 12, "ok": {"response": "apps", "apps": []}}),
    );
}

#[test]
fn response_tags_that_would_collide_with_a_request_carry_a_result_suffix() {
    // Not cosmetic. kotlinx.serialization compares serial descriptors by name,
    // arity and element *types*, ignoring property names — so `handshake` on both
    // sides, each `(u32, String)`, compared equal and shared one cached field-name
    // mapping. The Kotlin client then reported `daemon_version` missing while it
    // was sitting in the payload.
    assert_serializes_to(
        &Response::Handshake {
            protocol_version: 1,
            daemon_version: "0.1.0".to_owned(),
        },
        json!({
            "response": "handshake_result",
            "protocol_version": 1,
            "daemon_version": "0.1.0"
        }),
    );

    let stats = serde_json::to_value(Response::Stats {
        stats: Box::new(cch_wire::Stats {
            total: 0,
            by_kind: Vec::new(),
            top_packages: Vec::new(),
            top_exceptions: Vec::new(),
            trend: Vec::new(),
            crashed_app_count: 0,
            installed_app_count: 0,
        }),
    })
    .expect("serializes");
    assert_eq!(stats["response"], "stats_result");
}

#[test]
fn cursor_vectors() {
    assert_eq!(
        Cursor::new(
            SortKey::LastSeenDesc,
            CursorAnchor::Int(1_755_440_000_123),
            "0123456789abcdef0123456789abcdef",
        )
        .to_string(),
        "1|last_seen_desc|i|1755440000123|0123456789abcdef0123456789abcdef"
    );

    assert_eq!(
        Cursor::new(
            SortKey::PackageAsc,
            CursorAnchor::Text("com.example.app".to_owned()),
            "0123456789abcdef0123456789abcdef",
        )
        .to_string(),
        "1|package_asc|t|com.example.app|0123456789abcdef0123456789abcdef"
    );
}

#[test]
fn event_vectors() {
    assert_serializes_to(&Event::ConfigChanged, json!({"event": "config_changed"}));
    assert_serializes_to(
        &Event::Dropped {
            count: 12,
            since_ms: 1_755_440_000_000,
        },
        json!({"event": "dropped", "count": 12, "since_ms": 1_755_440_000_000i64}),
    );
}

#[test]
fn an_unknown_event_is_skipped_and_a_broken_known_one_is_not() {
    assert_eq!(
        Event::parse_lenient(r#"{"event":"invented_later","whatever":1}"#).expect("skips"),
        None
    );
    assert!(
        Event::parse_lenient(r#"{"event":"dropped"}"#).is_err(),
        "a known event missing its fields must be reported"
    );
}
