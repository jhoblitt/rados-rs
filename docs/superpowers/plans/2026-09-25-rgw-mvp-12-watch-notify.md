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
(already a dependency). No new dependency: the workspace `tokio` is
`features = ["full"]` (`Cargo.toml:31`, taken by `rados/Cargo.toml:82`),
and `dashmap` (88), `tokio-util` (83), `bytes` and `libc` (101) are already
in `rados/Cargo.toml`.

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
  - `CEPH_OSD_OP_WATCH` = `0x220f` (WR|DATA|15; `WR` is `0x2000`,
    `MODE_WR` at `types.rs:486`, so `osd_op!(WR, DATA, 15)` yields it); union `{u64 cookie, u64
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
  every send, pings and acks included, goes through `submit_op` (keyed by
  the header tid, so the caller allocates the tid and builds the whole
  `MOSDOp` first), and `submit_op` registers every op with the Tracker at
  `operation_timeout` (30 s by default); an `io_task` exit fails the
  session's pending ops but tells `OSDClient` nothing, the dead session
  stays in `OSDClient::sessions`, and nothing reopens a session that
  carries only watches (sessions open only through
  `get_or_create_session`, which replaces a dead one); the map-change
  scan (`collect_resend_ops`) migrates an op when its primary changes or
  its pool's `last_force_op_resend` advances, and on the drain path moves
  every op off an OSD that `check_session_health` reports down or whose
  address changed; once any op on a session migrates, the whole session
  is cancelled and drained; a `pg_num` change with the same primary
  restamps the pgid without a resend; there is no interval logic; the
  only background timers or tasks are the per-op `Tracker` (stopped by
  `tracker.shutdown()`, not by `shutdown_token`), the msgr keepalive and
  the OSDMap drain task, beside per-call `tokio::time::timeout`s
  (`await_op_result`, `wait_for_osdmap`) and msgr2's throttle and
  lossless-reconnect sleeps; `execute_op` retries only
  `OSDClientError::Connection`, on a fresh session, under
  `MAX_CONNECTION_RETRIES = 4` shared by its submit and await paths,
  which is up to five attempts.
- `OSDMap.require_osd_release` is a public field (no accessor), but
  `OSDMapIncremental::apply_to` discards `new_require_osd_release`
  (`new_require_osd_release: _`), so a client keeps the release it
  bootstrapped with. A later package fixes that; this plan does not
  depend on the field and does not touch it.
- OSD ids are `i32` everywhere (`OSDSession::osd_id`, `sessions:
  HashMap<i32, ..>`); every new signature here uses `i32`.
- `ENOTCONN` (-107) and `ETIMEDOUT` (-110) are added to the errno list
  in `osdclient/error.rs` beside `ENOENT`, in its negative-constant
  style, rather than taken from `libc`, so every errno the watch code
  compares against reads the same way.
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

1. The three op codes (`0x220f`, `0x1206`, `0x1207`), the three op
   unions and the three indata layouts are byte-exact (28-byte union, 3 and 20 bytes of padding; the notify indata's
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
   and no timeout; the reset hook fires once per io_task exit, spawns
   its work and does nothing after shutdown; the interval test compares
   up and acting separately. Pinned in Task 2's and Task 3's unit tests
   and in Task 4 (`watch_survives_a_session_reset`,
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
  notifier_gid: u64 }` with `fn decode(header_version: u16, front: &[u8])`,
  called with `msg.header.version.get()` since msgr2's header version is
  `U16` (`msgr2/header.rs:32`); the DATA segment is `msg.data`),
  `rados/src/osdclient/operation.rs` (`OpBuilder::{watch, notify,
  notify_ack}` if the builder is where compound users reach for them),
  a new `rados/src/osdclient/watch.rs` holding `NotifyAck { gid, cookie,
  reply: Bytes }`, `NotifyTimeout { gid, cookie }`, `NotifyResult {
  acks: Vec<NotifyAck>, missed: Vec<NotifyTimeout>, timed_out: bool }`
  and `fn decode_notify_reply(data: &[u8]) -> Result<(Vec<NotifyAck>,
  Vec<NotifyTimeout>)>`. The existing `rados/src/osdclient/watchers.rs`
  (LIST_WATCHERS, `WatchItem`) stays separate and unchanged.
