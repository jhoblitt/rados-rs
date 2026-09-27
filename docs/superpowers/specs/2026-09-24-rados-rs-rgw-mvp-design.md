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

What "RGW's single-site paths" means is decided by the sibling rgw-go
effort's exclusion list, `rgw-go/docs/exclusions.md`, published at
`https://github.com/jhoblitt/rgw-go/blob/main/docs/exclusions.md`, which
the owner
made canon for this fork on 2026-09-25: the feature set is what a
drop-in replacement for radosgw in a Rook cluster needs, sized by Rook's
object integration suite (`TestCephObjectSuite`) without multisite, and
every feature that list excludes is excluded here, every feature it
keeps is in scope here. This fork tracks that document; it does not
maintain a list of its own.

## Non-goals

- Multisite: no `cls_log`, `cls_timeindex`, `cls_fifo`, `cls_cmpomap`, no
  bucket-index log (`bilog`; a single-zone zonegroup leaves `log_data`
  off, so radosgw writes none either), no data or metadata log, no sync
  of any kind.
- The `bi_get`, `bi_put` and `bi_list` index methods: `bi_get` backs
  radosgw's index-repair readers and `bi_put`/`bi_list` its resharding
  and admin-tool paths, so they are out only while no path in scope
  needs them, not because of the multisite entry (rgw-go's document says
  the same). A later package can add them without touching this spec's
  goals.
- Persistent bucket notifications are in scope at the RADOS layer
  (`cls_2pc_queue` and the queue locks, package `cls-2pc-queue`); only
  Kafka and AMQP delivery are out, and delivery of any kind is RGW-level.
- Resharding as an action: the reshard queue ops are out. The resharding
  guard and `get_bucket_resharding` are in, because C++ RGW attaches the
  guard to every bucket-index write and a compatible client must too.
- `copy_from2` (radosgw does not use it: a copy shares
  the source's tails through `cls_refcount` under a new tag and streams
  data only when placement, storage class, encryption or head geometry
  force it, which the `refcount` module supports),
  `rgw_s3select` usage data beyond what the usage-log types carry.
- RGW features excluded from the driver, as rgw-go's exclusion list
  (`rgw-go/docs/exclusions.md`, canon for this fork since 2026-09-25)
  states them, listed here so that nothing in this fork is sized or
  tested for them; that document, not this summary, is authoritative: the D4N cache layer and the D3N
  read cache, the POSIX driver and every other non-RADOS driver, the
  Swift API (and with it the object expirer), Lua scripting, S3 Select,
  S3 Vectors and dedup, cloud transition and restore, Keystone and LDAP
  authentication, key management (SSE-KMS and SSE-S3 with every
  backend; SSE-C stays), the dynamic resharding worker (the guard stays,
  as above), librgw with NFS and SMB, QAT and UADK offload, and Kafka
  and AMQP notification delivery. None of them needs RADOS surface the packages below lack, so this is a
  driver-scope statement, not a transport one. What rgw-go keeps and
  this fork lacked, the roadmap now carries: `cls_lock` (the GC,
  lifecycle and reshard workers and the notification queues take
  radosgw's locks by name), `cls_2pc_queue` (persistent notifications)
  and `cls_otp` (MFA). Feature level follows rgw-go's encoding rule, which the owner
  adopted for rgw-rs on 2026-09-25: a gateway decodes every struct
  version the C++ decoders accept (no release floor for stored
  metadata, since never-rewritten users and buckets keep their original
  encoding) and encodes each persistent type at the version the
  cluster's own radosgw release writes, detected once from the OSD map's
  required OSD release and overridable for testing. At this fork's
  layer that rule splits by who writes the bytes: the bucket-index, OLH,
  GC, usage and lifecycle records are written and re-encoded by the
  OSD's object class at that OSD's version, so `rados-cls` encodes
  requests at the versions Squid's radosgw sends, decodes replies up to
  what `main` writes, and floors its decoders at Squid (every reply is
  re-encoded by the OSD; the raw-omap readers that would see older
  stored bytes, `bi_get` and `bi_list`, are out); where a later release changed a request or reply shape (`update_stats`
  version 2 in Tentacle v20.2.0; `read_olh_log` version 2 with
  `get_stales` and the version-8 index-entry metadata in Umbrella
  v21.1.0; the OLH epoch reply on link and unlink only on `main`, unreleased),
  the `release-shapes` package offers each shape, selected by the
  cluster's required OSD release, and the `rados` crate exposes that
  release for the driver's detection seam. radosgw itself does not key
  on the release: it sends the newest request shape with compat 1
  (older OSDs ignore the tail) and probes new methods for
  `EOPNOTSUPP`, so during a rolling upgrade it sends newer shapes than
  the map's release; keying on the map is stricter and never sends an
  OSD a shape it might not accept, which is the trade the rule makes. The driver-level types the gateway
  writes itself are rgw-rs's, and there the rule applies in full.
  Under Rook the rule meets stored bytes from a newer release than the
  cluster's. The operator writes the realm, zonegroup, zone and periods
  in `.rgw.root` with its own radosgw-admin, and Rook v1.20.7's operator
  image carries Ceph 20.2.4. So on a Squid v19.2.6 cluster `zone_info.*`
  is stored at struct version 18, Tentacle's (v20.2.4
  `src/rgw/driver/rados/rgw_zone.h:160`), where Squid's radosgw writes 15
  (v19.2.6 `:156`). This was verified on a rooket cluster on 2026-09-27
  after rgw-go reported it. The decode rule already covers these bytes,
  and a gateway that rewrites them writes the Squid encoding, as the
  cluster's own radosgw would.
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
`cls_*_client.h`. The core crate's existing `cls_lock` helpers
(`IoCtx::lock_exclusive`, `lock_shared`, `unlock`) stay as upstream's API;
the complete mirror of the class, all seven methods, is the `cls-lock`
package of `rados-cls`, and routing the core helpers through it is a
follow-up.

