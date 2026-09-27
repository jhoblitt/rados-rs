# rados-rs RGW MVP, plan 17 of N: `object-locator-routing`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Place every object by its full locator (pool, namespace,
locator key, optional hash override) exactly as Ceph's
`OSDMap::object_locator_to_pg` and `pg_pool_t::raw_pg_to_pg` do, and make
every placement and re-placement path in the OSD client use that one
computation. Today placement uses the name alone: an op on a namespaced or
keyed object goes to a PG that does not contain its hash, the OSD drops it
without replying, and the op hangs (5 of 6 `write_full`s in a namespace on
v19.2.2). RGW's metadata and log pools live in namespaces, and its
multipart and shadow objects carry locator keys.

**Architecture:** The computation moves to the owner of the pool: `PgPool`
gains `hash_key` (the pool's `object_hash` selects rjenkins or linux),
`pg_num_mask` and `raw_pg_to_pg` (`ceph_stable_mod`); `OSDMap` gains
`object_locator_to_pg` (raw PG, as C++) and `raw_pg_to_pg`. The OSD
client's `object_pg_in_map` takes an `ObjectId` and returns both the raw
hash and the actual PG from that one call, and every routing decision
stamps the MOSDOp's hobject hash and pgid together from it. No public
item is removed. `crush::placement::object_to_pg`/`object_to_osds` and
`ObjectId::calculate_hash` already take the key and namespace, so they
are fixed in place: `ceph_stable_mod` replaces `hash % pg_num`, and one
shared hash-input helper replaces their three copies; they keep rjenkins
because they never see the pool (every pool a v19 monitor creates uses
it), and their docs say so and point at the locator path. The name-only
`OSDMap::object_to_pg`/`object_to_osds` gain the pool's hash type and
`ceph_stable_mod`, and are `#[deprecated]` towards
`object_locator_to_pg`, because a name alone cannot place a namespaced
or keyed object. No internal caller uses any of them afterwards.
Listing carries the IoCtx's namespace (or all namespaces) in the PGNLS
request and returns each entry's namespace and locator key. The MOSDOp
wire format is unchanged.

**Tech Stack:** as plan 3. No new dependency (`serde_json` is already a
`rados` dev-dependency, `rados/Cargo.toml:112`, for parsing the
monitor's `osd map` reply in the cluster tests).

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`object-locator-routing` (after `watch-notify`)". One widening from the
research: the fix also replaces `hash % pg_num` with `ceph_stable_mod`
and honours the pool's hash type, because both sit in the same function
and a non-power-of-two `pg_num` misplaces even default-namespace objects.
A second widening, by controller ruling: listing by namespace (Task 4),
because RGW lists its metadata by namespace and the same namespace field
is what the PGNLS request lacks.
Out: cache-tier overlays (`Objecter::_calc_target` redirects to
`read_tier`/`write_tier` before placement, `src/osdc/Objecter.cc:2820-2833@v19.2.2`;
rados-rs has no tiering and this plan adds none).

## Global Constraints

Plans 3 to 12's Global Constraints apply where they fit (commit style,
gates, push after every gate, merge when green, upstream PR, offline
builds, no new dependencies). Plus:

- Branch `object-locator-routing` is based on the fork's `main` at
  c482382 (plan 12, `watch-notify`, merged: its tip aafec4e plus the
  merge commit). Everything lands in the `rados` crate;
  `rados-cls` is untouched.
- Ceph's placement, all at v19.2.2 (`git show v19.2.2:<path>` in
  `~/github/ceph`):
  - `OSDMap::object_locator_to_pg` (`src/osd/OSDMap.cc:2626-2637`): if
    `loc.hash >= 0` the raw PG is `pg_t(loc.hash, loc.pool)` after checking
    the pool exists (`-ENOENT` otherwise); else `map_to_pg(loc.pool,
    oid.name, loc.key, loc.nspace)`.
  - `OSDMap::map_to_pg` (`src/osd/OSDMap.cc:2606-2624`): `-ENOENT` for a
    missing pool; `ps = pool->hash_key(key, nspace)` when `key` is
    non-empty, else `pool->hash_key(name, nspace)`; the result is the RAW
    `pg_t(ps, pool)`, full 32-bit `ps`, not yet reduced.
  - `pg_pool_t::hash_key` (`src/osd/osd_types.cc:1785-1796`): empty
    namespace hashes `key` alone; otherwise the bytes `nspace + '\037' +
    key`; the function is `ceph_str_hash(object_hash, ...)`.
  - `ceph_str_hash` (`src/common/ceph_hash.cc:95-105`) dispatches on the
    pool's `object_hash`: `CEPH_STR_HASH_LINUX` 1, `CEPH_STR_HASH_RJENKINS`
    2 (`src/include/ceph_hash.h:4-5`), anything else returns `-1`. The
    linux hash is `hash = (hash + (c << 4) + (c >> 4)) * 11` over
    unsigned bytes in `unsigned` arithmetic (`ceph_hash.cc:83-92`). A v19
    monitor creates every pool with rjenkins (`src/mon/OSDMonitor.cc:8239`),
    so linux appears only on pools from old clusters.
  - `pg_pool_t::raw_pg_to_pg` (`src/osd/osd_types.cc:1806-1810`):
    `ps = ceph_stable_mod(ps, pg_num, pg_num_mask)`, with
    `pg_num_mask = (1 << cbits(pg_num - 1)) - 1` (`osd_types.cc:1662-1666`)
    and `ceph_stable_mod(x, b, bmask) = (x & bmask) < b ? x & bmask : x &
    (bmask >> 1)` (`src/include/rados.h:96-102`); `OSDMap::raw_pg_to_pg`
    looks up the pool and delegates (`src/osd/OSDMap.h:1432-1436`).
  - `Objecter::_calc_target` (`src/osdc/Objecter.cc:2783`): resolves the
    pool from `base_oloc.pool` (2797), copies `base_oid`/`base_oloc` to
    `target_oid`/`target_oloc` (2821-2822), computes the raw pgid with
    `object_locator_to_pg(target_oid, target_oloc)` (2842-2847; a
    `precalc_pgid` PG op uses `base_pgid` instead, 2836-2840), reduces it
    with `ceph_stable_mod(pgid.ps(), pg_num, pg_num_mask)` (2862-2863),
    maps that actual PG to up/acting (2864-2870), and keeps the raw pgid in
    `t->pgid` (2934, `Objecter.h:1847`) and the actual one in
    `t->actual_pgid` (2944-2953). The namespace, key and hash come only
    from the locator; the name contributes only when `key` is empty.
  - The MOSDOp carries both: `op_target_t::get_hobj`
    (`src/osdc/Objecter.h:1886-1893`) builds the hobject with hash
    `target_oloc.hash >= 0 ? target_oloc.hash : pgid.ps()` (the raw ps),
    and the v9 encode writes `pgid` (the actual spg) then
    `hobj.get_hash()`, then the locator derived from the hobject
    (`src/messages/MOSDOp.h:401-402, 131-135`); the OSD rebuilds
    `hobj.key`/`hobj.nspace` from the locator (`MOSDOp.h:606-608`).
  - The OSD's check, and why a misplacement hangs rather than fails:
    `PrimaryLogPG::do_op` drops any op whose hobject hash is not contained
    in the PG it was sent to, logging "does not contain" and sending no
    reply (`src/osd/PrimaryLogPG.cc:1972-1982`).
  - An hobject whose key equals its name stores an empty key
    (`src/common/hobject.h:136-142`), and `Objecter::linger_register`
    clears such a key (`Objecter.cc:805-807`); the hash input is the same
    either way, so rados-rs need not normalise it.
