# rados-rs RGW MVP, plan 16 of N: `release-shapes`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the spec's encoding rule executable: the `rados` crate
keeps the OSD map's `require_osd_release` current through incrementals
(today it keeps the bootstrap value forever) and exposes it, with a
config override for testing; `rados-cls` offers the `rgw` class's later
request shapes selected by that release (`update_stats` version 2 with
`dec_stats`, Tentacle v20.2.0; `read_olh_log` version 2 with
`get_stales` and index-entry metadata version 8 with the restore
fields, Umbrella v21.1.0), pinned byte-exactly against `ceph-dencoder`
v19.2.2, v20.2.4 and v21.1.0; and the crate's docs stop saying wrong
things about later releases. This is the campaign's last plan.

**Architecture:** Four commits. (1) `rados`: `apply_to` applies both
release fields as C++ does; a lossless `CephRelease(u8)` newtype with
C++'s ordering and `Display`; `require_osd_release()` on `OSDClient`,
`Client` and `IoCtx`, honouring `OSDClientConfig::assume_osd_release`.
The `OSDMap` field types stay `u8`. (2) `rados-cls`: each later field
is an `Option` on its request struct, and the struct's encoded version
follows it (`Some` encodes the newer version, decoding sets `Some` iff
`struct_v` is at least that version), so v19 and v21 bytes both round
trip; only the two constructors that build their own request take a
`CephRelease`, and caller-built metadata comes from
`DirEntryMeta::for_release`. (3) Doc corrections, each a cited fact.
(4) Cluster tests proving that the Squid class accepts every newer
shape, ignores the tail, and answers `EOPNOTSUPP` for the methods it
lacks.

**Tech Stack:** as plan 3. No new dependency.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`:
the goal's encoding-rule paragraph ("where a later release changed a
request or reply shape ... the `release-shapes` package offers each
shape, selected by the cluster's required OSD release") and the
roadmap's `release-shapes` line. Research:
`.superpowers/research/release-shapes.md` (cited as "report §N";
`path:line@tag` is a Ceph source line at that tag, `main` =
7ed73efc1be).

## Global Constraints

Plans 3 to 15's Global Constraints apply unchanged (Squid floor with
`rados::check_min_version!` in every hand-written `decode_content`;
every derive carries `#[denc(crate = "rados")]`; no `unwrap`/`expect`
on production paths; gates; container fmt/clippy by the controller;
push after every gate; merge when green, then an upstream PR; offline
builds with the scratchpad `CARGO_HOME`). Plus:

- Execution order was 11, 13, 14, 15, 12; branch `release-shapes` is
  based on the fork's `main` after plan 12 (`watch-notify`) merges, so
  plans 13 to 15 are in. That base differs from the tree this plan was
  verified on (`cls-rgw-olh` at 0e624f7) in three ways that matter
  here: `byte_enum!` has moved from `rgw::types` to the crate root
  (plan 15); `call::exec_returnvec` is gated
  `any(feature = "user", feature = "two_pc_queue")` (plan 14); and both
  `test-with-ceph.yml` loop pairs list the suites plans 12 to 15 added.
  Line numbers below are from 0e624f7. Re-find anything by name.
- Releases (`src/common/ceph_releases.h:13-37@main`): `unknown=0`, ...,
  `reef=18`, `squid=19`, `tentacle=20`, `umbrella=21`, `max=22`.
  v19.2.2's enum stops at `squid, max`. Tentacle stable is v20.2.0 to
  v20.2.4; Umbrella is the v21.1.0 RC (no v21.2.x yet); `main` is past
  the Umbrella branch point (report §0). The comparison operators
  (`ceph_releases.h:56-78@main`) treat any byte that is negative as
  `int8_t` ("we used to use -1 for invalid release", so 0xff) as less
  than every release.
- The rule (spec): `rados-cls` encodes each request at the version the
  cluster's own radosgw sends, keyed on `require_osd_release`.
  radosgw itself never reads the release (`grep require_osd_release
  src/rgw` is empty at `main`). It sends the newest request shape with
  compat 1 unconditionally, and probes new methods for `EOPNOTSUPP`
  (report §2.3). Keying on the map is stricter mid-upgrade and the same
  afterwards. The spec settles that trade; do not reopen it.
- How a v19.2.2 OSD takes newer shapes (report §2): an unknown method is
  `-EOPNOTSUPP` (`src/osd/PrimaryLogPG.cc:6148-6152@v19.2.2`). A request
  whose `struct_v` is higher but whose compat is within the decoder's
  range decodes, and `DECODE_FINISH` skips the unread tail
  (`src/include/encoding.h:1495-1507,1602-1611@v19.2.2`). Every shape in
  this plan keeps its compat, so a Squid OSD accepts it and ignores the
  new fields. Sending a later release's shape is still a caller error:
  the rule is not to send it, and the docs say so.
- The shapes, by release (report §2.4; request side only):

  | request | Squid (19) | Tentacle (20) | Umbrella (21) |
  |---|---|---|---|
  | `bucket_update_stats` (`rgw_cls_bucket_update_stats_op`) | v1 | v2, `dec_stats` empty | v2 |
  | `bucket_read_olh_log` (`rgw_cls_read_olh_log_op`) | v1 | v1 | v2, `get_stales = true` |
  | meta inside `bucket_complete_op` / `bucket_link_olh` | v7 | v7 | v8, restore zero |

  Sources: `encode(dec_stats)` `cls_rgw_ops.h:499-503@v20.2.4`, first in
  v20.2.0 (5194bb6bdd3); `encode(get_stales)` `cls_rgw_ops.h:292-298@v21.1.0`,
  first in v21.1.0 (b4b8c63ace1); meta v8 `cls_rgw_types.h:218-219,234-235@v21.1.0`,
  first in v21.1.0 (8d21ac4b8da). The outer `rgw_cls_obj_complete_op`
  (9, 7) and `rgw_cls_link_olh_op` (5, 1) are unchanged; only the
  embedded meta's bytes grow. radosgw of each release sends these
  forms unconditionally: Tentacle's reshard `flush()` is the only
  `update_stats` caller, with an empty `dec_stats`; v21 sends
  `get_stales = true`; and v21 sends meta v8 with zeros when there is
  no restore.
