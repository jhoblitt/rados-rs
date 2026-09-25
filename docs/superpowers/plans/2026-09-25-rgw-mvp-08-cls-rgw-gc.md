# rados-rs RGW MVP, plan 8 of N: `cls-rgw-gc`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `rgw` object class's omap-era garbage-collection
methods (`gc_set_entry`, `gc_defer_entry`, `gc_list`, `gc_remove`) as op
constructors and `IoCtx` functions in `rgw::gc`, whose request and reply
structs plan 5 already pinned, with cluster tests that pin the class's
time-index semantics on Ceph v19.2.2.

**Architecture:** One task of client code in the existing `rgw/gc.rs`
(feature `rgw`, whose first class calls landed in plan 7) and one of
cluster tests. No new types, no dencoder change. RGW reaches these
methods only as the fallback of the queue-based `rgw_gc` path (plan 5)
when a shard has not transitioned; the docs say how RGW composes the
two with the `version` class, but the composition itself is the
driver's.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-gc`: set entry, defer, list, remove".

## Global Constraints

Plans 3 to 7's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-gc` is based on the fork's `main` after plan 7 merges;
  `rgw::CLASS` is `"rgw"` and `feature = "rgw"` is in the `mod call` gate.
- Method names (`cls_rgw_const.h`): `gc_set_entry` (RD|WR),
  `gc_defer_entry` (RD|WR), `gc_list` (RD), `gc_remove` (RD|WR). No
  writing method replies.
- Server facts (`cls_rgw.cc`, identical on v19 and `main`): a GC shard
  keeps each chain twice in its omap, under the name key `"0_" + tag`
  and the time key `"1_" + "%011llu.%09u"` of its expiry (seconds and
  nanoseconds); the class sets the entry's `time` to the OSD's now plus
  `expiration_secs`, ignoring the client's `time`; `set_entry` upserts
  (an existing tag's time key moves); `defer_entry` re-keys an existing
  tag to now plus `expiration_secs` and is `ENOENT` for an absent tag;
  `list` takes the marker verbatim as an omap key (empty means the start
  of the time index), pages `max` entries (zero means 128) in expiry
  order, stops at the first key at or past now when `expired_only`, sets
  `truncated` and, only then, `next_marker` (the last key returned);
  `remove` deletes both keys of each tag and is a success for an absent
  tag; a decode failure of a stored entry is `EIO`.
- Which RGW path uses them, for the docs: `RGWGC::send_chain` enqueues
  through the queue class behind an `obj_version` check of 1 and falls
  back to `gc_set_entry` (no version check) when that fails with
  `ECANCELED` or `EPERM`; `RGWGC::list` reads a shard's omap entries and
  its `obj_version` until the shard proves transitioned; `gc_remove`
  retires processed tags on untransitioned shards; `gc_defer_entry` has
  no caller in v19 (`RGWRados::defer_gc` is disabled) and is deleted in
  `main`.

## Review Focus

1. The `list` marker is the omap key itself (`"1_..."`), not a tag, and
   `next_marker` is empty unless `truncated`. Pinned in Task 2.
2. `defer_entry` on an absent tag is `ENOENT`; `remove` of an absent tag
   is not an error. Pinned in Task 2.
3. Expiry is server-side: an entry set with `expiration_secs` 0 lists
   at once under `expired_only`; deferred by five seconds it disappears
   from the expired listing and returns after the wait. Pinned in Task 2.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `cls-rgw-gc` off the fork's `main` after plan
7 merges; the cluster up.

---

### Task 1: The client in `rgw::gc`

**Files:**
- Modify: `rados-cls/src/rgw/gc.rs`.

**Interfaces:**
- Op constructors, `-> Result<OSDOp>`: `set_entry_op(expiration_secs:
  u32, info: &GcObjInfo)` (encodes `SetEntryOp`), `defer_entry_op(expiration_secs:
  u32, tag: &str)` (`DeferEntryOp`), `list_op(marker: &str, max: u32,
  expired_only: bool)` (`ListOp`), `remove_op(tags: &[String])`
  (`RemoveOp`); `decode_list(&OpReply) -> Result<ListRet>`.
- Async over `(&IoCtx, &str, ...)`: `set_entry`, `defer_entry`, `list ->
  Result<ListRet>`, `remove`.
- The module doc gains the server facts above and the RGW composition
  note (queue first behind `version::check_op(objv 1, Eq)`, omap
  fallback on `ECANCELED`/`EPERM`).

Unit tests: each constructor names `rgw` and its method
(`rgwgc_set_entry`, `rgwgc_defer_entry`, `rgwgc_list`, `rgwgc_remove` as
the `indata` prefix); `list_op("", 0, true)` encodes plan 5's default
`ListOp` bytes with `max` 0; `decode_list` unwraps a `ListRet`.

Commit:

```
cls: the rgw class's omap-era GC methods

Mirrors cls_rgw_client.h's gc_set_entry, gc_defer_entry, gc_list and
gc_remove. cls_rgw.cc keeps each chain under a name key and a time key
and sets the expiry itself from the OSD clock; gc_list walks the time
index from a marker that is the omap key, and gc_remove is idempotent.
RGW reaches these only when a GC shard has not moved to the rgw_gc
queue.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 2: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_rgw_gc.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_gc`).

Helpers as plan 5's `chain(i)` (two objects per chain). Each test creates
its own object with `ioctx.create(&oid, true)`.

1. `rgw_gc_set_lists_in_expiry_order`: ten chains set with
   `expiration_secs` 0 in tag order; `list("", 8, true)` returns eight
   entries, `truncated`, non-empty `next_marker` starting with `"1_"`;
   `list(next_marker, 8, true)` returns the remaining two, not truncated,
   empty `next_marker`; `list("", 10, true)` returns all ten in tag order
   and is not truncated; `list("", 0, true)` (the 128 default) likewise.
2. `rgw_gc_set_upserts`: set `chain-0` with expiry 0, then again with
   expiry 300; `list("", 10, true)` is empty; `list("", 10, false)` has
   one entry whose `chain` is the second one's.
3. `rgw_gc_defer_and_remove`: set `chain-0` with expiry 0; `list(.., true)`
   has one entry; `defer_entry(5, "chain-0")`: the expired listing is
   empty and not truncated, the full listing has one; after `sleep(6 s)`
   the expired listing has one; `defer_entry(5, "missing")` is `ENOENT`;
   `remove(["chain-0"])`: both listings empty; `remove(["chain-0"])`
   again succeeds.

Commit:

```
cls: rgw omap GC cluster tests

Pins against Ceph v19: the time index lists in expiry order and pages
by an omap-key marker, an entry set twice keeps one time key, defer
moves it into the future and ENOENT for an unknown tag, and remove is
idempotent.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 3: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6 with no corpus change; cluster suites through
`cls_rgw_gc`; the PR body:

```
**Motivation.** A GC shard that has not moved to the `rgw_gc` queue is still read and retired through the `rgw` class's omap GC methods, and `RGWGC::send_chain` falls back to them when the queue refuses; a Rust RGW needs both paths to coexist with radosgw.

**What changed.** `rados-cls`'s `rgw::gc` gains op constructors and `IoCtx` functions for `gc_set_entry`, `gc_defer_entry`, `gc_list` and `gc_remove` over the structs plan 5 pinned; cluster tests pin the time-index ordering, paging by omap-key marker, upsert, defer and idempotent remove on v19.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh`; `cls-lock` if the owner
adds it; `watch-notify`. Deferred: the `gc_log_*` compositions of
`rgw_gc_log.cc` (the driver's), `gc_defer_entry`'s absence of callers.
