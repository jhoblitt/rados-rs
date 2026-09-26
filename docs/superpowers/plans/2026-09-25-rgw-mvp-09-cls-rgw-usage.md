# rados-rs RGW MVP, plan 9 of N: `cls-rgw-usage`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `rgw` object class's usage-log methods
(`user_usage_log_add`, `user_usage_log_read`, `user_usage_log_trim`,
`usage_log_clear`): their four request and reply structs pinned against
`ceph-dencoder` v19.2.2 and the corpus, op constructors and `IoCtx`
functions in `rgw::usage`, and cluster tests that pin aggregation,
paging, the bucket filter, trim and clear on Ceph v19.2.2.

**Architecture:** `rgw::usage` (plan 6's value types) gains the ops and
client behind the `rgw` feature. The usage log is a per-shard omap of
`rgw_usage_log_entry` values keyed twice (by time and by user); the
class aggregates on add, walks a range on read with an opaque `iter`,
and trims a range a thousand keys at a time until `ENODATA`.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-usage`: add, read, trim, clear".

## Global Constraints

Plans 3 to 8's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-usage` is based on the fork's `main` after plan 8
  merges; `rgw::usage` holds `UsageData`, `S3selectUsageData`,
  `UsageLogEntry`, `UsageLogInfo`, `UserBucket`; `rgw::CLASS` is `"rgw"`.
- Method names (`cls_rgw_const.h`): `user_usage_log_add` (RD|WR),
  `user_usage_log_read` (RD), `user_usage_log_trim` (RD|WR),
  `usage_log_clear` (WR). No writing method replies.
- Wire and dump are identical on v19 and `main` for all four structs
  (verified by diff). The oracle instances: `rgw_cls_usage_log_add_op`
  default = `02010e0000000101040000000000000000000000`
  (`{"info":{"entries":[]},"user":""}`); `rgw_cls_usage_log_read_op`
  `{1, 2, "owner", "bucket", "iter", 100}` =
  `02012f00000001000000000000000200000000000000050000006f776e6572040000006974657264000000060000006275636b6574`
  (wire: start, end, owner, iter, max_entries, bucket; dump: start_epoch,
  end_epoch, owner, bucket, iter, max_entries);
  `rgw_cls_usage_log_read_ret` `{[], true, "123"}` =
  `01010c000000000000000103000000313233` (dump: `truncated` as a bool,
  `next_iter`, `usage` as `[{"key": {user, bucket}, "val": entry}]`);
  `rgw_cls_usage_log_trim_op` `{1, 2, "user", "bucket"}` =
  `030222000000010000000000000002000000000000000400000075736572060000006275636b6574`
  (version 3, compat 2; `bucket` decoded only from v3).
- Server facts (`cls_rgw.cc`, v19; `main`'s `~`-prefixed keys for users
  starting with `0` and its payer-based trim are post-Squid and not
  mirrored): an add masks a read error as `EINVAL`, aggregates an
  existing entry under the same by-time key (`"%011llu_<user>_<bucket>"`,
  `<user>` being the payer when set, else the owner) and writes the
  by-user key too; a read with an empty `owner` walks by time from
  `start_epoch` (or `iter`) and stops at `end_epoch`, with an owner it
  walks that user's keys; `max_entries` (zero means 1000) counts omap
  keys scanned, not rows returned; entries below `start_epoch` and
  buckets other than `bucket` (when set) are skipped; the reply's
  `usage` merges every hour of a (payer-or-owner, bucket) pair into one
  value; `next_iter` is the last key scanned and is set only when
  `truncated`, so resumption is exclusive; a trim stats the object
  first (`ENOENT` when missing), scans up to a thousand omap keys of the
  range and removes both keys of each entry found (v19 derives the keys
  to remove from the owner, so a requester-pays record stored under a
  payer is found but never removed), and returns `ENODATA` only when the
  scan found no entry and was not truncated; two v19 states therefore
  answer 0 forever without progress (a payer-keyed entry in range; more
  than a thousand keys in range all skipped by `bucket`, since the trim
  carries no iter), which is why the client bounds its loop where
  `cls_rgw_client.cc` does not; a clear empties the omap
  and treats a missing object as success; a read on a missing object is
  `ENOENT` (RGW moves to the next shard).
- `rgw_cls_usage_log_add_op.user` is never set by the C++ client and
  never read by the class; it is encoded as an empty string.

## Review Focus

1. `rgw_cls_usage_log_read_op` declares `bucket` fourth but encodes it
   last, and dumps it fourth. Pinned against the oracle.
2. Aggregation on add and merging on read: two adds of the same
   (user, bucket, epoch) read back as their sum. Pinned in Task 2.
3. `iter` paging: `max_entries` counts keys, `next_iter` is the last key
   scanned, the walk resumes after it; a page can be shorter than
   `max_entries` when keys are skipped. Pinned in Task 2.
4. `trim` loops until `ENODATA`, bounded, and is `ENOENT` on a missing
   shard; `clear` on a missing shard succeeds; a trim by time over a
   payer-keyed entry gives up. Pinned in Task 2.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `cls-rgw-usage` off the fork's `main` after
plan 8 merges; the cluster up.

---

### Task 1: The structs and the client in `rgw::usage`

**Files:**
- Modify: `rados-cls/src/rgw/usage.rs`, dencoder (four arms), corpus
  table (four `TypeSpec`s: `rgw_cls_usage_log_add_op`,
  `rgw_cls_usage_log_read_op`, `rgw_cls_usage_log_read_ret`,
  `rgw_cls_usage_log_trim_op`; all have directories in both archives).

**Interfaces:**
- Types: `AddOp { info: UsageLogInfo, user: String }` (v2/1, derived;
  dump `info`, `user`); `ReadOp { start_epoch: u64, end_epoch: u64,
  owner, bucket, iter, max_entries: u32 }` (hand-written encode in the
  wire order, `MAX_DECODE_VERSION` 2, `bucket` read only from v2; custom
  `Serialize` in dump order); `ReadRet { usage: BTreeMap<UserBucket,
  UsageLogEntry>, truncated: bool, next_iter: String }` (v1; `Serialize`
  puts `truncated`, `next_iter`, then `usage` via `dump::map_entries`);
  `TrimOp { start_epoch, end_epoch, user, bucket }` (encode v3 compat 2;
  hand-written decode reading `bucket` from v3).
- Op constructors: `add_op(&UsageLogInfo)`, `read_op(user: &str, bucket:
  &str, start_epoch: u64, end_epoch: u64, max_entries: u32, iter: &str)`,
  `decode_read(&OpReply) -> Result<ReadRet>`, `trim_op(user, bucket,
  start_epoch, end_epoch)`, `clear_op()` (raw, empty).
- Async: `add`, `read -> Result<ReadRet>`, `trim` (sends the op until the OSD answers `ENODATA`, then `Ok(())`,
  at most `MAX_TRIM_ROUNDS` = 1000 times, then `OSDClientError::Other`
  naming the count; any other error returns),
  `clear`.

Unit tests pin the four oracle vectors and JSON above, plus a
`ReadRet` with one entry (`{"usage":[{"key":{"user":"u","bucket":"b"},"val":{...}}]}`),
and each constructor's class and method.

Commit:

```
cls: the rgw usage-log methods

Mirrors cls_rgw_client.h's usage_log_add, _read, _trim and _clear.
rgw_cls_usage_log_read_op encodes its bucket last though it declares
it fourth; the trim request is version 3 over compat 2. cls_rgw.cc
aggregates an add into the existing hour, pages a read by omap keys
scanned with an exclusive iter, trims a thousand keys per call until
ENODATA, and clears a missing shard without error.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_usage.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_usage`).