- rados-rs today (verified in the tree):
  - `pub type CephRelease = u8;` is `osdmap.rs:1350`. It is not
    re-exported (`osdclient/mod.rs` re-exports `OSDMap,
    OSDMapIncremental, PgMergeMeta, PgPool, PoolSnapInfo, UuidD` from
    `osdmap`), but `pub mod osdmap` makes it reachable as
    `rados::osdclient::osdmap::CephRelease`. Nothing in the workspace
    names it outside `osdmap.rs`.
  - Full map: `OSDMapOsdSection` decodes `require_min_compat_client`
    and then `require_osd_release` (`:1862-1863`). The decoder moves
    them into the pub fields `OSDMap::require_min_compat_client` and
    `require_osd_release` (`:2529-2530`, `:3464-3465`), and
    `impl Default for OSDMap` (`:2578`), which `OSDMap::new()`
    (`:3115`) calls, zeroes them.
  - Incremental: both decode as `u8` (`:1647-1648`) into the pub fields
    `new_require_min_compat_client` and `new_require_osd_release`
    (`:1427-1428`, copied at `:1999-2000`), defaulting to `0xff`
    (`:1904-1905`).
  - `OSDMapIncremental::apply_to` (`:2092`) lists both under
    "Intentionally not applied" (`:2150-2151`) and writes neither, so a
    client keeps the release it bootstrapped with. The client applies
    incrementals through `apply_incremental_map` -> `apply_to`
    (`client.rs:2215,2236`) and takes full maps whole (`apply_full_map`,
    `:2254`).
  - No accessor exists. `OSDClient::get_osdmap()` (`client.rs:314`) and
    `Client::osd_client()` (`rados/src/client.rs`) reach the raw field.
    `OSDClient` keeps its `config: OSDClientConfig` (`client.rs:29`,
    `Default` at `:64`), and every in-tree construction of the config
    uses `..Default::default()` (`rados/src/client.rs:400`,
    `rados/tests/osdclient_integration_test.rs:178`,
    `examples/rados.rs:168`).
  - `MonCephRelease` (`rados/src/denc/monmap.rs:31-69`) is a
    `num_enum::FromPrimitive` enum `Unknown=0 ... Squid=19, MAX=20` with
    `#[default] Unknown`, so a Tentacle monmap's 20 decodes as `MAX` and
    21 or 0xff as `Unknown`. It is lossy, it is not the OSD value, and
    this plan leaves it alone (Roadmap).
  - `call::exec_returnvec` is feature-gated (see the first bullet) and
    `rgw` is not in the gate. This plan does not add it, because the
    OLH epoch reply is deferred.
  - `rados-cls`: `UpdateStatsOp` is a derived `VersionedDenc` at version
    1 (`index.rs:1330-1336`). `ReadOlhLogOp` is hand-written at version 1
    with `MAX_DECODE_VERSION` 2, skipping `get_stales`.
    `DirEntryMeta` is hand-written at version 7 with
    `MAX_DECODE_VERSION` 8, skipping the restore fields (`index.rs:46-128`).
    `update_stats_op`, `update_stats`, `read_olh_log_op` and
    `read_olh_log` take no release.
- Deferred, each with its reason, and none of them modelled here:
  - The OLH epoch reply on `bucket_link_olh` and
    `bucket_unlink_instance` (a bare `u64` under RETURNVEC,
    `cls_rgw.cc:1929,2176@main`, 8ef9dd9ab27) is on `main` only, in no
    release tag, and feeds only radosgw's FIFO bilog, which is multisite
    and excluded. A Squid OSD returns empty outdata to a RETURNVEC link,
    so nothing breaks by not asking.
  - The resharding-only methods `bucket_init_index2`, `bi_put_entries`
    and `reshard_log_trim` (all v20.2.0), and the cloud-restore
    `bucket_refresh_instance` (`main` only, a4df52c0d1d), are excluded
    with the features they serve (resharding is out; cloud restore is an
    rgw-rs exclusion). So are `bi_list` v2 (`reshardlog`) and
    `reshard_add` v2 (`create_only`): the spec's `bi_*` and reshard-queue
    non-goals. Task 4 pins only that Squid answers `EOPNOTSUPP` to the
    four methods, sent as raw calls with no constructors added.
  - Umbrella's OLH server behaviour: `olh_epoch = 0` means "now in
    nanoseconds" (`cls_rgw.cc:1865,1927@v21.1.0`, 75c7b8ece79), and link
    and unlink log `CLS_RGW_OLH_OP_STALE` (4) entries
    (`cls_rgw.cc:1875,2009,2176,2198@v21.1.0`). The Squid cluster cannot
    exercise either, so Task 3 documents them on `olh.rs`. `OlhLogOp`
    already keeps the unknown byte.
- Oracles (report §3). The captures are under
  `/tmp/claude/release-shapes/{v19.2.2,v20.2.4,v21.1.0}/`, one `T.N`
  (binary), `T.N.hex` and `T.N.json` per dencoder test instance of the
  25 types in `types.txt` plus `cls_2pc_urgent_data`. They were taken
  with `capture.sh OUTDIR TYPE...` in the same directory, run inside
  each image as `podman run --rm --user "$(id -u):$(id -g)" -v /tmp:/tmp
  --entrypoint sh quay.io/ceph/ceph:<tag> /tmp/claude/release-shapes/capture.sh
  /tmp/claude/release-shapes/<tag> $(cat /tmp/claude/release-shapes/types.txt)`,
  from `quay.io/ceph/ceph:v19.2.2`, `quay.io/ceph/ceph:v20.2.4` (same
  digest as `v20.2.4-20260818`) and `quay.io/ceph/ceph:v21.1.0`. The
  `dencoder-v20` and `dencoder-v21` wrappers next to it run each image's
  `ceph-dencoder` directly. `/tmp` does not survive a reboot, so this
  plan inlines every pinned byte string and JSON document. The captures
  are provenance, not test inputs, and no test reads `/tmp`.
- The corpus harness (`rados-dencoder/tests/dencoder_corpus_comparison_test.rs`)
  stays on the v19.2.2 dencoder and the v19 archives. No 20.x or 21.x
  corpus archive exists locally (the newest is
  `19.2.0-404-g78ddc7f9027`), and every corpus sample of a changed type
  is a Squid shape, so decoding it sets every new `Option` to `None` and
  the harness's JSON and bytes are unchanged. Rust dumps keep v19.2.2's
  format, including the raw `storage_class`. The newer shapes are pinned
  only by the unit tests of Task 2.
- The Tentacle and Umbrella server behaviours (`dec_stats` subtracted,
  STALE entries filtered unless `get_stales`, restore fields stored,
  the in-class guard) cannot be exercised on the local v19.2.2 cluster.
  The PR says so. The captures are the only evidence for those shapes.

## Review Focus

1. `apply_to` mirrors `OSDMap::apply_incremental`
   (`src/osd/OSDMap.cc:2646-2662@main`, `:2541-2555@v19.2.2`):
   `require_osd_release` is applied when the byte read as `i8` is
   `>= 0`, so 0 applies and any of 0x80 to 0xff does not;
   `require_min_compat_client` is applied only when it is `> 0`. Both
   manifest entries move to "Applied below". C++'s flag side effects
   (`RECOVERY_DELETES`, `PGLOG_HARDLIMIT`) are not mirrored.
   `CephRelease`'s `Ord` agrees with C++ on every comparison between a
   set and an unset value while staying consistent with `Eq`, so
   `NONE >= UMBRELLA` is false. The `OSDMap` and `OSDMapIncremental`
   field types remain `u8`. Pinned in Task 1.
2. Content-derived versions: `encoding_version` returns the higher
   version exactly when the new `Option` is `Some`; decode sets `Some`
   exactly when `struct_v` reaches it; every v19.2.2, v20.2.4 and
   v21.1.0 instance of `rgw_cls_read_olh_log_op`,
   `rgw_bucket_dir_entry_meta`, `rgw_bucket_dir_entry`,
   `rgw_cls_obj_complete_op` and `rgw_cls_link_olh_op` round trips
   byte-exactly, and the hand-derived `update_stats` vectors do too.
   The thresholds are `>= TENTACLE` for `dec_stats` and `>= UMBRELLA`
   for `get_stales` and the restore fields. No other constructor
   gains a release parameter. Pinned in Task 2.
