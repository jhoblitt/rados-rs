# rados-rs RGW MVP, plan 5 of N: `cls-queue-gc`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `queue` object class (`cls_queue_client.h`: init, get
capacity, enqueue, list, remove) and the `rgw_gc` class RGW layers on it
(`cls_rgw_gc_client.h`: init, get capacity, enqueue, list, remove, defer)
to `rados-cls`, with the GC entry types they carry, wire bytes pinned in
unit tests, all twenty `ceph-dencoder`-registered types compared against
the corpus, and cluster tests against Ceph v19.2.2.

**Architecture:** Three modules behind three features. `queue` mirrors the
generic ring-buffer class. `rgw` is the home of the types the `rgw` class
and the classes built on it share (`cls_rgw_types.h`, `cls_rgw_ops.h`);
this plan seeds it with `types` (the object key, the GC chain, the GC
entry) and `gc` (the `cls_rgw_gc_*` request and reply structs), and
`cls-rgw-types` extends it. `rgw_gc` depends on both: its entries are GC
entries in a `queue` ring whose head carries the deferrals. Every struct
is encoded as Ceph does, with `OSDOp` constructors for compound use and
async free functions over `IoCtx`.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-queue-gc`". The spec lists `cls_rgw_obj_key` and the GC chain
types under `cls-rgw-types`; they land here because `rgw_gc` cannot
encode an entry without them, in the `rgw` module that `cls-rgw-types`
will extend.

## Global Constraints

Plan 3's and plan 4's Global Constraints apply unchanged (Squid floor;
`#[denc(crate = "rados")]` on every derive; no `unwrap`/`expect` on
production paths; a feature joins `default` in the commit that adds its
module; gates, container fmt/clippy, push after every gate, merge when
green then an upstream PR; the commit trailer; offline builds; no new
dependencies). Plus:

- Branch `cls-queue-gc` is based on the fork's `main` after plan 4
  merges; `rados-cls/src/dump.rs` exists with `utime`, `gmtime` and
  `civil_from_days`, and `lib.rs` gates it on `feature = "user"`.
- Every helper in `dump` is gated on the class features that use it, and
  the module on the union, so each single-class build and the empty build
  stay warning-free under CI's per-class clippy step. A helper lands in
  the commit of its first user, never ahead of it.
- The `call` module (after plan 3's fix wave) offers `op`, `raw_op`,
  `exec`, `exec_raw`, `decode`, `decode_bytes`; request-less methods use
  `raw_op`/`exec_raw` with `Bytes::new()`.
- Features: `queue = []`, `rgw = []`, `rgw_gc = ["queue", "rgw"]`;
  `lib.rs` gates `mod call` on `any(...)` of every class feature, and the
  CI step "Run clippy on each rados-cls class alone" loops over every
  feature name.
- A `ceph::real_time` that `dump` streams with `dump_stream(...) << t`
  (as `cls_rgw_gc_obj_info::dump` does) prints through
  `operator<<(ostream&, real_time)`: ISO 8601 in the process's local zone
  with six microsecond digits and a numeric `%z` offset, always the
  calendar form. `ceph-dencoder` runs in UTC both in the corpus container
  and on the CI runner, so the string is `YYYY-MM-DDTHH:MM:SS.uuuuuu+0000`.
  This is not plan 4's `encode_json` form (`...Z`, raw seconds below ten
  years).
- Ceph's `std::unordered_map` encodes in hash order; the Rust side keeps
  a `BTreeMap`, so a re-encode of a map with two or more entries can
  differ in element order from the bytes Ceph wrote. Every corpus sample
  of `cls_rgw_gc_urgent_data` has an empty map; the type doc states the
  caveat.
- Method names, verified in `cls_queue_const.h` and
  `cls_rgw_gc_const.h`: class `queue` has `queue_init`,
  `queue_get_capacity`, `queue_enqueue`, `queue_list_entries`,
  `queue_remove_entries`; class `rgw_gc` has `rgw_gc_queue_init`,
  `rgw_gc_queue_enqueue`, `rgw_gc_queue_list_entries`,
  `rgw_gc_queue_remove_entries`, `rgw_gc_queue_update_entry` (the defer).
  `cls_rgw_gc_queue_get_capacity` calls the `queue` class's method on the
  same object. Flags: the two `get_capacity`/`list_entries` methods are
  RD, the rest RD|WR.
- `cls_queue_get_stats_ret` is declared but no method, handler or client
  uses it; it is left out.
- A method flagged RD|WR whose C++ client reads its reply (the
  `ObjectWriteOperation::exec` overload with an `out` buffer, run with
  `librados::OPERATION_RETURNVEC`) gets its reply only when the request
  carries `OsdOpFlags::RETURNVEC`; use `call::exec_returnvec` (plan 4),
  and keep such a reply under `osd_max_write_op_reply_len` (64 bytes by
  default; more is `EOVERFLOW`). Every writing method in this plan is
  reply-less, so the `IoCtx` functions use `call::exec`.

## Review Focus

1. `cls_queue_marker` declares `offset` then `gen` and dumps in that
   order, but encodes `gen` then `offset`; the string form is
   `gen/offset`. Pinned in Task 2 against the corpus bytes.
2. Four types carry a version-2 conditional: `cls_queue_list_op` (Reef
   wrote version 1 without `end_marker`; the 18.2.0 corpus has two such
   samples), `cls_rgw_obj` (version 2 appended the full key after the
   three version-1 strings, so `key.name` is on the wire twice),
   `cls_rgw_gc_list_ret` (version 2 put `next_marker` between `entries`
   and `truncated`), `cls_rgw_gc_list_op` (`expired_only`, default true).
   The derive cannot express a version-gated field, so all four are
   hand-written. Pinned in Tasks 2 and 3 and by the corpus run.
3. Two dump conventions for booleans in one branch: `cls_queue_list_ret`
   uses `dump_bool` (JSON `true`), `cls_rgw_gc_list_ret` uses
   `dump_int((int)truncated)` (JSON `1`); and the GC entry's `time` is
   the `+0000` form. Pinned in Tasks 1 and 3.
4. Ring semantics as the class implements them: the first entry's marker
   is `0/1024` (the head is 1 KiB), each entry costs ten bytes plus its
   payload, `end_marker` on a list is exclusive, a remove `end_marker`
   must be the front or ahead of it within one wrap (else `EINVAL`), a
   payload that does not fit is `ENOSPC`, a second init is `EEXIST`.
   Pinned in Task 5.
5. `rgw_gc` semantics: an entry is due at enqueue time plus
   `expiration_secs`; `list_entries` with `expired_only` returns only
   due entries and always skips a tag deferred to a later time;
   `remove_entries` drops the first `num_entries` entries the class does
   not treat as deferred; `defer_entry` on a tag past
   `num_deferred_entries` is `ENOSPC` while re-deferring a known tag is
   not. Pinned in Task 5.

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cls-queue-gc` off the fork's `main` (which contains
  plan 4's merge) checked out in `$R`; the SDD ledger; the cluster up.

- [ ] **Step 1: Branch, cluster, ledger**

```bash
cd $R && git fetch origin main && git checkout -b cls-queue-gc origin/main && git log --oneline -1
grep -n 'pub(crate) fn gmtime\|fn civil_from_days' rados-cls/src/dump.rs
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
```
Expected: HEAD is the merge of the `cls-user` PR; both `dump` functions
present; three containers up. Then the workspace and ledger as in plan
3's Task 0.

---

### Task 1: `dump`: `bool_as_int` shared, and the module gated per helper

**Files:**
- Modify: `rados-cls/src/dump.rs`, `rados-cls/src/lib.rs`,
  `rados-cls/src/refcount.rs` (drop its private `bool_as_int`, use
  `crate::dump::bool_as_int`).

**Interfaces:**
- Produces: `pub(crate) fn bool_as_int<S: Serializer>(b: &bool, s: S)`
  gated `#[cfg(feature = "refcount")]` (Task 3 widens it to
  `any(feature = "refcount", feature = "rgw")`); `utime`, `gmtime` and
  `civil_from_days` gated `#[cfg(feature = "user")]`; the `mod dump;`
  line gated `any(feature = "refcount", feature = "user")`. The
  `real_time` renderer comes in Task 3 with its first user.

- [ ] **Step 1: Write the failing test**

Append to `dump.rs`'s test module (the `real_time` test below belongs to
Task 3, where the renderer lands):

```rust
    #[test]
    #[cfg(feature = "rgw")]
    fn streamed_real_time_is_iso_with_a_numeric_offset() {
        // operator<<(ostream&, real_time): calendar form even at the epoch,
        // microseconds, and the zone's %z, which is +0000 where dencoder runs.
        assert_eq!(iso_utc(&UTime { sec: 0, nsec: 0 }), "1970-01-01T00:00:00.000000+0000");
        assert_eq!(iso_utc(&UTime { sec: 21, nsec: 32 }), "1970-01-01T00:00:21.000000+0000");
        assert_eq!(
            iso_utc(&UTime { sec: 1_727_611_205, nsec: 747_275_000 }),
            "2024-09-29T12:00:05.747275+0000"
        );
    }

    #[test]
    #[cfg(feature = "refcount")]
    fn bool_as_int_prints_zero_or_one() {
        #[derive(serde::Serialize)]
        struct T(#[serde(serialize_with = "bool_as_int")] bool);
        assert_eq!(serde_json::to_string(&T(true)).expect("json"), "1");
        assert_eq!(serde_json::to_string(&T(false)).expect("json"), "0");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline dump`
Expected: compile error, `bool_as_int` not found.

- [ ] **Step 3: Implement**

