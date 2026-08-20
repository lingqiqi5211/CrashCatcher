use cch_config::{AppConfig, AppConfigPatch, GlobalConfigPatch, MuteScope};
use cch_model::RecordId;
use serde::{Deserialize, Serialize};

use crate::{
    Page, PageRequest, WireError,
    dto::{
        AppConfigResult, AppEntry, DeleteTarget, DialogTakeoverResult, ExportFormat,
        ExportRedaction, GlobalConfigResult, GroupSummary, ModuleStatus, MuteResult, PayloadChunk,
        PayloadOpened, RecordDetail, RecordSummary, RuntimeLogFile, Stats,
    },
};

/// What the client is asking for.
///
/// Externally tagged by `method`, matching the interface table in the design
/// document. Adding a variant is backward compatible; an older daemon answers
/// [`crate::ErrorCode::InvalidRequest`] for a method it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Handshake {
        protocol_version: u32,
        client_version: String,
    },
    ModuleStatus,
    ListGroups {
        #[serde(default)]
        page: PageRequest,
    },
    /// One group by id, for a detail screen opened straight from a list row.
    ///
    /// Its own method rather than a filter on `list_groups`: the caller knows
    /// exactly which group it wants, and reaching it through a page would mean
    /// paging over a result set of one.
    GetGroup {
        group_id: String,
    },
    ListRecords {
        group_id: String,
        #[serde(default)]
        page: PageRequest,
    },
    GetRecord {
        id: RecordId,
    },
    /// Asks for a readable descriptor over the record's full text.
    OpenPayload {
        id: RecordId,
    },
    /// Fallback for hosts where descriptor passing is unavailable.
    ReadPayload {
        handle: u64,
        offset: u64,
        len: u32,
    },
    ClosePayload {
        handle: u64,
    },
    ExportRecords {
        ids: Vec<RecordId>,
        format: ExportFormat,
        #[serde(default)]
        redaction: ExportRedaction,
    },
    DeleteRecords {
        #[serde(flatten)]
        target: DeleteTarget,
    },
    GetGlobalConfig,
    SetGlobalConfig {
        patch: GlobalConfigPatch,
    },
    GetAppConfig {
        package_name: String,
    },
    SetAppConfig {
        package_name: String,
        patch: AppConfigPatch,
    },
    ListApps {
        #[serde(default)]
        include_system_apps: bool,
        /// Platform processes, which are not apps; see `CrashFilter::include_system_processes`.
        #[serde(default)]
        include_system_processes: bool,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        limit: u32,
    },
    Stats {
        #[serde(default)]
        time_from_ms: Option<i64>,
        #[serde(default)]
        time_to_ms: Option<i64>,
        /// Width of a trend bucket in ms. `0` lets the daemon choose.
        #[serde(default)]
        bucket_ms: i64,
    },
    ReopenApp {
        package_name: String,
        user_id: i32,
    },
    MuteApp {
        package_name: String,
        scope: MuteScope,
    },
    /// Takes down the notification posted for a crash.
    ///
    /// The manager cannot do this itself: the notification was posted by the privileged
    /// bridge, and a notification belongs to the process that posted it. So acting on one
    /// of its buttons left it sitting there afterwards, which read as the button having
    /// done nothing at all.
    DismissNotification {
        record_id: RecordId,
    },
    SetDialogTakeover {
        enabled: bool,
    },
    /// The tail of one of the daemon's log files, and a listing of the rest.
    ///
    /// Separate from the status, which is polled to draw a screen: this can be hundreds of
    /// kilobytes that nobody wants until they go looking. `name` comes from a previous
    /// [`Response::RuntimeLog`]; absent or unknown reads the newest file.
    ReadRuntimeLog {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        max_bytes: u64,
    },
}

impl Request {
    /// Whether the daemon should refuse this until a handshake has agreed on a protocol.
    ///
    /// True for everything except the handshake itself. The version check is worth nothing if a
    /// client can decline to ask: [`Self::Handshake`] is an ordinary variant, so without this the
    /// daemon would happily serve a manager that skipped it, or one that asked, was told the
    /// versions differ, and carried on anyway. Refusing is the daemon's job rather than the
    /// manager's — the whole point is that the two are not the same version.
    #[must_use]
    pub const fn requires_handshake(&self) -> bool {
        !matches!(self, Self::Handshake { .. })
    }
}

