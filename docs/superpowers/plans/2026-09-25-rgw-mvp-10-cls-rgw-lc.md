# rados-rs RGW MVP, plan 10 of N: `cls-rgw-lc`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `rgw` object class's lifecycle methods (`lc_get_head`,
`lc_put_head`, `lc_get_entry`, `lc_set_entry`, `lc_rm_entry`,
`lc_get_next_entry`, `lc_list_entries`): their ten request and reply
structs (two pinned against `ceph-dencoder` v19.2.2 and the corpus, the
rest by construction), op
constructors and `IoCtx` functions in `rgw::lc`, and cluster tests that
pin the class's head, entry and paging semantics on Ceph v19.2.2, which
upstream does not test at all.

**Architecture:** `rgw::lc` (plan 6's `LcEntry` and `LcObjHead`) gains
the ops and client behind the `rgw` feature. A lifecycle shard is an
omap whose header is the encoded `cls_rgw_lc_obj_head` and whose keys
are the bucket keys `tenant:name:marker` with `cls_rgw_lc_entry` values;
three requests and three replies carry a version-2 entry over a
version-1 `pair<string, int>`, and the list reply echoes the request's
version.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-lc`: head get and put, entry get, set, rm, next, list".

## Global Constraints

Plans 3 to 9's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-lc` is based on the fork's `main` after plan 9 merges;
  `rgw::lc` holds `LcEntry { bucket, start_time: u64, status: u32 }` and
  `LcObjHead { start_date: i64, marker, shard_rollover_date: i64 }`.
- Method names (`cls_rgw_const.h`): `lc_get_head` (RD), `lc_put_head`
  (RD|WR), `lc_get_entry` (RD), `lc_set_entry` (RD|WR), `lc_rm_entry`
  (RD|WR), `lc_get_next_entry` (RD), `lc_list_entries` (RD). No writing
  method replies. Wire, dumps and server logic are identical on v19 and
  `main`.
- Oracle instances: `cls_rgw_lc_get_entry_ret` `{bucket1, 6000, 0}` =
  `02021d000000010117000000070000006275636b657431701700000000000000000000`,
  JSON `{"entry":{"bucket":"bucket1","start_time":6000,"status":0}}`;
  `cls_rgw_lc_set_entry_op` `{foo, 123, 456}` =
  `02021900000001011300000003000000666f6f7b00000000000000c8010000`, JSON
  `{"bucket":"foo","start_time":123,"status":456}` (flattened). Only
  these two are registered with `ceph-dencoder`; `cls_rgw_lc_rm_entry_op`
  has corpus directories but no registration and is left out of the
  corpus table.
- Legacy forms, not decoded (the floor rule of plan 6; every corpus
  sample here is version 2): version 1 of `set_entry_op`, `rm_entry_op`,
  `get_next_entry_ret` and `get_entry_ret` carried a `pair<string, int>`,
  and `list_entries_ret` versions 1 and 2 carried `map<string, int>`. The
  port encodes version 2 (entries) and version 3 (list request and
  reply), floors the entry types at 2 and the list reply at 3, and does
  not model the list request's `compat_v` (it always sends 3 and so
  always receives 3).
- Server facts (`cls_rgw.cc`, v19): the head is the omap header,
  `get_head` on an empty header returns a default head, a decode failure
  is `EINVAL`; `get_entry` is `ENOENT` for an absent key and `EIO` for an
  undecodable value; `set_entry` writes the key (creating the object if
  needed); `rm_entry` removes the key, a missing key is not an error, a
  missing object is `ENOENT`; `get_next_entry` returns the first key
  after `marker` (empty for the first) or a default entry with an empty
  bucket when none follows; `list_entries` returns up to `max_entries`
  keys after `marker` with `is_truncated`, reading legacy pair values
  too; every read on a missing object is `ENOENT` from the OSD.
- RGW's status values (`rgw_lc.h`): `lc_uninitial` 0, `lc_processing` 1,
  `lc_failed` 2, `lc_complete` 3; exported as `u32` constants
  `STATUS_UNINITIAL`, `STATUS_PROCESSING`, `STATUS_FAILED`,
  `STATUS_COMPLETE` with a doc naming the source. The `lc_process`
  exclusive lock RGW takes around processing is `cls_lock`, outside this
  plan.

## Review Focus

1. The entry-carrying requests and replies are version 2 over compat 2;
   the pair-shaped version 1 is below the floor and rejected. Pinned by
   construction in Task 1.
2. `list_entries_ret` floors at 3 (its map forms are rejected) and the
   vector form of version 3; the client sorts entries by bucket as the
   C++ decoder does. Pinned in Task 1.
3. `get_next_entry` after the last key is a default entry, not `ENOENT`;
   `get_entry` of an absent key is `ENOENT`. Pinned in Task 2.
4. The head lives in the omap header: `get_head` on a fresh object is the
   default head and `put_head` round-trips all three fields, including
   the rollover date the dump omits. Pinned in Task 2.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `cls-rgw-lc` off the fork's `main` after plan
9 merges; the cluster up.

---

### Task 1: The structs and the client in `rgw::lc`