- Modify: `rados/src/osdclient/mod.rs` (`pub mod watch;` beside `pub mod
  watchers;`, and `pub use watch::{..}` for the public types beside the
  `watchers` re-export) and `rados/src/lib.rs` (add them to the `pub use
  osdclient::{..}` list); Task 3 extends both lists with `Watcher` and
  `WatchEvent`.

**Interfaces:**
- Produces everything above; `MOSDOp::calculate_flags` keeps deriving
  READ/WRITE from the opcode; the linger code ORs `READ` onto watch ops
  to match Objecter.

Unit tests pin: `OpCode::Watch as u16 == 0x220f`, `Notify == 0x1206`,
`NotifyAck == 0x1207`, with `Watch.is_write()` and the other two
`is_read()`; `OSDOp::watch(0x1234, Watch, 0)`'s 28-byte union is
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
  `OSDClient::dispatch_from_osd`; the `io_task` exit path, and only it,
  calls `client.on_session_reset(osd_id)` on the upgraded `ctx.client`
  (its `Weak<OSDClient>`; nothing if the upgrade fails) after failing the
  pending ops. Every deliberate
  close already ends there: `close_stale_sessions`'s `session.close()`
  and `collect_resend_ops`'s `cancel_io_loop()` both cancel the io_task,
  so a second call from either would report the same reset twice),
  `rados/src/osdclient/client.rs` (`dispatch_from_osd` gains a
  `CEPH_MSG_WATCH_NOTIFY => self.handle_watch_notify(osd_id, msg)` arm;
  `handle_watch_notify` decodes the message and hands it to the linger
  registry without awaiting anything: it takes only `std::sync::Mutex`
  guards and sends on unbounded channels and oneshots;
  `on_session_reset(self: &Arc<Self>, osd_id: i32)` is a synchronous fn
  that returns at
  once if `shutdown_token` is cancelled (client shutdown runs the same
  exit path) and otherwise `tokio::spawn`s its work, never awaiting it
  inline: `OSDSession::close()` awaits the io_task's `JoinHandle`, and
  an inline reopen would hold `close()` for a full TCP and auth
  handshake; the spawned body is a stub here that the next task fills;
  a `lingers: DashMap<u64, Arc<Linger>>` field and a `next_cookie:
  AtomicU64` starting at 1001, initialised beside `next_tid`).