- Ceph's listing by namespace, at v19.2.2: `IoCtxImpl::nlist` copies the
  IoCtx's namespace into the list context (`src/librados/IoCtxImpl.cc:529`);
  `Objecter::list_nobjects` sends the PGNLS with `object_locator_t
  oloc(pool_id, list_context->nspace)` to the PG holding the cursor hash
  (`src/osdc/Objecter.cc:3849-3862`), so the request's hobject carries
  that namespace (rebuilt from the locator, `MOSDOp.h:606-608`); no key
  is sent. The OSD's PGNLS loop skips every candidate whose namespace
  differs from the request's unless the request's namespace is
  `librados::all_nspaces` (`src/osd/PrimaryLogPG.cc:1366-1369`), which is
  the one-byte string `"\001"` (`LIBRADOS_ALL_NSPACES`,
  `src/include/rados/rados_types.h:39`; `rados_types.hpp:331`), set with
  `rados_ioctx_set_namespace` before listing (`librados.h:211-214`). Each
  reply entry is a `ListObjectImpl` of namespace, name and locator key.
- The `ceph_stable_mod` fix moves where rados-rs sends ops on
  default-namespace objects in pools whose `pg_num` is not a power of
  two, for every name whose `hash % pg_num` differs from the stable-mod
  PG (`bar`, `obj`, `_shadow_obj.1`, `e`, `c` in the table below). Such
  data is out of reach across implementations: an object C++ clients
  wrote there is not in the PG rados-rs asked before this fix, and
  rados-rs never had such an object in the PG C++ clients ask. Checked
  against the OSD, the old PG never accepted the op, so nothing is stored
  misplaced: `do_op` drops any op whose hash its PG does not contain
  (`PrimaryLogPG.cc:1972-1982`), and `pg_t::contains` with
  `get_split_bits(pg_num)` (`src/osd/osd_types.h:467-471`,
  `osd_types.cc:830-843`) accepts exactly the hashes `ceph_stable_mod`
  maps to that PG. So before the fix a rados-rs write of such a name hung
  instead of landing, and a rados-rs read of a C++-written object hung
  instead of finding it; nothing needs migrating. Task 5's `pg_num 12`
  test pins the fix against the C++ `rados` CLI in both directions.
  The no-misplacement claim holds for rjenkins pools. On a linux-hash
  pool (none a v19 monitor creates) rados-rs stored objects under an
  rjenkins hash, the OSD accepted them, and after this fix neither
  client finds them; the plan accepts that.
- Oracles, checked for this plan: `osdmaptool --test-map-object <name>
  --pool <id>` takes no namespace or key (it builds
  `object_locator_t loc(pool)`, `src/tools/osdmaptool.cc:719-737`) and
  prints only the reduced PG; `ceph osd map <pool> <object> [<nspace>]`
  takes a namespace but no key (`src/mon/MonCommands.h:610-614`) and
  reports both `raw_pgid` and `pgid` (`src/mon/OSDMonitor.cc:5885-5932`).
  A key vector is therefore `ceph osd map <pool> <key> [<nspace>]`, which
  is the same computation by `map_to_pg`'s key-else-name rule
  (`OSDMap.cc:2618-2621`). Non-power-of-two reductions come from
  `osdmaptool --createsimple 3 --pg-bits 2 --pgp-bits 2
  --with-default-pool` (pool 1, `pg_num 12`) in `quay.io/ceph/ceph:v19.2.2`.
- rados-rs today (verified on the fork's `main` at c482382; cite by
  symbol; lines as of c482382; paths under `rados/src/`):
  `OSDClient::object_pg_in_map(osdmap, pool, oid)`
  (`osdclient/client.rs:1303`) builds `ObjectLocator::new(pool)` and
  calls `crush::placement::object_to_pg(oid, ..)`, which does honour a
  locator's key and namespace but receives neither, hard-codes rjenkins,
  and reduces with `hash % pg_num` (`crush/placement.rs:230-266`, 263);
  `object_to_osds_in_map(osdmap, pool, oid)` (1317) wraps it; its
  callers are `primary_osd` (580), `route_and_submit`'s routing and
  final re-derivation (1738, 1784), `cached_rescan_osds` (1348), called
  from `collect_resend_ops` (2694), and the drain re-route after an epoch
  change in `resend_single_migrated_op` (2819); `linger_interval` calls
  `object_pg_in_map` directly (595), and through it
  `scan_lingers_on_map_change` (1033). The hobject hash is
  separate: `ObjectId::calculate_hash` (`osdclient/types.rs:245-267`) does
  hash key-else-name with the `ns\x1f` prefix, rjenkins only, and is
  called by `execute_op` (1490), after a redirect (1608) and by
  `submit_once_in_map` (1868). So the wire hash is right and the pgid is
  wrong, which is exactly the OSD's silent-drop case. The wire already
  carries the right locator: `MOSDOp` encodes the spg, then
  `object.hash` (`osdclient/messages.rs:347-351`), then
  `ObjectLocator::from(&ObjectId)` with pool, key, namespace and hash -1
  (`messages.rs:389`, `osdclient/types.rs:475-484`). `ObjectId`
  (`types.rs:197-210`) has `pool`, `oid`, `snap`, `hash`, `namespace`,
  `key` and no hash override; `IoCtx::object_id` (`osdclient/ioctx.rs:171-180`)
  fills `namespace` and `key` from `set_namespace` (117) and
  `set_locator_key` (126), and every object method, `watch`, `notify`,
  `list_watchers` and `close_primary_session_for_test` included, goes
  through it. The resend scan loses the locator before it places:
  `OSDSession::get_pending_ops_metadata` returns `(tid, pool, oid,
  epoch)` (`osdclient/session.rs:1063-1076`) and `cached_rescan_osds`
  caches by `(pool, oid)`, so two ops on one name in two namespaces share
  an entry; `kick_into_session` (2937) and
  `fail_all_pending_ops_blocklisted` (3293) also destructure that tuple.
  PGNLS routes by `OSDClient::hash_to_pg` (`client.rs:2117`, called at
  2166 and 2361), a private copy of `ceph_stable_mod` that is
  correct. `OSDMap::object_to_pg`/`object_to_osds` (`osdclient/osdmap.rs:3416-3443`)
  are public, documented "not for production", call the same crush
  function, and are used by `rados/tests/osdclient_object_placement_test.rs`
  and `rados/benches/placement.rs`; `crush::placement::ceph_stable_mod` and
  `crush::placement::pg_num_mask` exist (`crush/placement.rs:281-302`;
  `crush/mod.rs` does not re-export them);
  `PgPool.object_hash: u8` is decoded (`osdclient/osdmap.rs:588`);
  `PgPool` is `Default` and `OSDMap::new()` exists, so unit tests build a
  map with one pool and no CRUSH map. `crush::placement::object_to_pg`,
  `object_to_osds` and `calculate_hash` have no users outside `rados/src`
  (checked across the workspace). Listing: `IoCtx::list_objects(cursor,
  max)` (`osdclient/ioctx.rs:444-463`) calls the public
  `OSDClient::list(pool, cursor, max)` (`client.rs:2297`), whose loop
  calls `query_pg_objects` (2157), which builds the PGNLS target as
  `ObjectId::new(pool, "")` (2192): no namespace, so the OSD lists only
  the default one. `list` returns a `ListResult` of `ListObjectEntry`
  (`types.rs:1573-1582`, the `ListObjectImpl` of `pg_nls_response.rs:17-24`
  with `nspace`, `oid`, `locator`), and `list_objects` keeps only the
  names (460); `IoCtx::ls` (486) and `list_objects_stream`
  (`list_stream.rs:40`) go through `list_objects`. rados-rs has no
  all-namespaces constant.