/// The successful answer to a [`Request`].
///
/// Every variant carries **named** fields, never a bare payload. An internally
/// tagged newtype variant would flatten the inner struct's fields next to the tag,
/// which is fine in Rust and a trap for any other language mirroring this: the
/// Kotlin client would have to inline the same fields by hand and re-inline them on
/// every change. Named fields nest cleanly on both sides.
///
/// **No tag here may equal a [`Request`] tag.** The three that naturally would —
/// handshake, module status, stats — carry a `_result` suffix instead. This is not
/// cosmetic: kotlinx.serialization compares serial descriptors by name, arity and
/// element *types*, ignoring property names, so `handshake` on both sides with the
/// same `(u32, String)` shape made two genuinely different types compare equal and
/// silently share a cached field-name mapping. The symptom was a required field
/// reported missing while it sat in the JSON. `no_response_tag_collides_with_a_request_tag`
/// keeps it from coming back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    #[serde(rename = "handshake_result")]
    Handshake {
        protocol_version: u32,
        daemon_version: String,
    },
    #[serde(rename = "module_status_result")]
    ModuleStatus {
        /// Boxed so one large variant does not set the size of every response.
        status: Box<ModuleStatus>,
    },
    Groups {
        page: Page<GroupSummary>,
    },
    Group {
        group: Box<GroupSummary>,
    },
    Records {
        page: Page<RecordSummary>,
    },
    Record {
        detail: Box<RecordDetail>,
    },
    PayloadOpened {
        payload: PayloadOpened,
    },
    PayloadChunk {
        chunk: PayloadChunk,
    },
    Closed,
    Export {
        text: String,
    },
    Deleted {
        removed_records: u64,
        removed_groups: u64,
    },
    GlobalConfig {
        result: Box<GlobalConfigResult>,
    },
    AppConfig {
        result: AppConfigResult,
    },
    Apps {
        apps: Vec<AppEntry>,
    },
    #[serde(rename = "stats_result")]
    Stats {
        stats: Box<Stats>,
    },
    Reopened {
        launched: bool,
    },
    /// False when the bridge was not connected and there was nothing to take down.
    NotificationDismissed {
        dismissed: bool,
    },
    Muted {
        result: MuteResult,
    },
    DialogTakeover {
        result: DialogTakeoverResult,
    },
    RuntimeLog {
        /// Which file this is, matching one of `files`.
        name: String,
        text: String,
        /// Whether anything was cut from the front to fit.
        truncated: bool,
        /// What this file weighs on disk, so a reader can see the tail is a tail.
        total_bytes: u64,
        /// Everything available, newest first. Travels with the content so switching files does
        /// not need a second round trip.
        files: Vec<RuntimeLogFile>,
    },
}

/// A request with its sequence number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub seq: u64,
    pub request: Request,
}

/// A response, carrying exactly one of `ok` or `err`.
///
/// Two optional fields rather than an externally tagged enum: serde would render
/// that as `{"ok":{…}}`, which no other language's default sealed-class encoding
/// matches. A pair of nullables is unambiguous everywhere, and
/// [`ResponseEnvelope::result`] restores the either-or guarantee for Rust callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Echoes the request's `seq`.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<Response>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<WireError>,
}

impl ResponseEnvelope {
    #[must_use]
    pub const fn ok(seq: u64, response: Response) -> Self {
        Self {
            seq,
            ok: Some(response),
            err: None,
        }
    }

    #[must_use]
    pub const fn err(seq: u64, error: WireError) -> Self {
        Self {
            seq,
            ok: None,
            err: Some(error),
        }
    }

    /// Interprets the envelope, rejecting one that carries neither or both.
    pub fn result(self) -> Result<Response, WireError> {
        match (self.ok, self.err) {
            (Some(response), None) => Ok(response),
            (None, Some(error)) => Err(error),
            (Some(_), Some(_)) => Err(WireError::malformed_frame(
                "response carries both ok and err",
            )),
            (None, None) => Err(WireError::malformed_frame(
                "response carries neither ok nor err",
            )),
        }
    }
}