- Create: the registry type in `rados/src/osdclient/watch.rs`: `struct
  Linger { cookie, object: ObjectId, kind: LingerKind, events:
  mpsc::UnboundedSender<WatchEvent>, state: std::sync::Mutex<LingerState> }`
  with `LingerKind::{Watch { timeout: u32 }, Notify { completion:
  std::sync::Mutex<Option<oneshot::Sender<(i32, Bytes)>>>, notify_id:
  AtomicU64 }}` and `LingerState { osd: Option<i32>, registered: bool,
  register_gen: u32, last_error: Option<i32>, watch_valid_thru:
  Option<Instant>, interval: Option<PgInterval> }` (`PgInterval` is Task
  3's). The guard is never held across an `.await`.
- Routing (`handle_watch_notify`): look the cookie up; unknown cookies
  are logged at debug and dropped. `DISCONNECT`: if `last_error` is none,
  set it to `ENOTCONN` (the new `error.rs` constant, already negative)
  and send `WatchEvent::Disconnect { code: ENOTCONN }`. `NOTIFY` on a
  watch linger: send `WatchEvent::Notify { notify_id,
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
  `rados/src/osdclient/ioctx.rs`, `rados/src/osdclient/error.rs`
  (`pub const ENOTCONN: i32 = -107;` and `pub const ETIMEDOUT: i32 =
  -110;` beside `ENOENT`), `rados/src/osdclient/osdmap.rs` (the up/acting
  split below), and the `watch` re-exports in `rados/src/osdclient/mod.rs`
  and `rados/src/lib.rs` (`Watcher`, `WatchEvent`).

**Interfaces and rules:**
- The shared submit helper: split the "build and submit once" part of
  `execute_op` (`client.rs:705-894`) into `async fn submit_once(&self,
  object: &ObjectId, ops: Vec<OSDOp>, priority: i32, extra_flags:
  OsdOpFlags) -> Result<(i32 /* osd */, u32 /* epoch */,
  oneshot::Receiver<Result<OpResult>>, ThrottlePermit<'_>)>` that does
  everything `execute_op` does before the await: the blocklist check,
  the throttle permit (`calc_op_budget`, held by the caller until the
  reply), `MOSDOp::new`, snapc, mtime for writes, the pause/full checks,
  placement through `object_to_osds_in_map`, `get_or_create_session`,
  tid and reqid, pgid and epoch stamping, then `session.submit_op`.
  `execute_op` becomes its retry loop around `submit_once` plus
  `await_op_result`, behaviour unchanged. Every linger op (register,
  reconnect, ping, ack, notify) goes through `submit_once` exactly once,
  with no retry, and takes a throttle permit like any other op. Every
  op `submit_op` registers carries the Tracker's `operation_timeout` (30
  s by default, `tracker.rs:25`), pings and acks included; a Tracker
  expiry reaches the linger code as an op error and is handled as the
  failure it is for that op kind.
- `OSDClient::linger_watch(self: &Arc<Self>, object, timeout) ->
  Result<Arc<Linger>>`: allocate the cookie, insert the linger, send
  `WATCH{Watch, timeout}` with `extra_flags` READ (so WRITE|READ) through
  `submit_once` WITHOUT `execute_op`'s connection-retry loop; on success
  mark `registered`, record the OSD and the `PgInterval`; on error
  remove the linger and return the error (`ENOENT` for a missing
  object).
- `send_reconnect(linger)`: a single `WATCH{Reconnect, gen:
  ++register_gen}` op, WRITE|READ, through `submit_once`, not retried;
  on success clear `last_error` and record the OSD and `PgInterval`; on
  error set `last_error` if none (mapping `ENOENT` to `ENOTCONN` as
  `_normalize_watch_error` does) and send one `Disconnect`.
- `on_session_reset`'s spawned body (Task 2 made it spawn and skip after
  shutdown): for every linger whose `osd == Some(osd_id)`: watches get
  `send_reconnect`, notifies are re-sent whole with `notify_id` reset to
  zero (the new primary allocates a new id). There is no separate
  reopen step: `submit_once` places the op against the current map and
  `get_or_create_session` replaces the dead session left in
  `OSDClient::sessions`. This also runs for sessions closed on purpose,
  since those end the io_task too.
