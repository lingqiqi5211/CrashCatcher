//! The daemon ↔ manager protocol: framing, requests, responses, events, and the
//! shared query vocabulary.
//!
//! Both sides of the socket are generated from what is defined here, and the
//! Kotlin client mirrors it field for field. Two rules keep the protocol
//! extensible:
//!
//! - **Unknown fields are ignored**, never rejected, so a newer peer can add one.
//! - **Unknown *event* kinds are skipped** ([`Event::parse_lenient`]), so adding a
//!   push does not break older clients.
//!
//! Bulk payloads never travel in a frame. `open_payload` hands over a descriptor
//! instead, which is what keeps a multi-megabyte ANR dump from having to be
//! chunked, escaped, and reassembled.

#![forbid(unsafe_code)]

mod bridge;
mod dto;
mod error;
mod event;
mod frame;
mod query;
mod rpc;

pub use bridge::{
    BridgeAction, BridgeCommand, BridgeEvent, BridgeHello, BridgePackageInfo, IntentSpec,
    NotificationAction, NotificationSpec,
};
pub use dto::{
    AppConfigResult, AppEntry, BridgeFacts, CollectorHealth, CollectorSource, DeleteTarget,
    DialogTakeoverResult, DialogTakeoverStatus, ExceptionCount, ExportFormat, ExportRedaction,
    GlobalConfigResult, GroupSummary, KindCount, MAX_PAYLOAD_CHUNK_BYTES, ModuleStatus, MuteResult,
    PackageCount, PackageIndexFacts, PayloadChunk, PayloadOpened, RecordDetail, RecordSummary,
    RuntimeFacts, RuntimeLogFile, Stats, StorageStatus, TrendBucket,
};
pub use error::{ErrorCode, WireError};
pub use event::Event;
pub use frame::{
    BRIDGE_SOCKET_NAME, ChannelKind, LENGTH_PREFIX_BYTES, MANAGER_SOCKET_NAME,
    MAX_FRAME_BODY_BYTES, PROTOCOL_VERSION, decode_frame, decode_length, encode_frame,
};
pub use query::{
    CrashFilter, Cursor, CursorAnchor, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, Page, PageRequest,
    SortKey,
};
pub use rpc::{AppConfigEntry, Request, RequestEnvelope, Response, ResponseEnvelope};
