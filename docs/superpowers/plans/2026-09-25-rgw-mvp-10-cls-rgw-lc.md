# rados-rs RGW MVP, plan 10 of N: `cls-rgw-lc`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the `VersionedDenc` derive a version floor and apply it to
the five derived types already shipped above version 1; then add the
`rgw` object class's lifecycle methods (`lc_get_head`, `lc_put_head`,
`lc_get_entry`, `lc_set_entry`, `lc_rm_entry`, `lc_get_next_entry`,
`lc_list_entries`): their ten request and reply structs (two pinned
against `ceph-dencoder` v19.2.2 and the corpus, the rest by
construction), op constructors and `IoCtx` functions in `rgw::lc`, and
cluster tests that pin the class's head, entry and paging semantics on
Ceph v19.2.2, which upstream does not test at all.

**Architecture:** `rados-denc-macros`' `VersionedDenc` gains
`#[denc(min_version = N, ceph_release = "...")]`, whose generated
decoder fails below `N` with the `VersionTooOld` error
`check_min_version!` returns. `rgw::lc` (plan 6's `LcEntry` and
`LcObjHead`) gains the ops and client behind the `rgw` feature. A
lifecycle shard is an omap whose header is the encoded
`cls_rgw_lc_obj_head` and whose keys are the bucket keys
`tenant:name:marker` with `cls_rgw_lc_entry` values; three requests and
three replies carry a version-2 entry over a version-1
`pair<string, int>`, and the list reply echoes the request's version.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-lc`: head get and put, entry get, set, rm, next, list".

## Global Constraints

Plans 3 to 9's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-lc` is based on the fork's `main` after plan 9 merges;
  `rgw::lc` holds `LcEntry { bucket, start_time: u64, status: u32 }` and
  `LcObjHead { start_date: i64, marker, shard_rollover_date: i64 }`.
- The derived `VersionedDenc` does not floor by itself: before Task 1 its
  `decode_content` ignored the decoded version (`_version`,
  rados-denc-macros/src/lib.rs:444) and checked only
  `compat_version > version`, so an older `struct_v` was read as the
  current layout (misread, or failed on a length that happened not to
  fit). Only `StructVDenc` had a floor (`min_struct_v`), and
  `check_min_version!` served only hand-written decoders. Task 1 adds
  `min_version`; the Squid floor rule (plan 6) is enforced on a derived
  type only where it declares `min_version`, and Task 2 declares it on
  every derived type above version 1 that plans 4 to 9 shipped.
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
  `get_next_entry_ret` and `get_entry_ret` carried a `pair<string, int>`
  (v1 `get_entry_ret` also took a whole entry), and `list_entries_ret`
  versions 1 and 2 carried `map<string, int>`. The port encodes version 2
  (entries) and version 3 (list request and reply) and floors them with
  the derive: the four entry messages
  `version = 2, compat = 2, min_version = 2, ceph_release = "Pacific v16+"`,
  `ListRet` `version = 3, compat = 1, min_version = 3, ceph_release =
  "Pacific v16+"`. The hint's basis: v15.2.0 writes the entry messages as
  1/1, v16.2.0 as 2/2 and the list reply as v3. It does not model the list
  request's `compat_v` (it always sends 3 and so always receives 3); the
  other five (`GetEntryOp`, `GetNextEntryOp`, `PutHeadOp`, `GetHeadRet`
  at version 1, `ListEntriesOp` at 3/1) carry no floor.
- Server facts (`cls_rgw.cc`, v19): the head is the omap header,
  `get_head` on an empty header returns a default head (4350-4362), a
  decode failure is `EINVAL`; `put_head` creates the object; `get_entry`
  is `ENOENT` for an absent key and `EIO` for an undecodable value;
  `set_entry` writes the key (creating the object if needed); `rm_entry`
  removes the key, a missing key is not an error; `get_next_entry` returns
  the first key after `marker` (empty for the first) or a default entry
  with an empty bucket when none follows (4264-4277); `list_entries`
  returns up to `max_entries` keys after `marker` with `is_truncated`,
  reading legacy pair values too, and `max_entries` 0 returns no entries
  with `is_truncated` true. On a missing object `rm_entry`, `get_entry`,
  `get_next_entry`, `list` and `get_head` are `ENOENT` from the OSD; only
  `set_entry` and `put_head` create it. Verified on v19.2.2 by the
  implementer; the module doc states them.