- Interval test: `pg_to_acting_osds` (`osdmap.rs:2721`) returns one flat,
  post-override vector with the primary swapped to index 0, and no
  up/up_primary exists, so the reconnect rule ("any new interval",
  because the OSD rebuilds its watches on every activation, and an
  up-set change re-peers the PG even when `pg_temp` holds the acting set)
  needs the split. Add `pub fn pg_to_up_acting(&self, pg: &PgId) ->
  Result<UpActing, RadosError>` returning `UpActing { up: Vec<i32>,
  up_primary: i32, acting: Vec<i32>, acting_primary: i32 }`: split the
  private `apply_pg_overrides` (`osdmap.rs:3031-3110`) at the `pg_temp`
  step, so `up`/`up_primary` are CRUSH plus `pg_upmap`,
  `pg_upmap_items`, primary selection and `pg_upmap_primaries`, and
  `acting` is `pg_temp` (else `up`) and `acting_primary` is
  `primary_temp`, else `pg_temp`'s first valid OSD, else `up_primary`,
  as C++'s separate outputs, without the flat vector's overwrite of `[0]`.
  `pg_to_acting_osds` and its cache keep their current results, pinned
  by a unit test that its output's first element equals
  `acting_primary` across the existing placement fixtures. `PgInterval
  { up, up_primary, acting, acting_primary, size, min_size, pg_num,
  pgp_num }` is recorded at each successful (re)send; a new interval is
  any field differing, or the pool's `canonical_last_force_op_resend()`
  (`pub(crate)`, `osdmap.rs:1012`) newer than the epoch of that send.
  `sort_bitwise`, `recovery_deletes` and stretch-mode fields are not
  compared (constant on Squid and later; stretch pools unsupported,
  `is_stretch_pool` returns false). Because the interval lives on the
  linger, the old map is not needed.
- Map change: `handle_osdmap` and `update_osdmap_state` take `self:
  &Arc<Self>` (their one caller, the drain task, holds an upgraded
  `Arc`); after `osdmap_tx.send` publishes the new map and before
  `scan_requests_on_map_change`, recompute each linger's `PgInterval`
  against it; a changed interval spawns `send_reconnect` (watch) or a
  whole re-send (notify), never awaited on the drain task; a pool that
  vanished (the `fail_if_pool_deleted` pattern, `client.rs:1995-2010`)
  fails the linger with `ENOENT` and one `Disconnect`.
- The ping task: spawned in `OSDClient::new` beside the OSDMap drain
  task, holding a `Weak<OSDClient>` like it, cancelled by
  `shutdown_token`, `interval(5 s)`; each tick sends `watch_ping(cookie,
  register_gen)` for every watch linger that is `registered` with no
  `last_error`, as a WRITE|READ op through `submit_once` with no retry,
  recording `sent = Instant::now()`, each ping's reply awaited in its
  own spawned task; a reply whose generation is not the current one is
  ignored; a success sets `watch_valid_thru = sent`; the first error
  (`ENOTCONN`, `ETIMEDOUT`, a connection loss, a Tracker expiry) sets
  `last_error` and sends one `Disconnect`, after which the watch is not
  pinged again until re-registered. A migrated or drained ping (the
  generic resend path) is ignored by its generation.
- `OSDClient::unwatch(linger) -> Result<()>`: send `WATCH{Unwatch}` as a
  normal `execute_op` (a write; resend-safe), then remove the linger from
  the registry; return the op's result (`ENOENT` if the object is gone).