Add to `dump.rs` now (Task 1):

```rust
/// `dump_int((int)b)`: a bool printed as `0` or `1`.
#[cfg(feature = "refcount")]
pub(crate) fn bool_as_int<S: serde::Serializer>(b: &bool, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u8(u8::from(*b))
}
```

and put `#[cfg(feature = "user")]` on `utime`, `gmtime` and
`civil_from_days` (plus `#[cfg(feature = "user")]` on their tests), and
change `lib.rs` to `#[cfg(any(feature = "refcount", feature = "user"))]
pub(crate) mod dump;`. Task 3 adds, with the `rgw` module that uses it:

```rust
/// A `real_time` that `dump` streams with `operator<<`: the calendar form
/// with microseconds and the zone offset, which is `+0000` where
/// `ceph-dencoder` runs.
#[cfg(feature = "rgw")]
pub(crate) fn real_time<S: serde::Serializer>(t: &UTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&iso_utc(t))
}

#[cfg(feature = "rgw")]
pub(crate) fn iso_utc(t: &UTime) -> String {
    let usec = t.nsec / 1000;
    let (year, month, day) = civil_from_days(i64::from(t.sec / 86_400));
    let secs = t.sec % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{usec:06}+0000",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
```

with `civil_from_days` widened to `any(feature = "user", feature = "rgw")`,
`bool_as_int` to `any(feature = "refcount", feature = "rgw")`, and the
`mod dump;` gate to `any(feature = "refcount", feature = "rgw", feature =
"user")`.

In `refcount.rs` (Task 1), delete the private `bool_as_int` and point its
`serialize_with` attributes at `"crate::dump::bool_as_int"`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline && cargo check -p rados-cls --no-default-features --features refcount --offline`
Expected: all pass (plan 4's count plus 1), no warnings in either.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/dump.rs rados-cls/src/lib.rs rados-cls/src/refcount.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: share bool_as_int through dump, gated per class

The refcount class's bool-as-int helper moves to dump, since the GC
list reply will need it too. Each renderer in dump is now gated on the
classes that use it, so a build of one class stays warning-free.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 2: The `queue` class

**Files:**
- Create: `rados-cls/src/queue.rs`.
- Modify: `rados-cls/Cargo.toml` (feature `queue`, added to `default`),
  `rados-cls/src/lib.rs` (`#[cfg(feature = "queue")] pub mod queue;` and
  `queue` in the `mod call` gate), `.github/workflows/ci.yml` (`queue` in
  the per-class clippy loop), `rados-dencoder/src/main.rs` (nine arms and
  the `list_types` lines), `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`
  (nine `TypeSpec`s).

**Interfaces:**
- Consumes: `crate::call`; `rados::{Denc, RadosError, VersionedDenc, VersionedEncode, UTime}`.
- Produces: `pub const CLASS: &str = "queue"`, `pub const HEAD_SIZE_1K:
  u64 = 1024`; types `Entry`, `Marker`, `Head`, `InitOp`, `EnqueueOp`,
  `ListOp`, `ListRet`, `RemoveOp`, `GetCapacityRet`; op constructors
  `init_op(u64)`, `get_capacity_op()`, `enqueue_op(Vec<Bytes>)`,
  `list_op(&str, u64, &str)`, `remove_entries_op(&str)`; decoders
  `decode_get_capacity(&OpReply) -> Result<u64>`, `decode_list(&OpReply)
  -> Result<ListRet>`; async `init`, `get_capacity`, `enqueue`,
  `list_entries`, `remove_entries`. Task 4 reuses `get_capacity_op`,
  `decode_get_capacity`, `get_capacity`.

- [ ] **Step 1: Write the failing tests**

Create `rados-cls/src/queue.rs` with the module doc, the imports, and
this test module (the code between comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use rados::osdclient::types::OpData;

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    #[test]
    fn marker_encodes_gen_before_offset() {
        // Corpus cls_queue_marker/fe7a542e...: gen 0, offset 745307.
        let m = Marker { offset: 745_307, gen: 0 };
        assert_eq!(
            bytes(&m),
            [1u8, 1, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x5b, 0x5f, 0x0b, 0, 0, 0, 0, 0]
        );
        assert_eq!(Marker::decode(&mut &bytes(&m)[..], 0).expect("decode"), m);
        assert_eq!(m.to_string(), "0/745307");
        assert_eq!(json(&m), r#"{"offset":745307,"gen":0}"#);
    }

    #[test]
    fn head_default_matches_the_corpus_instance() {
        // Corpus cls_queue_head/18dc7514...: a 1 KiB head, both markers at
        // its end, nothing else set; 78 bytes.
        let wire: Vec<u8> = [
            &[1u8, 1, 72, 0, 0, 0][..],
            &1024u64.to_le_bytes(),
            &[1, 1, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0],
            &[1, 1, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0],
            &0u64.to_le_bytes(),
            &0u64.to_le_bytes(),
            &[0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(bytes(&Head::default()), wire);
        assert_eq!(Head::decode(&mut &wire[..], 0).expect("decode"), Head::default());
        assert_eq!(
            json(&Head::default()),
            r#"{"max_head_size":1024,"queue_size":0,"max_urgent_data_size":0,"front_offset":1024,"front_gen":0,"tail_offset":1024,"tail_gen":0}"#
        );
    }

    #[test]
    fn entry_and_init_and_enqueue_dump_lengths_not_payloads() {
        // Corpus cls_queue_entry/9f633d96...: data "data", marker "marker".
        let e = Entry {
            data: Bytes::from_static(b"data"),
            marker: "marker".to_owned(),
        };
        assert_eq!(
            bytes(&e),
            b"\x01\x01\x12\x00\x00\x00\x04\x00\x00\x00data\x06\x00\x00\x00marker"
        );
        assert_eq!(json(&e), r#"{"marker":"marker","data_len":4}"#);

        // Corpus cls_queue_init_op/6202a90f...: 1024, 1024, "data".
        let i = InitOp {
            queue_size: 1024,
            max_urgent_data_size: 1024,
            urgent_data: Bytes::from_static(b"data"),
        };
        assert_eq!(
            bytes(&i),
            b"\x01\x01\x18\x00\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00\x04\x00\x00\x00data"
        );
        assert_eq!(
            json(&i),
            r#"{"queue_size":1024,"max_urgent_data_size":1024,"urgent_data_len":4}"#
        );

        // Corpus cls_queue_enqueue_op/c1504f3f...: one payload "data".
        let q = EnqueueOp {
            data: vec![Bytes::from_static(b"data")],
        };
        assert_eq!(
            bytes(&q),
            b"\x01\x01\x0c\x00\x00\x00\x01\x00\x00\x00\x04\x00\x00\x00data"
        );
        assert_eq!(json(&q), r#"{"data_vec_len":1}"#);
    }

    #[test]
    fn list_op_reads_reef_v1_and_writes_v2() {
        // Corpus 18.2.0 cls_queue_list_op: version 1, max 128, no end_marker.
        let v1 = b"\x01\x01\x0c\x00\x00\x00\x80\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let op = ListOp::decode(&mut &v1[..], 0).expect("decode");
        assert_eq!(
            op,
            ListOp {
                max: 128,
                start_marker: String::new(),
                end_marker: String::new(),
            }
        );
        assert_eq!(
            bytes(&op),
            b"\x02\x01\x10\x00\x00\x00\x80\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        );
        assert_eq!(json(&op), r#"{"max":128,"start_marker":""}"#);
    }

    #[test]
    fn list_ret_dumps_a_real_bool_and_the_entries() {
        let ret = ListRet {
            is_truncated: true,
            next_marker: "foo".to_owned(),
            entries: vec![
                Entry::default(),
                Entry {
                    data: Bytes::from_static(b"data"),
                    marker: "id".to_owned(),
                },
            ],
        };
        assert_eq!(
            json(&ret),
            r#"{"is_truncated":true,"next_marker":"foo","entries":[{"marker":"","data_len":0},{"marker":"id","data_len":4}]}"#
        );
        assert_eq!(ListRet::decode(&mut &bytes(&ret)[..], 0).expect("decode"), ret);
    }

    #[test]
    fn remove_and_capacity_are_one_field_each() {
        // Corpus cls_queue_remove_op/988d6dc7... and cls_queue_get_capacity_ret/01d854f6....
        let r = RemoveOp {
            end_marker: "0/145919".to_owned(),
        };
        assert_eq!(bytes(&r), b"\x01\x01\x0c\x00\x00\x00\x08\x00\x00\x000/145919");
        let c = GetCapacityRet { queue_capacity: 342 };
        assert_eq!(bytes(&c), b"\x01\x01\x08\x00\x00\x00\x56\x01\x00\x00\x00\x00\x00\x00");
        assert_eq!(json(&c), r#"{"queue_capacity":342}"#);
    }

    #[test]
    fn ops_name_the_class_and_method() {
        let op = init_op(4096).expect("op");
        assert!(op.indata.starts_with(b"queuequeue_init"));
        assert!(matches!(
            op.op_data,
            OpData::Call {
                class_len: 5,
                method_len: 10,
                ..
            }
        ));
        let op = get_capacity_op().expect("op");
        assert!(op.indata.starts_with(b"queuequeue_get_capacity"));
        assert!(matches!(op.op_data, OpData::Call { indata_len: 0, .. }));
        let op = list_op("0/1024", 7, "").expect("op");
        assert!(op.indata.starts_with(b"queuequeue_list_entries"));
        let op = remove_entries_op("0/1037").expect("op");
        assert!(op.indata.starts_with(b"queuequeue_remove_entries"));
    }

    #[test]
    fn decoders_unwrap_the_replies() {
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&GetCapacityRet { queue_capacity: 9 }, 0).expect("encode"),
        };
        assert_eq!(decode_get_capacity(&reply).expect("decode"), 9);
        let ret = ListRet {
            is_truncated: false,
            next_marker: "0/1065".to_owned(),
            entries: vec![],
        };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&ret, 0).expect("encode"),
        };
        assert_eq!(decode_list(&reply).expect("decode"), ret);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline queue`
Expected: compile errors, the types and functions do not exist.

- [ ] **Step 3: Implement the types and the client**

The module, above the tests:

```rust
//! The `queue` object class (`cls_queue_client.h`): a ring of opaque
//! entries inside one object. The head (1 KiB plus the urgent-data
//! allowance) sits at offset zero; entries follow, each ten bytes of
//! magic and length ahead of its payload; `front` and `tail` markers
//! delimit the live region and `gen` counts wraps. `queue_init`,
//! `queue_enqueue` and `queue_remove_entries` are RD|WR methods;
//! `queue_get_capacity` and `queue_list_entries` are RD.

use std::fmt;

use bytes::{Buf, BufMut, Bytes};
use rados::osdclient::error::Result;
use rados::osdclient::{IoCtx, OSDOp, OpReply};
use rados::{Denc, RadosError, VersionedDenc, VersionedEncode};
use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::call;

pub const CLASS: &str = "queue";

/// `QUEUE_HEAD_SIZE_1K`: the head's size before any urgent-data allowance,
/// and so the first entry's offset.
pub const HEAD_SIZE_1K: u64 = 1024;

/// `cls_queue_entry`: one payload and the marker naming its slot. The dump
/// carries the marker and the payload's length as `data_len`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct Entry {
    pub data: Bytes,
    pub marker: String,
}

impl Serialize for Entry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Entry", 2)?;
        state.serialize_field("marker", &self.marker)?;
        state.serialize_field("data_len", &(self.data.len() as u64))?;
        state.end()
    }
}

/// `cls_queue_marker`: a position in the ring, printed as `gen/offset`.
/// The wire order is `gen` then `offset`, the reverse of the declaration
/// and of the dump.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Marker {
    pub offset: u64,
    pub gen: u64,
}

impl VersionedEncode for Marker {
    const MAX_DECODE_VERSION: u8 = 1;

    fn encoding_version(&self, _features: u64) -> u8 {
        1
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.gen.encode(buf, features)?;
        self.offset.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        _struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        let gen = u64::decode(buf, features)?;
        let offset = u64::decode(buf, features)?;
        Ok(Self { offset, gen })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        Some(16)
    }
}

rados::impl_denc_for_versioned!(Marker);

impl fmt::Display for Marker {
    /// `cls_queue_marker::to_str`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.gen, self.offset)
    }
}

/// `cls_queue_head`: the ring's bookkeeping, stored at offset zero of the
/// object. The dump flattens the two markers and leaves the urgent data
/// out.
#[derive(Debug, Clone, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct Head {
    pub max_head_size: u64,
    pub front: Marker,
    pub tail: Marker,
    pub queue_size: u64,
    pub max_urgent_data_size: u64,
    pub urgent_data: Bytes,
}