The split exists so upstream can accept the transport work on its own
merits and take or leave RGW-specific encodings crate by crate, and so an
RGW build enables only the classes it calls.

## Components

### omap (branch `omap-ops`)

The ten `OMAP*` op codes from `rados.h`, each with an `OpBuilder` method
whose encoded arguments follow `Objecter.h` exactly (for example
`omap_get_vals` encodes `start_after`, `max_to_get`, `filter_prefix` in
that order; `omap_cmp` encodes a map of key to value and comparison
operator). Reply decoders return `BTreeMap<Bytes, Bytes>` plus the `more`
flag where the OSD sends one; keys are byte strings, never `String`, because
RGW's bucket index uses bytes outside UTF-8. `IoCtx` gains single-op conveniences. The
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
- `cls-rgw-bucket-index`: prepare, complete, list, dir header, init
  (`bucket_init_index2` is post-Squid and answers `EOPNOTSUPP` on v19),
  check and rebuild, suggest changes, remove obj, check mtime,
  check attrs prefix, store pg ver, set tag timeout, update stats, and the
  resharding guard with `get_bucket_resharding`.
- `cls-rgw-gc`: set entry, defer, list, remove.
- `cls-rgw-usage`: add, read, trim, clear.
- `cls-rgw-lc`: head get and put, entry get, set, rm, next, list.
- `cls-rgw-olh`: link, unlink instance, read and trim the OLH log, clear.
- `cls-lock`: lock, unlock, break, get info, set cookie, assert locked,
  with radosgw's lock names, cookies and durations documented per worker.
- `cls-2pc-queue`: init, get capacity, reserve (a writing method that
  replies, so `RETURNVEC`), commit, abort, list entries, list
  reservations, remove entries, expire reservations; what
  `rgw_notify.cc` does with them.
- `cls-otp`: create, remove, set, check and get, and the MFA seed types.
- `object-locator-routing` (after `watch-notify`): the OSD client places
  an object by its full locator (pool, namespace, key), not by its name
  alone. A review of `watch-notify` found `object_to_osds_in_map`
  ignores the namespace and locator key, so operations on namespaced
  objects reach the wrong OSD and hang (reproduced on v19.2.2); RGW's
  metadata and log pools live in namespaces.
