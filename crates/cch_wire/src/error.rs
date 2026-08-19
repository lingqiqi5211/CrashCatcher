use std::fmt;

use serde::{Deserialize, Serialize};

/// Machine-readable failure classes.
///
/// The UI branches on these rather than on message text, so the wording stays
/// free to change and can be localized on the Kotlin side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Missing, contradictory or out-of-range parameters.
    InvalidRequest,
    /// The frame itself did not parse; the connection is no longer trustworthy.
    MalformedFrame,
    /// Peer failed the uid → package → signing-pin check.
    Unauthorized,
    NotFound,
    PayloadTooLarge,
    /// A dependency the request needs is down (the bridge, a collector).
    Unavailable,
    /// Client and daemon disagree on [`crate::PROTOCOL_VERSION`].
    VersionMismatch,
    /// The page cursor was issued for a different sort order.
    ///
    /// Its own code on purpose: silently returning a scrambled page is worse
    /// than telling the client to restart the query.
    CursorInvalidated,
    Internal,
}

impl ErrorCode {
    /// Whether retrying the same request unchanged could plausibly succeed.
    #[must_use]
    pub const fn is_transient(self) -> bool {
        matches!(self, Self::Unavailable | Self::Internal)
    }
}

/// An error as it travels on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    /// Developer-facing detail. Never shown to users verbatim — the UI picks its
    /// own string from `code`.
    pub message: String,
}

impl WireError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    pub fn malformed_frame(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::MalformedFrame, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn cursor_invalidated(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::CursorInvalidated, message)
    }

    #[must_use]
    pub fn payload_too_large(bytes: u64) -> Self {
        Self::new(
            ErrorCode::PayloadTooLarge,
            format!("frame body of {bytes} bytes exceeds the limit"),
        )
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for WireError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_snake_case_on_the_wire() {
        let json = serde_json::to_string(&ErrorCode::CursorInvalidated).expect("serializes");
        assert_eq!(json, r#""cursor_invalidated""#);
    }

    #[test]
    fn errors_round_trip() {
        let error = WireError::not_found("no record 01J");
        let json = serde_json::to_string(&error).expect("serializes");
        let parsed: WireError = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, error);
    }

    #[test]
    fn only_unavailable_and_internal_are_worth_retrying() {
        assert!(ErrorCode::Unavailable.is_transient());
        assert!(ErrorCode::Internal.is_transient());
        for code in [
            ErrorCode::InvalidRequest,
            ErrorCode::MalformedFrame,
            ErrorCode::Unauthorized,
            ErrorCode::NotFound,
            ErrorCode::PayloadTooLarge,
            ErrorCode::VersionMismatch,
            ErrorCode::CursorInvalidated,
        ] {
            assert!(
                !code.is_transient(),
                "{code:?} must not invite a blind retry"
            );
        }
    }
}