impl Default for Head {
    fn default() -> Self {
        let start = Marker {
            offset: HEAD_SIZE_1K,
            gen: 0,
        };
        Self {
            max_head_size: HEAD_SIZE_1K,
            front: start,
            tail: start,
            queue_size: 0,
            max_urgent_data_size: 0,
            urgent_data: Bytes::new(),
        }
    }
}

impl Serialize for Head {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Head", 7)?;
        state.serialize_field("max_head_size", &self.max_head_size)?;
        state.serialize_field("queue_size", &self.queue_size)?;
        state.serialize_field("max_urgent_data_size", &self.max_urgent_data_size)?;
        state.serialize_field("front_offset", &self.front.offset)?;
        state.serialize_field("front_gen", &self.front.gen)?;
        state.serialize_field("tail_offset", &self.tail.offset)?;
        state.serialize_field("tail_gen", &self.tail.gen)?;
        state.end()
    }
}

/// `cls_queue_init_op`: `queue_size` usable bytes (the class adds the head
/// on top), an urgent-data allowance and its initial contents. The dump
/// prints the contents' length as `urgent_data_len`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct InitOp {
    pub queue_size: u64,
    pub max_urgent_data_size: u64,
    pub urgent_data: Bytes,
}

impl Serialize for InitOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("InitOp", 3)?;
        state.serialize_field("queue_size", &self.queue_size)?;
        state.serialize_field("max_urgent_data_size", &self.max_urgent_data_size)?;
        state.serialize_field("urgent_data_len", &(self.urgent_data.len() as u64))?;
        state.end()
    }
}

/// `cls_queue_enqueue_op`: the payloads to append, one entry each. The
/// dump prints only their count as `data_vec_len`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct EnqueueOp {
    pub data: Vec<Bytes>,
}

impl Serialize for EnqueueOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("EnqueueOp", 1)?;
        state.serialize_field("data_vec_len", &(self.data.len() as u64))?;
        state.end()
    }
}

/// `cls_queue_list_op`: up to `max` entries from `start_marker` (empty
/// for the front), stopping before `end_marker` when it is set. Version 2
/// added `end_marker`; Reef wrote version 1, and the dump leaves it out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ListOp {
    pub max: u64,
    pub start_marker: String,
    #[serde(skip)]
    pub end_marker: String,
}

impl VersionedEncode for ListOp {
    const MAX_DECODE_VERSION: u8 = 2;

    fn encoding_version(&self, _features: u64) -> u8 {
        2
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.max.encode(buf, features)?;
        self.start_marker.encode(buf, features)?;
        self.end_marker.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        let max = u64::decode(buf, features)?;
        let start_marker = String::decode(buf, features)?;
        let end_marker = if struct_v > 1 {
            String::decode(buf, features)?
        } else {
            String::new()
        };
        Ok(Self {
            max,
            start_marker,
            end_marker,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(ListOp);

/// `cls_queue_list_ret`. `next_marker` is where the next page starts, or
/// the tail when `is_truncated` is false.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ListRet {
    pub is_truncated: bool,
    pub next_marker: String,
    pub entries: Vec<Entry>,
}

/// `cls_queue_remove_op`: everything before `end_marker` goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct RemoveOp {
    pub end_marker: String,
}

/// `cls_queue_get_capacity_ret`: the usable bytes, `queue_size` less the head.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetCapacityRet {
    pub queue_capacity: u64,
}

/// `cls_queue_init`: lay out a ring of `size` usable bytes with no urgent
/// data. `EEXIST` if the object already holds a head.
pub fn init_op(size: u64) -> Result<OSDOp> {
    call::op(
        CLASS,
        "queue_init",
        &InitOp {
            queue_size: size,
            ..InitOp::default()
        },
    )
}

/// `cls_queue_get_capacity`: the op; decode its reply with
/// [`decode_get_capacity`].
pub fn get_capacity_op() -> Result<OSDOp> {
    call::raw_op(CLASS, "queue_get_capacity", Bytes::new())
}

/// Decode the reply to [`get_capacity_op`].
pub fn decode_get_capacity(reply: &OpReply) -> Result<u64> {
    Ok(call::decode::<GetCapacityRet>(reply)?.queue_capacity)
}

/// `cls_queue_enqueue`: append each payload as one entry, wrapping past
/// the end of the ring. `ENOSPC` when they do not all fit.
pub fn enqueue_op(data: Vec<Bytes>) -> Result<OSDOp> {
    call::op(CLASS, "queue_enqueue", &EnqueueOp { data })
}

/// `cls_queue_list_entries`: up to `max` entries from `start_marker`
/// (empty for the front), stopping before `end_marker` when it is not
/// empty. The C++ end-marker overload passes `u64::MAX` for `max`. Decode
/// the reply with [`decode_list`].
pub fn list_op(start_marker: &str, max: u64, end_marker: &str) -> Result<OSDOp> {
    call::op(
        CLASS,
        "queue_list_entries",
        &ListOp {
            max,
            start_marker: start_marker.to_owned(),
            end_marker: end_marker.to_owned(),
        },
    )
}

/// Decode the reply to [`list_op`].
pub fn decode_list(reply: &OpReply) -> Result<ListRet> {
    call::decode(reply)
}

/// `cls_queue_remove_entries`: move the front to `end_marker`, which must
/// be the front itself or ahead of it within one wrap; anything else is
/// `EINVAL`. A no-op on an empty ring.
pub fn remove_entries_op(end_marker: &str) -> Result<OSDOp> {
    call::op(
        CLASS,
        "queue_remove_entries",
        &RemoveOp {
            end_marker: end_marker.to_owned(),
        },
    )
}

/// Lay out a ring of `size` usable bytes on `oid`; see [`init_op`].
pub async fn init(ioctx: &IoCtx, oid: &str, size: u64) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "queue_init",
        &InitOp {
            queue_size: size,
            ..InitOp::default()
        },
    )
    .await?;
    Ok(())
}

/// The usable bytes of the ring on `oid`.
pub async fn get_capacity(ioctx: &IoCtx, oid: &str) -> Result<u64> {
    let out = call::exec_raw(ioctx, oid, CLASS, "queue_get_capacity", Bytes::new()).await?;
    Ok(call::decode_bytes::<GetCapacityRet>(out)?.queue_capacity)
}

/// Append `data` to the ring on `oid`; see [`enqueue_op`].
pub async fn enqueue(ioctx: &IoCtx, oid: &str, data: Vec<Bytes>) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "queue_enqueue", &EnqueueOp { data }).await?;
    Ok(())
}

