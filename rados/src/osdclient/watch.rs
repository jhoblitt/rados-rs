//! Watch/notify: the notify reply and, from the linger machinery, the
//! watches and notifies this client keeps registered with the OSDs.
//!
//! `LIST_WATCHERS` lives in [`crate::osdclient::watchers`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::Denc;
use crate::osdclient::error::{ENOTCONN, Result};
use crate::osdclient::messages::{
    CEPH_WATCH_EVENT_DISCONNECT, CEPH_WATCH_EVENT_NOTIFY, CEPH_WATCH_EVENT_NOTIFY_COMPLETE,
    MWatchNotify,
};
use crate::osdclient::types::ObjectId;

/// What a watch receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A notify on the watched object; ack it with the watcher's
    /// `notify_ack` so the notifier does not wait for its timeout.
    Notify {
        notify_id: u64,
        notifier_gid: u64,
        payload: Bytes,
    },
    /// The watch is in error; `code` is a negative errno (`ENOTCONN` when
    /// the OSD dropped it or the object was deleted, `ETIMEDOUT` when the
    /// OSD holds it disconnected). Sent once per error. The client keeps
    /// reconnecting a watch in error other than `ENOTCONN` every five
    /// seconds, and on session resets and map changes; a successful
    /// reconnect clears the error and events resume. After `ENOTCONN` no
    /// events follow until the caller watches again.
    Disconnect { code: i32 },
}

/// A watch or an in-flight notify: Objecter's `LingerOp`.
#[allow(dead_code)] // lingers are registered from the next commit
pub(crate) struct Linger {
    pub(crate) cookie: u64,
    pub(crate) object: ObjectId,
    pub(crate) kind: LingerKind,
    pub(crate) events: mpsc::UnboundedSender<WatchEvent>,
    pub(crate) state: std::sync::Mutex<LingerState>,
}

#[allow(dead_code)] // lingers are registered from the next commit
pub(crate) enum LingerKind {
    /// `timeout` is the watch timeout sent with the registration, in
    /// seconds (0 for the OSD's default).
    Watch { timeout: u32 },
    Notify {
        /// Taken by the first NOTIFY_COMPLETE, so a completion racing a
        /// re-send reaches the caller once.
        completion: std::sync::Mutex<Option<oneshot::Sender<(i32, Bytes)>>>,
        /// 0 until the NOTIFY op's reply carries the id.
        notify_id: AtomicU64,
    },
}

#[allow(dead_code)] // lingers are registered from the next commit
#[derive(Debug, Default)]
pub(crate) struct LingerState {
    pub(crate) osd: Option<i32>,
    pub(crate) registered: bool,
    pub(crate) register_gen: u32,
    pub(crate) last_error: Option<i32>,
    pub(crate) watch_valid_thru: Option<Instant>,
}

impl Linger {
    #[allow(dead_code)] // lingers are registered from the next commit
    pub(crate) fn new(
        cookie: u64,
        object: ObjectId,
        kind: LingerKind,
        events: mpsc::UnboundedSender<WatchEvent>,
    ) -> Self {
        Self {
            cookie,
            object,
            kind,
            events,
            state: std::sync::Mutex::new(LingerState::default()),
        }
    }

    pub(crate) fn lock_state(&self) -> std::sync::MutexGuard<'_, LingerState> {
        // The state is plain data, so a panic elsewhere cannot leave it
        // half-updated in a way worse than losing the linger.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record `code` as the watch's error and send one `Disconnect`,
    /// unless an error is already recorded. Returns whether it sent.
    pub(crate) fn fail(&self, code: i32) -> bool {
        {
            let mut state = self.lock_state();
            if state.last_error.is_some() {
                return false;
            }
            state.last_error = Some(code);
        }
        let _ = self.events.send(WatchEvent::Disconnect { code });
        true
    }

    fn deliver(&self, msg: MWatchNotify, data: Bytes) {
        match (msg.opcode, &self.kind) {
            (CEPH_WATCH_EVENT_DISCONNECT, _) => {
                self.fail(ENOTCONN);
            }
            (CEPH_WATCH_EVENT_NOTIFY, LingerKind::Watch { .. }) => {
                let _ = self.events.send(WatchEvent::Notify {
                    notify_id: msg.notify_id,
                    notifier_gid: msg.notifier_gid,
                    payload: msg.bl,
                });
            }
            (
                CEPH_WATCH_EVENT_NOTIFY_COMPLETE,
                LingerKind::Notify {
                    completion,
                    notify_id,
                },
            ) => {
                let known = notify_id.load(Ordering::Acquire);
                if known != 0 && known != msg.notify_id {
                    debug!(
                        "notify cookie {} completion for notify {} != {}, ignoring",
                        self.cookie, msg.notify_id, known
                    );
                    return;
                }
                let tx = completion
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(tx) = tx {
                    let _ = tx.send((msg.return_code, data));
                }
            }
            (opcode, _) => debug!(
                "watch-notify opcode {} does not apply to cookie {}, ignoring",
                opcode, self.cookie
            ),
        }
    }
}