- Known-answer vectors, from the local v19.2.2 cluster's `ceph osd map
  test-pool <object> [<nspace>] -f json` (pool 2, rjenkins, `pg_num 32`)
  and, in the last column, `osdmaptool` on the `pg_num 12` map:

  | nspace | object | raw ps | pg (32) | pg (12) |
  |---|---|---|---|---|
  | | `foo` | `7fc1f406` | `6` | `6` |
  | `ns1` | `foo` | `f4569544` | `4` | |
  | | `bar` | `efe6384b` | `b` | `b` |
  | `ns1` | `bar` | `73a75142` | `2` | |
  | `users.uid` | `testuser` | `a13aa4c1` | `1` | |
  | `gc` | `gc.0` | `031bb659` | `19` | |
  | | `gc.0` | `990e66d8` | `18` | `8` |
  | `control` | `notify.0` | `4eeacd0a` | `a` | |
  | `rgw` | `foo` | `0ecb73a9` | `9` | |
  | | `obj` | `aabc5e21` | `1` | `1` |
  | `ns1` | `obj` | `37eda02a` | `a` | |
  | | `_shadow_obj.1` | `e5f42c1a` | `1a` | `a` |
  | | `e` | `ef61efce` | `e` | `6` |
  | | `c` | `4fa4bedb` | `1b` | `b` |
  | | `a` | `29eec818` | `18` | `8` |

  `bar`, `obj`, `_shadow_obj.1`, `e` and `c` are the rows where
  `hash % 12` (`3`, `5`, `6`, `a`, `7`) differs from `ceph_stable_mod`. Key rows follow from the table:
  name `_multipart_obj.2~abc.1` with key `obj` is raw `aabc5e21`; any name
  with key `obj` in `ns1` is raw `37eda02a`. The linux hash has no
  cluster oracle (no v19 pool uses it); its vectors, `foo` ->
  `0024db2a` and `ns1\x1ffoo` -> `ce6afc4d`, are a transcription of
  `ceph_hash.cc:83-92`, and the test says so.

## Review Focus

1. The hash input is exactly C++'s: the key when non-empty, else the name;
   `ns + 0x1f` prefixed only when the namespace is non-empty; the pool's
   `object_hash` picks the function and an unknown type is an error
   (deliberately unlike C++, whose `ceph_str_hash` returns -1, hash
   0xffffffff, and places the object anyway); a
   hash override (`hash >= 0`) skips hashing but still requires the pool;
   the result is the RAW PG, reduced only by `raw_pg_to_pg` with
   `ceph_stable_mod` and `pg_num_mask`, never `%`. Pinned by Task 1's and
   Task 2's vector tests.
2. The MOSDOp's hobject hash and its pgid come from one call at every
   routing decision: `route_and_submit` (both derivations), the redirect
   retry, the resend scan and the drain re-route. PG ops (PGNLS) are the
   exception: `replace_pending` re-places them from the hash they carry,
   the listing cursor, and never rewrites it, as C++'s `precalc_pgid`
   path (`Objecter.cc:2836-2840`). A pgid that disagrees
   with the hash is the silent drop of `PrimaryLogPG.cc:1972-1982`; the
   cluster tests would hang without the per-op bound, so they bound
   every op.
3. No path still places by `(pool, name)`: `object_pg_in_map` and
   `object_to_osds_in_map` take `&ObjectId`; the resend metadata carries
   the op (its `ObjectId` and flags); `replace_pending` is the one
   re-placement rule, and the scan's cache keys on the hash input (pool,
   namespace, key-or-name) for object ops while PG ops skip the cache and
   are placed by their cursor hash; `linger_interval`, `primary_osd` and
   PGNLS use the same `PgPool` functions. A grep for `ObjectLocator::new(` and
   `pg_num)` in `osdclient/` is part of the review, as is a grep showing
   no non-test caller of `calculate_hash`, `crush::object_to_pg`,
   `crush::object_to_osds` or the deprecated `OSDMap` pair in `rados/`.
4. No upstream API break: nothing public is removed or changes
   signature. Added: `OSDMap::object_locator_to_pg`,
   `OSDMap::raw_pg_to_pg`, `PgPool::{hash_key, pg_num_mask,
   raw_pg_to_pg}`, `crush::hash::{ceph_str_hash, ceph_str_hash_linux,
   CEPH_STR_HASH_LINUX, CEPH_STR_HASH_RJENKINS}`,
   `OSDClient::list_in_namespace`, `IoCtx::list_object_entries`,
   `ALL_NSPACES`, and the re-exports of `ListResult` and
   `ListObjectEntry` from `osdclient` and the crate root. Fixed in place, still public, not deprecated:
   `crush::placement::object_to_pg`/`object_to_osds` (and their `crush`
   re-export) and `ObjectId::calculate_hash`, which take the key and
   namespace and now reduce with `ceph_stable_mod`; their docs say they
   assume rjenkins, the hash of every pool a v19 monitor creates, and
   point at `OSDMap::object_locator_to_pg` for the pool's own hash type.
   Fixed and deprecated: `OSDMap::object_to_pg`/`object_to_osds`, which
   take a name only; they now use the pool's hash type and
   `ceph_stable_mod`, their docs say they place default-namespace,
   unkeyed objects only, and `#[deprecated(note = "...")]` names
   `object_locator_to_pg` plus `raw_pg_to_pg`. Behaviour changes callers
   can see: placements in non-power-of-two pools now match Ceph, and
   `IoCtx::list_objects` on a context with a namespace lists that
   namespace instead of the default one, as librados does.