/// List entries of the ring on `oid`; see [`list_op`].
pub async fn list_entries(
    ioctx: &IoCtx,
    oid: &str,
    start_marker: &str,
    max: u64,
    end_marker: &str,
) -> Result<ListRet> {
    let out = call::exec(
        ioctx,
        oid,
        CLASS,
        "queue_list_entries",
        &ListOp {
            max,
            start_marker: start_marker.to_owned(),
            end_marker: end_marker.to_owned(),
        },
    )
    .await?;
    call::decode_bytes(out)
}

/// Drop everything before `end_marker` from the ring on `oid`; see
/// [`remove_entries_op`].
pub async fn remove_entries(ioctx: &IoCtx, oid: &str, end_marker: &str) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "queue_remove_entries",
        &RemoveOp {
            end_marker: end_marker.to_owned(),
        },
    )
    .await?;
    Ok(())
}
```

`Cargo.toml`: add `queue = []` and put it in `default`. `lib.rs`: add
`#[cfg(feature = "queue")] pub mod queue;` (alphabetical, after
`dump`/before `refcount`) and `feature = "queue"` to the `mod call` gate.
`ci.yml`: `for features in "" queue refcount version; do`.

`rados-dencoder/src/main.rs`: import `rados_cls::queue::{Entry as
QueueEntry, EnqueueOp as QueueEnqueueOp, GetCapacityRet as
QueueGetCapacityRet, Head as QueueHead, InitOp as QueueInitOp, ListOp as
QueueListOp, ListRet as QueueListRet, Marker as QueueMarker, RemoveOp as
QueueRemoveOp}`; add the arms under `// Object classes (rados-cls)`:

```rust
        "cls_queue_entry" => Some(type_info_denc::<QueueEntry>()),
        "cls_queue_marker" => Some(type_info_denc::<QueueMarker>()),
        "cls_queue_head" => Some(type_info_denc::<QueueHead>()),
        "cls_queue_init_op" => Some(type_info_denc::<QueueInitOp>()),
        "cls_queue_enqueue_op" => Some(type_info_denc::<QueueEnqueueOp>()),
        "cls_queue_list_op" => Some(type_info_denc::<QueueListOp>()),
        "cls_queue_list_ret" => Some(type_info_denc::<QueueListRet>()),
        "cls_queue_remove_op" => Some(type_info_denc::<QueueRemoveOp>()),
        "cls_queue_get_capacity_ret" => Some(type_info_denc::<QueueGetCapacityRet>()),
```

and two `list_types` lines in the OBJECT CLASSES block, matching the
neighbours' column. The corpus test: nine `TypeSpec::new("cls_queue_...",
None, false)` after the `cls_user_*` block.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline queue && cargo check --workspace --all-targets --offline`
Expected: 8 passed, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/Cargo.toml rados-cls/src/lib.rs rados-cls/src/queue.rs .github/workflows/ci.yml rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the queue class

Mirrors cls_queue_client.h: init, get_capacity, enqueue, list_entries
and remove_entries on the "queue" class, a ring of opaque entries in
one object. cls_queue_marker encodes gen before offset though it
declares and dumps offset first; cls_queue_list_op is version 2 with
end_marker, which Reef did not write. The class answers a second init
with EEXIST, a payload that does not fit with ENOSPC, and a remove
end_marker that is not the front or ahead of it with EINVAL.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 3: The `rgw` module: the GC entry types and the `cls_rgw_gc_*` structs

**Files:**
- Create: `rados-cls/src/rgw/mod.rs`, `rados-cls/src/rgw/types.rs`,
  `rados-cls/src/rgw/gc.rs`.
- Modify: `rados-cls/src/dump.rs` (`real_time`, `iso_utc`, their test,
  the widened gates; see Task 1), `rados-cls/Cargo.toml` (feature `rgw`,
  in `default`), `rados-cls/src/lib.rs`, `.github/workflows/ci.yml`,
  `rados-dencoder/src/main.rs` (nine arms), the corpus test (nine
  `TypeSpec`s).

**Interfaces:**
- Consumes: `crate::dump::{real_time, bool_as_int}`; `rados::UTime`.
- Produces: `rgw::types::{ObjKey, Obj, ObjChain, GcObjInfo}`;
  `rgw::gc::{SetEntryOp, DeferEntryOp, ListOp, ListRet, RemoveOp}`. Task
  4 encodes `SetEntryOp` and `ListOp` and decodes `ListRet`; the
  `cls-rgw-gc` plan adds the `rgw` class's client functions to `gc.rs`.

- [ ] **Step 1: Write the failing tests**

`rgw/mod.rs`:

```rust
//! Types the `rgw` object class and the classes RGW layers on it share
//! (`cls_rgw_types.h`, `cls_rgw_ops.h`). The class's own methods come
//! with the bucket-index, GC, usage, lifecycle and OLH work.

pub mod gc;
pub mod types;
```

`rgw/types.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    #[test]
    fn obj_key_is_two_strings() {
        // Corpus cls_rgw_obj_key/eedab0b3...
        let k = ObjKey {
            name: "name".to_owned(),
            instance: "instance".to_owned(),
        };
        assert_eq!(
            bytes(&k),
            b"\x01\x01\x14\x00\x00\x00\x04\x00\x00\x00name\x08\x00\x00\x00instance"
        );
        assert_eq!(json(&k), r#"{"name":"name","instance":"instance"}"#);
    }

    #[test]
    fn obj_writes_the_name_twice_and_dumps_loc_as_key() {
        // Corpus cls_rgw_obj/bfcd58c2...: pool "mypool", key.name "myoid",
        // loc "mykey", no instance; 53 bytes, version 2 over compat 1.
        let o = Obj {
            pool: "mypool".to_owned(),
            key: ObjKey {
                name: "myoid".to_owned(),
                instance: String::new(),
            },
            loc: "mykey".to_owned(),
        };
        let wire = b"\x02\x01\x2f\x00\x00\x00\x06\x00\x00\x00mypool\x05\x00\x00\x00myoid\x05\x00\x00\x00mykey\x01\x01\x0d\x00\x00\x00\x05\x00\x00\x00myoid\x00\x00\x00\x00";
        assert_eq!(bytes(&o), wire);
        assert_eq!(Obj::decode(&mut &wire[..], 0).expect("decode"), o);
        assert_eq!(
            json(&o),
            r#"{"pool":"mypool","oid":"myoid","key":"mykey","instance":""}"#
        );

        // A version-1 writer sent only the three strings.
        let v1 = b"\x01\x01\x17\x00\x00\x00\x01\x00\x00\x00p\x01\x00\x00\x00n\x01\x00\x00\x00l";
        let o = Obj::decode(&mut &v1[..], 0).expect("decode");
        assert_eq!((o.pool.as_str(), o.key.name.as_str(), o.loc.as_str()), ("p", "n", "l"));
        assert!(o.key.instance.is_empty());
    }

    #[test]
    fn gc_obj_info_matches_the_corpus_instance() {
        // Corpus cls_rgw_gc_obj_info/fd3d3891...: tag "footag", empty chain,
        // time {21, 32}; 34 bytes.
        let info = GcObjInfo {
            tag: "footag".to_owned(),
            chain: ObjChain::default(),
            time: UTime { sec: 21, nsec: 32 },
        };
        assert_eq!(
            bytes(&info),
            b"\x01\x01\x1c\x00\x00\x00\x06\x00\x00\x00footag\x01\x01\x04\x00\x00\x00\x00\x00\x00\x00\x15\x00\x00\x00\x20\x00\x00\x00"
        );
        assert_eq!(
            json(&info),
            r#"{"tag":"footag","chain":{"objs":[]},"time":"1970-01-01T00:00:21.000000+0000"}"#
        );
        assert_eq!(GcObjInfo::decode(&mut &bytes(&info)[..], 0).expect("decode"), info);
    }

    #[test]
    fn json_keeps_a_nul_in_a_tag() {
        // cls_rgw_gc_obj_info::dump uses the whole std::string, unlike
        // obj_refcount; the corpus tags end in a NUL and dencoder prints it.
        let info = GcObjInfo {
            tag: "t\0".to_owned(),
            ..GcObjInfo::default()
        };
        assert!(json(&info).starts_with(r#"{"tag":"t\u0000""#));
    }
}
```