**Files:**
- Modify: `rados-cls/src/rgw/lc.rs`, dencoder (two arms), corpus table
  (`cls_rgw_lc_get_entry_ret`, `cls_rgw_lc_set_entry_op`).

**Interfaces:**
- Types: `GetEntryOp { marker }` (v1, derived); `GetEntryRet { entry:
  LcEntry }` (v2/2, derived; dump `{"entry": ...}`); `SetEntryOp { entry }`
  (v2/2, derived; custom `Serialize` flattening `bucket`, `start_time`,
  `status`); `RmEntryOp { entry }` (v2/2); `GetNextEntryOp { marker }`
  (v1); `GetNextEntryRet { entry }` (v2/2); `PutHeadOp { head: LcObjHead
  }` (v1); `GetHeadRet { head }` (v1); `ListEntriesOp { marker,
  max_entries: u32 }` (v3/1, derived); `ListRet { entries: Vec<LcEntry>,
  is_truncated: bool }` (v3/1, derived). The derive's decode rejects a
  `struct_v` below its version, which is the floor here.
- Op constructors: `get_head_op()` (raw, empty), `put_head_op(&LcObjHead)`,
  `get_entry_op(marker: &str)`, `set_entry_op(&LcEntry)`,
  `rm_entry_op(&LcEntry)`, `get_next_entry_op(marker: &str)`,
  `list_op(marker: &str, max_entries: u32)`; decoders `decode_get_head
  -> LcObjHead`, `decode_get_entry -> LcEntry`, `decode_get_next_entry ->
  LcEntry`, `decode_list -> ListRet` with `entries` sorted by `bucket`
  (the C++ `cls_rgw_lc_list_decode` sorts; `is_truncated` is kept, which
  the C++ discards).
- Async: `get_head`, `put_head`, `get_entry`, `set_entry`, `rm_entry`,
  `get_next_entry`, `list`.

Unit tests pin: the two oracle vectors and JSON; the pair form
`010109000000010000006203000000` (that is `01 01 09000000 01000000 62
03000000`) rejected by `SetEntryOp`, `RmEntryOp`, `GetNextEntryRet` and
`GetEntryRet`; a v2 `ListRet` `02010e0000000100000001000000610100000000`
rejected; a v3 `ListRet` round trip of two entries; every constructor's
class and method (`rgwlc_get_head`, ...).

Commit:

```
cls: the rgw lifecycle methods

Mirrors cls_rgw_client.h's lc_get_head, lc_put_head, lc_get_entry,
lc_set_entry, lc_rm_entry, lc_get_next_entry and lc_list_entries. The
entry requests and replies are version 2, the list reply version 3;
their pair and map forms are below the Squid floor. cls_rgw.cc
keeps the head in the omap header, answers a missing next entry with
an empty one, and reads legacy pair values when listing.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_lc.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_lc`).

Helpers: `entry(bucket, status)` with `start_time` 0; keys
`":b1:m1"`, `":b2:m2"`, `":b3:m3"` (an empty tenant gives the leading
colon, as RGW's `get_bucket_lc_key` does).

1. `lc_head_lives_in_the_omap_header`: `get_head` on a fresh object is
   `LcObjHead::default()`; `put_head({10, "m", 20})` then `get_head` is
   `{10, "m", 20}`; `get_head` on an object that does not exist is
   `ENOENT`.
2. `lc_entries_set_get_next_list_rm`: `set_entry` three entries with
   statuses 0, 1, 3; `get_entry(":b2:m2")` is `{":b2:m2", 0, 1}`;
   `get_entry("nope")` is `ENOENT`; `get_next_entry("")` is `b1`;
   `get_next_entry(":b1:m1")` is `b2`; `get_next_entry(":b3:m3")` is the
   default entry (empty bucket); `list("", 2)` is `[b1, b2]` and
   truncated; `list(":b2:m2", 10)` is `[b3]`, not truncated; `set_entry`
   of `b1` with `start_time` 5 and `STATUS_COMPLETE` then `get_entry`
   shows both; `rm_entry(b2)` then `list("", 10)` is `[b1, b3]`;
   `rm_entry(b2)` again succeeds.

Commit:

```
cls: rgw lifecycle cluster tests

Pins against Ceph v19, which has no lc tests of its own: the head in
the omap header with a default when empty, get_entry ENOENT versus
get_next_entry's empty entry past the end, listing after a marker
with truncation, overwrite by set_entry, and idempotent rm_entry.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: corpus over the two names; cluster suites through
`cls_rgw_lc`; the PR body:

```
**Motivation.** RGW's lifecycle worker walks per-shard entries through the `rgw` class's lc methods and records its position in the shard head; a Rust RGW running lifecycle needs all seven.

**What changed.** `rados-cls`'s `rgw::lc` gains the ten lc requests and replies op constructors and `IoCtx` functions, and cluster tests that pin the head, entry and paging semantics upstream leaves untested.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-olh`; `cls-lock` if the owner adds it (the lifecycle worker's
`lc_process` lock needs it); `watch-notify`. Deferred: the list
request's `compat_v` echo (the port always sends version 3).