5. Listing: the PGNLS target's namespace is the IoCtx's (or
   `ALL_NSPACES`) and its key is empty, as `Objecter.cc:3854`; routing
   still comes from the cursor hash alone; each returned entry keeps its
   namespace and locator key. Pinned by Task 4's request-encoding tests
   and Task 5's two-namespace listing test.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `object-locator-routing` off the fork's `main`
at c482382; the cluster up.

---

### Task 1: `crush::hash`: the pool hash dispatcher

**Files:**
- Modify: `rados/src/crush/hash.rs`, `rados/src/crush/placement.rs`,
  `rados/src/osdclient/types.rs`.

**Interfaces:**
- Produces: `pub const CEPH_STR_HASH_LINUX: u8 = 1`,
  `pub const CEPH_STR_HASH_RJENKINS: u8 = 2`;
  `pub fn ceph_str_hash_linux(data: &[u8]) -> u32` (`u32` wrapping
  arithmetic, which equals C's `unsigned`);
  `pub fn ceph_str_hash(kind: u8, data: &[u8]) -> Option<u32>` (`None`
  for an unknown kind, where C++ returns `-1`);
  `pub(crate) fn hash_key(kind: u8, key: &str, ns: &str) -> Option<u32>`,
  the one copy of `pg_pool_t::hash_key`'s input rule (`key` alone for an
  empty namespace, else `ns + 0x1f + key`), which `PgPool::hash_key`
  (Task 2) and the rjenkins-only public helpers share.
- In this commit `crush::placement::object_to_pg` and
  `ObjectId::calculate_hash` switch their hash input to
  `crush::hash::hash_key(CEPH_STR_HASH_RJENKINS, ..)` (`Some` for
  rjenkins), replacing their own copies of the rule. The values are the
  same, so their existing tests pass unchanged, and `hash_key` has a
  caller from its first commit; `object_to_pg` keeps `% pg_num` until
  Task 3.

- [ ] **Step 1: Tests first**

Replace `test_ceph_str_hash_rjenkins`'s "non-zero, deterministic"
assertions with vectors from the table: rjenkins of `foo`, `bar`, `obj`,
`e`, and of the bytes `ns1\x1ffoo`, `users.uid\x1ftestuser`,
`gc\x1fgc.0` equal their raw ps. Linux: `foo` -> `0x0024db2a`,
`ns1\x1ffoo` -> `0xce6afc4d`, empty -> 0, with a comment that these are
transcribed from `ceph_hash.cc:83-92` because no v19 pool can use the
linux hash. `ceph_str_hash(1, ..)`/`(2, ..)` dispatch; `(0, ..)` and
`(3, ..)` are `None`. `hash_key(2, "foo", "ns1")` equals the rjenkins of
`ns1\x1ffoo` (`0xf4569544`) and `hash_key(2, "foo", "")` that of `foo`.

- [ ] **Step 2: Implement, run `cargo test -p rados --lib`, commit**

