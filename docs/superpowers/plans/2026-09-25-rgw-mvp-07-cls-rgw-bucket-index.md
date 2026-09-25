# rados-rs RGW MVP, plan 7 of N: `cls-rgw-bucket-index`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `rgw` object class's bucket-index methods: init, set tag
timeout, prepare, complete, list (and the header-only read), check and
rebuild index, suggest changes, update stats, the four resharding
methods, and the four head-object helpers (remove obj, store pg ver,
check attrs prefix, check mtime); their request and reply structs pinned
against `ceph-dencoder` v19.2.2 and the corpus; cluster tests that pin
the class's prepare/complete accounting, listing and guard semantics on
Ceph v19.2.2.

**Architecture:** `rgw::index` (plan 6) gains its ops and client, behind
the existing `rgw` feature: request and reply structs, `OSDOp`
constructors for compound use (RGW composes every index write with the
resharding guard in front), and async `IoCtx` functions. `rados` gains an
`execute_op_unchecked` that returns the reply whatever its result code,
because `bucket_list` answers `EFBIG` together with a valid reply that
the client must decode to advance.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-bucket-index`" (init only: `bucket_init_index2` is post-Squid).
Out, per the spec's non-goals: the bilog methods, the `bi_*` ops
(`bi_get`, `bi_put`, `bi_list`), the reshard queue, and the OLH methods
(plan `cls-rgw-olh`).

## Global Constraints

Plans 3 to 6's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-bucket-index` is based on the fork's `main` after plan
  6 merges; `rgw::index` holds `DirEntryMeta`, `DirEntry`, `DirHeader`,
  `Dir`, `BiEntry`, `BiLogEntry`, `BucketInstanceEntry`, `ReshardEntry`;
  `rgw::types` the enums (`ModifyOp`, `ObjCategory`, `ReshardStatus`,
  ...), `EntryVer`, `PendingInfo`, `CategoryStats`, `ZoneSet`, `ObjKey`.
- The oracle is `ceph-dencoder` v19.2.2; where Ceph `main` differs,
  v19.2.2 wins on encode and dump. Differences that matter here, all
  verified against `git show v19.2.6:src/cls/rgw/...` and the oracle:
  `rgw_cls_bucket_update_stats_op` is version 1 (`main` adds `dec_stats`
  in v2); `bucket_init_index2`, `bucket_refresh_instance`,
  `bi_put_entries` and `reshard_log_trim` do not exist on v19 (a v19 OSD
  answers `EOPNOTSUPP`); the v19 class does not guard index writes
  itself, so the client prepends `guard_bucket_resharding` to every index
  write exactly as C++ RGW does; the v19 guard trips on any status other
  than `NOT_RESHARDING`; `rgw_cls_obj_prepare_op::dump` on v19 prints
  `log_op`, `bilog_flags` and `zones_trace` (oracle:
  `{"op":0,"name":"name","tag":"tag","locator":"locator","log_op":false,"bilog_flags":0,"zones_trace":[]}`),
  which `main` dropped.
- Method names (`cls_rgw_const.h`, class `rgw`): `bucket_init_index`,
  `bucket_set_tag_timeout`, `bucket_list`, `bucket_check_index`,
  `bucket_rebuild_index`, `bucket_update_stats`, `bucket_prepare_op`,
  `bucket_complete_op`, `dir_suggest_changes`, `obj_remove`,
  `obj_store_pg_ver`, `obj_check_attrs_prefix`, `obj_check_mtime`,
  `set_bucket_resharding`, `clear_bucket_resharding`,
  `guard_bucket_resharding`, `get_bucket_resharding`. Flags: `bucket_list`,
  `bucket_check_index`, `obj_check_attrs_prefix`, `obj_check_mtime`,
  `guard_bucket_resharding`, `get_bucket_resharding` are RD;
  `obj_store_pg_ver` is WR; the rest RD|WR. No writing method here
  replies, so the returnvec rule does not apply.
