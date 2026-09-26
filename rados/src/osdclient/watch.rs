//! Watch/notify: the notify reply and, from the linger machinery, the
//! watches and notifies this client keeps registered with the OSDs.
//!
//! `LIST_WATCHERS` lives in [`crate::osdclient::watchers`].

use bytes::Bytes;

use crate::Denc;
use crate::osdclient::error::Result;

/// One watcher's acknowledgement of a notify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyAck {
    /// The acking watcher's client global id.
    pub gid: u64,
    /// The acking watch's cookie.
    pub cookie: u64,
    /// The payload the watcher acked with.
    pub reply: Bytes,
}

/// A watcher that was still registered but did not ack before the
/// notify timed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyTimeout {
    pub gid: u64,
    pub cookie: u64,
}

/// A completed notify, as librados hands it back: the acks, the watchers
/// that missed it, and whether the OSD completed it by timeout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifyResult {
    pub acks: Vec<NotifyAck>,
    pub missed: Vec<NotifyTimeout>,
    pub timed_out: bool,
}

/// Decode a NOTIFY_COMPLETE's data segment: the OSD's
/// `map<pair<gid, cookie>, bufferlist>` of acks, then its
/// `vector<pair<gid, cookie>>` of watchers that missed the notify.
pub fn decode_notify_reply(data: &[u8]) -> Result<(Vec<NotifyAck>, Vec<NotifyTimeout>)> {
    let mut buf = Bytes::copy_from_slice(data);
    let n = u32::decode(&mut buf, 0)?;
    let mut acks = Vec::new();
    for _ in 0..n {
        let gid = u64::decode(&mut buf, 0)?;
        let cookie = u64::decode(&mut buf, 0)?;
        let reply = Bytes::decode(&mut buf, 0)?;
        acks.push(NotifyAck { gid, cookie, reply });
    }
    let m = u32::decode(&mut buf, 0)?;
    let mut missed = Vec::new();
    for _ in 0..m {
        let gid = u64::decode(&mut buf, 0)?;
        let cookie = u64::decode(&mut buf, 0)?;
        missed.push(NotifyTimeout { gid, cookie });
    }
    Ok((acks, missed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_one_ack_and_one_miss() {
        let mut wire = vec![1, 0, 0, 0];
        wire.extend_from_slice(&11u64.to_le_bytes());
        wire.extend_from_slice(&12u64.to_le_bytes());
        wire.extend_from_slice(&[5, 0, 0, 0]);
        wire.extend_from_slice(b"reply");
        wire.extend_from_slice(&[1, 0, 0, 0]);
        wire.extend_from_slice(&21u64.to_le_bytes());
        wire.extend_from_slice(&22u64.to_le_bytes());

        let (acks, missed) = decode_notify_reply(&wire).expect("decode");
        assert_eq!(
            acks,
            vec![NotifyAck {
                gid: 11,
                cookie: 12,
                reply: Bytes::from_static(b"reply"),
            }]
        );
        assert_eq!(
            missed,
            vec![NotifyTimeout {
                gid: 21,
                cookie: 22
            }]
        );
    }

    #[test]
    fn decodes_empty_lists() {
        let (acks, missed) = decode_notify_reply(&[0; 8]).expect("decode");
        assert!(acks.is_empty());
        assert!(missed.is_empty());
    }

    #[test]
    fn rejects_a_truncated_reply() {
        assert!(decode_notify_reply(&[1, 0, 0, 0, 1]).is_err());
    }
}