```
crush: dispatch string hashing on the pool's hash type

A pool's object_hash selects the string hash Ceph places objects
with: rjenkins, or the linux dcache hash on pools from old clusters.
ceph_str_hash mirrors that dispatch, and the rjenkins tests now pin
values the v19.2.2 monitor reports for namespaced and plain names.
hash_key is the one copy of Ceph's hash input rule, and
crush::object_to_pg and ObjectId::calculate_hash now build their
input through it.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 2: `OSDMap::object_locator_to_pg` and `raw_pg_to_pg`

**Files:**
- Modify: `rados/src/osdclient/osdmap.rs`,
  `rados/tests/osdclient_object_placement_test.rs`,
  `rados/benches/placement.rs`.

**Interfaces:**
- `impl PgPool`:
  - `pub fn hash_key(&self, key: &str, ns: &str) -> Result<u32, RadosError>`:
    `osd_types.cc:1785-1796` via `crush::hash::hash_key(self.object_hash, ..)`;
    an unknown type is `RadosError::Protocol("pool object_hash N is not
    a known string hash")`.
  - `pub fn pg_num_mask(&self) -> u32` via `crush::placement::pg_num_mask(self.pg_num)`.
  - `pub fn raw_pg_to_pg(&self, pg: PgId) -> PgId`: seed through
    `crush::placement::ceph_stable_mod(seed, pg_num, pg_num_mask)`, pool unchanged.
- `impl OSDMap`:
  - `pub fn object_locator_to_pg(&self, oid: &str, loc: &ObjectLocator)
    -> Result<PgId, RadosError>`: the RAW PG, `OSDMap.cc:2606-2637` line
    for line (pool check first; `loc.hash >= 0` returns `PgId::new(pool,
    loc.hash as u32)`; else `hash_key(key-if-non-empty-else-oid,
    &loc.namespace)`). Doc cites both functions and says the result must
    go through `raw_pg_to_pg` before CRUSH, and that its seed is the
    hobject hash the MOSDOp carries.
  - `pub fn raw_pg_to_pg(&self, pg: PgId) -> Result<PgId, RadosError>`
    (`OSDMap.h:1432-1436`).
  - `object_to_pg(pool_id, name)` keeps its signature and becomes
    `raw_pg_to_pg(object_locator_to_pg(name, &ObjectLocator::new(pool_id)))`,
    so it gains the pool's hash type and `ceph_stable_mod`;
    `object_to_osds` keeps its signature and calls
    `self.pg_to_osds(&self.raw_pg_to_pg(self.object_locator_to_pg(name,
    &ObjectLocator::new(pool_id))?)?)` directly, not the deprecated
    `object_to_pg`. Both docs say
    they place default-namespace, unkeyed objects only, and both gain
    `#[deprecated(note = "places default-namespace, unkeyed objects
    only; use OSDMap::object_locator_to_pg and raw_pg_to_pg")]`.
- Internal callers of the deprecated pair move to the locator path:
  `rados/tests/osdclient_object_placement_test.rs` (every `object_to_pg`
  and `object_to_osds` call: 70, 102, 149 and 198-200) and
  `rados/benches/placement.rs` (113, 119) call
  `raw_pg_to_pg(object_locator_to_pg(name, &ObjectLocator::new(pool)))`
  (and then `pg_to_osds` where they wanted OSDs), so no `#[allow(deprecated)]` is needed outside
  the tests of the deprecated functions themselves.
- `rados/benches/placement.rs`'s `create_test_osdmap` sets
  `object_hash: CEPH_STR_HASH_RJENKINS` in its `PgPool` (the `Default` 0
  is not a known hash, so the locator path would return an error).

- [ ] **Step 1: Tests first** (in the existing `osdmap.rs` test module)

Helper `map_with_pool(id: u64, pg_num: u32, object_hash: u8) -> OSDMap`
(`OSDMap::new()` plus a `PgPool { object_hash, pg_num, pgp_num: pg_num,
..Default::default() }`). Tests:

1. `object_locator_to_pg_matches_ceph`: every table row: pool 2,
   `pg_num 32`, rjenkins; `ObjectLocator { pool_id: 2, namespace, .. }`;
   raw seed equals the raw ps and `raw_pg_to_pg` equals the `pg (32)`
   column.
2. `raw_pg_to_pg_is_stable_mod`: `pg_num 12`: the `pg (12)` column,
   including `e` -> 6, `c` -> 0xb, `_shadow_obj.1` -> 0xa.
3. `locator_key_replaces_the_name`: `(ns "", name
   "_multipart_obj.2~abc.1", key "obj")` -> raw `0xaabc5e21`; `(ns "ns1",
   name "x", key "obj")` -> raw `0x37eda02a`; a key equal to the name
   gives the same raw PG as no key.
4. `hash_override_skips_hashing`: `ObjectLocator::with_hash(2,
   0x7fc1f406)` with name `anything` -> raw `0x7fc1f406`, pg `6`.
5. `missing_pool_is_an_error` for both the hashed and the override path.
6. `linux_hash_pool`: `object_hash 1`, `pg_num 32`: `foo` -> raw
   `0x0024db2a`; `object_hash 9` is an error.
7. `object_to_pg_uses_stable_mod` (`#[allow(deprecated)]`, as it tests
   the deprecated function): `OSDMap::object_to_pg(1, "e")` on the
   `pg_num 12` map is seed 6; on a linux-hash pool, `foo` is
   `0x0024db2a` reduced.

- [ ] **Step 2: Implement, run the lib tests,
  `cargo test -p rados --test osdclient_object_placement_test` and
  `cargo bench -p rados --bench placement -- --test` (each bench runs
  once; no deprecation warning), commit**

```
osdclient: place objects by their full locator as Ceph does

OSDMap::object_locator_to_pg mirrors Ceph's: the locator's key stands
in for the name when set, a namespace is hashed as ns + 0x1f + key,
the pool's hash type picks the hash, and a hash override skips it. It
returns the raw PG; raw_pg_to_pg reduces it with ceph_stable_mod.
object_to_pg now goes through both, which also fixes its hash %
pg_num reduction for pools whose pg_num is not a power of two. It and
object_to_osds take a name only, so they stay for compatibility,
deprecated towards object_locator_to_pg; the placement test and bench
move to the locator path.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 3: Route every placement through the locator

**Files:**
- Modify: `rados/src/osdclient/client.rs`, `rados/src/osdclient/session.rs`,
  `rados/src/osdclient/types.rs`, `rados/src/crush/placement.rs`.

**Interfaces:**
- `OSDClient::object_pg_in_map(osdmap, object: &ObjectId) ->
  Result<Placement>` where `struct Placement { hash: u32, pg: PgId }`
  (private): `raw = osdmap.object_locator_to_pg(&object.oid,
  &ObjectLocator::from(object))`, `hash = raw.seed`, `pg =
  pool.raw_pg_to_pg(raw)`; a missing pool stays `PoolNotFound`, a hash
  error is `OSDClientError::Crush`. The doc names it the only way the
  client places an object and cites `Objecter.cc:2842-2863`.
- `object_to_osds_in_map(&self, osdmap, object: &ObjectId) ->
  Result<(u32, StripedPgId, Vec<i32>)>` (the hash first). Callers:
  `primary_osd`, `linger_interval` (through `object_pg_in_map`),
  `route_and_submit` (both derivations) and `replace_pending`.
- `fn replace_pending(osdmap, msg: &MOSDOp) -> Result<(u32, StripedPgId,
  Vec<i32>)>` (private): the one re-placement rule for an op already in
  flight. An op whose `flags` carry `OsdOpFlags::PGOP` (PGNLS) is placed
  by `pool.raw_pg_to_pg(PgId::new(pool, msg.object.hash))`, mapped to
  its OSDs as `object_to_osds_in_map` maps a PG, and returns
  `msg.object.hash` unchanged: that hash is the listing cursor and is
  never rewritten, as C++'s `precalc_pgid` path uses `base_pgid` and
  skips `object_locator_to_pg` (`Objecter.cc:2836-2840`). Any other op
  goes through `object_to_osds_in_map(osdmap, &msg.object)`. Callers:
  `cached_rescan_osds` and the drain re-route in
  `resend_single_migrated_op`.
- `route_and_submit` stamps `m.object.hash` beside `m.pgid` from the same
  result in the block that allocates the tid, so the redirect retry
  (which changes pool, key and namespace, `apply_redirect`) is re-hashed
  by the next iteration. `collect_resend_ops` and the drain re-route
  stamp `msg.object.hash` beside `msg.pgid` from `replace_pending`'s
  result, so both come from one call (for a PG op, the hash it already
  carried).
- Drop the three internal calls of `ObjectId::calculate_hash` (in
  `execute_op`, the redirect retry and `submit_once_in_map`); `ObjectId.hash`'s doc says it is the hobject hash,
  stamped at routing. `calculate_hash` itself stays public and
  undeprecated (it takes the key and namespace; its hash input is
  `hash_key` since Task 1): its doc says it assumes rjenkins, which is every pool a v19
  monitor creates, and points at `OSDMap::object_locator_to_pg` for the
  pool's own hash type.
- `OSDSession::get_pending_ops_metadata` returns `(tid, Arc<MOSDOp>,
  epoch)` (the op's `Arc`, cheap to clone; it carries the `ObjectId` and
  the flags `replace_pending` needs); `collect_resend_ops` passes the op
  and `cached_rescan_osds` calls `replace_pending`, skipping the cache for
  a PG op and otherwise keying it by `placement_key(&ObjectId) -> (u64,
  String, String)` = pool, namespace, key-or-name (the hash input; the
  name is irrelevant when a key is set). `fail_if_pool_deleted` takes
  `msg.object.pool`. The two other callers that destructure the tuple,
  `kick_into_session` and `fail_all_pending_ops_blocklisted`, are
  updated to the new shape.
- PGNLS: `hash_to_pg` is deleted; `query_pg_objects` and the list loop
  use `pool_info.raw_pg_to_pg(PgId::new(pool,
  cursor.hash)).seed`.
- `crush::placement::object_to_pg` and `object_to_osds` keep their
  signatures, their re-export at `crush/mod.rs:56` and their tests.
  `object_to_pg` (hashing through `hash_key` since Task 1) reduces with
  `ceph_stable_mod(hash, pg_num, pg_num_mask(pg_num))`
  instead of `hash % pg_num` (`placement.rs:263`); `object_to_osds`
  follows. Their docs say they assume rjenkins and return the reduced
  PG, and point at `OSDMap::object_locator_to_pg` for the pool's hash
  type and the raw hash. They are not deprecated: unlike the `OSDMap`
  pair they take the locator. `test_object_to_pg`,
  `test_object_to_pg_with_namespace`, `test_object_to_osds` and
  `test_pg_distribution` stay; any expectation that assumed `%` is
  recomputed with `ceph_stable_mod`, and a new
  `test_object_to_pg_stable_mod` pins `e` -> 6 and `c` -> 0xb at
  `pg_num 12`.

- [ ] **Step 1: Tests first** (in `client.rs`'s test module, on Task 2's
  `map_with_pool` shape; no CRUSH map needed at this level)

1. `object_pg_in_map_uses_namespace_and_key`: `ObjectId::with_namespace(2,
   "foo", "ns1")` -> hash `0xf4569544`, pg `2.4`; `ObjectId::new(2,
   "foo")` -> `0x7fc1f406`, `2.6`; `ObjectId { key: "obj", .. }` named
   `_multipart_obj.2~abc.1` -> `0xaabc5e21`, `2.1`.
2. `placement_key_separates_namespaces`: `foo` in `""` and in `ns1`
   differ; two names with the same key and namespace are equal.
3. `pgls_reduction_is_stable_mod`: on `pg_num 12`, cursor hash
   `0xef61efce` routes to PG 6.
4. `replace_pending_keeps_a_pgnls_cursor`: a PGNLS `MOSDOp` (flags carry
   `PGOP`, empty name) with cursor hash `0xef61efce` on a `pg_num 12`
   pool is placed on PG 6 and keeps hash `0xef61efce`.
5. `replace_pending_uses_the_locator`: non-PG ops on the namespaced
   (`foo` in `ns1` -> `0xf4569544`, `2.4`) and keyed
   (`_multipart_obj.2~abc.1` with key `obj` -> `0xaabc5e21`, `2.1`)
   vectors on `pg_num 32` return those hashes and PGs whatever hash the
   op carried.

Tests 4 and 5 need CRUSH to run, so their map also gets a small CRUSH
map built as `benches/placement.rs`'s `create_test_crush_map` builds
one; they assert the hash and the PG, not the OSDs.

- [ ] **Step 2: Implement; `cargo test -p rados --lib`; `rg
  'ObjectLocator::new\(|calculate_hash|% pg_num|hash_to_pg' rados/src`
  finds only the `ObjectLocator` and `calculate_hash` definitions,
  their own tests, and the deprecated `OSDMap::object_to_pg`'s
  `ObjectLocator::new(pool_id)`; `rg 'crush::object_to_(pg|osds)|placement::object_to_(pg|osds)'
  rados/src/osdclient` finds nothing; commit**

```
osdclient: route every op by its object's full locator

The client placed each op by the object's name alone while the
MOSDOp carried a hash that included the namespace and locator key, so
most ops on a namespaced or keyed object reached a PG that does not
contain the hash, and the OSD dropped them without a reply. Every
routing decision, the resend scan, the drain re-route and the linger
interval now take the hash and the PG from one object_locator_to_pg
call, and the resend scan no longer conflates one name in two
namespaces. crush::object_to_pg and object_to_osds now reduce with
ceph_stable_mod rather than %; they and ObjectId::calculate_hash stay
public, and no internal path calls them.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 4: List by namespace

**Files:**
- Modify: `rados/src/osdclient/client.rs`, `rados/src/osdclient/ioctx.rs`,
  `rados/src/osdclient/types.rs`, `rados/src/osdclient/mod.rs`,
  `rados/src/lib.rs`.

**Interfaces:**
- `pub const ALL_NSPACES: &str = "\u{1}"` in `osdclient/types.rs`.
  `ALL_NSPACES`, `ListResult` and `ListObjectEntry` are re-exported from
  `osdclient` (`mod.rs`'s `pub use types::{..}`) and the crate root
  (`lib.rs`'s `pub use osdclient::{..}`); today neither exports the two
  list types. `ALL_NSPACES`'s
  doc cites `rados_types.h:39` and `PrimaryLogPG.cc:1366-1369`, and says
  it is meaningful for listing only (as in librados, an object op on a
  context set to it hashes the literal byte).
- `OSDClient::list_in_namespace(&self, pool: u64, nspace: &str, cursor:
  Option<String>, max_entries: u64) -> Result<ListResult>`: today's
  `list` body with `nspace` threaded through. `list(pool, cursor, max)`
  keeps its signature and default-namespace behaviour by delegating with
  `""`.
- `query_pg_objects` gains `nspace: &str`; its target comes from a
  private `fn pgnls_target(pool: u64, cursor_hash: u32, nspace: &str) ->
  ObjectId` (`oid` and `key` empty, `namespace = nspace`, `hash =
  cursor_hash`), mirroring `object_locator_t oloc(pool_id,
  list_context->nspace)` (`Objecter.cc:3854`). The namespace reaches the
  OSD through the MOSDOp's locator (`messages.rs:389`); routing is
  unchanged (cursor hash through `raw_pg_to_pg`, Task 3). The cursor
  string stays the handle's hash.
- `IoCtx::list_objects(cursor, max)` keeps its signature and calls
  `list_in_namespace(self.pool_id, &self.namespace, ..)`; its doc says
  it lists the context's namespace, or every namespace under
  `ALL_NSPACES`, and returns names only. `ls` and `list_objects_stream`
  follow through it unchanged.
- `IoCtx::list_object_entries(&self, cursor: Option<String>,
  max_entries: usize) -> Result<ListResult>`: the same call, returning
  each `ListObjectEntry` (namespace, name, locator key), which the
  all-namespaces form needs to tell `foo` in two namespaces apart.

- [ ] **Step 1: Tests first** (in `client.rs`'s test module)

1. `pgnls_target_carries_the_namespace`: `pgnls_target(2, 0xef61efce,
   "ns1")` has namespace `ns1`, empty oid and key, hash `0xef61efce`;
   `ObjectLocator::from(&it)` is `{ pool_id: 2, key: "", namespace:
   "ns1", hash: -1 }`.
2. `pgnls_request_encodes_the_namespace`: a PGNLS `MOSDOp` built from
   that target (as `test_mosdop_encoding_v9`, `messages.rs:636-676`,
   builds one) encodes a front that contains the encoded locator with
   namespace `ns1`, and is `"ns1".len()` bytes longer than the same
   request with namespace `""` (rados-rs has no MOSDOp decoder,
   `messages.rs:439-450`, so the test searches for the locator's bytes).
3. `pgnls_request_all_namespaces`: the same with `ALL_NSPACES`: the
   locator's namespace is the single byte `0x01`.
4. `list_delegates_to_the_default_namespace`: `list` and
   `list_in_namespace(.., "", ..)` build the same target for one cursor
   (via `pgnls_target`; no cluster needed).

- [ ] **Step 2: Implement; `cargo test -p rados --lib`; commit**

```
osdclient: list objects in the IoCtx's namespace

PGNLS requests went out with no namespace, and the OSD returns only
objects in the request's namespace, so listing a namespaced context
showed the default namespace instead. The request now carries the
context's namespace, or ALL_NSPACES to list every namespace as
librados does, and list_object_entries returns each entry's namespace
and locator key. OSDClient::list keeps its signature and lists the
default namespace; list_in_namespace takes one.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 5: Cluster tests

**Files:**
- Create: `rados/tests/object_locator_routing.rs` (using `tests/common/mod.rs`).
- Modify: `.github/workflows/test-with-ceph.yml` (add
  `object_locator_routing` to both `for test in` lists of the `rados`
  steps, `test-with-ceph.yml:93,113`; those steps run on the host with
  the compose containers up, `docker/docker-compose.ceph.yml:3-6`, so
  the default `CEPH_EXEC` below reaches the v19.2.2 `rados` binary).

Helpers: `unique(prefix)` as `watch_notify.rs`; `within(fut)` wrapping each
op in `tokio::time::timeout(10 s)` and panicking with the object's
namespace, key and name, so a misplacement fails in seconds instead of
hanging on the Tracker's 30 s; `names(n)` = `obj-0000..`; `mon_osd_map(client,
pool, object, nspace) -> (u32 raw, u32 pg)` sending
`{"prefix":"osd map","pool":..,"object":..,"nspace":..,"format":"json"}`
through `client.mon_client().invoke` and parsing `raw_pgid`/`pgid`
(`"<pool>.<hex>"`); `ceph_cli(args, stdin) -> Vec<u8>` running the
v19.2.2 container's C++ CLI as `$CEPH_EXEC <args>` (default `docker exec
-i ceph-mon`; locally `CEPH_EXEC="podman exec -i ceph-mon"`), panicking
with its stderr on a non-zero exit. Each test removes what it created.

Tests (all `#[ignore]`):

1. `placement_matches_the_monitor`: for namespaces `""`, `ns1` and a
   unique one, 64 names each, and for five keys in `""` and `ns1`:
   `osdmap.object_locator_to_pg` plus `raw_pg_to_pg` (from
   `client.osd_client().get_osdmap()`) equal `mon_osd_map` (for a key the
   monitor is asked about the key, by `OSDMap.cc:2618-2621`).
2. `namespaced_objects_round_trip`: `ioctx.set_namespace(ns)`; 64 names:
   `write_full` (payload the name), `read` returns it, `stat` size
   matches; a second `IoCtx` in the default namespace `stat`s each name as
   `ENOENT`; a third in another namespace writes a different payload to
   the same names and each namespace reads back its own; `remove` all.
3. `keyed_objects_round_trip`: `set_locator_key(k)`, then again with a
   namespace: 64 names written, read, `setxattr`/`getxattr` once each,
   removed; every name's placement equals `mon_osd_map(k, ns)`.
4. `watch_and_notify_in_a_namespace`: client A in namespace `ns` watches
   16 names; client B in `ns` notifies each: one ack from A, no missed;
   `list_watchers` in `ns` shows A's cookie; one extra watch on a keyed
   object is notified the same way; unwatch all.
5. `namespaced_watch_survives_a_session_reset`: A watches one name in `ns`;
   `close_primary_session_for_test(name)` (which places through
   `primary_osd`); within 10 s B's notify is acked (plan 12's reconnect
   re-placed the namespaced linger).
6. `listing_is_per_namespace`: two unique namespaces `A` and `B`; eight
   names in `A` and eight in `B`, four of them the same names in both,
   plus one object in `A` under a locator key. An IoCtx in `A`: `ls`
   returns exactly `A`'s nine names and `list_object_entries` gives each
   namespace `A` and the keyed one its key; the same for `B`; a
   default-namespace `ls` contains none of them. An IoCtx set to
   `ALL_NSPACES`: `list_object_entries` over the whole pool contains
   every `(namespace, name)` pair written, both copies of the shared
   names included, with the keyed entry's locator (a superset check,
   since other tests write to the pool concurrently).
7. `stable_mod_matches_the_cpp_client`: creates a unique pool through
   the monitor with `{"prefix":"osd pool create","pool":..,"pg_num":12,
   "pgp_num":12,"autoscale_mode":"off"}` (`MonCommands.h:1115-1127`),
   waits (bounded, 60 s) until the client's OSDMap has it at `pg_num
   12` and `ceph_cli(["ceph","pg","ls-by-pool",pool,"-f","json"])`
   reports all twelve PGs active (a mgr command, `MgrCommands.h:22`, so
   not through `mon_client().invoke`). The body runs under
   `AssertUnwindSafe(..).catch_unwind()` (`futures::FutureExt`), then
   `delete_pool(name, true)` removes the pool
   (`mon_allow_pool_delete = true`, `docker-compose.ceph.yml:50`), then
   `std::panic::resume_unwind` re-raises any panic, so the pool goes
   whatever happened. Names:
   the table's `bar`, `obj`, `_shadow_obj.1`, `e` and `c`, where
   `hash % 12` and `ceph_stable_mod` disagree, plus 64 `names(64)`.
   Every name's client PG equals `mon_osd_map` (raw and reduced; `e` is
   `.6`). rados-rs to C++: `write_full` each name (payload the name),
   bounded by `within`, then `ceph_cli(["rados","-p",pool,"get",name,
   "-"])` returns the payload. C++ to rados-rs: `ceph_cli(["rados","-p",
   pool,"put","cli-"+name,"-"], payload)` for each, then rados-rs `read`
   returns it, bounded. Before this plan both directions fail on the five
   table names: the write and the read go to a PG that does not contain
   the hash, and `within` turns the OSD's silent drop into a failure.

The resend scan and the drain re-route are not exercised here: they
need a live client with ops in flight across a map change that moves a
PG, and the local cluster has one OSD. Task 3's `replace_pending` tests
cover the placement they use. The commit body says so.

Commit:

```
osdclient: object locator routing cluster tests

Against Ceph v19.2.2: the client's PG for plain, namespaced and keyed
objects equals the monitor's osd map answer for many names; writes,
reads and xattrs on namespaced and keyed objects complete, and the
namespaces stay isolated; watches and notifies in a namespace are
acked and survive a session reset; listing returns one namespace's
objects, or every namespace's with ALL_NSPACES. On a pool with pg_num
12, objects rados-rs writes are read by the C++ rados CLI and the
reverse, for names where hash % pg_num and ceph_stable_mod disagree.
Every op is bounded, so a misplaced op fails instead of hanging. The
resend scan and the drain re-route need ops in flight across a map
change that moves a PG, which a one-OSD cluster cannot produce; unit
tests cover the placement they use.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 6: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: per-commit proof; container fmt/clippy; no corpus
change; the whole `rados` cluster suite, `watch_notify` and
`object_locator_routing` included, plus every `rados-cls` suite (every
class call is an op the new routing places); `cargo doc` and `cargo
clippy --all-targets` show no deprecation warning outside tests of the
deprecated pair; the implementer for Tasks 2, 3 and 4 is the opus tier,
the whole-branch review the session model. The PR body (97 words, under
the 100-word ceiling; the Claude Code line last and its only link):

```
**Motivation.** The OSD client placed objects by name alone, ignoring namespace and locator key, so ops on namespaced or keyed objects reached a PG the OSD silently rejects, and hung. Listing ignored namespaces. RGW keeps metadata in namespaces and keys multipart objects.

**What changed.** Placement follows Ceph's `object_locator_to_pg`, with the pool's hash type and `ceph_stable_mod`, on every routing path; listing honours the IoCtx's namespace or all namespaces. No public API is removed. Unit tests pin Ceph v19.2.2 vectors; cluster tests check the monitor, the `rados` CLI and namespaced I/O, watches and listing.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cephx-aes256krb5` next, then `release-shapes`. Listing by namespace,
deferred in an earlier draft, is Task 4. Out of scope: cache-tier
overlays (`Objecter.cc:2820-2833`) and a hash override on `ObjectId` (`ObjectLocator` carries one and
`object_locator_to_pg` honours it; no RGW object op needs it).

## Controller rulings applied (2026-09-26)

1. No upstream API break: nothing public is removed; `crush::placement::object_to_pg`/`object_to_osds` and `ObjectId::calculate_hash` stay public and undeprecated, fixed in place (`ceph_stable_mod`, one shared hash-input helper) with docs naming their rjenkins assumption, since they already take the key and namespace; the name-only `OSDMap::object_to_pg`/`object_to_osds` gain the pool's hash type and `ceph_stable_mod` and are `#[deprecated]` towards `object_locator_to_pg`; internal callers, the placement test and the bench move to the locator path (Architecture, Review Focus 4, Tasks 1-3).
2. Namespaced listing is in scope as Task 4 (`ALL_NSPACES`, `list_in_namespace`, `list_object_entries`, `list_objects` following the IoCtx's namespace; request-encoding unit tests) with cluster test 6 in Task 5; the Roadmap deferral is gone and Review Focus 5 added.
3. Global Constraints state the stable-mod consequence for non-power-of-two pools, corrected against the OSD: the old PG never accepted such ops (`PrimaryLogPG.cc:1972-1982`, `pg_t::contains`), so they hung rather than storing misplaced data, and nothing needs migrating; Task 5 test 7 pins the fix on a test-created `pg_num 12` pool against the v19.2.2 `rados` CLI in both directions and `ceph osd map`. The table note on which rows `%` misplaces is corrected (`bar` and `obj` also differ).
4. Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and nothing after it; the PR body is 97 words and ends with the Claude Code line, its only link.

## Review edits applied (2026-09-26)

1. PGNLS re-placement: Task 3 adds private `replace_pending`; a `PGOP` op is placed by its cursor hash via `raw_pg_to_pg` and keeps it (`precalc_pgid`, `Objecter.cc:2836-2840`), any other op goes through `object_pg_in_map`; `get_pending_ops_metadata` returns the op's `Arc<MOSDOp>`; `collect_resend_ops`, `cached_rescan_osds` (no cache for PG ops), `resend_single_migrated_op`, `kick_into_session` and `fail_all_pending_ops_blocklisted` follow; the "hash cannot change for a live op" claim is gone; tests `replace_pending_keeps_a_pgnls_cursor` and `replace_pending_uses_the_locator`; Review Focus 2 and 3 name PG ops.
2. Task 2's `object_to_osds` calls `pg_to_osds(raw_pg_to_pg(object_locator_to_pg(..)))` directly; the placement test's calls at 70, 102, 149 and 198-200 move to the locator path.
3. The bench's `create_test_osdmap` sets `object_hash: CEPH_STR_HASH_RJENKINS`; its gate is `cargo bench -p rados --bench placement -- --test`.
4. Task 1 switches `crush::placement::object_to_pg` and `ObjectId::calculate_hash` to `hash_key(CEPH_STR_HASH_RJENKINS, ..)` (same values; `% pg_num` stays until Task 3); Task 3 no longer says so.
5. The stable-mod constraint limits the no-misplacement claim to rjenkins pools and accepts the unreachable linux-hash objects.
6. "Unit coverage only" is replaced: the resend scan and drain re-route need a live client; the `replace_pending` tests cover the placement they use (Task 5 text and commit body).
7. `crush::placement::ceph_stable_mod` and `crush::placement::pg_num_mask` everywhere.
8. Task 3's grep gate allows the deprecated `OSDMap::object_to_pg`'s `ObjectLocator::new(pool_id)`.
9. rados-rs line citations refreshed against c482382 ("cite by symbol; lines as of c482382"); Task 3 cites by symbol; "the old helper's cases move here" dropped.
10. Task 3's commit message states that `crush::object_to_pg` and `object_to_osds` now reduce with `ceph_stable_mod` and, with `calculate_hash`, stay public with no internal caller.
11. Task 4 and Review Focus 4 name the `ListResult` and `ListObjectEntry` re-exports.
12. Task 5 test 7 runs its body under `AssertUnwindSafe(..).catch_unwind()`, then `delete_pool`, then `resume_unwind`.
14. Review Focus 1 notes that an unknown `object_hash` being an error deliberately differs from C++ (-1, hash 0xffffffff, placed anyway).
- Branch base: the fork's `main` at c482382.