3. The override is read, never written: `require_osd_release()` returns
   `assume_osd_release` when set, else the current map's value, else
   `None`. It never touches the stored map, and `Client` and `IoCtx`
   delegate to `OSDClient`. Pinned by a pure-helper unit test in Task 1
   and the cluster test there.
4. The corrected docs are facts with citations (Task 3): the
   null-version bilog flag and the null-instance unlink change are
   Squid v19.2.3; the in-class guard is v20.2.0, and v20.2.0 also
   narrows the `guard_bucket_resharding` method's own trip condition;
   `ReshardStatus::IN_LOGRECORD = 3` prints `in-logrecord`;
   `STANDARD` in the meta dump is v19.2.4+ and a dump difference only;
   the Umbrella OLH semantics. None of them claims a behaviour that the
   Squid cluster tests contradict.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0: branch `release-shapes` off the fork's `main` after
plan 12 merges; the cluster up (`quay.io/ceph/ceph:v19.2.2`, per
`docker/docker-compose.ceph.yml`). Check `ls
/tmp/claude/release-shapes/v21.1.0 | wc -l` for the provenance
captures. They are not needed to build or test anything.

---

### Task 1: `rados`: `require_osd_release` kept through incrementals and exposed

**Files:**
- Modify: `rados/src/osdclient/osdmap.rs`. Replace the alias with the
  newtype. The eight field declarations that named the alias
  (`OSDMapIncremental` `:1427-1428`, `OSDMapIncrementalOsdSection`
  `:1586-1587`, `OSDMapOsdSection` `:1805-1806`, `OSDMap` `:2529-2530`)
  become `u8`, and `CephRelease::decode` at `:1862-1863` becomes
  `u8::decode`, so every field's type is unchanged. `apply_to` and its
  manifest change as described below.
- Modify: `rados/src/osdclient/client.rs` (the config field, the
  accessor, the helper), `rados/src/osdclient/ioctx.rs` (the
  accessor), `rados/src/client.rs` (the `Client` accessor, the
  `ClientBuilder` field and setter, passed into `OSDClientConfig` in
  `build`), `rados/src/osdclient/mod.rs` (`pub use
  osdmap::{CephRelease, ..}`), `rados/src/lib.rs` (`CephRelease` in the
  `pub use osdclient::{..}` list), and `rados/tests/common/mod.rs`
  (split `build_test_client` into `test_client_builder() ->
  Result<ClientBuilder, ..>`, which does the `CEPH_CONF`/`CEPH_KEYRING`
  resolution, plus the existing `build`. Behaviour is unchanged for
  current callers, including `rados-cls`'s tests, which include this
  file by `#[path]`).
- Create: `rados/tests/osdclient_release.rs`. Modify
  `.github/workflows/test-with-ceph.yml` to add `osdclient_release` to
  both `rados` `for test in` lists (the CRC-on and CRC-off loops).

**Interfaces:**
- Produces, in `osdmap.rs` where the alias was:
  ```rust
  /// `ceph_release_t`: a Ceph release as the OSD map carries it, one
  /// lossless byte. Values past the named ones (a release newer than
  /// this crate) are kept.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub struct CephRelease(pub u8);
  impl CephRelease {
      pub const UNKNOWN: Self = Self(0);
      pub const REEF: Self = Self(18);
      pub const SQUID: Self = Self(19);
      pub const TENTACLE: Self = Self(20);
      pub const UMBRELLA: Self = Self(21);
      /// An incremental's "no change" (`ceph_release_t{0xff}`).
      pub const NONE: Self = Self(0xff);
      /// Not negative as `i8`; C++ treats a negative byte as "no release".
      pub fn is_set(self) -> bool;
      /// `operator<<(ceph_release_t)`: `ceph_release_name` of the byte
      /// as an `int`, so 1..=21 give their names and every other value
      /// (0, 22 and up, 0x80 and up) gives "unknown".
      pub fn name(self) -> &'static str;
  }
  impl Ord for CephRelease       // key (is_set(), byte): unset sorts first, as C++ `operator<`
  impl PartialOrd for CephRelease // Some(self.cmp(other))
  impl fmt::Display for CephRelease // name()
  impl From<u8> for CephRelease; impl From<CephRelease> for u8;
  ```
  The names table is `src/common/ceph_strings.cc:77-127@main`
  (`argonaut` = 1 ... `umbrella` = 21). The C++ stream operator is
  `src/common/ceph_releases.cc:7-10@main`.
- `apply_to`: move `new_require_min_compat_client: _` and
  `new_require_osd_release: _` from the "Intentionally not applied"
  block into "Applied below". After the ratio block, add the following,
  with a comment naming `OSDMap.cc:2646-2662@main`:
  ```rust
  if (self.new_require_osd_release as i8) >= 0 {
      base.require_osd_release = self.new_require_osd_release;
  }
  if (self.new_require_min_compat_client as i8) > 0 {
      base.require_min_compat_client = self.new_require_min_compat_client;
  }
  ```
  The mon writes a real value only when the release changes (`ceph osd
  require-osd-release`, `set-require-min-compat-client`); every other
  incremental carries 0xff.
- `OSDClientConfig` gains `pub assume_osd_release: Option<CephRelease>`
  (default `None`), with this doc: "Overrides
  [`OSDClient::require_osd_release`] for testing: the release a caller
  selects request shapes by. The stored map is never changed."
- `OSDClient::require_osd_release(&self) -> Option<CephRelease>`. It
  returns `effective_release(self.config.assume_osd_release,
  self.osdmap_rx.borrow().as_deref())`, where `fn
  effective_release(assumed: Option<CephRelease>, map: Option<&OSDMap>)
  -> Option<CephRelease>` is private: `assumed.or(map.map(|m|
  CephRelease(m.require_osd_release)))`. Doc: the map's value is kept
  current through incrementals, `None` means no map yet, and
  `Some(CephRelease::UNKNOWN)` (0) is a map whose release was never
  set, which no v19 mon creates; a fresh v19 mon writes `squid` in
  `create_initial` (`src/mon/OSDMonitor.cc:677-687@v19.2.2`). It does
  not wait for a map; librados's `get_min_compatible_osd`
  (`src/librados/RadosClient.cc:436-447@main`) does.
- `Client::require_osd_release(&self) -> Option<CephRelease>` and
  `IoCtx::require_osd_release(&self) -> Option<CephRelease>` delegate to
  their `OSDClient`. `ClientBuilder::assume_osd_release(mut self,
  release: CephRelease) -> Self` sets `Some(release)`.
- `require_min_compat_client` is applied but not exposed. It is the
  mon's floor on client releases, not a class-shape selector (report
  §1.4).

Unit tests (in `osdmap.rs`'s `mod tests`, each case on a fresh
`OSDMap::new()` and `OSDMapIncremental::new(Epoch::new(1))` as
`test_apply_to_adopts_fsid_on_first_apply` builds them):
- `test_apply_to_follows_require_osd_release`: base 19. The default
  incremental (0xff) leaves 19; 0x80 leaves 19; 20 sets 20; 0 sets 0.
- `test_apply_to_follows_require_min_compat_client`: base 12. 0xff
  leaves 12; 0 leaves 12; 18 sets 18.
