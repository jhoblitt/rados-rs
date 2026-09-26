# rados-rs RGW MVP, plan 11 of N: `cls-rgw-olh`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `rgw` object class's object-versioning methods
(`bucket_link_olh`, `bucket_unlink_instance`, `bucket_read_olh_log`,
`bucket_trim_olh_log`, `bucket_clear_olh`): their six request and reply
structs pinned against `ceph-dencoder` v19.2.2 and the corpus, op
constructors and `IoCtx` functions in `rgw::olh`, and cluster tests that
pin v19's OLH epoch and log semantics, which differ from `main`'s.

**Architecture:** `rgw::olh` (plan 6's `OlhEntry`, `OlhLogEntry`,
`OlhLogOp`) gains the ops and client behind the `rgw` feature. The OLH
("object logical head") is the bucket-index entry that names a versioned
object's current instance; linking and unlinking rewrite it and append
to its pending log, which RGW replays against the head object and then
trims.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-olh`: link, unlink instance, read and trim the OLH log, clear".

## Global Constraints

Plans 3 to 10's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-olh` is based on the fork's `main` after plan 10
  merges; `rgw::index` has `prepare_op`/`complete_op`/`list_op` (plan 7),
  `rgw::types` the enums and `ZoneSet`, `rgw::olh` the entry types.
- Method names (`cls_rgw_const.h`): `bucket_link_olh` (RD|WR),
  `bucket_unlink_instance` (RD|WR), `bucket_read_olh_log` (RD),
  `bucket_trim_olh_log` (RD|WR), `bucket_clear_olh` (RD|WR). On v19 no
  writing method replies; `main` returns the committed epoch as a bare
  `u64` from link and unlink (needing `RETURNVEC`), which this plan does
  not model (a v19 OSD sends nothing).