- `OSDClient::notify(self: &Arc<Self>, object, payload, timeout_ms) ->
  Result<NotifyResult>`: register a notify linger (cookie, oneshot);
  send `NOTIFY{cookie, timeout_secs, payload}` through `submit_once`
  where `timeout_secs` is `timeout_ms / 1000` or 10 when that is 0
  (librados's `client_notify_timeout`); decode the op reply's `u64` into
  `notify_id` (unless a completion already arrived); an op error removes
  the linger and returns it (`ENOENT` for a missing object); await the
  completion with an outer bound of `timeout_secs + 30 s` (a lost
  `NOTIFY_COMPLETE` after a reset is recovered by the re-send, the bound
  only protects against a dead OSD; on expiry return
  `OSDClientError::Timeout`); decode the DATA into `NotifyResult` with
  `timed_out = return_code == ETIMEDOUT`; remove the linger.
- `IoCtx` surface, following the file's `oid: impl Into<String>`
  convention (`list_watchers`, `ioctx.rs:730`) and `object_id()` for
  namespace and locator: `pub async fn watch(&self, oid: impl
  Into<String>) -> Result<Watcher>` and `watch_with_timeout(oid, secs)`;
  `pub async fn notify(&self, oid: impl Into<String>, payload: Bytes,
  timeout_ms: u64) -> Result<NotifyResult>`; `pub fn instance_id(&self)
  -> u64` returning the client's global id (librados's
  `rados_get_instance_id`; `OSDClient.global_id` is private, so add
  `OSDClient::global_id(&self) -> u64` behind it); `pub struct Watcher {
  cookie: u64, events: mpsc::UnboundedReceiver<WatchEvent>, .. }` with
  `pub fn cookie(&self)`, `pub async fn recv(&mut self) ->
  Option<WatchEvent>`, `pub async fn notify_ack(&self, notify_id: u64,
  reply: Bytes) -> Result<()>` (a `NOTIFY_ACK` op through `submit_once`;
  the OSD's reply is 0, awaited so the send is not dropped), `pub async
  fn unwatch(self) -> Result<()>`, `pub fn check(&self) ->
  Result<Duration>` (the age of `watch_valid_thru`, or `last_error` as
  the error; librados's `watch_check`), and `Drop` that removes the
  linger and stops pings. `pub enum WatchEvent { Notify { notify_id:
  u64, notifier_gid: u64, payload: Bytes }, Disconnect { code: i32 } }`.
- Test hooks, `cfg(test)`-free but documented as test aids:
  `OSDClient::set_watch_pings_enabled(bool)` (like
  `objecter_inject_no_watch_ping`) so the cluster test can let a watch
  time out, and `OSDClient::close_session_for_test(osd: i32)`, which
  closes that session through `OSDSession::close()` (and so through the
  io_task exit hook), reached from `IoCtx` by
  `close_primary_session_for_test(oid)`, which resolves the object's
  current primary with `object_to_osds_in_map` and calls it.

Unit tests without a cluster: the ping task skips lingers with
`last_error`; a stale-generation ping reply changes nothing; a
`Disconnect` is sent once; `pg_to_up_acting` against the existing
placement fixtures (with and without `pg_temp`/`primary_temp`) and the
`PgInterval` comparison (a `pg_temp`-only change is a new interval, an
unrelated pool's change is not).

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
- Modify: `.github/workflows/test-with-ceph.yml` (add `watch_notify` to
  both `for test in` lists, the CRC-on loop and the CRC-off loop, so the
  `rados` integration steps run this suite with CRC on and off).

Tests (all `#[ignore]`), mirroring `test/librados/watch_notify.cc`; two
clients where a second gid is needed (`create_ioctx` builds a fresh
`Client` per call, so two calls give two `OSDClient`s and two gids);
"A's gid" is `A.instance_id()` (Task 3), and a `list_watchers` entry's
gid is `WatchItem.name.num.get()`:

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
   watches `o`; B removes `o`; A receives `Disconnect { ENOTCONN }` within
   30 s; `A.check()` is `Err(ENOTCONN)`; `A.unwatch()` is `ENOENT`.
6. `watch_times_out_without_pings` (`Watch3Timeout`): A watches with
   timeout 4 s, `check()` is `Ok` under a second, pings are disabled with
   the hook; within 10 s A receives `Disconnect { ENOTCONN }` (the OSD
   times the pinged watch out with the session up and sends DISCONNECT);
   B's notify then returns no acks and no missed; A re-watches (pings back
   on) and B's next notify gets one ack.
7. `watch_survives_a_session_reset`: A watches `o`; the controller closes
   A's session to the object's primary through the test-only
   `IoCtx::close_primary_session_for_test(oid)` over
   `OSDClient::close_session_for_test(osd: i32)` (added in Task 3 next to
   the ping hook); within 10 s B's notify is acked by A (the reconnect
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

## Pre-flight patches applied (2026-09-25)

Evidence from `.superpowers/sdd/2026-09-25-rgw-mvp-12-watch-notify/preflight.md`
(tree 81c5bc5, paths under `rados/src/`).