- `cephx-aes256krb5` (after `object-locator-routing`): the cephx key type
  `CEPH_CRYPTO_AES256KRB5` (AES256-CTS-HMAC-SHA384-192, RFC 8009) that
  v19.2.6 and v20.2.4 add and make the default for new keys; without it
  the client cannot read a keyring a fresh current Squid or Tentacle
  cluster (Rook included) hands out. Reported by the rgw-go session,
  verified in `ceph_fs.h` at the tags.
- `release-shapes` (after `watch-notify`): the cluster's required OSD
  release exposed from the OSD map (and kept through incrementals, which
  today discard it, as C++ applies it when the byte read as `i8` is not
  negative) with a config override; the later request shapes of the
  `rgw` class selected by release (`update_stats` version 2, Tentacle;
  `read_olh_log` version 2 with `get_stales` and metadata version 8,
  Umbrella), with the unreleased `main`-only OLH epoch reply deferred
  until it ships; the docs the crate got wrong about later releases
  corrected (the null-version bilog flag shipped in Squid v19.2.3, the
  class guards its own index writes from v20.2.0, `ReshardStatus` gains
  `IN_LOGRECORD`). The resharding-only methods (`bucket_init_index2`,
  `bi_put_entries`, `reshard_log_trim`) and the cloud-restore
  `bucket_refresh_instance` stay out with the features they serve.

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

## Upstream Ceph bug registry

Added 2026-09-27 at the owner's request. Work on this fork keeps meeting
defects in ceph/ceph itself, and a client that must coexist with radosgw
has to work around them. `docs/superpowers/ceph-upstream-bugs.md` on this
branch records each one, so the knowledge outlives the plan or review
that found it.

- What belongs: a defect in ceph/ceph's own behaviour that a correct
  client meets. That covers a hang, a crash, a wrong result, accounting
  that drifts, an encoder and decoder that disagree, a race a client must
  retry around, and code that contradicts its own documentation. A
  difference between releases belongs only when the older behaviour was
  a defect that a later release fixed.
- What does not: bugs in rados-rs, which are fixed here; problems in
  Rook, rooket or a deployment, which go to their own trackers;
  intentional release differences, which the release-shapes docs cover;
  and C++ behaviour that is merely surprising, which the parity notes in
  the code cover.
- Every entry records the component, the symptom as a client sees it,
  the releases affected, and the release and commit that fixed it if
  any. It also records the evidence, as `tag:path:line` and how it was
  established: source reading, `ceph-dencoder`, or a named cluster test.
  Then how rados-rs handles it (a workaround, a doc note, or a test that
  pins it, naming the commit or PR), who found it and when, and its
  upstream report status.
- An entry is `confirmed` only when that evidence is in it; otherwise it
  is `suspected`, with what still has to be checked. Entries keep stable
  IDs (`CEPH-BUG-NNN`) and are updated in place: when a later release
  fixes one, when a claim is corrected, or when it is filed upstream.
- Whoever confirms a Ceph defect adds or updates its entry in the same
  session: a plan, an implementer, a reviewer or the controller. A plan
  that finds one lists the entry in its Roadmap, and the controller
  checks the registry before closing each package.
- Reporting a bug on tracker.ceph.com or opening a ceph/ceph PR is
  outward-facing and needs the owner's explicit instruction for that
  report. Until then the status is `not filed`.
- The rgw-go effort shares this cluster layout and meets the same Ceph.
  New entries are sent to the rgw-go session, and bugs it confirms are
  added here, verified as for any other entry.

## Branches and pull requests

Each package is a branch off upstream `main`, reviewed and CI-checked as
a draft PR against the fork's `main` and merged there once CI is green,
as a merge commit that keeps the branch because the upstream PR points
at it; the fork's `main` is therefore the one git revision the RGW port
depends on. As each fork PR merges, an upstream PR opens from the same
branch; it stacks on the fork-merged packages not yet upstream. The rgw
branches stack on `cls-rgw-types` until it merges, then rebase.

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