/// Per-app config as it appears in a list, so the apps screen needs one round
/// trip rather than one per row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfigEntry {
    pub package_name: String,
    pub config: AppConfig,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CrashFilter, ErrorCode, SortKey};

    fn round_trip(request: &Request) -> Request {
        let json = serde_json::to_string(request).expect("serializes");
        serde_json::from_str(&json).expect("deserializes")
    }

    #[test]
    fn requests_are_tagged_by_method() {
        let json = serde_json::to_string(&Request::ModuleStatus).expect("serializes");
        assert_eq!(json, r#"{"method":"module_status"}"#);
    }

    /// Only the handshake may be served before there is one, and the cheapest-looking request is
    /// no exception: reading the module status without agreeing on a protocol is exactly what a
    /// mismatched manager would try next.
    #[test]
    fn only_the_handshake_is_served_before_a_handshake() {
        assert!(
            !Request::Handshake {
                protocol_version: 1,
                client_version: "0.1.0".into(),
            }
            .requires_handshake()
        );
        assert!(Request::ModuleStatus.requires_handshake());
        assert!(
            Request::ListGroups {
                page: PageRequest::default(),
            }
            .requires_handshake()
        );
        assert!(
            Request::SetDialogTakeover { enabled: true }.requires_handshake(),
            "a write least of all"
        );
    }

    #[test]
    fn every_request_variant_round_trips() {
        let requests = vec![
            Request::Handshake {
                protocol_version: 1,
                client_version: "0.1.0".into(),
            },
            Request::ModuleStatus,
            Request::ListGroups {
                page: PageRequest {
                    filter: CrashFilter {
                        packages: vec!["com.example.app".into()],
                        ..CrashFilter::default()
                    },
                    sort: SortKey::OccurrenceDesc,
                    cursor: None,
                    limit: 25,
                },
            },
            Request::ListRecords {
                group_id: "abc".into(),
                page: PageRequest::default(),
            },
            Request::OpenPayload { id: sample_id() },
            Request::ReadPayload {
                handle: 7,
                offset: 0,
                len: 1024,
            },
            Request::ClosePayload { handle: 7 },
            Request::DeleteRecords {
                target: DeleteTarget::All,
            },
            Request::SetGlobalConfig {
                patch: GlobalConfigPatch::default(),
            },
            Request::SetAppConfig {
                package_name: "com.example.app".into(),
                patch: AppConfigPatch::default(),
            },
            Request::MuteApp {
                package_name: "com.example.app".into(),
                scope: MuteScope::UntilUnlock,
            },
            Request::SetDialogTakeover { enabled: true },
        ];

        for request in requests {
            assert_eq!(round_trip(&request), request);
        }
    }

    #[test]
    fn delete_target_is_flattened_alongside_the_method() {
        let json = serde_json::to_string(&Request::DeleteRecords {
            target: DeleteTarget::Group {
                group_id: "g1".into(),
            },
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"method":"delete_records","target":"group","group_id":"g1"}"#
        );
    }

    #[test]
    fn optional_request_fields_may_be_omitted() {
        let request: Request =
            serde_json::from_str(r#"{"method":"list_groups"}"#).expect("page may be omitted");
        assert_eq!(
            request,
            Request::ListGroups {
                page: PageRequest::default()
            }
        );

        let stats: Request =
            serde_json::from_str(r#"{"method":"stats"}"#).expect("stats args may be omitted");
        assert_eq!(
            stats,
            Request::Stats {
                time_from_ms: None,
                time_to_ms: None,
                bucket_ms: 0,
            }
        );
    }

    #[test]
    fn an_unknown_method_is_a_parse_failure_the_daemon_can_report() {
        let parsed = serde_json::from_str::<Request>(r#"{"method":"invented_later"}"#);
        assert!(
            parsed.is_err(),
            "an older daemon must notice rather than silently do nothing"
        );
    }

    #[test]
    fn envelopes_keep_ok_and_err_distinguishable() {
        let ok_json =
            serde_json::to_string(&ResponseEnvelope::ok(1, Response::Closed)).expect("serializes");
        let err_json =
            serde_json::to_string(&ResponseEnvelope::err(1, WireError::not_found("gone")))
                .expect("serializes");

        // Exactly one key is present in each, so a client cannot read past the one
        // it forgot to check.
        assert!(ok_json.contains(r#""ok""#) && !ok_json.contains(r#""err""#));
        assert!(err_json.contains(r#""err""#) && !err_json.contains(r#""ok""#));

        let parsed: ResponseEnvelope = serde_json::from_str(&err_json).expect("deserializes");
        assert_eq!(
            parsed.result().map(|_| ()).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn an_envelope_with_neither_or_both_is_refused() {
        let neither = ResponseEnvelope {
            seq: 1,
            ok: None,
            err: None,
        };
        assert_eq!(
            neither.result().map(|_| ()).unwrap_err().code,
            ErrorCode::MalformedFrame
        );

        let both = ResponseEnvelope {
            seq: 1,
            ok: Some(Response::Closed),
            err: Some(WireError::not_found("gone")),
        };
        assert_eq!(
            both.result().map(|_| ()).unwrap_err().code,
            ErrorCode::MalformedFrame
        );
    }

    #[test]
    fn response_variants_nest_their_payload_under_a_name() {
        let json = serde_json::to_string(&Response::Deleted {
            removed_records: 3,
            removed_groups: 1,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"response":"deleted","removed_records":3,"removed_groups":1}"#
        );

        // The nested form is what keeps the Kotlin mirror mechanical.
        let apps = serde_json::to_string(&Response::Apps { apps: Vec::new() }).expect("serializes");
        assert_eq!(apps, r#"{"response":"apps","apps":[]}"#);
    }

    #[test]
    fn sequence_numbers_survive_the_round_trip() {
        let envelope = RequestEnvelope {
            seq: 42,
            request: Request::ModuleStatus,
        };
        let json = serde_json::to_string(&envelope).expect("serializes");
        let parsed: RequestEnvelope = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed.seq, 42);
    }

    fn sample_id() -> RecordId {
        cch_model::RecordIdGenerator::new().next(1_755_440_000_123)
    }
}