`rgw/gc.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rgw::types::GcObjInfo;
    use rados::encode_with_capacity;

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    #[test]
    fn set_entry_op_nests_the_info_as_obj_info() {
        // Corpus cls_rgw_gc_set_entry_op/68510e49...: 123 seconds, default info.
        let op = SetEntryOp {
            expiration_secs: 123,
            info: GcObjInfo::default(),
        };
        assert_eq!(
            bytes(&op),
            b"\x01\x01\x20\x00\x00\x00\x7b\x00\x00\x00\x01\x01\x16\x00\x00\x00\x00\x00\x00\x00\x01\x01\x04\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        );
        assert_eq!(
            json(&op),
            r#"{"expiration_secs":123,"obj_info":{"tag":"","chain":{"objs":[]},"time":"1970-01-01T00:00:00.000000+0000"}}"#
        );
    }

    #[test]
    fn defer_and_remove_are_the_omap_era_shapes() {
        // Corpus cls_rgw_gc_defer_entry_op/1191bd44... and cls_rgw_gc_remove_op/39615566....
        let d = DeferEntryOp {
            expiration_secs: 5,
            tag: "mychain".to_owned(),
        };
        assert_eq!(bytes(&d), b"\x01\x01\x0f\x00\x00\x00\x05\x00\x00\x00\x07\x00\x00\x00mychain");
        assert_eq!(json(&d), r#"{"expiration_secs":5,"tag":"mychain"}"#);
        let r = RemoveOp {
            tags: vec!["tag1".to_owned(), "tag2".to_owned()],
        };
        assert_eq!(
            bytes(&r),
            b"\x01\x01\x14\x00\x00\x00\x02\x00\x00\x00\x04\x00\x00\x00tag1\x04\x00\x00\x00tag2"
        );
        assert_eq!(json(&r), r#"{"tags":["tag1","tag2"]}"#);
    }

    #[test]
    fn list_op_defaults_expired_only_and_reads_v1_as_true() {
        // Corpus cls_rgw_gc_list_op/500d868f...: "mymarker", 2312, true.
        let op = ListOp {
            marker: "mymarker".to_owned(),
            max: 2312,
            ..ListOp::default()
        };
        assert!(op.expired_only);
        assert_eq!(
            bytes(&op),
            b"\x02\x01\x11\x00\x00\x00\x08\x00\x00\x00mymarker\x08\x09\x00\x00\x01"
        );
        assert_eq!(json(&op), r#"{"marker":"mymarker","max":2312,"expired_only":true}"#);
        // Corpus cls_rgw_gc_list_op/ac427b0d...: "", 10, false.
        let wire = b"\x02\x01\x09\x00\x00\x00\x00\x00\x00\x00\x0a\x00\x00\x00\x00";
        let op = ListOp::decode(&mut &wire[..], 0).expect("decode");
        assert_eq!((op.max, op.expired_only), (10, false));
        // Version 1 had no flag; the C++ constructor's default applies.
        let v1 = b"\x01\x01\x08\x00\x00\x00\x00\x00\x00\x00\x0a\x00\x00\x00";
        assert!(ListOp::decode(&mut &v1[..], 0).expect("decode").expired_only);
    }

    #[test]
    fn list_ret_puts_next_marker_before_truncated_and_dumps_it_as_int() {
        // Corpus cls_rgw_gc_list_ret/07ac230e...: one default entry, "", truncated.
        let ret = ListRet {
            entries: vec![GcObjInfo::default()],
            next_marker: String::new(),
            truncated: true,
        };
        let wire = b"\x02\x01\x25\x00\x00\x00\x01\x00\x00\x00\x01\x01\x16\x00\x00\x00\x00\x00\x00\x00\x01\x01\x04\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01";
        assert_eq!(bytes(&ret), wire);
        assert_eq!(ListRet::decode(&mut &wire[..], 0).expect("decode"), ret);
        assert!(json(&ret).ends_with(r#""next_marker":"","truncated":1}"#));
        // Version 1: entries then truncated, no marker.
        let v1 = b"\x01\x01\x05\x00\x00\x00\x00\x00\x00\x00\x01";
        let ret = ListRet::decode(&mut &v1[..], 0).expect("decode");
        assert!(ret.entries.is_empty() && ret.next_marker.is_empty() && ret.truncated);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline rgw::`
Expected: compile errors.

- [ ] **Step 3: Implement**

`rgw/types.rs`:

```rust
//! `cls_rgw_types.h`: the pieces every rgw-side class shares. This file
//! holds what the GC classes need; the bucket index, usage, lifecycle
//! and OLH types join it with their classes.

use bytes::{Buf, BufMut};
use rados::{Denc, RadosError, UTime, VersionedDenc, VersionedEncode};
use serde::Serialize;
use serde::ser::SerializeStruct;

/// `cls_rgw_obj_key` (`rgw_obj_index_key`): an object's name and, when it
/// is a version, its instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ObjKey {
    pub name: String,
    pub instance: String,
}

/// `cls_rgw_obj`: one object of a GC chain. Version 2 appended the full
/// key after the three strings version 1 wrote, so `key.name` is on the
/// wire twice. The dump calls `key.name` "oid" and `loc` "key".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Obj {
    pub pool: String,
    pub key: ObjKey,
    pub loc: String,
}

impl VersionedEncode for Obj {
    const MAX_DECODE_VERSION: u8 = 2;

    fn encoding_version(&self, _features: u64) -> u8 {
        2
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.pool.encode(buf, features)?;
        self.key.name.encode(buf, features)?;
        self.loc.encode(buf, features)?;
        self.key.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        let pool = String::decode(buf, features)?;
        let name = String::decode(buf, features)?;
        let loc = String::decode(buf, features)?;
        let key = if struct_v >= 2 {
            ObjKey::decode(buf, features)?
        } else {
            ObjKey {
                name,
                instance: String::new(),
            }
        };
        Ok(Self { pool, key, loc })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(Obj);

impl Serialize for Obj {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Obj", 4)?;
        state.serialize_field("pool", &self.pool)?;
        state.serialize_field("oid", &self.key.name)?;
        state.serialize_field("key", &self.loc)?;
        state.serialize_field("instance", &self.key.instance)?;
        state.end()
    }
}

/// `cls_rgw_obj_chain`: the objects one GC entry deletes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ObjChain {
    pub objs: Vec<Obj>,
}

/// `cls_rgw_gc_obj_info`: a GC entry: the tag naming it, the chain to
/// delete, and when it falls due. This is the payload of every `rgw_gc`
/// queue entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GcObjInfo {
    pub tag: String,
    pub chain: ObjChain,
    #[serde(serialize_with = "crate::dump::real_time")]
    pub time: UTime,
}
```

`rgw/gc.rs`:

```rust
//! The `rgw` class's GC request and reply structs (`cls_rgw_ops.h`).
//! The omap-era GC methods send them; the `rgw_gc` class reuses
//! [`SetEntryOp`], [`ListOp`] and [`ListRet`] for its queue.

use bytes::{Buf, BufMut};
use rados::{Denc, RadosError, VersionedDenc, VersionedEncode};
use serde::Serialize;

use crate::rgw::types::GcObjInfo;

/// `cls_rgw_gc_set_entry_op`: an entry due `expiration_secs` from now.
/// The dump nests the entry as `obj_info`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct SetEntryOp {
    pub expiration_secs: u32,
    #[serde(rename = "obj_info")]
    pub info: GcObjInfo,
}

/// `cls_rgw_gc_defer_entry_op`: the omap-era defer, by tag. The `rgw_gc`
/// class defers with its own request carrying the whole entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct DeferEntryOp {
    pub expiration_secs: u32,
    pub tag: String,
}

/// `cls_rgw_gc_list_op`: up to `max` entries after `marker`. Version 2
/// added `expired_only`, which the C++ constructor defaults to true; a
/// version-1 request means the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListOp {
    pub marker: String,
    pub max: u32,
    pub expired_only: bool,
}

impl Default for ListOp {
    fn default() -> Self {
        Self {
            marker: String::new(),
            max: 0,
            expired_only: true,
        }
    }
}

impl VersionedEncode for ListOp {
    const MAX_DECODE_VERSION: u8 = 2;

    fn encoding_version(&self, _features: u64) -> u8 {
        2
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.marker.encode(buf, features)?;
        self.max.encode(buf, features)?;
        self.expired_only.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        let marker = String::decode(buf, features)?;
        let max = u32::decode(buf, features)?;
        let expired_only = if struct_v >= 2 {
            bool::decode(buf, features)?
        } else {
            true
        };
        Ok(Self {
            marker,
            max,
            expired_only,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(ListOp);

/// `cls_rgw_gc_list_ret`. Version 2 put `next_marker` between the entries
/// and `truncated`; the dump prints `truncated` as an int.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ListRet {
    pub entries: Vec<GcObjInfo>,
    pub next_marker: String,
    #[serde(serialize_with = "crate::dump::bool_as_int")]
    pub truncated: bool,
}

impl VersionedEncode for ListRet {
    const MAX_DECODE_VERSION: u8 = 2;

    fn encoding_version(&self, _features: u64) -> u8 {
        2
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.entries.encode(buf, features)?;
        self.next_marker.encode(buf, features)?;
        self.truncated.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        let entries = Vec::<GcObjInfo>::decode(buf, features)?;
        let next_marker = if struct_v >= 2 {
            String::decode(buf, features)?
        } else {
            String::new()
        };
        let truncated = bool::decode(buf, features)?;
        Ok(Self {
            entries,
            next_marker,
            truncated,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(ListRet);

/// `cls_rgw_gc_remove_op`: the omap-era remove, by tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct RemoveOp {
    pub tags: Vec<String>,
}
```

`Cargo.toml`: `rgw = []`, in `default`. `lib.rs`: `#[cfg(feature =
"rgw")] pub mod rgw;` and `rgw` in the `mod call` gate (the module does
not call yet, but the gate lists every class feature so the rule stays
mechanical). `ci.yml`: `rgw` in the loop.

Dencoder: import `rados_cls::rgw::gc::{DeferEntryOp as RgwGcDeferEntryOp,
ListOp as RgwGcListOp, ListRet as RgwGcListRet, RemoveOp as
RgwGcRemoveOp, SetEntryOp as RgwGcSetEntryOp}` and
`rados_cls::rgw::types::{GcObjInfo, Obj as RgwObj, ObjChain as
RgwObjChain, ObjKey as RgwObjKey}`; arms:

```rust
        "cls_rgw_obj_key" => Some(type_info_denc::<RgwObjKey>()),
        "cls_rgw_obj" => Some(type_info_denc::<RgwObj>()),
        "cls_rgw_obj_chain" => Some(type_info_denc::<RgwObjChain>()),
        "cls_rgw_gc_obj_info" => Some(type_info_denc::<GcObjInfo>()),
        "cls_rgw_gc_set_entry_op" => Some(type_info_denc::<RgwGcSetEntryOp>()),
        "cls_rgw_gc_defer_entry_op" => Some(type_info_denc::<RgwGcDeferEntryOp>()),
        "cls_rgw_gc_list_op" => Some(type_info_denc::<RgwGcListOp>()),
        "cls_rgw_gc_list_ret" => Some(type_info_denc::<RgwGcListRet>()),
        "cls_rgw_gc_remove_op" => Some(type_info_denc::<RgwGcRemoveOp>()),
```

plus `list_types` lines; nine `TypeSpec`s in the corpus test.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline rgw:: && cargo check --workspace --all-targets --offline`
Expected: 8 passed, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/Cargo.toml rados-cls/src/dump.rs rados-cls/src/lib.rs rados-cls/src/rgw .github/workflows/ci.yml rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the rgw module with the GC entry types

cls_rgw_types.h's object key, GC chain and GC entry, and
cls_rgw_ops.h's GC request and reply structs, in the module the rgw
class's own methods will join. cls_rgw_obj version 2 appended the full
key after the version-1 strings, so the name is on the wire twice;
cls_rgw_gc_list_ret version 2 put next_marker before truncated;
cls_rgw_gc_list_op's expired_only defaults to true. The GC entry's time
dumps as operator<< streams a real_time: the calendar form with
microseconds and the zone's %z offset, +0000 where ceph-dencoder runs.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 4: The `rgw_gc` class

**Files:**
- Create: `rados-cls/src/rgw_gc.rs`.
- Modify: `rados-cls/Cargo.toml` (`rgw_gc = ["queue", "rgw"]`, in
  `default`), `rados-cls/src/lib.rs`, `.github/workflows/ci.yml`,
  `rados-dencoder/src/main.rs` (two arms), the corpus test (two
  `TypeSpec`s).

**Interfaces:**
- Consumes: `crate::queue::{get_capacity, get_capacity_op,
  decode_get_capacity}`; `crate::rgw::gc::{SetEntryOp, ListOp,
  ListRet}`; `crate::rgw::types::GcObjInfo`.
- Produces: `pub const CLASS: &str = "rgw_gc"`, `pub const
  LIST_DEFAULT_MAX: u32 = 128`; types `UrgentData`, `InitOp`,
  `RemoveEntriesOp`, `DeferEntryOp`; op constructors `init_op(u64, u64)`,
  `enqueue_op(u32, &GcObjInfo)`, `list_entries_op(&str, u32, bool)`,
  `remove_entries_op(u64)`, `defer_entry_op(u32, &GcObjInfo)`;
  `decode_list_entries(&OpReply) -> Result<ListRet>`; re-exports
  `get_capacity_op`, `decode_get_capacity`, `get_capacity` from `queue`;
  async `init`, `enqueue`, `list_entries`, `remove_entries`,
  `defer_entry`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use rados::osdclient::types::OpData;

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    #[test]
    fn urgent_data_dumps_each_tag_as_its_own_value() {
        // Corpus cls_rgw_gc_urgent_data/75025f3e...: empty map, 10, 0, 0.
        let u = UrgentData {
            num_urgent_data_entries: 10,
            ..UrgentData::default()
        };
        assert_eq!(
            bytes(&u),
            b"\x01\x01\x10\x00\x00\x00\x00\x00\x00\x00\x0a\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        );
        assert_eq!(
            json(&u),
            r#"{"urgent_data_map":{},"num_urgent_data_entries":10,"num_head_urgent_entries":0,"num_xattr_urgent_entries":0}"#
        );
        let mut u = UrgentData::default();
        u.urgent_data_map
            .insert("chain-1".to_owned(), UTime { sec: 7, nsec: 0 });
        assert_eq!(
            bytes(&u),
            b"\x01\x01\x23\x00\x00\x00\x01\x00\x00\x00\x07\x00\x00\x00chain-1\x07\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        );
        assert!(json(&u).starts_with(r#"{"urgent_data_map":{"chain-1":"chain-1"}"#));
        assert_eq!(UrgentData::decode(&mut &bytes(&u)[..], 0).expect("decode"), u);
    }

    #[test]
    fn init_remove_and_defer_are_plain_structs() {
        // Corpus cls_rgw_gc_queue_init_op/400582ba...: 344, 10.
        let i = InitOp {
            size: 344,
            num_deferred_entries: 10,
        };
        assert_eq!(
            bytes(&i),
            b"\x01\x01\x10\x00\x00\x00\x58\x01\x00\x00\x00\x00\x00\x00\x0a\x00\x00\x00\x00\x00\x00\x00"
        );
        assert_eq!(json(&i), r#"{"size":344,"num_deferred_entries":10}"#);
        let r = RemoveEntriesOp { num_entries: 2 };
        assert_eq!(bytes(&r), b"\x01\x01\x08\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00");
        let d = DeferEntryOp {
            expiration_secs: 10,
            info: GcObjInfo::default(),
        };
        assert_eq!(
            bytes(&d),
            b"\x01\x01\x20\x00\x00\x00\x0a\x00\x00\x00\x01\x01\x16\x00\x00\x00\x00\x00\x00\x00\x01\x01\x04\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
        );
        assert_eq!(DeferEntryOp::decode(&mut &bytes(&d)[..], 0).expect("decode"), d);
    }

    #[test]
    fn ops_name_the_class_and_method() {
        let op = init_op(4096, 10).expect("op");
        assert!(op.indata.starts_with(b"rgw_gcrgw_gc_queue_init"));
        assert!(matches!(
            op.op_data,
            OpData::Call {
                class_len: 6,
                method_len: 17,
                ..
            }
        ));
        let op = enqueue_op(0, &GcObjInfo::default()).expect("op");
        assert!(op.indata.starts_with(b"rgw_gcrgw_gc_queue_enqueue"));
        let op = list_entries_op("", 0, true).expect("op");
        assert!(op.indata.starts_with(b"rgw_gcrgw_gc_queue_list_entries"));
        let op = remove_entries_op(1).expect("op");
        assert!(op.indata.starts_with(b"rgw_gcrgw_gc_queue_remove_entries"));
        let op = defer_entry_op(10, &GcObjInfo::default()).expect("op");
        assert!(op.indata.starts_with(b"rgw_gcrgw_gc_queue_update_entry"));
        // get_capacity is the queue class's method on the same object.
        let op = get_capacity_op().expect("op");
        assert!(op.indata.starts_with(b"queuequeue_get_capacity"));
    }

    #[test]
    fn decoder_unwraps_the_list_reply() {
        let ret = ListRet {
            entries: vec![GcObjInfo::default()],
            next_marker: "0/1037".to_owned(),
            truncated: true,
        };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&ret, 0).expect("encode"),
        };
        assert_eq!(decode_list_entries(&reply).expect("decode"), ret);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline rgw_gc`
Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
//! The `rgw_gc` object class (`cls_rgw_gc_client.h`): RGW's garbage
//! collection queue. Entries are [`GcObjInfo`]s in a `queue` ring whose
//! head carries [`UrgentData`], the deferrals. `rgw_gc_queue_init`,
//! `rgw_gc_queue_enqueue`, `rgw_gc_queue_remove_entries` and
//! `rgw_gc_queue_update_entry` (the defer) are RD|WR methods;
//! `rgw_gc_queue_list_entries` is RD. Capacity is read with the `queue`
//! class's own method.
//!
//! What `cls_rgw_gc.cc` does with them: an entry falls due at enqueue
//! time plus `expiration_secs`; a listing skips any entry whose tag is
//! deferred to a later time and, with `expired_only`, any entry not yet
//! due; a removal walks the first `num_entries` entries the class does
//! not treat as deferred and moves the front past them, dropping a
//! deferral whose time equals its entry's; a defer records the tag's
//! new due time without touching the ring and is `ENOSPC` once more
//! tags are deferred than the init allowed.

use std::collections::BTreeMap;

use bytes::Bytes;
use rados::osdclient::error::Result;
use rados::osdclient::{IoCtx, OSDOp, OpReply};
use rados::{Denc, UTime, VersionedDenc};
use serde::Serialize;
use serde::ser::{SerializeMap, SerializeStruct};

use crate::call;
use crate::rgw::gc::{ListOp, ListRet, SetEntryOp};
use crate::rgw::types::GcObjInfo;

pub use crate::queue::{decode_get_capacity, get_capacity, get_capacity_op};

pub const CLASS: &str = "rgw_gc";

/// `GC_LIST_DEFAULT_MAX`: what the class uses when a list or remove asks
/// for zero.
pub const LIST_DEFAULT_MAX: u32 = 128;

/// `cls_rgw_gc_urgent_data`: deferred tags and their new due times, plus
/// the allowance from the init and how many live in the head and in the
/// `cls_queue_urgent_data` xattr. The class inits the head with no room
/// for them, so every deferral lands in the xattr. Ceph keeps the map
/// unordered, so a re-encode of two or more entries may order them
/// differently from the bytes Ceph wrote. The dump prints each tag as its
/// own value.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct UrgentData {
    pub urgent_data_map: BTreeMap<String, UTime>,
    pub num_urgent_data_entries: u32,
    pub num_head_urgent_entries: u32,
    pub num_xattr_urgent_entries: u32,
}

struct TagsAsValues<'a>(&'a BTreeMap<String, UTime>);

impl Serialize for TagsAsValues<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for tag in self.0.keys() {
            map.serialize_entry(tag, tag)?;
        }
        map.end()
    }
}