- RGW's status values (`rgw_lc.h`): `lc_uninitial` 0, `lc_processing` 1,
  `lc_failed` 2, `lc_complete` 3; exported as `u32` constants
  `STATUS_UNINITIAL`, `STATUS_PROCESSING`, `STATUS_FAILED`,
  `STATUS_COMPLETE` with a doc naming the source. The `lc_process`
  exclusive lock RGW takes around processing is `cls_lock`, outside this
  plan.

## Review Focus

1. The derive's floor: `min_version` makes the generated `decode_content`
   call `check_min_version!`, so the error is `VersionTooOld` exactly as a
   hand-written decoder's; `ceph_release` is mandatory with it; a floor
   above the version is a compile-time panic; without the attribute the
   decoder is unchanged. Pinned in Task 1.
2. The five pre-existing derived types above version 1 now reject
   version 1 with `VersionTooOld`, and the corpus for the registered ones
   still passes. Pinned in Task 2.
3. The entry-carrying requests and replies are version 2 over compat 2
   with `min_version = 2`; the pair-shaped version 1 is rejected with
   `VersionTooOld`. Pinned by construction in Task 3.
4. `list_entries_ret` floors at 3 via `min_version = 3` (its map forms
   are rejected with `VersionTooOld`) and the vector form of version 3;
   the client sorts entries by bucket as the C++ decoder does. Pinned in
   Task 3.
5. `get_next_entry` after the last key is a default entry, not `ENOENT`;
   `get_entry` of an absent key is `ENOENT`. Pinned in Task 4.
6. The head lives in the omap header: `get_head` on a fresh object is the
   default head and `put_head` round-trips all three fields, including
   the rollover date the dump omits. Pinned in Task 4.

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0; branch `cls-rgw-lc` off the fork's `main` after plan
9 merges; the cluster up.

---

### Task 1: `rados-denc-macros`: a `min_version` floor for `VersionedDenc`

**Files:**
- Modify: `rados-denc-macros/src/lib.rs`, `rados/src/denc/codec.rs`
  (tests).

**Interfaces:**
- `#[denc(min_version = N, ceph_release = "...")]` on a `VersionedDenc`
  type, parsed like `min_struct_v` (a `u8` literal) into
  `DencAttrs::min_version`, and listed with the other keys in the
  attribute doc.
- With it, the generated `decode_content` binds the decoded version as
  `struct_v` and first runs
  `check_min_version!(struct_v, N, stringify!(Type), release)`, so a
  lower `struct_v` fails with
  `RadosError::Codec(CodecError::VersionTooOld { got, min, type_name,
  ceph_release })`, exactly as hand-written decoders do; the
  `compat_version > version` check follows unchanged.
- `ceph_release` is mandatory with `min_version` and a `min_version`
  above `version` is rejected; both are proc-macro panics, so compile
  errors. No attribute means no change (the parameter stays `_version`).
- The derive's docs gain a "Version floor" section with the example
  `#[denc(version = 2, compat = 2, min_version = 2, ceph_release =
  "Pacific v16+")]`.

Unit test, in `rados/src/denc/codec.rs` because a proc-macro crate cannot
run its own derive: `versioned_denc_min_version_rejects_older_headers`
with `FlooredV2` (`version = 2, compat = 2, min_version = 2,
ceph_release = "Test v2+"`) and `UnflooredV2` (no floor), each one `u32`;
the v1 frame `01 01 04000000 07000000` fails on `FlooredV2` with
`VersionTooOld { got: 1, min: 2, type_name: "FlooredV2", ceph_release:
"Test v2+" }`, the v2 frame decodes, and `UnflooredV2` still reads the v1
frame.

Commit:

```
denc-macros: VersionedDenc gains a min_version floor

A derived VersionedDenc type accepted any struct_v up to its version
and read the current layout from it, so an older encoding was misread
or failed on a length that happened not to fit. The crate's floor rule
needs the derive to enforce it: #[denc(min_version = N, ceph_release =
"...")] makes the generated decode_content fail below N with the
VersionTooOld error check_min_version! returns. Without the attribute
the decoder is unchanged.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 2: `rados-cls`: floor the derived types above version 1

**Files:**
- Modify: `rados-cls/src/rgw/types.rs`, `rados-cls/src/rgw/lc.rs`,
  `rados-cls/src/user.rs`, `rados-cls/src/rgw/usage.rs`,
  `rados-cls/src/rgw/index.rs`.

Each derived type plans 4 to 9 shipped with a version above 1 takes
`min_version` at the version v19 writes, and its doc gains "the decoder
floors at version 2":

| Type | Attribute | `ceph_release` |
|---|---|---|
| `PendingInfo` (`rgw_bucket_pending_info`) | `version = 2, compat = 2, min_version = 2` | `"Bobtail v0.56+"` |
| `LcObjHead` (`cls_rgw_lc_obj_head`) | `version = 2, compat = 2, min_version = 2` | `"Reef v18+"` |
| `ListBucketsOp` (`cls_user_list_buckets_op`) | `version = 2, compat = 1, min_version = 2` | `"Jewel v10+"` |
| `AddOp` (`rgw_cls_usage_log_add_op`) | `version = 2, compat = 1, min_version = 2` | `"Jewel v10+"` |
| `CheckMtimeOp` (`rgw_cls_obj_check_mtime`) | `version = 2, compat = 1, min_version = 2` | `"Jewel v10+"` |

Unit tests: one rejection test per type (`pending_info_floors_at_version_2`,
`lc_obj_head_floors_at_version_2`, `list_buckets_op_floors_at_version_2`,
`add_op_floors_at_version_2`; `CheckMtimeOp`'s existing v1 test tightened
from `is_err()`), each rewriting a default encoding's header to `01 01`
(or, for `CheckMtimeOp`, the v1 wire `010109000000010000000200000001`)
and asserting `VersionTooOld { got: 1, min: 2, .. }`.

Commit:

```
cls: floor the derived types at what Squid writes

A derived VersionedDenc type accepted any older struct_v and misread
its content as the current layout. Plans 4 through 9 shipped five
derived types above version 1: PendingInfo, LcObjHead, ListBucketsOp,
AddOp and CheckMtimeOp. Each now takes min_version at the version v19
writes, so an older encoding fails with VersionTooOld as the
hand-written decoders' floors do.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 3: The structs and the client in `rgw::lc`

**Files:**
- Modify: `rados-cls/src/rgw/lc.rs`, `rados-cls/src/rgw/mod.rs`,
  dencoder (two arms), corpus table (`cls_rgw_lc_get_entry_ret`,
  `cls_rgw_lc_set_entry_op`).

**Interfaces:**
- Types, all derived: `GetEntryOp { marker }` (v1); `GetEntryRet { entry:
  LcEntry }` (v2/2, floored; dump `{"entry": ...}`); `SetEntryOp { entry }`
  (v2/2, floored; custom `Serialize` flattening `bucket`, `start_time`,
  `status`); `RmEntryOp { entry }` (v2/2, floored); `GetNextEntryOp {
  marker }` (v1); `GetNextEntryRet { entry }` (v2/2, floored); `PutHeadOp
  { head: LcObjHead }` (v1); `GetHeadRet { head }` (v1); `ListEntriesOp {
  marker, max_entries: u32 }` (v3/1); `ListRet { entries: Vec<LcEntry>,
  is_truncated: bool }` (v3/1, floored at 3). "Floored" is Task 1's
  `min_version` with `ceph_release = "Pacific v16+"`, per Global
  Constraints; the derive enforces nothing without it. The `SetEntryOp`
  `Serialize` is the one hand-written piece.
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
- The module doc states the server facts of Global Constraints, including
  `ENOENT` for every call on a missing shard other than `set_entry` and
  `put_head`.

Unit tests pin: the two oracle vectors and JSON; the pair form
`010109000000010000006203000000` (that is `01 01 09000000 01000000 62
03000000`) rejected with `VersionTooOld` by `SetEntryOp`, `RmEntryOp`,
`GetNextEntryRet` and `GetEntryRet`, and a v1 frame around a whole entry
rejected by `GetEntryRet`; a v2 `ListRet`
`02010e0000000100000001000000610100000000` rejected with
`VersionTooOld`; a v3 `ListRet` round trip of two entries; `decode_list`
sorting; the decoders unwrapping the replies; every constructor's class,
method and request bytes (`rgwlc_get_head`, ...).

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
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 4: Cluster tests, and the README feature row