Helpers: `entry(owner, bucket, epoch, bytes_sent, ops)` builds a
`UsageLogEntry` whose `usage_map["get_obj"]` and `total_usage` carry
`bytes_sent`, `bytes_received = bytes_sent * 2`, `ops`, `successful_ops
= ops`; `info(entries)` wraps them; `E = 1_755_892_800` (an hour
boundary, as RGW rounds); `key(user, bucket) -> UserBucket`.

1. `usage_add_aggregates_and_read_merges`: `add` `[entry("u1", "b1", E,
   100, 1)]` twice, then `[entry("u1", "b1", E + 3600, 5, 1)]`; `read("u1",
   "", 0, E + 7200, 100, "")` returns one key `(u1, b1)` with `bytes_sent`
   205, `ops` 3, `usage_map["get_obj"]` the same, not truncated, empty
   `next_iter`; `read("", "", 0, E + 60, 100, "")` (by time) returns the
   first hour only: `bytes_sent` 200; `read("u1", "b2", ..)` is empty;
   `read("u1", "", E + 3600, E + 7200, ..)` returns `bytes_sent` 5.
2. `usage_read_pages_by_iter`: add five entries for `u2` with buckets
   `b0..b4` at `E`; `read("u2", "", 0, E + 60, 2, "")` is truncated with
   a non-empty `next_iter` and two keys; loop passing `next_iter` back
   until not truncated; the union of keys is the five buckets and no
   page repeats a key.
3. `usage_payer_keys_the_entry`: add `entry("u3", "b1", E, 7, 1)` with
   `payer = "p3"`; `read("p3", "", ..)` returns key `(p3, b1)`;
   `read("u3", "", ..)` is empty.
4. `usage_trim_and_clear`: on a fresh object, `trim("u1", "", 0, u64::MAX)`
   is `ENOENT` (the class stats first); `clear` on the same missing
   object succeeds; add the entries of test 1 again; `trim("u1", "", 0, E
   + 60)` then `read("u1", "", 0, E + 7200, ..)` returns only the second
   hour; `trim("", "", 0, u64::MAX)` (by time) empties it; add again,
   `clear`, `read` returns nothing.
5. `usage_trim_gives_up_on_a_payer_keyed_entry`: add an entry with
   `payer = "p4"`; `trim("", "", 0, u64::MAX)` fails with
   `OSDClientError::Other` after `MAX_TRIM_ROUNDS` (the class finds the
   entry each round and removes nothing); `trim("u4", "", 0, u64::MAX)`
   succeeds at once; `read("p4", ..)` still returns the entry.

Commit:

```
cls: rgw usage-log cluster tests

Pins against Ceph v19: adds aggregate into the hour and reads merge
hours per user and bucket, the bucket filter and the half-open epoch
range, paging by an exclusive iter counting keys scanned, entries
keyed by the payer when set, trim by user and by time until ENODATA
with ENOENT on a missing shard, and clear on a missing shard as
success.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: corpus over the four names; cluster suites through
`cls_rgw_usage`; the PR body:

```
**Motivation.** RGW flushes its per-user usage counters into the `rgw` class's usage log and `radosgw-admin usage` reads and trims them there; a Rust RGW that reports usage needs the same four methods.

**What changed.** `rados-cls`'s `rgw::usage` gains the add, read, trim and clear requests and replies (pinned against `ceph-dencoder` v19.2.2 and the corpus), op constructors and `IoCtx` functions, and cluster tests pinning aggregation, paging, the bucket filter, trim and clear on v19.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-lc`; `cls-rgw-olh`; `cls-lock` if the owner adds it;
`watch-notify`. Deferred: `main`'s `~`-prefixed by-user keys and
`rgw_usage_log_key_transition`, `main`'s payer-based trim.
