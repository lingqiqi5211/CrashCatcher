use serde::{Deserialize, Serialize};

use crate::WireError;

/// Abstract-namespace socket the manager app connects to.
///
/// Passed verbatim to `LocalSocketAddress` with `Namespace.ABSTRACT` on the
/// Kotlin side; Android prepends the conventional NUL byte on the wire.
pub const MANAGER_SOCKET_NAME: &str = "crash_catcher_daemon_manager";

/// Abstract-namespace socket the privileged Java bridge connects to.
pub const BRIDGE_SOCKET_NAME: &str = "crash_catcher_daemon_bridge";

/// Bytes of big-endian length prefix in front of every frame body.
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Largest permitted frame body.
///
/// Bulk payloads never travel in a frame — they are handed over as a file
/// descriptor — so this only has to fit a page of list rows.
pub const MAX_FRAME_BODY_BYTES: usize = 1024 * 1024;

/// Protocol version carried in the handshake.
///
/// **Bump whenever one side's change requires the other to be updated with it** — not only
/// for a breaking reshape. A purely additive request looks harmless because the old side
/// ignores what it does not recognise, but the result is a manager whose button silently
/// does nothing against a module from last week. A refused handshake naming both versions is
/// a better answer than a feature that quietly is not there.
///
/// It is also the signal CI reads: a bump means the APK and the module must be built and
/// shipped together, and a change that leaves this alone is one either half can take on its
/// own.
///
/// 2: `dismiss_notification` — the manager cannot take down a notification the bridge posted,
///    so acting on one needs a daemon that forwards the cancel. Also `package_installed` on
///    `GroupSummary` and `AppEntry`, and the platform-process filter on `list_apps`: the daemon
///    now tells apps and platform processes apart, and a manager that cannot read that would
///    present `/vendor/bin/hw/…` as an app and offer to launch it.
pub const PROTOCOL_VERSION: u32 = 2;

/// Which lane a connection is, decided by its very first frame.
///
/// One socket serves both so the manager needs a single connect path; the daemon
/// dispatches on this discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelKind {
    /// Request/response, every frame carrying a `seq`.
    Control,
    /// One-way pushes from the daemon.
    Subscribe,
}

/// Encodes a body into a length-prefixed frame.
pub fn encode_frame(body: &[u8]) -> Result<Vec<u8>, WireError> {
    if body.len() > MAX_FRAME_BODY_BYTES {
        return Err(WireError::payload_too_large(body.len() as u64));
    }
    let mut frame = Vec::with_capacity(LENGTH_PREFIX_BYTES + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Reads the declared body length out of a 4-byte prefix.
pub fn decode_length(prefix: &[u8; LENGTH_PREFIX_BYTES]) -> Result<usize, WireError> {
    let length = u32::from_be_bytes(*prefix) as u64;
    if length > MAX_FRAME_BODY_BYTES as u64 {
        return Err(WireError::payload_too_large(length));
    }
    Ok(length as usize)
}

/// Splits a complete frame into its body, rejecting a mismatched prefix.
pub fn decode_frame(frame: &[u8]) -> Result<&[u8], WireError> {
    if frame.len() < LENGTH_PREFIX_BYTES {
        return Err(WireError::malformed_frame(
            "frame is shorter than its length prefix",
        ));
    }
    let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
    prefix.copy_from_slice(&frame[..LENGTH_PREFIX_BYTES]);
    let declared = decode_length(&prefix)?;
    let body = &frame[LENGTH_PREFIX_BYTES..];
    if body.len() != declared {
        return Err(WireError::malformed_frame(format!(
            "frame body length mismatch: declared {declared} bytes, got {} bytes",
            body.len()
        )));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn frames_round_trip() {
        let body = br#"{"seq":1}"#;
        let frame = encode_frame(body).expect("small body encodes");
        assert_eq!(
            &frame[..LENGTH_PREFIX_BYTES],
            &(body.len() as u32).to_be_bytes()
        );
        assert_eq!(decode_frame(&frame).expect("round trip"), body);
    }

    #[test]
    fn empty_body_is_legal() {
        let frame = encode_frame(&[]).expect("empty body encodes");
        assert_eq!(frame, vec![0, 0, 0, 0]);
        assert_eq!(decode_frame(&frame).expect("round trip"), &[] as &[u8]);
    }

    #[test]
    fn length_prefix_is_big_endian() {
        // 0x0102 == 258, so the prefix must read 00 00 01 02, not 02 01 00 00.
        let frame = encode_frame(&vec![0u8; 258]).expect("encodes");
        assert_eq!(&frame[..4], &[0x00, 0x00, 0x01, 0x02]);
    }

    #[test]
    fn oversized_bodies_are_refused_on_both_sides() {
        let too_big = vec![0u8; MAX_FRAME_BODY_BYTES + 1];
        assert_eq!(
            encode_frame(&too_big).map(|_| ()).unwrap_err().code,
            ErrorCode::PayloadTooLarge
        );

        let lying_prefix = ((MAX_FRAME_BODY_BYTES + 1) as u32).to_be_bytes();
        assert_eq!(
            decode_length(&lying_prefix).map(|_| ()).unwrap_err().code,
            ErrorCode::PayloadTooLarge
        );
    }

    #[test]
    fn truncated_and_mismatched_frames_are_rejected() {
        assert_eq!(
            decode_frame(&[0, 0]).map(|_| ()).unwrap_err().code,
            ErrorCode::MalformedFrame
        );
        // Declares 4 bytes, carries 2.
        assert_eq!(
            decode_frame(&[0, 0, 0, 4, 1, 2])
                .map(|_| ())
                .unwrap_err()
                .code,
            ErrorCode::MalformedFrame
        );
    }

    #[test]
    fn channel_kind_wire_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&ChannelKind::Control).expect("serializes"),
            r#"{"kind":"control"}"#
        );
        assert_eq!(
            serde_json::to_string(&ChannelKind::Subscribe).expect("serializes"),
            r#"{"kind":"subscribe"}"#
        );
    }
}
