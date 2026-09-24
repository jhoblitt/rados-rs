# rados-rs: the client surface an MVP Rust RGW needs

Design for a fork of tchaikov/rados-rs (jhoblitt/rados-rs) that adds what a
single-site RGW port needs from a RADOS client, in a form upstream can take
one pull request at a time. Written 2026-09-24 against rados-rs at its
2026-05-19 head and ceph/ceph `main` at `e234256339f`. Ceph floor: Squid.

## Goal

A Rust RGW driver for RADOS needs four things from its client library that
rados-rs does not have today: omap operations, `cmpxattr` and a few small
ops, watch/notify with the persistence librados calls a linger op, and
client encodings for the object classes RGW calls. After this work, every
RADOS interaction of RGW's single-site data and metadata paths can be
expressed against the fork.

## Non-goals

- Multisite: no `cls_log`, `cls_timeindex`, `cls_fifo`, `cls_2pc_queue`,
  `cls_cmpomap`, no bucket-index log (`bilog`) or `bi_*` repair ops, no
  data or metadata log.
- Resharding as an action: the reshard queue ops are out. The resharding
  guard and `get_bucket_resharding` are in, because C++ RGW attaches the
  guard to every bucket-index write and a compatible client must too.
- `cls_otp` (MFA), `copy_from2` (the MVP copies by read and write),
  `rgw_s3select` usage data beyond what the usage-log types carry.
- The RGW driver itself. This fork carries protocol and transport only;
  nothing here knows RGW's pool layout or object naming.

## Constraints inherited from upstream

The fork follows rados-rs's own `.claude/CLAUDE.md`, so that every branch
is an upstream candidate:

- Upstream's minimum Ceph is Quincy; this fork's new code sets its floor
  at Squid (v19). Every new `decode_content` calls `check_min_version!` at
  the version Squid emits, no `decode_if_version!` branches are written
  for formats older than that, and the only cluster the tests run against
  is v19. Supporting Quincy or Reef is deliberately not invested in; if an
  upstream review asks for it on a package, that is the owner's call at
  that time.
- Encoding goes through `Denc`; raw buffer reads and writes appear only
  inside `Denc` impls. Duplicate constants across modules are a smell.
- No `unwrap` or `expect` on production paths; errors propagate with `?`.
- Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace --all-targets`,
  and the cluster tests behind `#[ignore]` that CI runs against a
  single-container Ceph v19.2.2.
- Field names are simple (`sec`, not `tv_sec`); JSON dumps for the corpus
  harness use custom `Serialize` impls where names differ.

## Architecture

Two layers, both in the fork.

**Layer one extends the OSD client** in the `rados` crate: new op codes and
op-data variants, `OpBuilder` methods, typed reply decoders, and one new
inbound message path. It keeps the crate what upstream says it is, a
RADOS client.

**Layer two is a new workspace crate, `rados-cls`**, one module per object
class behind a Cargo feature of the same name (`version`, `refcount`,
`user`, `queue`, `rgw-gc`, `rgw`). It is pure protocol: request and reply
structs with `Denc` derives, and async functions that issue a `Call` op
through `IoCtx::exec` or add one to a builder for compound use, then decode
the reply. Each function mirrors one function of the corresponding
`cls_*_client.h`. `cls_lock` stays in the core crate where it already
lives.

The split exists so upstream can accept the transport work on its own
merits and take or leave RGW-specific encodings crate by crate, and so an
RGW build enables only the classes it calls.

## Components

### omap (branch `omap-ops`)

The ten `OMAP*` op codes from `rados.h`, each with an `OpBuilder` method
whose encoded arguments follow `Objecter.h` exactly (for example
`omap_get_vals` encodes `start_after`, `max_to_get`, `filter_prefix` in
that order; `omap_cmp` encodes a map of key to value and comparison
operator). Reply decoders return `BTreeMap<String, Bytes>` plus the `more`
flag where the OSD sends one. `IoCtx` gains single-op conveniences. The
builder path matters more than the conveniences: RGW batches omap writes
with xattr and data ops in one transaction, and a compound op is the only
way to get the OSD's atomicity.

### cmpxattr and small ops (branch `cmpxattr-small-ops`)

`cmpxattr` reuses the existing `Xattr` op-data's comparison fields, which
the C++ `ceph_osd_op` shares between get, set and compare. `setallochint`
adds an op-data variant. `zero` uses the existing extent variant. Bulk
`getxattrs` decodes a map. `assert_exists` is the builder form RGW uses
before overwrites. `list_watchers` decodes the watcher list and belongs
here rather than with watch/notify because it is a plain read op.

### watch/notify (branch `watch-notify`)

The one piece with state. Public surface: `IoCtx::watch(oid)` returns a
`Watcher` owning a cookie and a receiver of `WatchEvent::{Notify,
Disconnect}`; `Watcher::notify_ack(notify_id, reply)`; `Watcher::unwatch()`;
`IoCtx::notify(oid, payload, timeout)` resolves to the acks and the
timed-out watchers. Internals:

- A linger registry in `OSDClient`. A watch is an op that must be re-sent
  as `CEPH_OSD_WATCH_OP_RECONNECT` whenever its session resets or its PG
  maps elsewhere, because the OSD drops a watch whose client session is
  gone; Objecter's `_send_linger` is the reference. It is pinged with
  `CEPH_OSD_WATCH_OP_PING` on a timer well inside the OSD's watch timeout,
  as `_send_linger_ping` does, and a failed ping surfaces as `Disconnect`.
