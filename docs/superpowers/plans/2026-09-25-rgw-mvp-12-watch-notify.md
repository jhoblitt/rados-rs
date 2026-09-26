# rados-rs RGW MVP, plan 12 of N: `watch-notify`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the `rados` crate librados's watch/notify: `IoCtx::watch`
returning a `Watcher` that receives notifies and disconnects, kept alive
by pings and re-registered across session resets and PG interval
changes; `Watcher::notify_ack`; `Watcher::unwatch`; `IoCtx::notify`
resolving to the acks and the watchers that missed it. This is the one
stateful piece of the transport and the last package; RGW's metadata
cache invalidation and realm watcher sit on it.

**Architecture:** Four layers, each its own commit. (1) The wire: three
op codes, two `OpData` variants, their `Denc` arms and `OSDOp`
constructors, and the `MWatchNotify` message decoder with the notify
reply decoder. (2) Delivery: the OSD session forwards `CEPH_MSG_WATCH_NOTIFY`
to `OSDClient`, which routes it by cookie through a linger registry to a
non-blocking channel, and the session tells the client when it resets.
(3) Lifetime: the registry re-sends a `RECONNECT` for every watch on the
reset OSD and on every new PG interval, pings each healthy watch every
five seconds with a generation tag, and turns the first failed ping,
reconnect or `DISCONNECT` into one `WatchEvent::Disconnect`. (4) The
public API and the cluster tests that mirror `test/librados/watch_notify.cc`.
The design follows `Objecter`'s linger machinery (v19.2.6) point for
point where rados-rs's structure allows, and the plan says where it does
not.

**Tech Stack:** as plan 3; `tokio::sync::mpsc` (unbounded) for events,
`tokio::time::interval` for the ping task, `DashMap` as the registry
(already a dependency).

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"watch/notify (branch `watch-notify`)". Two corrections to that section,
both from the research against `Objecter.cc`: routing is by cookie for
every event, `NOTIFY_COMPLETE` included (it can reach the client before
the `NOTIFY` op reply that carries the notify id, so a notify id is
checked only once known); and a watch must be re-sent as `RECONNECT` on
any new PG interval, not only when the primary changes, because the OSD
rebuilds every watch as disconnected whenever the PG activates.

## Global Constraints

Plans 3 to 11's Global Constraints apply where they fit (commit style,
gates, push after every gate, merge when green, upstream PR, offline
builds, no new dependencies). Plus:

- Branch `watch-notify` is based on the fork's `main` after plan 11
  merges. Everything lands in the `rados` crate (`osdclient`, `msgr2`);
  `rados-cls` is untouched.