**Files:**
- Create: `rados-cls/tests/cls_rgw_lc.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both loops gain
  `cls_rgw_lc`).
- Modify, as a separate commit after the tests: `README.md` (the
  `rados-cls` row lists `version`, `refcount`, `user`, `queue`, `rgw`,
  `rgw_gc`). The branch folded this into the task rather than giving it
  one of its own.

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

Deviation: the `[b3]` comparison is `std::slice::from_ref(&b3)`, since
clippy 1.98 rejects `[b3.clone()]` there.

Commits:

```
cls: rgw lifecycle cluster tests

Pins against Ceph v19, which has no lc tests of its own: the head in
the omap header with a default when empty, get_entry ENOENT versus
get_next_entry's empty entry past the end, listing after a marker
with truncation, overwrite by set_entry, and idempotent rm_entry.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

```
docs: README lists every rados-cls feature

The crate table's row fell behind the queue, rgw and rgw_gc features
added since plan 5.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: per-commit build over all five commits; unit tests
for `rados` (Task 1's test) and `rados-cls`; corpus over the two lc names
plus the three floored types the dencoder registers (`rgw_bucket_pending_info`, `cls_user_list_buckets_op`, `rgw_cls_usage_log_add_op`); cluster
suites through `cls_rgw_lc` plus the six regression suites (Task 2
touches their types); the PR body:

```
**Motivation.** RGW's lifecycle worker walks per-shard entries through the `rgw` class's lc methods and records its position in the shard head; a Rust RGW running lifecycle needs all seven.

**What changed.** `rados-cls`'s `rgw::lc` gains the ten lc requests and replies, op constructors and `IoCtx` functions, and cluster tests that pin the head, entry and paging semantics upstream leaves untested. On the way, the `VersionedDenc` derive gains a `min_version` floor (it accepted any older struct version and misread it), the crate's derived types are floored at what Squid writes, and the README's feature list catches up.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-olh`; `cls-lock` if the owner adds it (the lifecycle worker's
`lc_process` lock needs it); `watch-notify`. Deferred: the list
request's `compat_v` echo (the port always sends version 3).

## Patched after execution (2026-09-25)

- Global Constraints: the false claim that the derived `VersionedDenc` rejects a `struct_v` below its version replaced by what it did (lib.rs:444, only `compat_version > version`) and the rule that a derived type floors only with `min_version`.
- New Task 1 (`rados-denc-macros` `min_version` + `ceph_release`, `VersionTooOld` via `check_min_version!`, test in `rados/src/denc/codec.rs`, "Version floor" doc section) and Task 2 (five derived types floored: `PendingInfo` Bobtail, `LcObjHead` Reef, `ListBucketsOp`/`AddOp`/`CheckMtimeOp` Jewel, rejection tests); lc tasks renumbered 3 and 4, gate Task 5.
- Task 3: lc types derived with `min_version` (entry messages 2/2/2, `ListRet` 3/1/3, "Pacific v16+"; basis v15.2.0 1/1, v16.2.0 2/2 and v3 list reply); rejection tests assert `VersionTooOld`; the v1-entry-frame, sorting, decoder and request-bytes tests added to the list.
- Server facts: missing-object `ENOENT` for `rm_entry`, `get_entry`, `get_next_entry`, `list`; `put_head` creates; `max_entries` 0 gives no entries and truncated; default next entry (cls_rgw.cc:4264-4277) and default head (4350-4362) cited.
- Task 4: `std::slice::from_ref(&b3)` deviation (clippy 1.98 rejects `[b3.clone()]`); README feature row folded in as a separate commit.
- Review Focus renumbered and rewritten: items 1-2 for the derive floor and the sweep; 3-4 say the floor is `min_version` and the error `VersionTooOld`.
- Every commit message is the branch's verbatim, trailers included; Goal, Architecture, gate scope and PR body widened to the derive fix.