- A decoder for `MWatchNotify` (`CEPH_MSG_WATCH_NOTIFY`, the one OSD
  message type the client ignores today): cookie, notify id, opcode,
  payload, notifier gid. Routing is by cookie for notify and disconnect
  events and by notify id for notify completion.
- Notify is a read op whose reply carries the notify id; completion
  arrives later as a message, so the pending notify is a registry entry
  with a timeout, not a request future.

RGW uses this for metadata-cache invalidation across gateways; a single
gateway can run without it, which is why it is its own branch and can
land last.

### `rados-cls` (branches `cls-crate` onward)

- `cls-crate`: the crate, its feature layout, the `version` module
  (`cls_version_set/inc/read/check`) and `refcount`
  (`get/put/read/set`). Both are small and prove the pattern.
- `cls-user`: bucket list, set and remove, header and stats reset, the
  account-resource functions.
- `cls-queue-gc`: `cls_queue` (init, enqueue, list, remove, capacity) and
  `cls_rgw_gc` queue functions on top of it.
- `cls-rgw-types`: the shared types every rgw module encodes
  (`rgw_bucket_dir_entry` and its meta, `rgw_bucket_entry_ver`,
  `rgw_bucket_dir_header`, `cls_rgw_obj_key`, the category stats, the
  pending-info map, `rgw_usage_log_entry`, the GC chain types, the LC
  head and entry, the OLH entry and log entry). This branch is the base
  the remaining rgw branches stack on until it merges.
- `cls-rgw-bucket-index`: prepare, complete, list, dir header, init and
  init2, check and rebuild, suggest changes, remove obj, check mtime,
  check attrs prefix, store pg ver, set tag timeout, update stats, and the
  resharding guard with `get_bucket_resharding`.
- `cls-rgw-gc`: set entry, defer, list, remove.
- `cls-rgw-usage`: add, read, trim, clear.
- `cls-rgw-lc`: head get and put, entry get, set, rm, next, list.
- `cls-rgw-olh`: link, unlink instance, read and trim the OLH log, clear.

Every struct records the `ENCODE_START` version and compat it mirrors, and
its floor is what C++ RGW on Squid writes.

## Data flow

A request: the caller builds an `OpBuilder` (possibly several ops), the
client maps the object to a PG and OSD, encodes `MOSDOp`, and awaits
`MOSDOpReply`; per-op return codes and outdata come back in `OpResult`,
already the shape compound reads need. Nothing changes here except new op
kinds.

A watch: `watch` registers the linger entry, sends the initial `WATCH` op,
and returns the handle; the registry re-sends on session and map events
and pings on its timer. A `MWatchNotify` arriving on any OSD session is
decoded and routed by cookie; the handle's channel delivers it.
`notify_ack` is an ordinary op carrying the notify id and cookie.

A cls call: the module encodes the request struct, issues `Call` with the
class and method names, and decodes outdata into the reply struct. Errors
are the OSD's errno for that op.

## Error handling

Per-op return codes stay errno-shaped so RGW can map them as the C++ does
(`ENOENT`, `EEXIST`, `ECANCELED` from a failed guard, `EBUSY` from a lock).
Decode failures are `RadosError`. Watch disconnects are events on the
watcher, never errors on unrelated ops. A notify that times out returns
the list of watchers that did not ack; it is not an error.

## Testing

- Unit: encode and decode round trips for every new struct and op
  payload, with hand-checked bytes for the small ones.
- Corpus: for every new type the ceph-object-corpus carries (146 RGW types
  across its archives, including `rgw_bucket_dir_entry`, `RGWUserInfo`
  and the `rgw_cls_*_op` structs), the existing dencoder comparison
  harness proves decode, re-encode and cross-implementation equality
  against `ceph-dencoder`.
- Cluster: `#[ignore]` tests against the CI's single-container Ceph v19.2.2,
  run locally the same way under podman, calling each op and each cls
  method against a real OSD with the class loaded. Watch/notify adds a
  two-client test and a watch that survives an OSD restart.
- Gates as upstream's CI defines them.

## Branches and pull requests

Fork `main` mirrors upstream and never diverges. Each package is a branch
off upstream `main`, reviewed and CI-checked as a draft PR against the
fork's `main` that is never merged there; upstream PRs open from the same
branches when the owner says so, at most three at a time before checking
in. An `integration` branch merges every package so the RGW port can
depend on one git revision. The rgw branches stack on `cls-rgw-types`
until it merges, then rebase.

## Risks

- Watch/notify is two thirds of the transport work and the only part with
  failover semantics; it is validated only by the cluster tests.
- Upstream's pace and taste are the owner's; a package he declines lives
  on in the integration branch at a rebase cost per upstream commit.
- Encoding versions: a type whose Squid floor is misjudged decodes
  nothing from a real cluster. The corpus harness catches the shape;
  cluster tests against v19 catch the floor.
- Floor mismatch with upstream: packages carry Squid floors where upstream
  keeps Quincy. That is a known divergence, not an oversight, and the cost
  of closing it is not part of this work.