impl Serialize for UrgentData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("UrgentData", 4)?;
        state.serialize_field("urgent_data_map", &TagsAsValues(&self.urgent_data_map))?;
        state.serialize_field("num_urgent_data_entries", &self.num_urgent_data_entries)?;
        state.serialize_field("num_head_urgent_entries", &self.num_head_urgent_entries)?;
        state.serialize_field("num_xattr_urgent_entries", &self.num_xattr_urgent_entries)?;
        state.end()
    }
}

/// `cls_rgw_gc_queue_init_op`: `size` usable bytes and how many tags may
/// be deferred at once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct InitOp {
    pub size: u64,
    pub num_deferred_entries: u64,
}

/// `cls_rgw_gc_queue_remove_entries_op`. Not registered with
/// `ceph-dencoder`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct RemoveEntriesOp {
    pub num_entries: u64,
}

/// `cls_rgw_gc_queue_defer_entry_op`: the entry to defer, whole, and its
/// new delay. Not registered with `ceph-dencoder`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct DeferEntryOp {
    pub expiration_secs: u32,
    pub info: GcObjInfo,
}

/// `cls_rgw_gc_queue_init`: a ring of `size` usable bytes that may hold
/// `num_deferred_entries` deferrals. `EEXIST` if the object already
/// holds a ring.
pub fn init_op(size: u64, num_deferred_entries: u64) -> Result<OSDOp> {
    call::op(
        CLASS,
        "rgw_gc_queue_init",
        &InitOp {
            size,
            num_deferred_entries,
        },
    )
}

/// `cls_rgw_gc_queue_enqueue`: append `info`, due `expiration_secs` from
/// now (the class sets `info.time` itself). `ENOSPC` when it does not fit.
pub fn enqueue_op(expiration_secs: u32, info: &GcObjInfo) -> Result<OSDOp> {
    call::op(
        CLASS,
        "rgw_gc_queue_enqueue",
        &SetEntryOp {
            expiration_secs,
            info: info.clone(),
        },
    )
}

/// `cls_rgw_gc_queue_list_entries`: up to `max` entries (zero means
/// [`LIST_DEFAULT_MAX`]) after `marker`, only the due ones when
/// `expired_only`, never one deferred to a later time. Decode the reply
/// with [`decode_list_entries`]; its `next_marker` is set only when
/// `truncated`.
pub fn list_entries_op(marker: &str, max: u32, expired_only: bool) -> Result<OSDOp> {
    call::op(
        CLASS,
        "rgw_gc_queue_list_entries",
        &ListOp {
            marker: marker.to_owned(),
            max,
            expired_only,
        },
    )
}

/// Decode the reply to [`list_entries_op`].
pub fn decode_list_entries(reply: &OpReply) -> Result<ListRet> {
    call::decode(reply)
}

/// `cls_rgw_gc_queue_remove_entries`: drop the first `num_entries`
/// entries (zero means [`LIST_DEFAULT_MAX`]) that are not deferred past
/// their time, and whatever deferred entries lie among them.
pub fn remove_entries_op(num_entries: u64) -> Result<OSDOp> {
    call::op(
        CLASS,
        "rgw_gc_queue_remove_entries",
        &RemoveEntriesOp { num_entries },
    )
}

/// `cls_rgw_gc_queue_defer_entry`: make `info`'s tag due `expiration_secs`
/// from now without re-queueing it. `ENOSPC` once more tags are deferred
/// than the init allowed; deferring a tag again only moves its time.
pub fn defer_entry_op(expiration_secs: u32, info: &GcObjInfo) -> Result<OSDOp> {
    call::op(
        CLASS,
        "rgw_gc_queue_update_entry",
        &DeferEntryOp {
            expiration_secs,
            info: info.clone(),
        },
    )
}

/// Lay out the GC ring on `oid`; see [`init_op`].
pub async fn init(ioctx: &IoCtx, oid: &str, size: u64, num_deferred_entries: u64) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "rgw_gc_queue_init",
        &InitOp {
            size,
            num_deferred_entries,
        },
    )
    .await?;
    Ok(())
}

/// Append `info` to the GC ring on `oid`; see [`enqueue_op`].
pub async fn enqueue(ioctx: &IoCtx, oid: &str, expiration_secs: u32, info: &GcObjInfo) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "rgw_gc_queue_enqueue",
        &SetEntryOp {
            expiration_secs,
            info: info.clone(),
        },
    )
    .await?;
    Ok(())
}

/// List GC entries on `oid`; see [`list_entries_op`].
pub async fn list_entries(
    ioctx: &IoCtx,
    oid: &str,
    marker: &str,
    max: u32,
    expired_only: bool,
) -> Result<ListRet> {
    let out = call::exec(
        ioctx,
        oid,
        CLASS,
        "rgw_gc_queue_list_entries",
        &ListOp {
            marker: marker.to_owned(),
            max,
            expired_only,
        },
    )
    .await?;
    call::decode_bytes(out)
}

/// Drop the first `num_entries` GC entries on `oid`; see
/// [`remove_entries_op`].
pub async fn remove_entries(ioctx: &IoCtx, oid: &str, num_entries: u64) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "rgw_gc_queue_remove_entries",
        &RemoveEntriesOp { num_entries },
    )
    .await?;
    Ok(())
}

/// Defer `info`'s tag on `oid`; see [`defer_entry_op`].
pub async fn defer_entry(ioctx: &IoCtx, oid: &str, expiration_secs: u32, info: &GcObjInfo) -> Result<()> {
    call::exec(
        ioctx,
        oid,
        CLASS,
        "rgw_gc_queue_update_entry",
        &DeferEntryOp {
            expiration_secs,
            info: info.clone(),
        },
    )
    .await?;
    Ok(())
}
```

If `Bytes` ends up unused, drop the import. `Cargo.toml`: `rgw_gc =
["queue", "rgw"]`, in `default`. `lib.rs`: `#[cfg(feature = "rgw_gc")]
pub mod rgw_gc;` and `rgw_gc` in the gate. `ci.yml`: `rgw_gc` in the loop
(the loop now reads `"" queue refcount rgw rgw_gc user version`).

Dencoder: import `rados_cls::rgw_gc::{InitOp as RgwGcQueueInitOp,
UrgentData as RgwGcUrgentData}`; arms
`"cls_rgw_gc_urgent_data"` and `"cls_rgw_gc_queue_init_op"`; two
`list_types` lines; two `TypeSpec`s.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline rgw_gc && cargo check --workspace --all-targets --offline`
Expected: 4 passed, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/Cargo.toml rados-cls/src/lib.rs rados-cls/src/rgw_gc.rs .github/workflows/ci.yml rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the rgw_gc class

Mirrors cls_rgw_gc_client.h: init, enqueue, list_entries,
remove_entries and defer_entry on the "rgw_gc" class, with capacity
read through the queue class's method as the C++ does. Entries are
cls_rgw_gc_obj_info in a queue ring whose head carries
cls_rgw_gc_urgent_data, the deferrals. cls_rgw_gc.cc dates an entry at
enqueue plus expiration_secs, hides a tag deferred to a later time
from listings, and refuses a defer past the init's allowance with
ENOSPC.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 5: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_queue_gc.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both `rados-cls` loops
  gain `cls_queue_gc`).

**Interfaces:**
- Consumes: Tasks 2 to 4; `rados/tests/common/mod.rs` via `#[path]`;
  `IoCtx::create`.
- Produces: four `#[ignore]` tests.

- [ ] **Step 1: Write the tests**