- `ceph_release_names_and_order`: the five constants' bytes;
  `SQUID.to_string() == "squid"`, `TENTACLE` gives `"tentacle"`,
  `UMBRELLA` gives `"umbrella"`, `CephRelease(1)` gives `"argonaut"`,
  and `UNKNOWN`, `CephRelease(22)` and `NONE` each give `"unknown"`;
  `NONE < UNKNOWN < REEF < SQUID < TENTACLE < UMBRELLA <
  CephRelease(22)`; `CephRelease(0x80) < UNKNOWN`; `!NONE.is_set()`,
  `UNKNOWN.is_set()`; `u8::from(SQUID) == 19`.
- In `client.rs`'s tests: `effective_release(None, None) == None`;
  `(None, Some(map with 19)) == Some(SQUID)`; `(Some(TENTACLE),
  Some(map with 19)) == Some(TENTACLE)`; `(Some(UMBRELLA), None) ==
  Some(UMBRELLA)`.

Cluster tests (`rados/tests/osdclient_release.rs`, all `#[ignore]`, run
with `CEPH_CONF=... cargo test -p rados --test osdclient_release --
--ignored`):
1. `require_osd_release_is_squid_and_survives_incrementals`:
   `build_test_client()`. `client.require_osd_release()`,
   `client.osd_client().require_osd_release()` and the test pool's
   `ioctx.require_osd_release()` are all `Some(CephRelease::SQUID)`.
   With `e0` the current epoch, `osd_client().create_pool(<unique>,
   None)`, then `wait_for_latest_osdmap(10 s)`; the epoch is now above
   `e0`, and the release is still `Some(SQUID)`, because those
   incrementals carried 0xff. `delete_pool(<unique>, true)`, wait
   again, and it is still `Some(SQUID)`. The upward flip cannot be
   exercised: `require-osd-release` never goes down and cannot pass the
   daemons' release. The commit body says so.
2. `assume_osd_release_takes_precedence`: `test_client_builder()?
   .assume_osd_release(CephRelease::TENTACLE).build()`. The client and
   an `IoCtx` report `Some(TENTACLE)`, and
   `client.osd_client().get_osdmap().await?.require_osd_release == 19`
   (the map is untouched).

- [ ] **Step 1: Implement, test, commit**

Commit:

```
osdclient: follow require_osd_release through incrementals and expose it

OSDMapIncremental::apply_to listed both release fields as not applied,
so a client kept the require_osd_release it bootstrapped with through
every later incremental. Apply them as OSDMap::apply_incremental does:
the OSD release when its byte is not negative as i8 (0xff means no
change), the minimum client release only when it is above unknown.

CephRelease becomes a lossless byte newtype with C++'s ordering and
names, and OSDClient, Client and IoCtx gain require_osd_release(),
which the new assume_osd_release config field overrides for testing
without touching the map. The map fields stay u8. On the v19 test
cluster the release reads squid and survives incrementals; the flip
to a later release cannot be exercised there.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 2: `rados-cls`: the later request shapes, selected by release

**Files:**
- Modify: `rados-cls/src/rgw/index.rs` (`DirEntryMeta`, `RestoreInfo`,
  `UpdateStatsOp`, `update_stats_op`, `update_stats`, their tests),
  `rados-cls/src/rgw/olh.rs` (`ReadOlhLogOp`, `read_olh_log_op`,
  `read_req`, `read_olh_log`, their tests), `rados-cls/src/rgw/mod.rs`
  (the "Release shapes" doc paragraph), and the existing callers:
  `rados-cls/tests/cls_rgw_index.rs` (`update_stats` in
  `index_check_and_rebuild`) and `rados-cls/tests/cls_rgw_olh.rs` (the
  `read_log` helper and the wrong-tag call). Both pass
  `CephRelease::SQUID`, so their behaviour is unchanged. The
  dencoder arms and the corpus table are unchanged.

**Interfaces and wire (all versions and compats are C++'s):**
- `DirEntryMeta` (`rgw_bucket_dir_entry_meta`, encode 7 or 8, compat 3,
  `MAX_DECODE_VERSION` 8, floor 7) gains `pub restore:
  Option<RestoreInfo>`, and the new type is:
  ```rust
  /// The restore fields of `rgw_bucket_dir_entry_meta` version 8
  /// (Umbrella v21.1.0+, `cls_rgw_types.h:218-219@v21.1.0`).
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct RestoreInfo {
      /// `RGWRestoreStatus`: 0 None, 1 RestoreAlreadyInProgress,
      /// 2 CloudRestored, 3 RestoreFailed (`rgw_sal.h:173-178@v21.1.0`).
      pub status: u8,
      /// `restore_expiry_date`, a `real_time` encoded as `UTime` (u32
      /// seconds, u32 nanoseconds); zero when not applicable.
      pub expiry_date: UTime,
  }
  ```
  On the wire, after `appendable`, version 8 adds `status` (u8) and then
  `expiry_date` (u32 sec, u32 nsec). `encoding_version` is 8 when
  `restore.is_some()`, else 7. `encode_content` writes the two fields
  only when `Some`; `decode_content` reads them into `Some` when
  `struct_v >= 8`, else `None`. The dump appends `restore_status`
  (number) and `restore_expiry_date` (`DumpUtime`, which is
  `utime_t::gmtime`, as v21's `encode_json` of a `real_time` prints it)
  after `appendable`, only when `Some`. The struct-field count passed
  to `serialize_struct` follows. The struct doc drops "version 8's
  restore fields are not modelled".
- `DirEntryMeta::for_release(release: CephRelease) -> Self`: the
  default meta with `restore = (release >= CephRelease::UMBRELLA)
  .then(RestoreInfo::default)`. Doc: build request metadata (inside
  `CompleteOp`, `LinkOlhOp`) from this and then fill the fields, because
  Umbrella's radosgw sends version 8 with zeros when nothing was
  restored. A meta decoded from a listing keeps the version it arrived
  in, and re-encodes the same way, as C++ does. `for_release` with a
  release newer than the OSD's is the caller error of the rule.
- `UpdateStatsOp` (`rgw_cls_bucket_update_stats_op`, no dencoder type):
  drop the derive and hand-write `VersionedEncode` + `impl_denc_for_versioned!`.
  Encode 2 when `dec_stats.is_some()`, else 1; compat 1;
  `MAX_DECODE_VERSION` 2; floor 1. Fields `absolute: bool`, `stats`,
  and new `pub dec_stats: Option<BTreeMap<ObjCategory, CategoryStats>>`.
  Wire: `absolute`, `stats`, then `dec_stats` when `Some`
  (`cls_rgw_ops.h:498-512@v20.2.4`). The dump is `absolute`, `stats`,
  then `dec_stats` as the same `{"key","val"}` entries when `Some`
  (`cls_rgw_ops.cc:382-395@v20.2.4`). Doc facts: a Tentacle+ class
  subtracts `dec_stats` from the header when not `absolute`, and
  answers `EINVAL` for a non-empty `dec_stats` with `absolute`
  (`cls_rgw.cc:809-830@v20.2.4`); a Squid class ignores the field.
- `update_stats_op(release: CephRelease, absolute: bool, stats:
  &BTreeMap<ObjCategory, CategoryStats>) -> Result<OSDOp>` and `async
  fn update_stats(ioctx, oid, release, absolute, stats) -> Result<()>`
  set `dec_stats = (release >= CephRelease::TENTACLE).then(BTreeMap::new)`,
  the empty map every Tentacle+ radosgw sends. A caller that needs a
  non-empty `dec_stats` builds `UpdateStatsOp` itself.
- `ReadOlhLogOp` (`rgw_cls_read_olh_log_op`, encode 1 or 2, compat 1,
  `MAX_DECODE_VERSION` 2, floor 1) gains
  `#[serde(skip_serializing_if = "Option::is_none")] pub get_stales:
  Option<bool>`. The wire appends it as a `bool` when `Some`, and
  `encoding_version` is 2 when `Some`. Decode sets `Some` when
  `struct_v >= 2`. The dump appends `get_stales` when `Some`
  (`cls_rgw_ops.cc:259-265@v21.1.0`). Doc: an Umbrella+ class
  returns `CLS_RGW_OLH_OP_STALE` (4) entries only when `get_stales` is
  true, and otherwise filters them "for backward compatibility"
  (`cls_rgw.cc:2289-2292@v21.1.0`). Umbrella's radosgw always sends
  `true` and handles op 4 when replaying. A Squid class ignores the
  field.
- `read_olh_log_op(release: CephRelease, olh: &ObjKey, ver_marker: u64,
  olh_tag: &str) -> Result<OSDOp>` and `async fn read_olh_log(ioctx,
  oid, release, olh, ver_marker, olh_tag) -> Result<ReadOlhLogRet>`:
  `read_req` sets `get_stales = (release >=
  CephRelease::UMBRELLA).then_some(true)`.
- Every signature names `rados::CephRelease`. No other constructor
  changes. `CompleteOp`, `LinkOlhOp` and `DirEntry` carry the meta's
  version without code changes.
- `rgw/mod.rs` gains a "Release shapes" paragraph: the request table
  from Global Constraints. Requests follow the cluster's
  `require_osd_release` (`IoCtx::require_osd_release()`, or
  `OSDClientConfig::assume_osd_release`), and replies decode up to what
  Umbrella writes. Building a request for a release later than the
  OSD's is a caller error: a Squid OSD skips a compat-1 tail, so
  nothing breaks, but the rule is not to send it. The deferred shapes
  are named with their reasons, in one sentence each.

Unit-test pins. Embed each byte string and JSON document as a literal,
with a comment naming its capture file (`<tag>/<type>.<n>`) or marking
it "hand-derived". The crate's JSON is compact serde output; every
capture JSON below is the compact form with `storage_class` set to
`""`. v19.2.4 and later print an empty `storage_class` as `STANDARD`
(Task 3), and the crate keeps v19.2.2's raw dump.

- `ReadOlhLogOp`:
  - `v19.2.2/rgw_cls_read_olh_log_op.2` (hex-identical in `v20.2.4`) =
    `01011a0000000101080000000000000000000000000000000000000000000000` decodes with `get_stales:
    None`, re-encodes identically and dumps
    `{"olh":{"name":"","instance":""},"ver_marker":0,"olh_tag":""}`. Instance 1 is the existing
    `read_olh_log_op_is_sent_as_version_one` pin, which now also asserts
    `get_stales == None`; its `reframed(.., 2, .., &[1])` case now
    decodes to `Some(true)`.
  - `v21.1.0/rgw_cls_read_olh_log_op.1` =
    `02012600000001010c000000040000006e616d65000000007b00000000000000070000006f6c685f74616701`: `{olh: key("name"),
    ver_marker: 123, olh_tag: "olh_tag", get_stales: Some(true)}`
    encodes to it, decodes from it, and dumps
    `{"olh":{"name":"name","instance":""},"ver_marker":123,"olh_tag":"olh_tag","get_stales":true}`.
  - `v21.1.0/rgw_cls_read_olh_log_op.2` =
    `02011b000000010108000000000000000000000000000000000000000000000000`: the default with
    `Some(false)`; dump `{"olh":{"name":"","instance":""},"ver_marker":0,"olh_tag":"","get_stales":false}`.
  - `read_olh_log_op(SQUID | TENTACLE, &key("name"), 123, "olh_tag")`'s
    request is the v19 instance-1 bytes; `(UMBRELLA, ..)`'s is
    `v21.1.0/..1` above. Extend `ops_name_the_rgw_class_and_their_method`.
- `DirEntryMeta`:
  - `v20.2.4/rgw_bucket_dir_entry_meta.{1,2}` are hex-identical to
    v19.2.2's. Instance 1 is the existing
    `dir_entry_meta_dump_order_differs_from_wire_order` pin, now also
    asserting `restore == None`. Instance 2 =
    `0703320000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000` is `DirEntryMeta::default()`
    and `DirEntryMeta::for_release(SQUID)` and `for_release(TENTACLE)`.
  - `v21.1.0/rgw_bucket_dir_entry_meta.1` =
    `08035c00000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000002d202964900000000`: `meta_instance()` with
    `restore: Some(RestoreInfo { status: 2, expiry_date: UTime { sec:
    1234567890, nsec: 0 } })`. It round trips and dumps
    `{"category":1,"size":100,"mtime":"0.000000","etag":"etag","storage_class":"","owner":"owner","owner_display_name":"display name","content_type":"content/type","accounted_size":0,"user_data":"","appendable":false,"restore_status":2,"restore_expiry_date":"2009-02-13T23:31:30.000000Z"}`. The same value with
    `restore: None` encodes to the v19 instance-1 bytes.
  - `v21.1.0/rgw_bucket_dir_entry_meta.2` =
    `08033b0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000`: `DirEntryMeta::for_release(UMBRELLA)`
    (`restore: Some(RestoreInfo::default())`); dump
    `{"category":0,"size":0,"mtime":"0.000000","etag":"","storage_class":"","owner":"","owner_display_name":"","content_type":"","accounted_size":0,"user_data":"","appendable":false,"restore_status":0,"restore_expiry_date":"0.000000"}`.
  - Update `decoders_accept_mains_version_and_skip_its_tail`: meta v8
    with nine zero bytes now decodes to the meta with `restore:
    Some(RestoreInfo::default())` and re-encodes to the same bytes. The
    version-9 rejection stays. The comment names Umbrella v21.1.0 for
    meta v8 and Tentacle v20.2.0 for header v8 and reshard entry v3.
    Those two stay skipped, not modelled, because they are reply-only
    and out of this plan.
- Embedding types, each decoded from the v21.1.0 bytes, re-encoded
  identically and dumped as given; their v19.2.2 instances are already
  pinned and now also assert `meta.restore == None`:
  - `v21.1.0/rgw_bucket_dir_entry.1` = `0803a2000000040000006e616d65d2040000000000000108035c00000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000002d20296490000000000000000070000006c6f6361746f720101040000000182d20400030000007461670000000000000000000000000000`,
    dump `{"name":"name","instance":"","ver":{"pool":1,"epoch":1234},"locator":"locator","exists":true,"meta":{"category":1,"size":100,"mtime":"0.000000","etag":"etag","storage_class":"","owner":"owner","owner_display_name":"display name","content_type":"content/type","accounted_size":0,"user_data":"","appendable":false,"restore_status":2,"restore_expiry_date":"2009-02-13T23:31:30.000000Z"},"tag":"tag","flags":0,"pending_map":[],"versioned_epoch":0}`.
  - `v21.1.0/rgw_bucket_dir_entry.2` = `080381000000040000006e616d65d2040000000000000108033b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000070000006c6f6361746f720101040000000182d20400030000007461670000000000000000000000000000`,
    dump `{"name":"name","instance":"","ver":{"pool":1,"epoch":1234},"locator":"locator","exists":true,"meta":{"category":0,"size":0,"mtime":"0.000000","etag":"","storage_class":"","owner":"","owner_display_name":"","content_type":"","accounted_size":0,"user_data":"","appendable":false,"restore_status":0,"restore_expiry_date":"0.000000"},"tag":"tag","flags":0,"pending_map":[],"versioned_epoch":0}`.
  - `v21.1.0/rgw_bucket_dir_entry.3` = `0803790000000000000000000000000000000008033b0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001010a00000088ffffffffffffffff0000000000000000000000000000000000000000`,
    dump `{"name":"","instance":"","ver":{"pool":-1,"epoch":0},"locator":"","exists":false,"meta":{"category":0,"size":0,"mtime":"0.000000","etag":"","storage_class":"","owner":"","owner_display_name":"","content_type":"","accounted_size":0,"user_data":"","appendable":false,"restore_status":0,"restore_expiry_date":"0.000000"},"tag":"","flags":0,"pending_map":[],"versioned_epoch":0}`.
  - `v21.1.0/rgw_cls_obj_complete_op.1` (168 B) =
    `0907a200000001640000000000000008035c00000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000002d20296490000000003000000746167070000006c6f6361746f720000000001010200000002640001010c000000040000006e616d6500000000000000000000`, dump
    `{"op":1,"name":"name","instance":"","locator":"locator","ver":{"pool":2,"epoch":100},"meta":{"category":1,"size":100,"mtime":"0.000000","etag":"etag","storage_class":"","owner":"owner","owner_display_name":"display name","content_type":"content/type","accounted_size":0,"user_data":"","appendable":false,"restore_status":2,"restore_expiry_date":"2009-02-13T23:31:30.000000Z"},"tag":"tag","log_op":false,"bilog_flags":0,"zones_trace":[]}`.
  - `v21.1.0/rgw_cls_obj_complete_op.2` (129 B) =
    `09077b00000000000000000000000008033b000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001010a00000088ffffffffffffffff00000101080000000000000000000000000000000000`, dump
    `{"op":0,"name":"","instance":"","locator":"","ver":{"pool":-1,"epoch":0},"meta":{"category":0,"size":0,"mtime":"0.000000","etag":"","storage_class":"","owner":"","owner_display_name":"","content_type":"","accounted_size":0,"user_data":"","appendable":false,"restore_status":0,"restore_expiry_date":"0.000000"},"tag":"","log_op":false,"bilog_flags":0,"zones_trace":[]}`.
  - `v21.1.0/rgw_cls_link_olh_op.1` (176 B) =
    `0501aa00000001010c000000040000006e616d6500000000070000006f6c685f74616701060000006f705f74616708035c00000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000002d2029649000000007b00000000000000010000000000000000000000000000000000000000000000`, dump
    `{"key":{"name":"name","instance":""},"olh_tag":"olh_tag","delete_marker":true,"op_tag":"op_tag","meta":{"category":1,"size":100,"mtime":"0.000000","etag":"etag","storage_class":"","owner":"owner","owner_display_name":"display name","content_type":"content/type","accounted_size":0,"user_data":"","appendable":false,"restore_status":2,"restore_expiry_date":"2009-02-13T23:31:30.000000Z"},"olh_epoch":123,"log_op":true,"bilog_flags":0,"unmod_since":"0.000000","high_precision_time":false,"zones_trace":[]}`.
  - `v21.1.0/rgw_cls_link_olh_op.2` (126 B) =
    `050178000000010108000000000000000000000000000000000000000008033b00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000`, dump
    `{"key":{"name":"","instance":""},"olh_tag":"","delete_marker":false,"op_tag":"","meta":{"category":0,"size":0,"mtime":"0.000000","etag":"","storage_class":"","owner":"","owner_display_name":"","content_type":"","accounted_size":0,"user_data":"","appendable":false,"restore_status":0,"restore_expiry_date":"0.000000"},"olh_epoch":0,"log_op":false,"bilog_flags":0,"unmod_since":"0.000000","high_precision_time":false,"zones_trace":[]}`.
- `UpdateStatsOp` (hand-derived from `cls_rgw_ops.h:498-512@v20.2.4`
  and `cls_rgw_ops.cc:382-395@v20.2.4`; not a dencoder type; `CS` is
  the existing test's `CategoryStats {1, 4096, 1, 0}`, encoded
  `0302200000000100000000000000001000000000000001000000000000000000000000000000`):
  - The existing v1 pin `{true, {NONE: CS}}` =
    `01012c0000000101000000000302200000000100000000000000001000000000000001000000000000000000000000000000`
    keeps its bytes and JSON with `dec_stats: None`.
  - `{false, {}, None}` = `0101050000000000000000`.
  - `{false, {}, Some({})}` = `020109000000000000000000000000`, dump
    `{"absolute":false,"stats":[],"dec_stats":[]}`.
  - `{false, {MAIN: CS}, Some({})}` =
    `020130000000000100000001030220000000010000000000000000100000000000000100000000000000000000000000000000000000`.
  - `{false, {}, Some({MAIN: CS})}` =
    `020130000000000000000001000000010302200000000100000000000000001000000000000001000000000000000000000000000000`,
    dump `{"absolute":false,"stats":[],"dec_stats":[{"key":1,"val":{"total_size":1,"total_size_rounded":4096,"num_entries":1,"actual_size":0}}]}`.
  - Each decodes back to its value. `with_tail(&bytes(&last), 3,
    &[0xaa])` decodes to `last` with the byte skipped (buffer empty
    after decode); `{false, {}, Some({})}`'s bytes reframed to version 1
    with its four `dec_stats` bytes cut (`reframed(.., 1, 5, &[])`) are
    exactly `0101050000000000000000` and decode to `{false, {}, None}`.
  - `update_stats_op(SQUID, false, &{})`'s request is
    `0101050000000000000000`; `(TENTACLE, ..)` and `(UMBRELLA, ..)` give
    `020109000000000000000000000000`. Update the existing
    `update_stats_op(false, &stats)` case in the index constructor test
    to pass `SQUID`.

- [ ] **Step 1: Implement, test, commit**

Commit:

```
cls: rgw request shapes of Tentacle and Umbrella, selected by release