/// Hand a decoded `MWatchNotify` to the linger its cookie names; `data` is
/// the message's data segment. Every event is routed by cookie, as
/// Objecter does, and nothing here awaits: the caller is the session's
/// I/O loop.
pub(crate) fn route_watch_notify(
    lingers: &DashMap<u64, std::sync::Arc<Linger>>,
    msg: MWatchNotify,
    data: Bytes,
) {
    let Some(linger) = lingers.get(&msg.cookie).map(|l| std::sync::Arc::clone(&l)) else {
        debug!(
            "watch-notify opcode {} for unknown cookie {}, dropping",
            msg.opcode, msg.cookie
        );
        return;
    };
    linger.deliver(msg, data);
}

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

    const WATCH_COOKIE: u64 = 1001;
    const NOTIFY_COOKIE: u64 = 1002;

    fn event(opcode: u8, cookie: u64, notify_id: u64) -> MWatchNotify {
        MWatchNotify {
            opcode,
            cookie,
            ver: 0,
            notify_id,
            bl: Bytes::from_static(b"payload"),
            return_code: 0,
            notifier_gid: 4242,
        }
    }

    struct Registry {
        lingers: DashMap<u64, std::sync::Arc<Linger>>,
        watch_rx: mpsc::UnboundedReceiver<WatchEvent>,
        done_rx: oneshot::Receiver<(i32, Bytes)>,
    }

    fn registry() -> Registry {
        let lingers = DashMap::new();
        let (tx, watch_rx) = mpsc::unbounded_channel();
        lingers.insert(
            WATCH_COOKIE,
            std::sync::Arc::new(Linger::new(
                WATCH_COOKIE,
                ObjectId::new(1, "o"),
                LingerKind::Watch { timeout: 0 },
                tx,
            )),
        );
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, _) = mpsc::unbounded_channel();
        lingers.insert(
            NOTIFY_COOKIE,
            std::sync::Arc::new(Linger::new(
                NOTIFY_COOKIE,
                ObjectId::new(1, "o"),
                LingerKind::Notify {
                    completion: std::sync::Mutex::new(Some(done_tx)),
                    notify_id: AtomicU64::new(0),
                },
                tx,
            )),
        );
        Registry {
            lingers,
            watch_rx,
            done_rx,
        }
    }

    #[test]
    fn a_notify_reaches_the_watch_channel() {
        let mut r = registry();
        route_watch_notify(
            &r.lingers,
            event(CEPH_WATCH_EVENT_NOTIFY, WATCH_COOKIE, 77),
            Bytes::new(),
        );
        assert_eq!(
            r.watch_rx.try_recv(),
            Ok(WatchEvent::Notify {
                notify_id: 77,
                notifier_gid: 4242,
                payload: Bytes::from_static(b"payload"),
            })
        );
        assert!(r.watch_rx.try_recv().is_err());
    }

    #[test]
    fn a_repeated_disconnect_is_reported_once() {
        let mut r = registry();
        for _ in 0..2 {
            route_watch_notify(
                &r.lingers,
                event(CEPH_WATCH_EVENT_DISCONNECT, WATCH_COOKIE, 0),
                Bytes::new(),
            );
        }
        assert_eq!(
            r.watch_rx.try_recv(),
            Ok(WatchEvent::Disconnect { code: ENOTCONN })
        );
        assert!(r.watch_rx.try_recv().is_err());
        let linger = r.lingers.get(&WATCH_COOKIE).unwrap();
        assert_eq!(linger.lock_state().last_error, Some(ENOTCONN));
    }

    #[test]
    fn a_completion_before_the_notify_id_is_known_completes_once() {
        let mut r = registry();
        let data = Bytes::from_static(&[0; 8]);
        route_watch_notify(
            &r.lingers,
            event(CEPH_WATCH_EVENT_NOTIFY_COMPLETE, NOTIFY_COOKIE, 9),
            data.clone(),
        );
        assert_eq!(r.done_rx.try_recv(), Ok((0, data)));
        // A second completion has no sender left to complete.
        route_watch_notify(
            &r.lingers,
            event(CEPH_WATCH_EVENT_NOTIFY_COMPLETE, NOTIFY_COOKIE, 9),
            Bytes::new(),
        );
    }

    #[test]
    fn a_completion_for_another_notify_id_is_ignored() {
        let mut r = registry();
        if let LingerKind::Notify { notify_id, .. } = &r.lingers.get(&NOTIFY_COOKIE).unwrap().kind {
            notify_id.store(10, Ordering::Release);
        }
        route_watch_notify(
            &r.lingers,
            event(CEPH_WATCH_EVENT_NOTIFY_COMPLETE, NOTIFY_COOKIE, 9),
            Bytes::new(),
        );
        assert!(r.done_rx.try_recv().is_err());
        let mut done = event(CEPH_WATCH_EVENT_NOTIFY_COMPLETE, NOTIFY_COOKIE, 10);
        done.return_code = crate::osdclient::error::ETIMEDOUT;
        route_watch_notify(&r.lingers, done, Bytes::new());
        assert_eq!(
            r.done_rx.try_recv(),
            Ok((crate::osdclient::error::ETIMEDOUT, Bytes::new()))
        );
    }

    #[test]
    fn an_unknown_cookie_is_dropped() {
        let mut r = registry();
        route_watch_notify(
            &r.lingers,
            event(CEPH_WATCH_EVENT_NOTIFY, 5, 1),
            Bytes::new(),
        );
        assert!(r.watch_rx.try_recv().is_err());
    }
}