```rust
//! Cluster tests for the queue and rgw_gc classes. Run with:
//!   CEPH_CONF=... cargo test -p rados-cls --test cls_queue_gc -- --ignored --nocapture

#[path = "../../rados/tests/common/mod.rs"]
mod common;

use bytes::Bytes;
use common::create_ioctx;
use rados::OSDClientError;
use rados_cls::rgw::types::{GcObjInfo, Obj, ObjChain, ObjKey};
use rados_cls::{queue, rgw_gc};

const EEXIST: i32 = 17;
const EINVAL: i32 = 22;
const ENOSPC: i32 = 28;

fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("{prefix}-{}-{nanos}", std::process::id())
}

fn is_osd_error(err: &OSDClientError, errno: i32) -> bool {
    matches!(err, OSDClientError::OSDError { code, .. } if *code == -errno)
}

fn payloads(ret: &queue::ListRet) -> Vec<&[u8]> {
    ret.entries.iter().map(|e| e.data.as_ref()).collect()
}

fn markers(ret: &queue::ListRet) -> Vec<&str> {
    ret.entries.iter().map(|e| e.marker.as_str()).collect()
}

fn tags(ret: &rgw_gc::ListRet) -> Vec<&str> {
    ret.entries.iter().map(|e| e.tag.as_str()).collect()
}

/// test_cls_rgw_gc.cc's create_obj: two objects per chain.
fn chain(i: u32) -> GcObjInfo {
    let obj = |j: u32| Obj {
        pool: format!("pool-{i}.{j}"),
        key: ObjKey {
            name: format!("oid-{i}.{j}"),
            instance: String::new(),
        },
        loc: format!("loc-{i}.{j}"),
    };
    GcObjInfo {
        tag: format!("chain-{i}"),
        chain: ObjChain {
            objs: vec![obj(1), obj(2)],
        },
        ..GcObjInfo::default()
    }
}

#[tokio::test]
#[ignore]
async fn queue_ring_init_enqueue_list_remove() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-queue-ring");
    ioctx.create(&oid, true).await.expect("create");

    queue::init(&ioctx, &oid, 1024).await.expect("init");
    assert_eq!(queue::get_capacity(&ioctx, &oid).await.expect("capacity"), 1024);
    let err = queue::init(&ioctx, &oid, 1024).await.expect_err("second init");
    assert!(is_osd_error(&err, EEXIST), "{err:?}");

    // Empty: nothing, not truncated, the next marker is the front.
    let ret = queue::list_entries(&ioctx, &oid, "", 10, "").await.expect("list");
    assert!(ret.entries.is_empty() && !ret.is_truncated);
    assert_eq!(ret.next_marker, "0/1024");

    // Each entry costs ten bytes of magic and length plus its payload.
    queue::enqueue(
        &ioctx,
        &oid,
        vec![
            Bytes::from_static(b"one"),
            Bytes::from_static(b"two"),
            Bytes::from_static(b"three"),
        ],
    )
    .await
    .expect("enqueue");
    let page = queue::list_entries(&ioctx, &oid, "", 2, "").await.expect("list");
    assert_eq!(payloads(&page), [b"one".as_ref(), b"two"]);
    assert_eq!(markers(&page), ["0/1024", "0/1037"]);
    assert!(page.is_truncated);
    assert_eq!(page.next_marker, "0/1050");
    let rest = queue::list_entries(&ioctx, &oid, &page.next_marker, 10, "")
        .await
        .expect("list");
    assert_eq!(payloads(&rest), [b"three".as_ref()]);
    assert_eq!(markers(&rest), ["0/1050"]);
    assert!(!rest.is_truncated);
    assert_eq!(rest.next_marker, "0/1065", "the tail");

    // end_marker is exclusive.
    let until = queue::list_entries(&ioctx, &oid, "", u64::MAX, "0/1050")
        .await
        .expect("list");
    assert_eq!(payloads(&until), [b"one".as_ref(), b"two"]);

    // Remove up to (not including) the third entry, then everything.
    queue::remove_entries(&ioctx, &oid, "0/1050")
        .await
        .expect("remove");
    let ret = queue::list_entries(&ioctx, &oid, "", 10, "").await.expect("list");
    assert_eq!(payloads(&ret), [b"three".as_ref()]);
    let err = queue::remove_entries(&ioctx, &oid, "0/1")
        .await
        .expect_err("behind the front");
    assert!(is_osd_error(&err, EINVAL), "{err:?}");
    queue::remove_entries(&ioctx, &oid, "0/1065")
        .await
        .expect("remove to the tail");
    let ret = queue::list_entries(&ioctx, &oid, "", 10, "").await.expect("list");
    assert!(ret.entries.is_empty() && !ret.is_truncated);

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn queue_enqueue_past_capacity_is_enospc() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-queue-full");
    ioctx.create(&oid, true).await.expect("create");

    queue::init(&ioctx, &oid, 32).await.expect("init");
    let err = queue::enqueue(&ioctx, &oid, vec![Bytes::from(vec![0u8; 40])])
        .await
        .expect_err("does not fit");
    assert!(is_osd_error(&err, ENOSPC), "{err:?}");
    queue::enqueue(&ioctx, &oid, vec![Bytes::from(vec![0u8; 22])])
        .await
        .expect("exactly fills the ring");
    let err = queue::enqueue(&ioctx, &oid, vec![Bytes::new()])
        .await
        .expect_err("a full ring");
    assert!(is_osd_error(&err, ENOSPC), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn rgw_gc_enqueue_list_remove() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-rgw-gc");
    ioctx.create(&oid, true).await.expect("create");

    rgw_gc::init(&ioctx, &oid, 4096, 10).await.expect("init");
    assert_eq!(rgw_gc::get_capacity(&ioctx, &oid).await.expect("capacity"), 4096);
    let ret = rgw_gc::list_entries(&ioctx, &oid, "", 10, false)
        .await
        .expect("list");
    assert!(ret.entries.is_empty() && !ret.truncated);

    // Two due now, one due in five minutes.
    for i in 0..3 {
        let secs = if i == 2 { 300 } else { 0 };
        rgw_gc::enqueue(&ioctx, &oid, secs, &chain(i))
            .await
            .expect("enqueue");
    }
    let due = rgw_gc::list_entries(&ioctx, &oid, "", 10, true)
        .await
        .expect("list expired");
    assert_eq!(tags(&due), ["chain-0", "chain-1"]);
    assert!(!due.truncated);
    assert_eq!(due.entries[0].chain.objs[1].key.name, "oid-0.2");

    // Paging: next_marker is set only when truncated.
    let first = rgw_gc::list_entries(&ioctx, &oid, "", 1, false)
        .await
        .expect("list");
    assert_eq!(tags(&first), ["chain-0"]);
    assert!(first.truncated && !first.next_marker.is_empty());
    let second = rgw_gc::list_entries(&ioctx, &oid, &first.next_marker, 1, false)
        .await
        .expect("list");
    assert_eq!(tags(&second), ["chain-1"]);
    assert!(second.truncated);
    let third = rgw_gc::list_entries(&ioctx, &oid, &second.next_marker, 1, false)
        .await
        .expect("list");
    assert_eq!(tags(&third), ["chain-2"]);
    assert!(!third.truncated && third.next_marker.is_empty());

    rgw_gc::remove_entries(&ioctx, &oid, 2).await.expect("remove");
    let left = rgw_gc::list_entries(&ioctx, &oid, "", 10, false)
        .await
        .expect("list");
    assert_eq!(tags(&left), ["chain-2"]);

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn rgw_gc_defer_entry() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-rgw-gc-defer");
    ioctx.create(&oid, true).await.expect("create");

    rgw_gc::init(&ioctx, &oid, 4096, 1).await.expect("init");
    rgw_gc::enqueue(&ioctx, &oid, 0, &chain(0)).await.expect("enqueue");
    rgw_gc::enqueue(&ioctx, &oid, 0, &chain(1)).await.expect("enqueue");

    // A deferred tag disappears from listings until its new time.
    rgw_gc::defer_entry(&ioctx, &oid, 600, &chain(1))
        .await
        .expect("defer");
    let ret = rgw_gc::list_entries(&ioctx, &oid, "", 10, false)
        .await
        .expect("list");
    assert_eq!(tags(&ret), ["chain-0"]);

    // Deferring it again only moves its time; a second tag is one too many.
    rgw_gc::defer_entry(&ioctx, &oid, 900, &chain(1))
        .await
        .expect("defer again");
    let err = rgw_gc::defer_entry(&ioctx, &oid, 600, &chain(0))
        .await
        .expect_err("past the allowance");
    assert!(is_osd_error(&err, ENOSPC), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}
```

`test-with-ceph.yml`: both loops read `for test in cls_version_refcount
cls_user cls_queue_gc`.

- [ ] **Step 2: Run against the cluster**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_queue_gc -- --ignored --nocapture`
Expected: 4 passed. (`queue_enqueue` accepts an entry whose end lands
exactly on `queue_size` and then wraps the tail to the head with `gen`
plus one, which is the "exactly full" state the next enqueue refuses;
verified in `cls_queue_src.cc`.)

- [ ] **Step 3: Commit**

```bash
git add rados-cls/tests/cls_queue_gc.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: queue and rgw_gc cluster tests

Pins against Ceph v19: the first entry sits at 0/1024 and each costs
ten bytes plus its payload, a list end_marker is exclusive, a remove
end_marker behind the front is EINVAL, a payload past the free space
is ENOSPC and a second init is EEXIST; for rgw_gc, expired_only hides
an entry until it is due, next_marker is set only when truncated,
remove_entries counts from the front, a deferred tag leaves listings,
and a defer past the init's allowance is ENOSPC.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 6: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6, with: the per-commit proof from the branch base; the
container clippy run over the workspace and over every single class
feature; the corpus loop over the twenty registered names (nine
`cls_queue_*`, `cls_rgw_obj_key`, `cls_rgw_obj`, `cls_rgw_obj_chain`,
`cls_rgw_gc_obj_info`, `cls_rgw_gc_set_entry_op`,
`cls_rgw_gc_defer_entry_op`, `cls_rgw_gc_list_op`, `cls_rgw_gc_list_ret`,
`cls_rgw_gc_remove_op`, `cls_rgw_gc_urgent_data`,
`cls_rgw_gc_queue_init_op`; expect `cls_queue_enqueue_op` to report only
for the 19.2.0 archive); the cluster suites `cls_version_refcount`,
`cls_user`, `cls_queue_gc`; the PR body:

```
**Motivation.** RGW's garbage collector keeps its work in the `rgw_gc` object class, a queue of GC entries with deferrals in the head; a Rust RGW needs `cls_rgw_gc_client.h` and the `queue` class under it.

**What changed.** Three modules of `rados-cls`: `queue` (`cls_queue_client.h`), `rgw` seeded with the GC entry types and the `cls_rgw_gc_*` structs of `cls_rgw_ops.h`, and `rgw_gc` (`cls_rgw_gc_client.h`); the version-2 shapes hand-written (`cls_queue_list_op`, `cls_rgw_obj`, `cls_rgw_gc_list_op`, `cls_rgw_gc_list_ret`); unit, corpus and cluster tests.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Then merge when green and open the upstream PR from the same branch.

---

## Roadmap for later plans

`cls-rgw-types` (extends `rgw::types`); `cls-rgw-bucket-index`;
`cls-rgw-gc` (the `rgw` class's omap-era GC methods, in `rgw::gc`);
`cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh`; `watch-notify`. Deferred
minors carried forward: `cls_queue_get_stats_ret` is dead in Ceph and
left out; `cls_rgw_gc_queue_remove_entries_op` and
`cls_rgw_gc_queue_defer_entry_op` have no `ceph-dencoder` registration
upstream; plan 4's and plan 3's deferred minors.