- The wire facts, verified in `src/include/rados.h`, `src/include/ceph_fs.h`,
  `src/messages/MWatchNotify.h`, `src/osdc/Objecter.{h,cc}` and
  `src/librados/IoCtxImpl.cc` at v19.2.6 (identical at `main`):
  - `CEPH_OSD_OP_WATCH` = `0x120f` (WR|DATA|15); union `{u64 cookie, u64
    ver (0), u8 op, u32 gen, u32 timeout}` then three zero bytes; no indata,
    no outdata. Sub-ops `UNWATCH` 0, `LEGACY_WATCH` 1, `WATCH` 3,
    `RECONNECT` 5, `PING` 7. Objecter sends the register, the reconnect
    and the ping with flags WRITE|READ; `UNWATCH` as a plain write.
  - `CEPH_OSD_OP_NOTIFY` = `0x1206` (RD|DATA|6); union `{u64 cookie}` (the
    notifier's linger cookie) padded to 28; indata `u32 prot_ver = 1, u32
    timeout_secs, u32 len + payload`; outdata `u64 notify_id`. A notify on a
    missing object is `ENOENT`.
  - `CEPH_OSD_OP_NOTIFY_ACK` = `0x1207` (RD|DATA|7); union zero; indata `u64
    notify_id, u64 watch_cookie, u32 len + reply`; the OSD always answers 0.
  - `CEPH_MSG_WATCH_NOTIFY` = 44, header version 3, compat 1, no
    `ENCODE_START`; front segment `u8 msg_ver (1), u8 opcode, u64 cookie,
    u64 ver, u64 notify_id, u32 len + bl, i32 return_code (header version
    >= 2), u64 notifier_gid (>= 3)`. Opcodes `NOTIFY` 1 (to a watcher: the
    watch cookie, the notify id, the payload in `bl`, the notifier's gid),
    `NOTIFY_COMPLETE` 2 (to the notifier: its notify cookie, `bl` empty,
    the reply map in the message's DATA segment, `return_code` 0 or
    `-ETIMEDOUT`), `DISCONNECT` 3 (to a watcher: cookie only).
  - The notify reply map: `u32 n, n x {u64 gid, u64 cookie, u32 len,
    bytes}` (acks, keyed by the watcher's gid and its watch cookie), then
    `u32 m, m x {u64 gid, u64 cookie}` (watchers that never acked).
  - OSD behaviour: `osd_client_watch_timeout` 30 s, overridden per watch
    by the op's `timeout`; a pinged watch expires without a ping; the OSD
    sends `DISCONNECT` only while the connection is up (a timeout with the
    session alive, or the object deleted); after a session reset the
    client learns of the loss only from a failed `RECONNECT` or `PING`
    (`ENOTCONN` when the watch is gone from the object, `ETIMEDOUT` when
    the OSD wants a reconnect); `WATCH` is idempotent; `UNWATCH` on a
    deleted object is `ENOENT`, on an unregistered cookie 0; a `PING`
    reply for an older generation is ignored by the client; the watch key
    on the OSD is `(cookie, client.<global_id>)`, so the client's gid is
    part of the identity; a notify with no watchers completes at once with
    empty lists; a watcher removed by timeout counts neither as acked nor
    missed; librados sends `client_notify_timeout` = 10 s when the caller
    passes 0 (RGW always does), and the OSD substitutes its own 30 s default
    for a wire value of 0.
- rados-rs today (verified in the tree): `CEPH_MSG_WATCH_NOTIFY` falls
  into the session I/O loop's "unexpected message" arm and is dropped;
  `dispatch_from_osd` has arms only for `OPREPLY` and `BACKOFF`; the route
  closure is awaited inline in the I/O loop, so a handler must not block;
  the pre-send filter drops any `OSD_OP` without a `pending_ops` entry, so
  every send, pings and acks included, goes through `submit_op`; an
  `io_task` exit fails the session's pending ops but tells `OSDClient`
  nothing, and nothing reopens a session that carries only watches; the
  map-change scan (`collect_resend_ops`) migrates ops only when the
  primary changes or `last_force_op_resend` advances; the only timers are
  the per-op `Tracker`, the msgr keepalive and the OSDMap drain task;
  `execute_op` retries a lost connection four times on a fresh session.
- Cookies come from a client-wide counter starting at 1001 (the C++ tests
  assert `cookie > 1000`); they must be unique per `(gid, object)` and
  never reused within a process.
- Every timer and task is cancelled by the client's `shutdown_token`;
  dropping a `Watcher` without `unwatch` removes it from the registry and
  stops its pings (the OSD then expires it), and does not send anything.
- Event delivery never awaits user code: `WatchEvent`s go through an
  unbounded `mpsc`; a `Watcher` whose receiver is dropped has its sends
  fail silently.

## Review Focus

1. The three op unions and the three indata layouts are byte-exact
   (28-byte union, 3 and 20 bytes of padding; the notify indata's
   `prot_ver` 1; the ack's field order). Pinned in Task 1 unit tests.
2. `MWatchNotify` decode gated on the msgr header version, `NOTIFY_COMPLETE`'s
   reply read from the DATA segment, and the reply decoder's two lists.
   Pinned in Task 1 against hand-built vectors.
3. Routing by cookie for all three events; a `NOTIFY_COMPLETE` that
   arrives before the op reply still completes the notify; a late
   completion for a superseded notify id is ignored. Pinned in Task 2 with
   an in-process fake reply and in Task 4 on the cluster.
4. Lifetime: pings every five seconds tagged with the generation; a
   reconnect bumps the generation and a stale ping reply is ignored; one
   `Disconnect` per failure and no further pings until the caller
   re-watches; reconnect on session reset and on any new interval, with
   the reconnect a bare single `WATCH{RECONNECT}` op carrying no other ops
   and no timeout. Pinned in Task 4 (`watch_survives_a_session_reset`,
   `watch_times_out_without_pings`).
5. The public API's errors: `watch` on a missing object `ENOENT`; a
   `Disconnect` event carries `ENOTCONN`; `unwatch` after the object is
   deleted `ENOENT`; a timed-out `notify` returns `Ok` with `timed_out ==
   true` and the missed list, as librados hands the reply to the caller
   whatever the code. Pinned in Task 4.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `watch-notify` off the fork's `main` after
plan 11 merges; the cluster up.

---

### Task 1: The wire: op codes, op data, message and reply decoders

**Files:**
- Modify: `rados/src/osdclient/types.rs` (`OpCode::{Watch, Notify,
  NotifyAck}`; `OpData::{Watch { cookie, ver, op, gen, timeout }, Notify {
  cookie }}`; constructors `OSDOp::watch(cookie, op: WatchOp, timeout:
  u32)`, `OSDOp::watch_ping(cookie, gen)`, `OSDOp::notify(cookie,
  timeout_secs, payload: Bytes)`, `OSDOp::notify_ack(notify_id, cookie,
  reply: Bytes)`; `pub enum WatchOp { Unwatch = 0, LegacyWatch = 1, Watch =
  3, Reconnect = 5, Ping = 7 }`), `rados/src/osdclient/denc_types.rs`
  (encode and decode arms for the two variants, padded to 28),
  `rados/src/osdclient/messages.rs` (`CEPH_MSG_WATCH_NOTIFY = 44`, the
  `CEPH_WATCH_EVENT_*` constants, `pub struct MWatchNotify { opcode: u8,
  cookie: u64, ver: u64, notify_id: u64, bl: Bytes, return_code: i32,
  notifier_gid: u64 }` with `fn decode(header_version: u8, front: &[u8])`),
  `rados/src/osdclient/operation.rs` (`OpBuilder::{watch, notify,
  notify_ack}` if the builder is where compound users reach for them),
  a new `rados/src/osdclient/watch.rs` holding `NotifyAck { gid, cookie,
  reply: Bytes }`, `NotifyTimeout { gid, cookie }`, `NotifyResult {
  acks: Vec<NotifyAck>, missed: Vec<NotifyTimeout>, timed_out: bool }`
  and `fn decode_notify_reply(data: &[u8]) -> Result<(Vec<NotifyAck>,
  Vec<NotifyTimeout>)>`.

**Interfaces:**
- Produces everything above; `MOSDOp::calculate_flags` keeps deriving
  READ/WRITE from the opcode; the linger code ORs `READ` onto watch ops
  to match Objecter.

Unit tests pin: `OSDOp::watch(0x1234, Watch, 0)`'s 28-byte union is
`cookie LE, 8 zero bytes, 0x03, 4 zero bytes, 4 zero bytes, 3 zero bytes`;
`watch_ping(c, 7)` puts `7` in `gen`; `notify(c, 10, b"hi")` has union
`cookie` plus 20 zero bytes and indata `01000000 0a000000 02000000 6869`;
`notify_ack(5, 6, b"r")` has a zero union and indata `05.. 06.. 01000000
72`; the `Denc` round trip of both variants; `MWatchNotify::decode` of a
hand-built `NOTIFY` front (`01 01 <cookie> <ver> <notify_id> <len+bl>
<rc> <gid>`) with header version 3, of a version-1 front without `rc` and
`gid` (defaulting to 0), and of a `DISCONNECT`; `decode_notify_reply` of
`01000000 <gid> <cookie> 05000000 reply 01000000 <gid2> <cookie2>` giving one
ack with payload `reply` and one missed, and of `00000000 00000000` giving
none.

Commit:

```
osdclient: the watch, notify and notify_ack ops and the watch-notify message

CEPH_OSD_OP_WATCH carries everything in its 28-byte union (cookie, the
sub-op, the registration generation, the timeout); NOTIFY sends a
protocol version, a timeout and a payload and answers with the notify
id; NOTIFY_ACK sends the notify id, the watch cookie and a reply. The
OSD's CEPH_MSG_WATCH_NOTIFY has no ENCODE_START envelope and gates its
last two fields on the messenger header version; a NOTIFY_COMPLETE
carries its reply map, acks then misses, in the data segment.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: Delivery: the session forwards watch events and reports resets

**Files:**
- Modify: `rados/src/osdclient/session.rs` (the I/O loop's message match
  gains `CEPH_MSG_WATCH_NOTIFY` next to `OPREPLY`/`BACKOFF`, forwarding to
  `OSDClient::dispatch_from_osd`; the `io_task` exit path calls
  `client.on_session_reset(osd_id)` through its `Weak<OSDClient>` after
  failing the pending ops, and so does every deliberate close
  (`close_stale_sessions`, the cancel in `collect_resend_ops`)),
  `rados/src/osdclient/client.rs` (`dispatch_from_osd` gains a
  `CEPH_MSG_WATCH_NOTIFY => self.handle_watch_notify(osd_id, msg)` arm;
  `handle_watch_notify` decodes the message and hands it to the linger
  registry without awaiting anything but a `try_send`; `on_session_reset`
  is a stub here that the next task fills; a `lingers: DashMap<u64,
  Arc<Linger>>` field and a `next_cookie: AtomicU64` starting at 1001).
- Create: the registry type in `rados/src/osdclient/watch.rs`: `struct
  Linger { cookie, object: ObjectId, kind: LingerKind, events:
  mpsc::UnboundedSender<WatchEvent>, state: Mutex<LingerState> }` with
  `LingerKind::{Watch { timeout: u32 }, Notify { completion:
  oneshot::Sender<(i32, Bytes)>, notify_id: AtomicU64 }}` and `LingerState {
  osd: Option<u32>, registered: bool, register_gen: u32, last_error:
  Option<i32>, watch_valid_thru: Option<Instant> }`.
- Routing (`handle_watch_notify`): look the cookie up; unknown cookies
  are logged at debug and dropped. `DISCONNECT`: if `last_error` is none,
  set it to `-ENOTCONN` and send `WatchEvent::Disconnect { code: -ENOTCONN
  }`. `NOTIFY` on a watch linger: send `WatchEvent::Notify { notify_id,
  notifier_gid, payload }`. `NOTIFY_COMPLETE` on a notify linger: if the
  linger's `notify_id` is non-zero and differs from the message's, ignore
  (a stale completion of a re-sent notify); else complete the oneshot
  once with `(return_code, data)`, where `data` is the message's DATA
  segment.

Unit test: with a hand-built `MWatchNotify` fed to `handle_watch_notify`
against a registry holding one watch and one notify linger (no cluster):
a `NOTIFY` lands in the watch's channel with the right fields; a
`DISCONNECT` produces exactly one `Disconnect` even when repeated; a
`NOTIFY_COMPLETE` with the notify's id still zero completes it; a second
one with a different id is ignored.

Commit:

```
osdclient: deliver watch-notify messages by cookie and report session resets

The session forwards CEPH_MSG_WATCH_NOTIFY to the client instead of
dropping it, and tells the client when its I/O task ends or it is
closed on purpose. The client keeps a registry of lingers keyed by
cookie and routes every event by that cookie, as Objecter does; a
NOTIFY_COMPLETE can beat the op reply that carries the notify id, so
the id is checked only once known.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: Lifetime: register, reconnect, ping, notify completion

**Files:**
- Modify: `rados/src/osdclient/client.rs`, `rados/src/osdclient/watch.rs`,
  `rados/src/osdclient/ioctx.rs`.

**Interfaces and rules:**
- `OSDClient::linger_watch(object, timeout) -> Result<Arc<Linger>>`:
  allocate the cookie, insert the linger, send `WATCH{Watch, timeout}`
  with flags WRITE|READ as a tracked op through `submit_op` on the
  object's current primary WITHOUT `execute_op`'s connection-retry loop;
  on success mark `registered`, record the OSD; on error remove the
  linger and return the error (`ENOENT` for a missing object).
- `send_reconnect(linger)`: a single `WATCH{Reconnect, gen: ++register_gen}`
  op, flags WRITE|READ, tracked, not retried; on success clear
  `last_error` and set the OSD; on error set `last_error` if none (mapping
  `ENOENT` to `ENOTCONN` as `_normalize_watch_error` does) and send one
  `Disconnect`.
- `on_session_reset(osd_id)`: reopen the session, then for every linger
  whose `osd == osd_id`: watches get `send_reconnect`, notifies are re-sent
  whole with `notify_id` reset to zero (the new primary allocates a new
  id). This also runs for sessions closed on purpose.
- Map change: in `update_osdmap_state`, before the ordinary-op scan,
  recompute each linger's target with interval-change semantics (a new
  interval means the acting set, up set, primary, `pg_num` or the pool's
  `last_force_op_resend` changed for its PG; reuse the crate's OSDMap
  helpers and add an `is_new_interval` if none exists); a changed
  interval triggers `send_reconnect` (watch) or a whole re-send (notify);
  a pool that vanished fails the linger with `ENOENT` and one `Disconnect`.
- The ping task: spawned in `OSDClient::new` beside the OSDMap drain
  task, cancelled by `shutdown_token`, `interval(5 s)`; each tick sends
  `watch_ping(cookie, register_gen)` for every watch linger that is
  `registered` with no `last_error`, as a tracked op with no retry,
  recording `sent = Instant::now()`; a reply whose generation is not the
  current one is ignored; a success sets `watch_valid_thru = sent`; the
  first error (`ENOTCONN`, `ETIMEDOUT`, a connection loss) sets
  `last_error` and sends one `Disconnect`, after which the watch is not
  pinged again until re-registered. A migrated or drained ping (the
  generic resend path) is ignored by its generation.
- `OSDClient::unwatch(linger) -> Result<()>`: send `WATCH{Unwatch}` as a
  normal `execute_op` (a write; resend-safe), then remove the linger from
  the registry; return the op's result (`ENOENT` if the object is gone).
- `OSDClient::notify(object, payload, timeout_ms) -> Result<NotifyResult>`:
  register a notify linger (cookie, oneshot); send `NOTIFY{cookie,
  timeout_secs, payload}` where `timeout_secs` is `timeout_ms / 1000` or
  10 when that is 0 (librados's `client_notify_timeout`); decode the op
  reply's `u64` into `notify_id` (unless a completion already arrived);
  an op error removes the linger and returns it (`ENOENT` for a missing
  object); await the completion with an outer bound of `timeout_secs + 30
  s` (a lost `NOTIFY_COMPLETE` after a reset is recovered by the re-send,
  the bound only protects against a dead OSD; on expiry return
  `OSDClientError::Timeout`); decode the DATA into `NotifyResult` with
  `timed_out = return_code == -ETIMEDOUT`; remove the linger.
- `IoCtx` surface: `pub async fn watch(&self, oid: &str) ->
  Result<Watcher>` and `watch_with_timeout(oid, secs)`; `pub async fn
  notify(&self, oid: &str, payload: Bytes, timeout_ms: u64) ->
  Result<NotifyResult>`; `pub struct Watcher { cookie: u64, events:
  mpsc::UnboundedReceiver<WatchEvent>, .. }` with `pub fn cookie(&self)`,
  `pub async fn recv(&mut self) -> Option<WatchEvent>`, `pub async fn
  notify_ack(&self, notify_id: u64, reply: Bytes) -> Result<()>` (a tracked
  `NOTIFY_ACK` op through `submit_op`; the OSD's reply is 0, awaited so
  the send is not dropped), `pub async fn unwatch(self) -> Result<()>`,
  `pub fn check(&self) -> Result<Duration>` (the age of
  `watch_valid_thru`, or `last_error` as the error; librados's
  `watch_check`), and `Drop` that removes the linger and stops pings.
  `pub enum WatchEvent { Notify { notify_id: u64, notifier_gid: u64,
  payload: Bytes }, Disconnect { code: i32 } }`.
- A test hook `OSDClient::set_watch_pings_enabled(bool)` (`cfg(test)`-free
  but documented as a test aid, like `objecter_inject_no_watch_ping`) so
  the cluster test can let a watch time out.

Unit tests without a cluster: the ping task skips lingers with
`last_error`; a stale-generation ping reply changes nothing; a
`Disconnect` is sent once.

Commit:

```
osdclient: linger lifetime for watches and notifies

A watch is re-sent as RECONNECT with a new generation whenever its
session resets or its PG enters a new interval, because the OSD
rebuilds every watch as disconnected on activation; it is pinged
every five seconds and the first failed ping, reconnect or DISCONNECT
becomes one Disconnect event. A notify is re-sent whole on the same
triggers, with its id reset, and completes from the NOTIFY_COMPLETE's
data segment; a wire timeout of zero becomes librados's ten seconds.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 4: Cluster tests

**Files:**
- Create: `rados/tests/watch_notify.rs` (using `tests/common/mod.rs`).
- Modify: `.github/workflows/test-with-ceph.yml` (the `rados` integration
  steps run this suite too, CRC on and off).

Tests (all `#[ignore]`), mirroring `test/librados/watch_notify.cc`; two
clients where a second gid is needed (`create_ioctx` twice gives two
`OSDClient`s):

1. `watch_on_a_missing_object_is_enoent`.
2. `notify_reaches_the_watcher_and_returns_its_reply` (`WatchNotify2`): A
   watches `o`; B notifies `o` with `hello`, timeout 0; A receives `Notify`
   with the payload and the notifier gid equal to B's, acks with `reply`;
   B's result has one ack with gid A, cookie A's, payload `reply`, no
   missed, `timed_out` false; `A.check()` is `Ok`; `notify` of a missing
   object is `ENOENT`; `list_watchers` shows A's cookie and gid.
3. `two_watches_from_one_client` (`WatchNotify2Multi`): two watchers on
   `o` from A; both receive; both ack; the result has two acks with
   distinct cookies.
4. `notify_times_out_when_a_watcher_does_not_ack`
   (`WatchNotify2Timeout`): A watches and never acks; B notifies with
   timeout 1000 ms: `timed_out` true, no acks, one missed naming A; a
   second notify with a 300 s timeout, acked, succeeds; A is still
   healthy.
5. `deleting_the_object_disconnects_the_watch` (`Watch2Delete`): A
   watches `o`; B removes `o`; A receives `Disconnect { -ENOTCONN }` within
   30 s; `A.check()` is `Err(ENOTCONN)`; `A.unwatch()` is `ENOENT`.
6. `watch_times_out_without_pings` (`Watch3Timeout`): A watches with
   timeout 4 s, `check()` is `Ok` under a second, pings are disabled with
   the hook; within 10 s A receives `Disconnect { -ENOTCONN }` (the OSD
   times the pinged watch out with the session up and sends DISCONNECT);
   B's notify then returns no acks and no missed; A re-watches (pings back
   on) and B's next notify gets one ack.