1. `CEPH_OSD_OP_WATCH` `0x120f` -> `0x220f` in the wire facts; opcode values pinned in Task 1 tests and Review Focus 1 (`types.rs:486` `MODE_WR = 0x2000`; ceph `rados.h:230,295`; NOTIFY/ACK `rados.h:260-261` unchanged).
2. `MWatchNotify::decode(header_version: u16, ..)` via `msg.header.version.get()` (`msgr2/header.rs:32`).
3. OSD ids `i32`: `LingerState.osd`, `on_session_reset`, `close_session_for_test` (`session.rs:114`, `client.rs:80`).
4. Task 2 reset hook only on the io_task exit path, `tokio::spawn`ed, skipped once `shutdown_token` is cancelled; dropped the calls from `close_stale_sessions`/`collect_resend_ops` (`client.rs:1843-1861`, `1819-1821`; `session.rs:982-998` `close()` awaits the `JoinHandle`); Task 3's "reopen" folded into `submit_once` + `get_or_create_session` (`client.rs:402`).
5. Task 3 names `submit_once`, split out of `execute_op` (`client.rs:705-894`), with the throttle permit (714) and the Tracker's 30 s `operation_timeout` on every op (`session.rs:733-737`, `tracker.rs:25`); `ThrottlePermit<'_>` is `throttle.rs:164`.
6. `is_new_interval` (none exists) replaced by a new `OSDMap::pg_to_up_acting` split out of `apply_pg_overrides` (`osdmap.rs:3031-3110`) and a `PgInterval` recorded on the linger; chose the up/acting split because the reconnect rule is "any new interval" and `pg_to_acting_osds` (`osdmap.rs:2721`) has no up set; `handle_osdmap`/`update_osdmap_state` take `&Arc<Self>` (sole caller `client.rs:266`), linger scan after `osdmap_tx.send` (2340), before the op scan (2365).
7. `ENOTCONN` (-107) and `ETIMEDOUT` (-110) added to `osdclient/error.rs` (list at 8-14), not `libc`; event codes written as the negative constants (`ENOTCONN`, not `-ENOTCONN`) throughout Tasks 2-4.
8. `osdclient/mod.rs` (13-32, 34-53) and `src/lib.rs` (41-47) named for `pub mod watch` and re-exports; `watchers.rs` noted as separate.
9. `IoCtx::instance_id()` over a new `OSDClient::global_id()` in Task 3 (`client.rs:87` private); tests use it and `WatchItem.name.num.get()` (`watchers.rs:18-27`, `types.rs:318-323`).
10. `LingerState` (and the notify completion slot) under `std::sync::Mutex`; `handle_watch_notify` never awaits (`session.rs:15` is tokio's `Mutex`).
11. `IoCtx::{watch, watch_with_timeout, notify}` take `oid: impl Into<String>` (`ioctx.rs:730`).
12. "rados-rs today": drain-path migration and whole-session drain (`client.rs:1697-1761`, 1712-1719), `pg_num` restamp without resend (1829-1833); per-call timeouts (`client.rs:126`, 353) and msgr2 sleeps (`msgr2/throttle.rs:372`, `msgr2/protocol.rs:1626`); Tracker stopped by `tracker.shutdown()` (`client.rs:2081`); `execute_op` up to five attempts under `MAX_CONNECTION_RETRIES = 4` (781, 908, 932); `submit_op` keyed by tid (`session.rs:696`).
13. Global Constraint added: `require_osd_release` public (`osdmap.rs:2530`) but `apply_to` discards `new_require_osd_release` (2151); a later package fixes it.
14. "No new dependencies" confirmed in Tech Stack (`Cargo.toml:31`; `rados/Cargo.toml:82,83,88,101`).
15. Task 4 workflow: `watch_notify` added to both `for test in` lists (`test-with-ceph.yml:93,113`); test 7 reaches the hook through `IoCtx::close_primary_session_for_test(oid)` (new, so the test can name the primary).