- Squid-versus-`main` facts (verified against `git show v19.2.6:...`):
  `rgw_cls_read_olh_log_op` is version 1 on v19 (`main` adds
  `get_stales` in v2); the port encodes v1 and, being a request, never
  decodes v2 outside the dencoder. On v19 an `olh_epoch` of 0 in link or
  unlink means the class assigns the next epoch (the first OLH gets 2;
  1 is reserved for plain entries converted to versioned), while `main`
  uses nanoseconds since the epoch. A stale epoch (below the OLH's) on
  v19 writes the instance as non-current and returns 0 without touching
  the OLH. `main` guards these writes in the class; v19 does not, so RGW
  prepends `guard_bucket_resharding` (plan 7) to them too.
- Oracle instances (all identical on v19 and `main` except the read
  request): `rgw_cls_link_olh_op` instance 1 `{key name, olh_tag
  "olh_tag", delete_marker, op_tag "op_tag", meta = plan 6's meta
  instance 1, olh_epoch 123, log_op}` (167 B) =
  `0501a100000001010c000000040000006e616d6500000000070000006f6c685f74616701060000006f705f74616707035300000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f7479706500000000000000000000000000000000007b00000000000000010000000000000000000000000000000000000000000000`,
  JSON keys `key`, `olh_tag`, `delete_marker`, `op_tag`, `meta`,
  `olh_epoch`, `log_op`, `bilog_flags`, `unmod_since` (`utime`, `"0.000000"`),
  `high_precision_time`, `zones_trace` (bare array);
  `rgw_cls_unlink_instance_op` instance 1 `{key name, op_tag "op_tag",
  olh_epoch 124, log_op}` (53 B) =
  `03012f00000001010c000000040000006e616d6500000000060000006f705f7461677c000000000000000100000000000000000000`,
  JSON `{"key":{"name":"name","instance":""},"op_tag":"op_tag","olh_epoch":124,"log_op":true,"bilog_flags":0,"zones_trace":[]}`
  (no `olh_tag`); `rgw_cls_read_olh_log_op` instance 1 `{olh name,
  ver_marker 123, olh_tag "olh_tag"}` (43 B, version 1) =
  `01012500000001010c000000040000006e616d65000000007b00000000000000070000006f6c685f746167`;
  `rgw_cls_read_olh_log_ret` instance 1 `{ {1: [plan 6's OLH log entry
  instance 1]}, truncated }` (83 B) =
  `01014d00000001000000010000000000000001000000010136000000d20400000000000001060000006f705f74616701011c000000080000006b65792e6e616d650c0000006b65792e696e7374616e63650101`,
  JSON `{"log":[{"key":1,"val":[{...}]}],"is_truncated":true}`;
  `rgw_cls_trim_olh_log_op` instance 1 `{olh "olh.name", ver 100, olh_tag
  "olh_tag"}` (47 B) =
  `010129000000010110000000080000006f6c682e6e616d65000000006400000000000000070000006f6c685f746167`;
  `rgw_cls_bucket_clear_olh_op` instance 1 `{key "key.name", olh_tag
  "olh_tag"}` (39 B) =
  `010121000000010110000000080000006b65792e6e616d6500000000070000006f6c685f746167`.
- Server facts (`cls_rgw.cc`, v19): `link_olh` is `EINVAL` on a decode
  failure, returns the instance read's error except `ENOENT` when
  `delete_marker` is set, and is `ECANCELED` when the OLH exists with a
  different tag and is not pending removal; with `unmod_since` set and an
  existing instance whose mtime is not older, it returns 0 without
  linking (second resolution unless `high_precision_time`); a newer epoch (or an equal one whose instance does not sort
  after the current one, `olh.key.instance >= op.key.instance`)
  promotes the instance to current, demotes the previous one, and appends `LINK_OLH` (plus
  `REMOVE_INSTANCE` when replacing) to the OLH log keyed by the OLH
  epoch; `unlink_instance` is `ENOENT` for a missing instance, converts
  a plain entry to versioned when there is no OLH, promotes the next
  version when the unlinked one was current, and on the last version
  logs `UNLINK_OLH` and marks the OLH `pending_removal`; `read_olh_log`
  is `EINVAL` when the key has an instance, `ECANCELED` on a tag
  mismatch (a missing OLH has tag `""`), returns the whole log when it
  fits in 1000 epochs above `ver_marker`, else 1000 with `is_truncated`;
  reading an empty log is undefined behaviour in the class (do not do
  it); `trim_olh_log` erases every epoch at or below `ver` under the same
  tag rules; `clear_olh` is `EINVAL` for a key with an instance,
  `ECANCELED` on a tag mismatch, removes the OLH and, only when it is a
  version marker, the plain entry.

## Review Focus

1. `rgw_cls_link_olh_op` (5/1) always writes a `u64` seconds copy of
   `unmod_since` before the `real_time`; the decoder floors at 5 and
   reads both, dropping the seconds. Pinned against the oracle.
2. `rgw_cls_unlink_instance_op` (3/1) dumps no `olh_tag`; the read
   request is version 1 on v19. Pinned.
3. The v19 epoch rule: 0 means class-assigned, first 2, then increasing;
   an explicit epoch below the OLH's leaves the OLH alone. Pinned in Task
   2.
4. Link, unlink, tag mismatch and clear observed through `list` with
   `list_versions`: current flags, demotion, promotion, delete markers.
   Pinned in Task 2.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `cls-rgw-olh` off the fork's `main` after plan
10 merges; the cluster up.

---

### Task 1: The structs and the client in `rgw::olh`

**Files:**
- Modify: `rados-cls/src/rgw/olh.rs`, dencoder (six arms), corpus table
  (six: `rgw_cls_link_olh_op`, `rgw_cls_unlink_instance_op`,
  `rgw_cls_read_olh_log_op`, `rgw_cls_read_olh_log_ret`,
  `rgw_cls_trim_olh_log_op`, `rgw_cls_bucket_clear_olh_op`).

**Interfaces:**
- Types: `LinkOlhOp { key: ObjKey, olh_tag: String, delete_marker: bool,
  op_tag: String, meta: DirEntryMeta, olh_epoch: u64, log_op: bool,
  bilog_flags: u16, unmod_since: UTime, high_precision_time: bool,
  zones_trace: ZoneSet }` (hand-written: encode v5 compat 1 in the wire
  order with the `u64` seconds before `unmod_since`; decode floors at 5
  with `check_min_version!` and reads the wire order, dropping the
  seconds; `Serialize` in dump order with `unmod_since` via
  `dump::utime`, `bilog_flags` as a number, `zones_trace` bare);
  `UnlinkInstanceOp { key, op_tag, olh_epoch, log_op, bilog_flags,
  olh_tag, zones_trace }` (hand-written: encode v3/1; floor 3; `Serialize`
  omits `olh_tag`); `ReadOlhLogOp { olh: ObjKey, ver_marker: u64, olh_tag }` (v1,
  hand-written: it encodes version 1 and decodes `main`'s version 2 by
  skipping the `get_stales` tail, which a derive capped at its own
  version cannot); `ReadOlhLogRet {
  log: BTreeMap<u64, Vec<OlhLogEntry>>, is_truncated: bool }` (v1;
  `Serialize`: `log` via `dump::map_entries`, then `is_truncated`);
  `TrimOlhLogOp { olh, ver: u64, olh_tag }` (v1); `ClearOlhOp { key,
  olh_tag }` (v1).
- Op constructors: `link_olh_op(&LinkOlhOp)`,
  `unlink_instance_op(&UnlinkInstanceOp)`, `read_olh_log_op(olh: &ObjKey,
  ver_marker: u64, olh_tag: &str)`, `decode_read_olh_log(&OpReply) ->
  Result<ReadOlhLogRet>`, `trim_olh_log_op(olh: &ObjKey, ver: u64,
  olh_tag: &str)`, `clear_olh_op(key: &ObjKey, olh_tag: &str)`.
- Async: `link_olh`, `unlink_instance`, `read_olh_log ->
  Result<ReadOlhLogRet>`, `trim_olh_log`, `clear_olh`. The writes take
  no guard themselves (plan 7's rule).

Unit tests pin the six oracle vectors and JSON, a version-2
`LinkOlhOp` vector rejected (below the floor), and each constructor's
class and method.

Commit:

```
cls: the rgw OLH methods

Mirrors cls_rgw_client.h's link_olh, unlink_instance, get_olh_log,
trim_olh_log and clear_olh. rgw_cls_link_olh_op still writes the
seconds of unmod_since ahead of the real_time it superseded; the read
request is version 1 on v19, where get_stales does not exist. On v19
an olh_epoch of 0 asks the class for the next epoch and nothing is
returned, unlike main.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_olh.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_olh`).

Helpers: a shard per test (`create` + `index::init_index`);
`put_version(oid, name, instance, tag, epoch, size)` runs
`index::prepare(ADD, tag, key{name, instance})`, `index::complete(ADD,
tag, key, EntryVer{1, epoch}, meta(size))`, then
`link_olh(LinkOlhOp{key, olh_tag: format!("olh-{name}"), op_tag: tag,
meta(size), olh_epoch: epoch, ..})`; `versions(oid, name) ->
Vec<DirEntry>` = `list` with `list_versions` filtered to `key.name ==
name`; `current(entries) -> Option<&str>` the instance of the entry with
`is_current()`.

1. `olh_link_promotes_and_logs`: `put_version("o", "v1", "t1", 10, 100)`;
   `versions` has one entry `v1`, `is_current`, `flags & FLAG_VER != 0`;
   a plain `list` (no versions) shows `o` with instance `v1`;
   `put_version("o", "v2", "t2", 20, 200)`: `v2` current, `v1` not;
   `read_olh_log(key("o"), 0, "olh-o")` has epochs 10 and 20, each with a
   `LINK_OLH` entry naming its instance, not truncated;
   `read_olh_log(key("o"), 10, "olh-o")` has epoch 20 only;
   `read_olh_log(key("o"), 0, "wrong")` is `ECANCELED`;
   `link_olh` of `v3` with `olh_tag "wrong"` is `ECANCELED`;
   `trim_olh_log(key("o"), 10, "olh-o")` then the full read has epoch 20
   only.
2. `olh_stale_epoch_leaves_the_head`: after `v1` at 10 and `v2` at 20,
   `put_version("o", "v0", "t0", 15, 50)` succeeds, `v2` stays current and
   `v0` is listed non-current.
3. `olh_unlink_promotes_the_previous_version`: `v1` at 10, `v2` at 20;
   `unlink_instance(UnlinkInstanceOp{key{o, v2}, op_tag "u1", olh_epoch
   30, olh_tag "olh-o", ..})`: `versions` has `v1` current and no `v2`;
   `unlink_instance` of `{o, "missing"}` is `ENOENT`; unlinking `v1`
   (epoch 40) leaves no visible `o` in a plain `list` and the OLH log's
   last epoch holds an `UNLINK_OLH` entry.
4. `olh_delete_marker_and_clear`: `v1` at 10; `link_olh` of `{o, "dm1"}`
   with `delete_marker`, default meta, epoch 20, op_tag "d1": the plain
   `list` shows no `o`, `versions` shows `dm1` current with
   `is_delete_marker()` and `v1` not current; `clear_olh(key("o"),
   "wrong")` is `ECANCELED`; `clear_olh(key("o"), "olh-o")` succeeds;
   `clear_olh` again with tag `""` succeeds (a missing OLH has the empty
   tag).
5. `olh_epoch_zero_is_assigned_by_the_class`: `put_version("p", "a",
   "ta", 0, 1)` then `("p", "b", "tb", 0, 1)`; `read_olh_log(key("p"), 0,
   "olh-p")` has epochs 2 and 3 exactly; `b` is current.

Commit:

```
cls: rgw OLH cluster tests

Pins against Ceph v19: linking promotes the newer epoch and demotes
the old one, a stale epoch leaves the head alone, unlinking promotes
the previous version and logs UNLINK_OLH on the last, a delete marker
hides the object from plain listings, tag mismatches are ECANCELED,
clear_olh removes the head, and an olh_epoch of 0 is assigned by the
class starting at 2.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: corpus over the six names; cluster suites through
`cls_rgw_olh`; the PR body:

```
**Motivation.** Object versioning in RGW is the bucket index's OLH: every versioned PUT, DELETE and version listing goes through link, unlink, and the OLH log; a Rust RGW with versioning needs these five methods of the `rgw` class.

**What changed.** `rados-cls`'s `rgw::olh` gains the six OLH requests and replies (pinned against `ceph-dencoder` v19.2.2 and the corpus, the read request at the version v19 speaks), op constructors and `IoCtx` functions, and cluster tests that pin v19's epoch assignment, promotion, demotion, delete markers, tag checks and log trimming.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-lock` if the owner adds it; `watch-notify`. Deferred: `main`'s
`get_stales`, `bucket_refresh_instance`, the `u64` epoch reply of `main`'s
link and unlink (`RETURNVEC`), `OLHLogOp::STALE`.

## Patched after execution (2026-09-25)

- The equal-epoch promotion rule follows `cls_rgw.cc` (`>=` on the instance, not `>`); `ReadOlhLogOp` is hand-written so it can decode main's version 2; `index.rs`'s `DumpUtime` and `ZonesTraceBare` became `pub(super)` for `olh.rs`; the wrong-tag link in cluster test 1 uses epoch 30.