7. `watch_survives_a_session_reset`: A watches `o`; the controller closes
   A's session to the object's primary through a test-only
   `OSDClient::close_session_for_test(osd)` (added in Task 3 next to the
   ping hook); within 10 s B's notify is acked by A (the reconnect
   re-registered the watch; `check()` stays `Ok`).
8. `unwatch_then_notify_finds_no_watcher`: after `unwatch`, B's notify
   completes at once with empty lists.

Commit:

```
osdclient: watch/notify cluster tests

Pins against Ceph v19: a notify's payload, notifier gid and reply
round-trip, per-watch acks and misses, a one-second timeout reported
as timed out with the missed watcher, DISCONNECT on object deletion
and on a missed ping, ENOENT for a missing object and for an unwatch
after deletion, and a watch that survives a session reset.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: no corpus change; the whole `rados` suite plus this
one on the cluster; the implementer for Tasks 1 to 3 is the opus tier
(this is design work inside a transport, not transcription) and the
whole-branch review is the session model; the PR body:

```
**Motivation.** RGW invalidates every gateway's metadata cache by watch/notify on the control pool's `notify.N` objects and watches its realm's control object; a Rust RGW that coexists with radosgw must speak both, and rados-rs drops `CEPH_MSG_WATCH_NOTIFY` today.

**What changed.** The `rados` crate gains the watch, notify and notify_ack ops, the `MWatchNotify` decoder, a linger registry routed by cookie, reconnects on session reset and on every new PG interval, generation-tagged pings, and the `IoCtx::watch`/`Watcher`/`IoCtx::notify` API with librados's semantics; eight cluster tests mirror `test/librados/watch_notify.cc`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

This closes the spec's roadmap unless `cls-lock` is added. Deferred:
`LEGACY_WATCH` (librados `watch` v1), `watch_flush`, a client-side
`notify` timer beyond the outer bound, `RGWCacheNotifyInfo` (an RGW
type, the driver's), the msgr-level ping of idle linger sessions
(Objecter's `toping`; the five-second op pings keep the session busy
instead).