Tentacle's radosgw sends bucket_update_stats at version 2 with an
empty dec_stats; Umbrella's sends bucket_read_olh_log at version 2
with get_stales set, and index-entry metadata at version 8 with the
restore status and expiry. Each new field is an Option whose presence
sets the encoded version, so the v19.2.2, v20.2.4 and v21.1.0
ceph-dencoder instances all round-trip byte for byte, and the
constructors that build their own request take the cluster's
require_osd_release to choose. DirEntryMeta::for_release does the
same for metadata the caller builds. Every change keeps compat 1 or
3, so a Squid OSD skips the tail; sending a later release's shape is
still a caller error.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 3: `rados-cls`: what the docs got wrong about later releases

**Files:**
- Modify: `rados-cls/src/rgw/index.rs`, `rados-cls/src/rgw/olh.rs`,
  `rados-cls/src/rgw/types.rs`.

Each fact below replaces or extends the named doc and keeps its
citation in the doc.

1. **Null version is Squid v19.2.3, not Tentacle.**
   - `BILOG_NULL_VERSION` (`index.rs:592`, now "Tentacle v20+; a v19
     cluster never sets it") becomes: "`RGW_BILOG_NULL_VERSION`. radosgw
     sets it on a null-version delete from Squid v19.2.3 (c860a396697,
     012d8ebd71f; `rgw_rados.cc:7233,10355@main`), and
     `cls_rgw_bucket_unlink_instance` passes it."
   - `BiLogEntry::is_null_verid` (`index.rs:708`) drops "always false
     on v19": entries written by v19.2.3+ radosgw, or unlinked by this
     crate on any Squid class, carry it.
   - `unlink_instance_op` gains the null-instance handling that release
     changed. The v19.2.2 class maps instance `"null"` to `""`
     server-side (`cls_rgw.cc:1889-1892@v19.2.2`); v19.2.3 and later no
     longer map it in `unlink_instance`'s key. radosgw from v19.2.3
     unlinks the null version with instance `""` and `bilog_flags |=
     BILOG_NULL_VERSION`.
   - The ruling to document: the crate always sends that v19.2.3+ form.
     Send `""` with the flag, never `"null"`. Every Squid class, v19.2.2
     included, records `bilog_flags` in the bilog entry it logs
     (`cls_rgw.cc:2022,166@v19.2.2`) and handles an instance of `""` the
     same way, so this form behaves identically on every Squid point
     release; only radosgw's side changed in v19.2.3, and
     `require_osd_release`, which is 19 for all of them, cannot tell
     them apart anyway.
   - `link_olh_op`'s "`ENOENT` for a new delete marker on an object
     whose current version already is one" becomes "on v19.2.2
     (`cls_rgw.cc:1673-1693@v19.2.2`); v19.2.3 removed the check and
     links it".
2. **The class guards itself from v20.2.0.**
   - Extend the module docs of `index.rs` (`:8-13`, "Ceph v19's class
     does not guard itself") and `olh.rs` (the "take no resharding
     guard of their own" sentence). From Tentacle v20.2.0 (d011c522bb1)
     the class runs `guard_bucket_resharding(hctx, header)`
     (`cls_rgw.cc:906-921@v20.2.4`) inside prepare, complete, link_olh,
     unlink_instance, trim_olh_log, clear_olh, suggest_changes and
     rebuild_index. It answers -2300 when the header is `IN_PROGRESS`,
     or `IN_LOGRECORD` with `reshardlog_entries >=
     rgw_reshardlog_threshold`.
   - So on those releases a write can fail with -2300 even without
     `guard_op`. `guard_op` is still what radosgw prepends: `main`'s
     link does `assert_exists`, then the guard, then the link
     (`rgw_rados.cc:9573-9597@main`). Keep prepending it.
   - Also correct `guard_bucket_resharding_op`'s doc ("anything but
     `NOT_RESHARDING`"). That is v19's `header.resharding()`
     (`cls_rgw.cc:4606@v19.2.2`). From v20.2.0 the method calls the same
     helper (`cls_rgw.cc:4995-5016@v20.2.4`), so `DONE` no longer
     trips it, and `IN_LOGRECORD` trips it only at the threshold. (The
     report does not list this one; it was verified against the tags
     while drafting this plan.)
3. **`ReshardStatus` gains `IN_LOGRECORD = 3`** in `types.rs`
   (`cls_rgw_types.h:735-756@v20.2.4`, v20.2.0). `as_str` returns
   `"in-logrecord"` for it, and `BucketInstanceEntry` gains `pub fn
   resharding_in_logrecord(&self) -> bool` (`cls_rgw_types.h:804@v20.2.4`).
   `resharding()` is already `!= NOT_RESHARDING`. Pin, hand-derived
   from `v19.2.2/cls_rgw_bucket_instance_entry.1`
   (`0301090000000000000000ffffffff`) with the status byte set to 3:
   `0301090000000300000000ffffffff` decodes to `IN_LOGRECORD`, re-encodes
   identically and dumps `{"reshard_status":"in-logrecord"}`, and
   `resharding_in_logrecord()` is true while `resharding_in_progress()`
   is false.
4. **`STANDARD` is a v19.2.4+ dump difference.** On `DirEntryMeta`'s
   doc: the crate dumps `storage_class` raw, as v19.2.2's dencoder
   does. From Squid v19.2.4, `rgw_bucket_dir_entry_meta::dump` prints
   an empty one as `STANDARD` through `get_canonical_storage_class`
   (`cls_rgw_types.cc:199-217@main`; absent at v19.2.3, present at
   v19.2.4). The bytes are the same; only the dump differs, which is why
   the corpus harness stays on the v19.2.2 dencoder.
5. **Umbrella's OLH semantics** (deferred, documented only). Add to
   `olh.rs`'s server facts, as the v21.1.0 differences from the v19
   rule it states: `olh_epoch = 0` makes the class use the current time
   in nanoseconds since 1970 rather than the OLH's epoch plus one
   (`cls_rgw.cc:1865,1927@v21.1.0`). Link and unlink log `STALE` (4)
   entries, which `read_olh_log` returns only with `get_stales`
   (`cls_rgw.cc:1875,2009,2176,2198,2289-2292@v21.1.0`). Change the
   `OlhLogOp` doc's "`CLS_RGW_OLH_OP_STALE` (4) is not modelled" to
   add "(Umbrella v21.1.0+; kept as the raw byte)".

Also correct the stale "`main`" attributions in the docs Task 2 did not
already rewrite. The `ReadOlhLogOp` doc says "`main`'s version 2", and
it is Umbrella v21.1.0.

Unit tests: the `ReshardStatus` pin above. Everything else is docs,
checked by `cargo doc -p rados-cls --no-deps` with warnings denied (a
stale intra-doc link fails it).

- [ ] **Step 1: Implement, test, commit**

Commit:

```
cls: correct what the rgw docs say about later releases

The null-version bilog flag and the unlink that no longer maps a
"null" instance shipped in Squid v19.2.3, not Tentacle; the crate
sends that form, which a v19.2.2 class treats the same. From Tentacle
v20.2.0 the class guards its own index and OLH writes and its guard
method no longer trips on a finished reshard, so guard_op stays what
radosgw prepends but a write can fail with -2300 without it.
ReshardStatus gains IN_LOGRECORD; the STANDARD storage class in newer
dumps is a v19.2.4 dump change only; and Umbrella's nanosecond OLH
epochs and stale log entries are described where the v19 rules are.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 4: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_release_shapes.rs`. It uses
  `#[path = "../../rados/tests/common/mod.rs"] mod common;` and copies
  `unique`, `is_osd_error`, `key`, `instance`, `meta(size)`,
  `olh_tag`, `shard`, `add_instance`, `link_req` and `versions` from
  `cls_rgw_olh.rs`, as each suite keeps its own.
- Modify: `.github/workflows/test-with-ceph.yml`, adding
  `cls_rgw_release_shapes` to both `cls_*` `for test in` lists.

Every test starts by asserting `ioctx.require_osd_release() ==
Some(CephRelease::SQUID)`: these tests pin what a Squid class does with
a later shape, and they would mean something else on a newer cluster.
All are `#[ignore]`. `EOPNOTSUPP` is 95.

1. `osd_release_is_squid`: the assertion above on a fresh `IoCtx`.
2. `update_stats_tentacle_shape_is_accepted_as_v1`:
   - Take two shards `a` and `b`, with `S = {MAIN: {total_size 1024,
     total_size_rounded 4096, num_entries 1, actual_size 1024}}`.
     `index::update_stats(a, SQUID, false, &S)` and `(b, TENTACLE,
     false, &S)` both succeed, and the two `dir_header(..).stats` are
     equal and equal `S`.
   - Then send a raw v2 with a non-empty `dec_stats` to `b`:
     `ioctx.exec(&b, "rgw", "bucket_update_stats",
     rados::encode_with_capacity(&UpdateStatsOp { absolute: false,
     stats: S, dec_stats: Some(S) }, 0)?)`. It succeeds, and `b`'s
     `MAIN` is now `2 x S`. The Squid class skipped `dec_stats`; a
     Tentacle class would have left `S`.
   - Also send the same raw op with `absolute: true`. It succeeds and
     sets `MAIN` to `S`. Tentacle would answer `EINVAL`.
3. `read_olh_log_umbrella_shape_is_accepted`: `add_instance` and
   `link_olh` of `o`/`v1` (epoch 0), so the log is not empty (the class
   reads past the end of an empty log). `olh::read_olh_log(.., SQUID,
   &key("o"), 0, &olh_tag("o"))` and the same with `UMBRELLA` both
   succeed and return equal `ReadOlhLogRet`s.
4. `meta_v8_in_complete_is_stored_as_v7`: prepare `x` (`ADD`, tag
   `t`), then complete it with `meta = DirEntryMeta { restore:
   Some(RestoreInfo { status: 2, expiry_date: UTime { sec: 1234567890,
   nsec: 0 } }), ..meta(1024) }`. It succeeds. The `MAIN` stats show
   `num_entries 1` and `total_size 1024`. `list` returns `x` with
   `meta.restore == None` and every other meta field as sent: the Squid
   class decoded the v8 meta, skipped the tail and stored version 7.
5. `meta_v8_in_link_olh_is_stored_as_v7`: `add_instance` and `link_olh`
   `o`/`v1`. Then `link_olh` a delete marker `o`/`dm` with
   `delete_marker: true` and a v8 meta with the same restore and
   `etag "dm"`. On v19 the class writes the delete marker's instance
   entry from `op.meta` (`init_as_delete_marker`,
   `cls_rgw.cc:1333-1341,1743@v19.2.2`). `versions(.., "o")` shows `dm`
   current, `is_delete_marker()`, with `meta.etag == "dm"` and
   `meta.restore == None`.
6. `later_methods_are_eopnotsupp`: on an initialised shard, each of
   `bucket_init_index2`, `bi_put_entries`, `reshard_log_trim` and
   `bucket_refresh_instance` sent as `ioctx.exec(&oid, "rgw", method,
   Bytes::new())` is `OSDError { code: -95 }`. The method lookup fails
   before any decode: `OpInfo::set_from_op` maps `get_method_flags`'s
   `-ENOENT` to `-EOPNOTSUPP` (`src/osd/osd_op_util.cc:189-196@v19.2.2`)
   before `PrimaryLogPG.cc:6148-6152` runs. No constructors are added.

State in the module doc that what Tentacle and Umbrella themselves do
with these shapes cannot be tested here, and that the Task 2 pins are
the evidence for those shapes.

- [ ] **Step 1: Implement, run on the cluster, commit**

Commit:

```
cls: release-shape cluster tests

Pins against Ceph v19.2.2, whose required OSD release reads squid: an
update_stats built for Tentacle changes the header as version 1 does
and a non-empty dec_stats tail is ignored; a read_olh_log built for
Umbrella returns the same log; version-8 metadata inside a complete
or a delete-marker link is accepted and stored as version 7; and
bucket_init_index2, bi_put_entries, reshard_log_trim and
bucket_refresh_instance answer EOPNOTSUPP. What Tentacle and Umbrella
do with these shapes cannot be exercised on this cluster.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6:
- Per-commit proof.
- Container fmt/clippy, including `rados-cls` with only `rgw` and with
  no default features.
- `cargo doc` with warnings denied.
- The corpus harness over the rgw names, unchanged, to prove that the
  v19 samples still decode with `None` and re-encode identically.
- On the cluster: `osdclient_release`, the whole `cls_rgw_*` family
  (`cls_rgw_index` and `cls_rgw_olh` now pass `SQUID`) and
  `cls_rgw_release_shapes`.

The implementer for Tasks 1 and 2 is the opus tier, because Task 1 is
transport design and Task 2's version logic must be exact. Tasks 3 and
4 can also go to opus, and the whole-branch review stays on the session
model. The PR body:

```
**Motivation.** The design's encoding rule sends each class request at the version the cluster's radosgw writes, keyed on the OSD map's `require_osd_release`, which rados-rs decoded but dropped from every incremental.

**What changed.** `rados` applies both release fields from incrementals as C++ does and exposes `require_osd_release()` with a test override; `rados-cls` offers `update_stats` v2 (Tentacle) and `read_olh_log` v2 and metadata v8 (Umbrella) by release, pinned against v20.2.4 and v21.1.0 captures, and corrects its docs about later releases.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

## Roadmap for later plans

This closes the campaign. Deferred, with the reason each waits:
- The OLH epoch reply on link and unlink. It is on `main` only and
  feeds only the multisite FIFO bilog. When a release tag carries it,
  add `link_olh_epoch`/`unlink_instance_epoch` returning `Option<u64>`
  (empty outdata is `None`), a `decode_olh_epoch(&OpReply)` for the
  compound form, `rgw` in `exec_returnvec`'s gate, and a threshold
  above `UMBRELLA` (report §4.3).
- `bucket_init_index2`, `bi_put_entries`, `reshard_log_trim`,
  `bi_list` v2 and `reshard_add` v2 (resharding, a non-goal).
  `bucket_refresh_instance` (cloud restore, excluded; `main` only).
- Umbrella's nanosecond OLH epochs and `STALE` entries, as behaviour:
  an `OlhLogOp::STALE` constant and replay handling belong to rgw-rs's
  OLH logic once it targets Umbrella.
- Reply-side v20 fields that are still skipped rather than modelled:
  `DirHeader::reshardlog_entries` (header v8), `ReshardEntry::initiator`
  (v3), `BiIndexType` 4 (`ReshardDeleted`); and `main`'s
  `ObjCategory` 5 (`MultiPart`) and `rgw_gc::init` with
  `num_deferred = 0` (both unreleased).
- `MonCephRelease` is lossy: a Tentacle monmap's `min_mon_release` 20
  decodes as `MAX`, and 21 or 0xff as `Unknown`. It needs `Tentacle`
  and `Umbrella` variants, or replacement by `CephRelease` (an API
  change to `MonMap`).
- A newer-oracle mode for the corpus harness, keyed by an
  `ORACLE_RELEASE` allowlist of the known dump deltas (report §3.3), for
  when 20.x or 21.x corpus archives exist.
- Squid point-release drift that `require_osd_release` cannot see
  (v19.2.3's null-version semantics, v19.2.4's `cls_2pc_urgent_data`
  v3) stays documented, not selected.