- `CLS_RGW_ERR_BUSY_RESHARDING` is 2300; RGW passes `ret_err = -2300` to
  the guard, and the OSD returns that as the op's error, so
  `OSDClientError::OSDError { code: -2300, .. }` is what a guarded write
  sees during a reshard.
- `bucket_list` with `num_entries == 0` is the header read; there is no
  separate method. With `num_entries > 0`, a reply that is truncated with
  no entries comes back with result `EFBIG` (27) and a valid encoded
  reply whose `marker` is where to resume; the async `list` loops on
  it. Whether the OSD ships the reply payload alongside a negative op
  result is a research gap; Task 4's `index_list_advances_past_invalid_entries`
  settles it, and if the payload is missing the loop must be rewritten
  to page by the caller's own last key (report, do not guess).
- Suggestions are a raw concatenation of `u8 op` and an encoded
  `rgw_bucket_dir_entry`: `op` is `b'r'` (114) or `b'u'` (117), or'ed
  with `0x80` when the change should be bilogged.
- The hand-written decoders read every version branch the C++ has where
  the type is a request the dencoder round-trips (prepare, complete,
  list op, list ret); legacy one-byte headers below the compat threshold
  are not decoded (plan 6's rule).

## Review Focus

1. `rgw_cls_obj_prepare_op` (7/5) and `rgw_cls_obj_complete_op` (9/7):
   `op` is one byte, `key.name` is only written by old versions while the
   full key comes late, `complete` writes `ver.epoch` twice, `remove_objs`
   is a list of keys (strings before version 7). Pinned against the
   oracle's 50-byte and 159-byte instances.
2. `rgw_cls_list_op` (6/4) encodes `num_entries` first though it declares
   `start_obj` first; `rgw_cls_list_ret` (4/2) carries `marker` only from
   version 4 and dumps `is_truncated` as an int and no `marker`.
3. The guard: every index write constructor family has a `guarded_*`
   variant, or rather one helper `guard_op(-2300)` the tests prepend, and
   the cluster test proves a guarded prepare fails with -2300 and leaves
   the index untouched while the reshard status is `IN_PROGRESS`.
4. Accounting rules of `bucket_complete_op` as v19 implements them: a
   prepare leaves stats alone; a complete with a stale epoch on the same
   pool is a cancel; a DEL of an entry with a pending tag keeps the key
   with `exists = false`; `remove_objs` are unaccounted and removed.
   Pinned in Task 4.
5. The head-object helpers act on the object the op names, not an index
   shard: `check_attrs_prefix` and `check_mtime` fail with `ECANCELED`,
   `store_pg_ver` writes a `u64` xattr, `remove_obj` keeps only the
   prefixed xattrs. Pinned in Task 4.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0, branch `cls-rgw-bucket-index` off the fork's `main`
after plan 6 merges; the cluster up.

---

### Task 1: `rados`: `IoCtx::execute_op_unchecked`

**Files:**
- Modify: `rados/src/osdclient/ioctx.rs`.

**Interfaces:**
- Produces: `pub async fn execute_op_unchecked(&self, oid: impl
  Into<String>, op: BuiltOp) -> Result<OpResult>` next to `execute_op`:
  the same send, no `check_op_result`; transport failures are still
  errors. Doc: "for methods that answer a negative result together with a
  reply the caller must read, such as `cls_rgw`'s `bucket_list` and its
  `EFBIG`". `execute_op` becomes `execute_op_unchecked` plus the check.

- [ ] **Step 1: Implement, test, commit**

Unit test in the ioctx test module if one exists for `execute_op`'s error
path; otherwise the cluster test in Task 4 is its coverage (say so in the
commit body). Commit:

```
osdclient: execute_op_unchecked keeps the reply of a failed op

A class method can answer a negative result and still encode a reply
the client must read: cls_rgw's bucket_list returns EFBIG with the
marker to resume from. execute_op drops the reply with the error;
this variant returns it and leaves the result code to the caller.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: `rgw::index`: the request and reply structs

**Files:**
- Modify: `rados-cls/src/rgw/index.rs`, `rados-cls/src/rgw/types.rs`
  (`CheckMtimeType`), dencoder (eleven arms), corpus table (eleven:
  `rgw_cls_tag_timeout_op`, `rgw_cls_obj_prepare_op`,
  `rgw_cls_obj_complete_op`, `rgw_cls_list_op`, `rgw_cls_list_ret`,
  `rgw_cls_check_index_ret`, `rgw_cls_obj_remove_op`,
  `rgw_cls_obj_store_pg_ver_op`, `rgw_cls_obj_check_attrs_prefix`,
  `cls_rgw_set_bucket_resharding_op`,
  `cls_rgw_clear_bucket_resharding_op`,
  `cls_rgw_guard_bucket_resharding_op`; that is twelve names, all with
  corpus directories in both archives).

**Interfaces:**
- Produces in `types.rs`: `byte_enum! CheckMtimeType { EQ = 0, LT = 1,
  LE = 2, GT = 3, GE = 4 }`.
- Produces in `index.rs`: `TagTimeoutOp`, `PrepareOp`, `CompleteOp`,
  `ListOp`, `ListRet`, `CheckIndexRet`, `UpdateStatsOp`, `RemoveObjOp`,
  `StorePgVerOp`, `CheckAttrsPrefixOp`, `CheckMtimeOp`,
  `SetBucketReshardingOp`, `ClearBucketReshardingOp`,
  `GuardBucketReshardingOp`, `GetBucketReshardingOp`,
  `GetBucketReshardingRet`; `Suggestion { op: SuggestOp, log: bool,
  entry: DirEntry }` with `SuggestOp { Remove, Update }` and
  `encode_suggestions(&[Suggestion]) -> Bytes`.

Facts and pins (oracle instances; `unhex` helper as plan 6):

- `TagTimeoutOp` (`rgw_cls_tag_timeout_op`): v1; `tag_timeout: u64`; dump
  `tag_timeout` (number). Pin `{23323}` =
  `0101080000001b5b000000000000`.
- `PrepareOp` (`rgw_cls_obj_prepare_op`): encode version 7, compat 5,
  `MAX_DECODE_VERSION` 7. Fields `op: ModifyOp`, `key: ObjKey`, `tag`,
  `locator`, `log_op: bool`, `bilog_flags: u16`, `zones_trace: ZoneSet`.
  Wire: `op` u8, `tag`, `locator`, `log_op`, `key`, `bilog_flags`,
  `zones_trace`. Decode: `op`; `key.name` if v < 5; `tag`; `locator` v2+;
  `log_op` v4+; `key` v5+; `bilog_flags` v6+; `zones_trace` v7+. Dump (v19):
  `op` (number), `name`, `tag`, `locator`, `log_op`, `bilog_flags`,
  `zones_trace` (bare array). `Default` has `op: ModifyOp::UNKNOWN`. Pin
  instance 1 `{ADD, name "name", tag "tag", locator "locator"}` (50 B) =
  `07052c0000000003000000746167070000006c6f6361746f720001010c000000040000006e616d6500000000000000000000`
  and its JSON above; instance 0 (default) =
  `07051e000000030000000000000000000101080000000000000000000000000000000000`.
- `CompleteOp` (`rgw_cls_obj_complete_op`): encode version 9, compat 7,
  `MAX_DECODE_VERSION` 9. Fields `op: ModifyOp` (default `ADD`), `key`,
  `locator`, `ver: EntryVer`, `meta: DirEntryMeta`, `tag` (C++ `op_tag`),
  `log_op`, `bilog_flags: u16`, `remove_objs: Vec<ObjKey>`, `zones_trace`.
  Wire: `op` u8, `ver.epoch` u64, `meta`, `tag`, `locator`, `remove_objs`,
  `ver`, `log_op`, `key`, `bilog_flags`, `zones_trace`. Decode: `op`;
  `key.name` if v < 7; `ver.epoch`; `meta`; `tag`; `locator` v2+;
  `remove_objs` as strings (names only) if 4 <= v < 7, else as keys;
  `ver` v5+ (else `pool = -1`; note the later `ver` overwrites the epoch
  read earlier); `log_op` v6+; `key` v7+; `bilog_flags` v8+;
  `zones_trace` v9+. Dump: `op`, `name`, `instance`, `locator`, `ver`,
  `meta`, `tag`, `log_op`, `bilog_flags`, `zones_trace`. Pin instance 1
  `{DEL, name "name", locator "locator", ver {2, 100}, tag "tag", meta =
  plan 6's meta instance 1}` (159 B) =
  `09079900000001640000000000000007035300000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000003000000746167070000006c6f6361746f720000000001010200000002640001010c000000040000006e616d6500000000000000000000`
  = `{"op":1,"name":"name","instance":"","locator":"locator","ver":{"pool":2,"epoch":100},"meta":{...},"tag":"tag","log_op":false,"bilog_flags":0,"zones_trace":[]}`.
- `ListOp` (`rgw_cls_list_op`): encode version 6, compat 4,
  `MAX_DECODE_VERSION` 6. Fields `start_obj: ObjKey`, `num_entries: u32`,
  `filter_prefix`, `list_versions: bool`, `delimiter`. Wire:
  `num_entries`, `filter_prefix`, `start_obj`, `list_versions`,
  `delimiter`. Decode: `start_obj.name` if v < 4; `num_entries`;
  `filter_prefix` v3+; `start_obj` v4+; `list_versions` v5+; `delimiter`
  v6+. Dump: `start_obj` (the name only), `num_entries`. Pin instance 1
  (55 B) =
  `060431000000640000000d00000066696c7465725f7072656669780101110000000900000073746172745f6f626a000000000000000000`
  = `{"start_obj":"start_obj","num_entries":100}`.
- `ListRet` (`rgw_cls_list_ret`): version 4, compat 2, `MAX_DECODE_VERSION`
  4. Fields `dir: Dir`, `is_truncated: bool`, `marker: ObjKey`. Decode
  `marker` only if v >= 4. Dump: `dir`, `is_truncated` (int); no `marker`.
  Pin instance 3 `{default dir, truncated}` (85 B) =
  `04024f00000002023a00000007023000000000000000000000000000000000000000000000000000000000000000000000000301090000000000000000ffffffff0000000000010101080000000000000000000000`
  = `{"dir":{"header":{"ver":0,"master_ver":0,"stats":[],"new_instance":{"reshard_status":"not-resharding"}},"map":[]},"is_truncated":1}`.
- `CheckIndexRet`: v1; `existing_header: DirHeader`, `calculated_header:
  DirHeader`; dump both. Pin instance 0 = two default headers, 114 B
  (`01016c000000` + 2 x `0702300000...`).
- `UpdateStatsOp` (`rgw_cls_bucket_update_stats_op`, no dencoder
  registration): version 1 (v19); `absolute: bool`, `stats:
  BTreeMap<ObjCategory, CategoryStats>`. Dump: `absolute`, `stats` as
  `map_entries` with number keys. Pin `{true, {NONE: {1, 4096, 1, 0}}}` =
  `01012c000000 01 01000000 00 0302200000000100000000000000001000000000000001000000000000000000000000000000`.
- `RemoveObjOp`: v1; `keep_attr_prefixes: Vec<String>`. Pin instance 0 =
  `01014900000003000000130000006b6565705f617474725f707265666978657331130000006b6565705f617474725f707265666978657332130000006b6565705f617474725f707265666978657333`.
- `StorePgVerOp`: v1; `attr`. Pin `{"attr"}` = `0101080000000400000061747472`.
- `CheckAttrsPrefixOp`: v1; `check_prefix`, `fail_if_exist: bool`. Pin
  `{"prefix", true}` = `01010b0000000600000070726566697801`.
- `CheckMtimeOp` (`rgw_cls_obj_check_mtime`, no registration, no dump):
  version 2, compat 1; `mtime: UTime`, `kind: CheckMtimeType` (`type`),
  `high_precision_time: bool`. Pin `{{1, 2}, LT, true}` =
  `02010a000000010000000200000001 01`.
- `SetBucketReshardingOp`: v1; `entry: BucketInstanceEntry`; dump `entry`.
  Pin default = `01010f0000000301090000000000000000ffffffff`.
- `ClearBucketReshardingOp`, `GetBucketReshardingOp`: v1, empty; dump
  `{}`. Pin `010100000000`.
- `GuardBucketReshardingOp`: v1; `ret_err: i32`; dump `ret_err`. Pin
  `{0}` = `01010400000000000000`; `{-2300}` content `04f7ffff`.
- `GetBucketReshardingRet`: v1; `new_instance: BucketInstanceEntry`; no
  reference dump exists (declared, never defined); serialize as
  `{"new_instance": ...}`. Pin `{NOT_RESHARDING}` =
  `01010f0000000301090000000000000000ffffffff`.
- `Suggestion`/`encode_suggestions`: `[0x75 | 0x80 if log]` then the
  entry's bytes, concatenated. Pin one update suggestion of a default
  entry: `75` + plan 6's default `DirEntry` bytes, and `f5` with `log`.

Registered types' dencoder arms use the C++ names; the corpus table gets
the twelve names listed under Files.

Commit:

```
cls: the rgw bucket-index request and reply structs

rgw_cls_obj_prepare_op and rgw_cls_obj_complete_op carry a one-byte op
and put the full key late on the wire, the complete op writing the
epoch twice; rgw_cls_list_op encodes num_entries ahead of the start
key; rgw_cls_list_ret dumps is_truncated as an int and hides its
marker. Every shape is what Ceph v19.2.2's dencoder writes, including
the prepare op's dump of log_op, bilog_flags and zones_trace.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: `rgw::index`: the client

**Files:**
- Modify: `rados-cls/src/rgw/index.rs`.

**Interfaces:**
- `pub const CLASS: &str = "rgw"` (in `rgw/mod.rs`, shared by every rgw
  method module), `pub const ERR_BUSY_RESHARDING: i32 = 2300`.
- Op constructors, `-> Result<OSDOp>`: `init_index_op()` (raw, empty);
  `set_tag_timeout_op(u64)`; `prepare_op(&PrepareOp)`;
  `complete_op(&CompleteOp)`; `list_op(&ListOp)`; `dir_header_op()`
  (`ListOp` with `num_entries` 0); `check_index_op()` (raw, empty);
  `rebuild_index_op()` (raw, empty); `update_stats_op(bool,
  &BTreeMap<ObjCategory, CategoryStats>)`; `suggest_changes_op(&[Suggestion])`
  (raw with the concatenation); `remove_obj_op(&[String])`;
  `store_pg_ver_op(&str)`; `check_attrs_prefix_op(&str, bool)`;
  `check_mtime_op(UTime, CheckMtimeType, bool)`;
  `set_bucket_resharding_op(ReshardStatus)`;
  `clear_bucket_resharding_op()`; `guard_bucket_resharding_op(i32)` and
  `guard_op()` = `guard_bucket_resharding_op(-ERR_BUSY_RESHARDING)`;
  `get_bucket_resharding_op()`.
- Decoders: `decode_list(&OpReply) -> Result<ListRet>`,
  `decode_dir_header(&OpReply) -> Result<DirHeader>` (a `ListRet`'s
  `dir.header`), `decode_check_index(&OpReply) -> Result<CheckIndexRet>`,
  `decode_get_bucket_resharding(&OpReply) -> Result<BucketInstanceEntry>`.
- Async over `(&IoCtx, &str, ...)`: `init_index`, `set_tag_timeout`,
  `prepare`, `complete`, `list` (see below), `dir_header`, `check_index`,
  `rebuild_index`, `update_stats`, `suggest_changes`, `remove_obj`,
  `store_pg_ver`, `check_attrs_prefix`, `check_mtime`,
  `set_bucket_resharding`, `clear_bucket_resharding`,
  `guard_bucket_resharding`, `get_bucket_resharding`. The index writes
  (`prepare`, `complete`, `suggest_changes`, `rebuild_index`,
  `update_stats`, `set_tag_timeout`) do NOT prepend the guard themselves:
  RGW decides per call site, and the constructors are the compound
  building blocks; the module doc says so and Task 4 shows the guarded
  shape.
- `list(ioctx, oid, &ListOp) -> Result<ListRet>`: sends the op through
  `execute_op_unchecked`; a result of 0 decodes the reply; a result of
  `-27` (`EFBIG`) decodes the reply, replaces `start_obj` with its
  `marker` and sends again, at most 64 times before returning
  `OSDClientError::OSDError { code: -27, .. }`; any other negative result
  is that error. Document that RGW loops the same way.

Docs state the server facts from `cls_rgw.cc` (v19): a second init is
`EINVAL` (RGW pairs init with `create(true)` for `EEXIST`); a prepare with
an empty tag is `EINVAL` and does not bump the header; a complete whose
tag is not pending is `EINVAL`; the accounting and cancel rules of Review
Focus 4; `bucket_list` skips entries flagged `FLAG_VER_MARKER`, without
`list_versions` skips non-visible entries and any entry named like
`start_obj`, collapses names under `delimiter` into one entry flagged
`FLAG_COMMON_PREFIX`, and never returns the `0x80` namespace; suggestions
apply only once the entry has no pending tags younger than the tag
timeout (header's, else the OSD's `rgw_pending_bucket_index_op_expiration`,
else 120 s) and only if `index_ver` is not older than the stored entry's;
`rebuild_index` resets `master_ver`, `max_marker` and the reshard status;
`check_attrs_prefix` is `ECANCELED` when the object's having an xattr with
the prefix equals `fail_if_exist`; `check_mtime` compares `object mtime
<kind> mtime` at second resolution unless `high_precision_time` and is
`ECANCELED` when false, a missing object counting as mtime 0;
`store_pg_ver` writes the PG's current version as a `u64` xattr;
`remove_obj` deletes the object and, if any xattr matched a prefix,
recreates it with only those; the v19 guard trips on any reshard status
other than `NOT_RESHARDING`; `set_bucket_resharding` stores only the
status.

Unit tests: every constructor names `rgw` and its method; `guard_op`
encodes `-2300`; decoders unwrap; `list` is covered by Task 4.

Commit:

```
cls: the rgw bucket-index client

Mirrors cls_rgw_client.h's bucket-index calls as op constructors and
IoCtx functions. bucket_list answers EFBIG together with a reply whose
marker says where to resume, so list loops on it as RGW does; index
writes take no guard of their own because RGW prepends
guard_bucket_resharding per call site, and v19's class does not guard
itself.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 4: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_index.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_index`).

Helpers: `unique`, `is_osd_error`, a `key(name)`; `meta(size)` with
`category MAIN`, `size`, `accounted_size = size`, `etag "e"`;
`prepare(ioctx, oid, op, tag, name)`; `complete(ioctx, oid, op, tag,
name, epoch, size)` with `ver = EntryVer { pool: 1, epoch }`; `stats(ioctx,
oid) -> CategoryStats` = `dir_header(...).stats[MAIN]` or default;
`names(ret: &ListRet) -> Vec<String>` from `dir.entries` keys in order.
Every test creates its own shard object with `ioctx.create(&oid, true)`
and `init_index`, and removes it at the end.

Tests (all `#[ignore]`):

1. `index_init_and_header`: `init_index` ok; a second `init_index` is
   `EINVAL`; `dir_header` has `ver == 1`, empty stats, `tag_timeout == 0`;
   `set_tag_timeout(120)` then `dir_header` has `tag_timeout == 120` and
   `ver == 2`.
2. `index_prepare_then_complete_accounts`: ten names; after each
   `prepare(ADD)` the stats are unchanged (`num_entries == 0`); after each
   `complete(ADD, epoch i + 1, size 1024)` `num_entries == i + 1`,
   `total_size == 1024 * (i + 1)`, `total_size_rounded == 4096 * (i + 1)`;
   `list` returns the ten names in order, not truncated; a complete with a
   tag that was never prepared is `EINVAL`.
3. `index_stale_epoch_is_a_cancel`: ten prepares with tags `t0..t9` on
   one name; completes in the order epoch 10, 9, ..., 1 with sizes
   `1024 * epoch`: after all, `num_entries == 1` and `total_size == 10240`
   (only the first, highest epoch counted); `list` shows one entry whose
   `ver.epoch == 10`.
4. `index_delete_with_a_pending_tag_keeps_the_key`: add `x` (prepare +
   complete, epoch 1); `prepare(DEL, "d")` and `prepare(ADD, "a")` on `x`;
   `complete(DEL, "d", epoch 2)`: stats `num_entries == 0`, and `list`
   with `list_versions` still returns `x` with `exists == false` (the
   pending `a` keeps the key; plain `list` hides it); `complete(ADD, "a",
   epoch 3, 2048)`: `num_entries == 1`, `total_size == 2048`, `x` visible.
5. `index_list_pages_and_delimits`: add `a/1`, `a/2`, `b`, `c/1`, `d`;
   `list(num_entries 2)` returns `[a/1, a/2]` truncated with a non-empty
   marker; `list` from that marker returns the remaining three, not
   truncated; `list` with `delimiter "/"` returns `[a/, b, c/, d]` where
   `a/` and `c/` carry `FLAG_COMMON_PREFIX` (`is_common_prefix()`) and the
   others do not; `list` with `filter_prefix "a/"` and no delimiter
   returns `[a/1, a/2]`; `start_obj = key("b")` returns `[c/1, d]` (the
   start is exclusive and its own name is skipped).
6. `index_list_skips_the_special_namespace`: `omap_set` two raw keys
   `"\u{80}1000_junk"` and `"\u{80}0_junk"` with any value beside two real
   entries; `list` returns only the real names; `dir_header` still works.
7. `index_list_advances_past_invalid_entries`: `omap_set` 9000 entries
   `inv-00000..inv-08999` whose value is an encoded `DirEntry` with
   `flags = FLAG_VER_MARKER`, `exists = true`, `meta(1)`, plus one real
   entry `z` added through prepare/complete; `list(num_entries 1)` returns
   `[z]`. This is the `EFBIG` path (the class gives up after eight
   attempts with nothing to return and asks the client to advance); if
   the async `list` fails with -27 instead, the OSD does not ship the
   reply with the error and the loop must page by the last key seen:
   report it rather than weakening the test.
8. `index_suggest_after_the_tag_expires`: `set_tag_timeout(1)`;
   `prepare(ADD, "p", "s")` without completing; a `Suggestion { Update,
   entry: DirEntry { key s, exists true, meta(512), .. } }` sent at once
   is ignored (`num_entries == 0`); after `sleep(2 s)` the same suggestion
   is applied (`num_entries == 1`, `total_size == 512`) and `list` shows
   `s`; a `Remove` suggestion for `s` (same entry) then removes it
   (`num_entries == 0`); repeating the remove is a no-op success.
9. `index_check_and_rebuild`: add two objects; `update_stats(absolute
   true, {MAIN: {1, 4096, 1, 1}})` corrupts the header; `check_index`
   returns `existing_header.stats[MAIN] != calculated_header.stats[MAIN]`
   with the calculated one holding the real sums; `rebuild_index` then
   makes `dir_header` match the calculated stats and bumps `ver`.
10. `index_resharding_guard`: `get_bucket_resharding` is `NOT_RESHARDING`;
    a compound `guard_op() + prepare_op(..)` succeeds; `set_bucket_resharding(IN_PROGRESS)`;
    `get_bucket_resharding` is `IN_PROGRESS`; the same compound fails with
    `OSDError { code: -2300 }` and `dir_header` shows no pending entry
    (stats unchanged, `list` with `list_versions` empty);
    `guard_bucket_resharding(-2300)` alone fails the same way;
    `clear_bucket_resharding`; the compound succeeds again.
11. `head_object_helpers`: `write_full` an object and `setxattr`
    `user.rgw.a` and `user.other`; `check_attrs_prefix("user.rgw.", true)`
    is `ECANCELED`, `("user.rgw.", false)` ok, `("user.none.", true)` ok,
    `("user.none.", false)` `ECANCELED`, `("", ..)` `EINVAL`; with `m =
    stat(...).mtime`: `check_mtime(m, EQ, false)` ok, `(m, GT, false)`
    `ECANCELED`, `(m - 1 s, GT, false)` ok, `(m + 1 s, LT, false)` ok;
    `store_pg_ver("user.pgver")` then `getxattr` decodes as a `u64` `> 0`;
    `remove_obj(["user.rgw."])`: the object still exists, `getxattrs`
    holds `user.rgw.a` only; `remove_obj([])`: the object is gone;
    `remove_obj([])` again is `ENOENT`.

Commit:

```
cls: rgw bucket-index cluster tests

Pins against Ceph v19: a second init is EINVAL; prepare leaves the
stats alone and complete accounts them; a stale epoch is a cancel; a
delete under another pending tag keeps the key; listing pages by
marker, collapses under a delimiter, skips the start name and the
0x80 namespace, and advances past invalid entries; a suggestion waits
for the tag timeout; rebuild restores the calculated stats; the guard
fails a compound write with -2300 while resharding; and the four
head-object helpers behave as cls_rgw.cc says.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: per-commit proof; container fmt/clippy including each
class alone; corpus over the twelve registered names; the cluster suites
`cls_version_refcount`, `cls_user`, `cls_queue_gc`, `cls_rgw_index`; the
PR body:

```
**Motivation.** Every object write RGW makes goes through the bucket index's prepare/complete protocol, every listing through `bucket_list`, and every index write behind the resharding guard; a Rust RGW needs these methods of the `rgw` class before it can store a single object.

**What changed.** `rados-cls`'s `rgw::index` gains the bucket-index requests and replies (pinned against `ceph-dencoder` v19.2.2 and the corpus), op constructors and `IoCtx` functions for init, tag timeout, prepare, complete, list and header read, check and rebuild, suggest, update stats, the four resharding methods and the four head-object helpers; `rados` gains `execute_op_unchecked` for `bucket_list`'s `EFBIG`-with-reply; eleven cluster tests pin the class's accounting, listing and guard semantics on v19.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-gc` (`rgw::gc`: the omap-era set entry, defer, list, remove);
`cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh` (link, unlink instance, read
and trim the OLH log, clear; on v19 `olh_epoch = 0` means the server
increments and nothing is returned, unlike `main`); `cls-lock` if the
owner adds it; `watch-notify`. Deferred: `bucket_init_index2`,
`bucket_refresh_instance`, `bi_put_entries`, `reshard_log_trim` and
`dec_stats` (post-Squid); the bilog and `bi_*` methods (spec non-goals);
`main`'s in-class guard.
