# rados-rs RGW MVP, plan 14 of N: `cls-2pc-queue`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `2pc_queue` object class (`cls_2pc_queue_client.h`:
init, get capacity, get topic stats, reserve, commit, abort, list
reservations, list entries, remove entries, expire reservations) to
`rados-cls`, with the eight `ceph-dencoder`-registered types compared
against the corpus, the two unregistered ones pinned from corpus bytes,
a head reader that exposes the class's own bookkeeping, and cluster
tests against Ceph v19.2.2 that pin its semantics and its quirks,
including the capacity leak Squid ships.

**Architecture:** One module `two_pc_queue` behind feature
`two_pc_queue = ["queue"]` (a Rust identifier cannot start with a
digit; the class name stays `2pc_queue`). The class is a `queue` ring
whose head carries `cls_2pc_urgent_data` in its urgent data, so the
module reuses plan 5's `queue` types (`InitOp`, `ListOp`, `ListRet`,
`Entry`, `Marker`, `Head`, `GetCapacityRet`) and decoders wherever the
class forwards to `cls_queue`, and adds 2pc structs, `OSDOp`
constructors naming class `2pc_queue`, and async free functions over
`IoCtx`. `reserve` is a writing method that replies, so it goes through
`call::exec_returnvec` (plan 4's `reset_stats2` precedent). `queue`
gains the one `cls_queue` type the class adds (`cls_queue_get_stats_ret`)
and the entry-overhead constant.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-2pc-queue`: init, get capacity, reserve (a writing method that
replies, so `RETURNVEC`), commit, abort, list entries, list
reservations, remove entries, expire reservations; what `rgw_notify.cc`
does with them." The goal's canon (rgw-go's `docs/exclusions.md`) keeps
persistent notifications at the RADOS layer and excludes only Kafka and
AMQP delivery. `get_topic_stats` is not in the spec's list; it is here
because `radosgw-admin topic stats` calls it and it is one small RD
method. "What `rgw_notify.cc` does with them" is the module doc's RGW
section (Task 2): facts the driver mirrors, not code.

Source of every Ceph fact below: `.superpowers/research/cls-2pc-queue.md`
(cited as "report §N"), whose citations are `S:` = `git show
v19.2.2:<path>` and `M:` = ceph `main` at `7ed73efc1be`; paths without a
directory are under `src/cls/2pc_queue/`.

## Global Constraints

Plans 3 to 11's and plan 13's Global Constraints apply unchanged (Squid
floor; `#[denc(crate = "rados")]` on every derive; no `unwrap`/`expect`
on production paths; a feature joins `default` in the commit that adds
its module; a `dump` helper or a `call` gate widens in the commit of its
first user; gates, container fmt/clippy, push after every gate, merge
when green then an upstream PR; offline builds with the scratchpad
`CARGO_HOME`; no new dependencies). Plus:

- Branch `cls-2pc-queue` is based on the fork's `main` after plan 13
  (`cls-lock`) merges. Execution order is 11, 13, 14, 15, then 12. At
  that base `call::exec_returnvec` is gated `#[cfg(feature = "user")]`
  with the doc line "Only the `user` class has such a method so far";
  `dump::real_time`/`iso_utc` are gated `feature = "rgw"`,
  `civil_from_days` and the `use rados::UTime` line `any(feature =
  "user", feature = "rgw")`, the `mod dump;` line `any(feature =
  "refcount", feature = "rgw", feature = "user")`. Task 0 confirms; if
  plan 13 changed any of them, widen what is there by
  `feature = "two_pc_queue"`.
- Commit messages: subject `two_pc_queue: ...`, a body of what and why,
  then exactly these two trailers:
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s`.
- Methods (`S:cls_2pc_queue_const.h:5-14`, registration
  `S:cls_2pc_queue.cc:664-675`; `main` has the same ten names and
  flags). Class `2pc_queue`:

  | Method | Flags | Request | Reply |
  |---|---|---|---|
  | `2pc_queue_init` | RD\|WR | `queue::InitOp` (only `queue_size` set) | none |
  | `2pc_queue_get_capacity` | RD | none | `queue::GetCapacityRet` |
  | `2pc_queue_get_topic_stats` | RD | none | `queue::GetStatsRet` (new) |
  | `2pc_queue_reserve` | RD\|WR | `ReserveOp` | `ReserveRet`, needs `RETURNVEC` |
  | `2pc_queue_commit` | RD\|WR | `CommitOp` | none |
  | `2pc_queue_abort` | RD\|WR | `AbortOp` | none |
  | `2pc_queue_list_reservations` | RD | none | `ReservationsRet` |
  | `2pc_queue_list_entries` | RD | `queue::ListOp` | `queue::ListRet` |
  | `2pc_queue_remove_entries` | RD\|WR | `RemoveOp` (the class also accepts a v1 `queue::RemoveOp`) | none |
  | `2pc_queue_expire_reservations` | RD\|WR | `ExpireOp` | none |

  Init, capacity and list forward to `cls_queue` (`S:cls_2pc_queue.cc:22-56`,
  `:554-579`); there is no enqueue without a reservation. The object is
  also readable by the plain `queue` class (upstream's `GetCapacity` test).
- `reserve` is the only 2pc write with a reply: `cls_2pc_queue_reserve_ret`,
  10 bytes, under the 64-byte `osd_max_write_op_reply_len` default
  (`S:src/common/options/global.yaml.in:3723-3729`, enforced
  `S:src/osd/PrimaryLogPG.cc:4211-4229`). RGW always passes
  `librados::OPERATION_RETURNVEC` (`S:src/rgw/driver/rados/rgw_notify.cc:1128-1131`,
  `:1209-1212`, `:582-583`), and the C++ header warns the op cannot be
  batched with other read/write ops (`S:cls_2pc_queue_client.h:49-55`).
  The async `reserve` uses `call::exec_returnvec`; `reserve_op`'s doc
  says the compound needs `OpBuilder::returnvec()` and should hold only
  that op. The RD replies (list, reservations, stats, capacity) are not
  capped.
- Wire facts (report §2): `ceph::coarse_real_time` is `rados::UTime` on
  the wire (u32 sec, u32 nsec) and `coarse_real_time::min()` is the
  epoch; every timestamp dumps through `dump_stream(...) << t`, which is
  plan 5's `dump::real_time` (`YYYY-MM-DDTHH:MM:SS.uuuuuu+0000`).
  `cls_2pc_reservations` is an `unordered_map<u32, cls_2pc_reservation>`
  encoded as `u32 count, (u32 id, reservation)*`; the Rust side is
  `BTreeMap<u32, Reservation>` (the `rgw_gc::UrgentData` precedent), so
  a re-encode of two or more reservations orders them ascending where
  Ceph wrote hash order, and the dump is ascending where `ceph-dencoder`
  prints libstdc++ bucket order (neither wire order nor sorted for 11 or
  more entries, reverse wire order below that; report §2 "Corpus
  directories"). `cls_2pc_urgent_data` and
  `cls_2pc_queue_reservations_ret` are therefore `is_exception` rows in
  the corpus table: multi-entry samples report as format differences,
  not failures, and 0- and 1-entry samples still compare exactly.
  `bl_data_vec` dumps as base64 strings (`encode_json` of a
  `bufferlist`) through the workspace's existing `base64` crate (0.21,
  already compiled into every build through `rados`): no new crate,
  download or lockfile entry.
- The floor rule (plan 5): every hand-written `decode_content` calls
  `rados::check_min_version!` at the version v19 writes, except where a
  default corpus archive holds an older sample. Applied here, with the
  release hints found by `git log -S` and `git tag --contains` in the
  Ceph tree (not taken from the report, whose shape section proposed
  "Reef v18+", the archive rather than the first writer):
  - `Reservation` and `UrgentData`: v19 writes 2, but the 18.2.0 corpus
    holds version-1 samples (`cls_2pc_reservation/fff84520…`,
    `cls_2pc_urgent_data/ff4ac206…`, and v1 reservations inside
    `cls_2pc_queue_reservations_ret`), so they decode version 1 and
    floor at 1 with hint `"Pacific v16+"`: version 1 is the class's
    first form, commit `4fba777a1d1` ("cls/queue: add 2-phase-commit
    queue implementation"), first tag v16.1.0. The type docs name the
    18.2.0 archive as the reason. Version 2 (`entries`,
    `committed_entries`) came with `9d64b3f3e6e`, first tag v19.0.0.
  - `RemoveOp`: floors at 2 with hint `"Squid v19+"` through the
    derive's `min_version`: `10addc64853` added the struct with
    `entries_to_remove` at version 1 and `855098f191e` made it version 2
    with the field behind `struct_v > 1`; both first tagged v19.0.0. A
    version-1 request on the wire is byte-identical to
    `cls_queue_remove_op` (`queue::RemoveOp`), which the class still
    accepts.
  - `UrgentData`: `MAX_DECODE_VERSION` 3 (main, v19.2.4 and v20.2.3
    write `ENCODE_START(3,1)` with the same layout; v3 only records that
    `reserved_size` is trustworthy, `M:cls_2pc_queue_types.h:79-100`);
    it encodes version 2, what a v19.2.2 OSD writes.
  - The other types are derive-based version 1.
- Timestamps in reservations are the OSD's `coarse_real_clock::now()`
  (`S:cls_2pc_queue.cc:145`, `:181`); expiry compares them with a
  client-supplied time, so cluster tests take thresholds from
  `list_reservations`, never from the client clock.
- Every test object is created in the same compound as its init:
  `OpBuilder::new().create(true).op(two_pc_queue::init_op(size)?)`, as
  RGW does; the cluster is the local v19.2.2 (`CEPH_CONF=/tmp/ceph/ceph.conf`,
  pool `test-pool`), and every test removes what it created.

## Review Focus

1. `RETURNVEC`: `2pc_queue_reserve` is RD|WR and returns the id in the
   write's out-data, which the OSD clears unless the request carries
   `CEPH_OSD_FLAG_RETURNVEC`; `reserve` uses `call::exec_returnvec`
   (gate widened to `any(feature = "user", feature = "two_pc_queue")`),
   the reply is 10 bytes under the 64-byte cap, and without the flag
   the reservation is made but its id is lost. Pinned in Task 2
   (`decode_reserve` of the 10-byte reply and of an empty one) and Task
   3 (`reserve_without_returnvec_loses_the_id`).
2. The reservation map: `BTreeMap<u32, Reservation>`, ascending on the
   wire and in the dump where Ceph uses libstdc++ hash order, handled by
   two `is_exception` corpus rows with the reason in a comment;
   `Reservation` does not dump `entries`, `UrgentData` does not dump
   `committed_entries`; both decode Reef's version 1 (floor at 1,
   "Pacific v16+", because of the 18.2.0 archive) and `UrgentData`
   decodes `main`'s version 3 while writing 2. Pinned in Task 1.
3. Accounting, including Squid's leak: `reserve` adds `size +
   10·entries` to `reserved_size` (`S:cls_2pc_queue.cc:123`, `:137`),
   but on v19.2.2 `commit`, `abort` and `expire` subtract only `size`
   (`:300`, `:386`/`:396`→`:401`, `:493`/`:522`→`:543`), so ten bytes
   per reserved entry are lost for good (fixed by `00ad83d3ab2` and
   `7f4eaee30cb` on main, backported to v19.2.4 via `a77d096f719` and
   to v20.2.3; `UrgentData` version 3 marks a fixed writer). No repair
   helper: `reserved_size` is never taken from a request, so a client
   cannot repair it on a Squid OSD. `commit` adds the reservation's
   `entries` (not the payload count) to `committed_entries`;
   `remove_entries` subtracts `entries_to_remove`, or, when that is 0,
   the number of entries the class counts up to `end_marker`. Pinned in
   Task 3 (`remove_entries_and_committed_entries`) and Task 4
   (`squid_leaks_the_entry_overhead`, which asserts the leak when the
   head's urgent data is version 2 and its absence at version 3).
4. Server quirks, each documented on the item it affects: ids start at
   1, are `u32`, and wrap to `NO_ID`; `EAGAIN` on an id collision
   (reachable only after a wrap; documented, not pinned); `ENOSPC` iff
   `size + reserved_size + 10·entries > free bytes`; abort of an unknown
   id is success; expire removes `timestamp < stale_time` strictly and
   writes nothing when nothing expired; the 785th v2 reservation spills
   into the `cls_queue_urgent_data` xattr (27 + 30·n bytes against
   23552) and `has_xattrs` is never cleared; on Squid, `commit` of an
   id missing from a spilled queue compares an xattr-map iterator with
   the head map's `end()` (`S:cls_2pc_queue.cc:273`, fixed
   `M:cls_2pc_queue.cc:321`), so its `ENOENT` is not reliable there
   (documented, not pinned: the result is undefined behaviour). Pinned in
   Tasks 3 and 4 wherever deterministic.

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cls-2pc-queue` off the fork's `main` (which contains
  plan 13's merge) checked out in `$R`; the SDD ledger; the cluster up.

- [ ] **Step 1: Branch, cluster, ledger**

```bash
cd $R && git fetch origin main && git checkout -b cls-2pc-queue origin/main && git log --oneline -1
grep -n 'cfg(feature = "user")\]' rados-cls/src/call.rs
grep -n 'cfg(.*feature = "rgw"' rados-cls/src/dump.rs rados-cls/src/lib.rs
grep -n 'for features in' .github/workflows/ci.yml
grep -n 'for test in cls_' .github/workflows/test-with-ceph.yml
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
```
Expected: HEAD is the merge of the `cls-lock` PR; `exec_returnvec`
gated on `user` alone; the `dump` gates as in Global Constraints; three
containers up. Then the workspace and ledger as in plan 3's Task 0.

---

### Task 1: The types, `dump::base64_vec`, and the dencoder

**Files:**
- Create: `rados-cls/src/two_pc_queue.rs`.
- Modify: `rados-cls/src/queue.rs` (`ENTRY_OVERHEAD`, `GetStatsRet`, one
  test), `rados-cls/src/dump.rs` (`base64_vec`, widened gates),
  `rados-cls/src/lib.rs` (`mod dump` gate; `#[cfg(feature =
  "two_pc_queue")] pub mod two_pc_queue;` after `rgw_gc`),
  `rados-cls/Cargo.toml` (`two_pc_queue = ["queue"]`, appended to
  `default`; `base64 = { workspace = true }`),
  `.github/workflows/ci.yml` (`two_pc_queue` in the per-class loop,
  between `rgw_gc` and `user`), `README.md` (`two_pc_queue` in the
  `rados-cls` row's class list), `rados-dencoder/src/main.rs` (eight arms,
  one `list_types` line), `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`
  (eight `TypeSpec`s).

**Interfaces:**
- Consumes: `rados::{Denc, RadosError, UTime, VersionedDenc, VersionedEncode}`;
  `crate::dump::{real_time, base64_vec}`.
- Produces, in `queue`: `pub const ENTRY_OVERHEAD: u64 = 10`
  (`QUEUE_ENTRY_OVERHEAD`, a `u16` magic plus a `u64` length ahead of
  every payload); `GetStatsRet { queue_size: u64, queue_entries: u32 }`
  (`cls_queue_get_stats_ret`, `S:src/cls/queue/cls_queue_ops.h:214-234`,
  v1, derive `VersionedDenc`, no `Serialize`: Ceph has no dump).
- Produces, in `two_pc_queue` (fields in wire order):
  - consts `CLASS: &str = "2pc_queue"`, `NO_ID: u32 = 0`,
    `MAX_URGENT_DATA_SIZE: u64 = 23552`, `HEAD_SIZE: u64 = 24576`
    (`queue::HEAD_SIZE_1K + MAX_URGENT_DATA_SIZE`: the head's
    allowance, and so the first entry's offset),
    `URGENT_DATA_XATTR: &str = "cls_queue_urgent_data"`;
  - `Reservation { size: u64, timestamp: UTime, entries: u32 }`,
    hand-written v2/c1, `MAX_DECODE_VERSION` 2;
  - `UrgentData { reserved_size: u64, last_id: u32, reservations:
    BTreeMap<u32, Reservation>, has_xattrs: bool, committed_entries: u32 }`,
    hand-written, encodes v2/c1, `MAX_DECODE_VERSION` 3;
  - derive v1/c1: `ReserveOp { size: u64, entries: u32 }`,
    `ReserveRet { id: u32 }`, `CommitOp { id: u32, data: Vec<Bytes> }`,
    `AbortOp { id: u32 }`, `ExpireOp { stale_time: UTime }`,
    `ReservationsRet { reservations: BTreeMap<u32, Reservation> }`;
  - derive v2/c1 with `min_version = 2, ceph_release = "Squid v19+"`:
    `RemoveOp { end_marker: String, entries_to_remove: u32 }`, no
    `Serialize` (Ceph has no dump and no test instances).
  Task 2 adds the client to the same file.

- [ ] **Step 1: Write the failing tests**

Create `rados-cls/src/two_pc_queue.rs` with a one-line module doc
(`//! The \`2pc_queue\` object class; see the client below.`, replaced
in Task 2), the imports, and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use std::fmt::Debug;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    fn decode<T: Denc>(hex: &str) -> T {
        T::decode(&mut &unhex(hex)[..], 0).expect("decode")
    }

    /// Encode, decode and dump all match one instance.
    fn pin<T: Denc + Serialize + PartialEq + Debug>(v: &T, hex: &str, dump: &str) {
        assert_eq!(bytes(v), unhex(hex));
        assert_eq!(&decode::<T>(hex), v);
        assert_eq!(json(v), dump);
    }

    #[test]
    fn ops_match_the_oracle_instances() {
        // ceph-dencoder v19.2.2 `select_test N encode export` / dump_json.
        pin(&AbortOp { id: 1 }, "01010400000001000000", r#"{"id":1}"#);
        pin(
            &CommitOp {
                id: 123,
                data: vec![Bytes::from_static(b"foo"), Bytes::from_static(b"bar")],
            },
            "0101160000007b0000000200000003000000666f6f03000000626172",
            r#"{"id":123,"bl_data_vec":["Zm9v","YmFy"]}"#,
        );
        // Instances 1 and 2 are identical: coarse_real_time::min() is the epoch.
        pin(
            &ExpireOp::default(),
            "0101080000000000000000000000",
            r#"{"stale_time":"1970-01-01T00:00:00.000000+0000"}"#,
        );
        pin(&ReserveOp::default(), "01010c000000000000000000000000000000", r#"{"size":0,"entries":0}"#);
        pin(
            &ReserveOp { size: 123, entries: 456 },
            "01010c0000007b00000000000000c8010000",
            r#"{"size":123,"entries":456}"#,
        );
        pin(&ReserveRet { id: 123 }, "0101040000007b000000", r#"{"id":123}"#);
        // Corpus 18.2.0 cls_2pc_queue_expire_op/7ace2f35....
        let e: ExpireOp = decode("010108000000077bcf64889b822b");
        assert_eq!(e.stale_time, UTime { sec: 1_691_319_047, nsec: 729_979_784 });
        assert_eq!(json(&e), r#"{"stale_time":"2023-08-06T10:50:47.729979+0000"}"#);
    }

    #[test]
    fn reservation_is_v2_dumps_without_entries_and_reads_reef_v1() {
        pin(
            &Reservation::default(),
            "0201140000000000000000000000000000000000000000000000",
            r#"{"size":0,"timestamp":"1970-01-01T00:00:00.000000+0000"}"#,
        );
        pin(
            &Reservation { size: 123, ..Reservation::default() },
            "0201140000007b00000000000000000000000000000000000000",
            r#"{"size":123,"timestamp":"1970-01-01T00:00:00.000000+0000"}"#,
        );
        // Corpus 19.2.0 cls_2pc_reservation/ff1b6965...: entries 23, not dumped.
        pin(
            &Reservation {
                size: 358,
                timestamp: UTime { sec: 1_727_592_400, nsec: 841_226_706 },
                entries: 23,
            },
            "0201140000006601000000000000d0f7f866d219243217000000",
            r#"{"size":358,"timestamp":"2024-09-29T06:46:40.841226+0000"}"#,
        );
        // Corpus 18.2.0 cls_2pc_reservation/fff84520...: version 1, no entries.
        let v1: Reservation = decode("010110000000fa00000000000000e97acf643817981b");
        assert_eq!(
            v1,
            Reservation {
                size: 250,
                timestamp: UTime { sec: 1_691_319_017, nsec: 462_952_248 },
                entries: 0,
            }
        );
        assert_eq!(bytes(&v1), unhex("020114000000fa00000000000000e97acf643817981b00000000"));
        assert_eq!(json(&v1), r#"{"size":250,"timestamp":"2023-08-06T10:50:17.462952+0000"}"#);
    }

    #[test]
    fn urgent_data_matches_the_oracle_and_the_leak_witness() {
        pin(
            &UrgentData::default(),
            "020115000000000000000000000000000000000000000000000000",
            r#"{"reserved_size":0,"last_id":0,"reservations":[],"has_xattrs":false}"#,
        );
        let mut u = UrgentData {
            reserved_size: 123,
            last_id: 456,
            has_xattrs: true,
            ..UrgentData::default()
        };
        u.reservations.insert(
            789,
            Reservation { size: 1, timestamp: UTime::default(), entries: 2 },
        );
        pin(
            &u,
            "0201330000007b00000000000000c8010000010000001503000002011400000001000000000000000000000000000000020000000100000000",
            r#"{"reserved_size":123,"last_id":456,"reservations":[{"id":789,"size":1,"timestamp":"1970-01-01T00:00:00.000000+0000"}],"has_xattrs":true}"#,
        );
        // Corpus 19.2.0 cls_2pc_urgent_data/fc55a9f4...: no reservation open,
        // committed_entries 253, yet reserved_size 2530 = 253 x 10: the leak.
        pin(
            &UrgentData {
                reserved_size: 2530,
                last_id: 11,
                committed_entries: 253,
                ..UrgentData::default()
            },
            "020115000000e2090000000000000b0000000000000000fd000000",
            r#"{"reserved_size":2530,"last_id":11,"reservations":[],"has_xattrs":false}"#,
        );
    }

    #[test]
    fn urgent_data_reads_reef_v1_and_main_v3_and_writes_v2() {
        // Corpus 18.2.0 cls_2pc_urgent_data/ff4ac206...: no committed_entries.
        let v1: UrgentData = decode("0101110000009015000000000000180000000000000000");
        assert_eq!(v1, UrgentData { reserved_size: 5520, last_id: 24, ..UrgentData::default() });
        assert_eq!(bytes(&v1), unhex("020115000000901500000000000018000000000000000000000000"));
        // main and v19.2.4+ write version 3 with the same layout.
        let v3: UrgentData = decode("030115000000e2090000000000000b0000000000000000fd000000");
        assert_eq!((v3.reserved_size, v3.committed_entries), (2530, 253));
        assert_eq!(bytes(&v3)[0], 2);
    }

    #[test]
    fn reservations_ret_dumps_and_encodes_in_ascending_id_order() {
        pin(&ReservationsRet::default(), "01010400000000000000", r#"{"reservations":[]}"#);
        // Oracle instance 2: ids 2 then 1 on the wire (libstdc++ order).
        let r: ReservationsRet = decode(
            "01014000000002000000020000000201140000000000000000000000000000000000000000000000010000000201140000000000000000000000000000000000000000000000",
        );
        assert_eq!(r.reservations.keys().copied().collect::<Vec<_>>(), [1, 2]);
        assert_eq!(
            json(&r),
            r#"{"reservations":[{"id":1,"size":0,"timestamp":"1970-01-01T00:00:00.000000+0000"},{"id":2,"size":0,"timestamp":"1970-01-01T00:00:00.000000+0000"}]}"#
        );
        // Derived, not an oracle: the BTreeMap re-encodes ascending.
        assert_eq!(
            bytes(&r),
            unhex("01014000000002000000010000000201140000000000000000000000000000000000000000000000020000000201140000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn remove_op_matches_the_corpus_and_refuses_version_1() {
        // Derived: v19.2.2's ceph-dencoder does not register this type.
        // Corpus 19.2.0 cls_2pc_queue_remove_op/e1487ca5....
        let r = RemoveOp {
            end_marker: "0/31796".to_owned(),
            entries_to_remove: 42,
        };
        assert_eq!(bytes(&r), unhex("02010f00000007000000302f33313739362a000000"));
        assert_eq!(decode::<RemoveOp>("02010f00000007000000302f33313739362a000000"), r);
        // Version 1 on the wire is cls_queue_remove_op, queue::RemoveOp.
        assert!(RemoveOp::decode(&mut &unhex("01010b00000007000000302f3331373936")[..], 0).is_err());
    }
}
```

Append to `queue.rs`'s test module:

```rust
    #[test]
    fn get_stats_ret_matches_the_corpus() {
        // Derived: v19.2.2's ceph-dencoder does not register this type.
        // Corpus 19.2.0 cls_queue_get_stats_ret/bf93a2b1....
        let s = GetStatsRet {
            queue_size: 19_762,
            queue_entries: 782,
        };
        let wire = b"\x01\x01\x0c\x00\x00\x00\x32\x4d\x00\x00\x00\x00\x00\x00\x0e\x03\x00\x00";
        assert_eq!(bytes(&s), wire);
        assert_eq!(GetStatsRet::decode(&mut &wire[..], 0).expect("decode"), s);
    }
```

Append to `dump.rs`'s test module:

```rust
    #[test]
    #[cfg(feature = "two_pc_queue")]
    fn commit_op_dumps_payloads_as_padded_base64() {
        // bufferlist::encode_base64: RFC 4648 alphabet, '=' padding, no breaks.
        let op = crate::two_pc_queue::CommitOp {
            id: 1,
            data: [&b"a"[..], b"ab", b"foo", b"\xff\xfe"]
                .into_iter()
                .map(Bytes::copy_from_slice)
                .collect(),
        };
        assert_eq!(
            serde_json::to_string(&op).expect("json"),
            r#"{"id":1,"bl_data_vec":["YQ==","YWI=","Zm9v","//4="]}"#
        );
    }
```

and widen `streamed_real_time_is_iso_with_a_numeric_offset`'s gate to
`any(feature = "rgw", feature = "two_pc_queue")`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline two_pc_queue`
Expected: compile errors, the types do not exist.

- [ ] **Step 3: Implement**

`dump.rs`: widen `real_time`, `iso_utc` to `any(feature = "rgw",
feature = "two_pc_queue")`; `civil_from_days` and `use rados::UTime`
to `any(feature = "user", feature = "rgw", feature = "two_pc_queue")`;
add, gated `#[cfg(feature = "two_pc_queue")]` (with `use bytes::Bytes;`
under the same gate):

```rust
/// A `vector<bufferlist>` dumped with `encode_json`: `bufferlist::encode_base64`
/// strings, the padded standard alphabet with no line breaks.
pub(crate) fn base64_vec<S: serde::Serializer>(v: &[Bytes], s: S) -> Result<S::Ok, S::Error> {
    use base64::Engine as _;
    s.collect_seq(v.iter().map(|b| base64::engine::general_purpose::STANDARD.encode(b)))
}
```

`lib.rs`: the `mod dump;` gate gains `feature = "two_pc_queue"`; add the
`two_pc_queue` module line (not yet the `mod call` gate: nothing calls
a class until Task 2).

`queue.rs`, after `HEAD_SIZE_1K` and after `GetCapacityRet`:

```rust
/// `QUEUE_ENTRY_OVERHEAD`: the `u16` magic and `u64` length ahead of
/// every payload in the ring.
pub const ENTRY_OVERHEAD: u64 = 10;

/// `cls_queue_get_stats_ret`, the `2pc_queue` class's topic statistics:
/// the ring's used bytes (entry overheads included) and its committed
/// entry count. Ceph gives it no dump and does not register it with
/// `ceph-dencoder`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetStatsRet {
    pub queue_size: u64,
    pub queue_entries: u32,
}
```

`two_pc_queue.rs`, above the tests (imports: `std::collections::BTreeMap`,
`bytes::{Buf, BufMut, Bytes}`, `rados::{Denc, RadosError, UTime,
VersionedDenc, VersionedEncode}`, `serde::Serialize`):

```rust
pub const CLASS: &str = "2pc_queue";

/// `cls_2pc_reservation::NO_ID`: never a live id; `last_id` wraps to it
/// after `u32::MAX` reservations.
pub const NO_ID: u32 = 0;

/// The urgent-data allowance `2pc_queue_init` gives the head.
pub const MAX_URGENT_DATA_SIZE: u64 = 23_552;

/// The head's size (`queue::HEAD_SIZE_1K` plus [`MAX_URGENT_DATA_SIZE`])
/// and so the offset of the first entry: markers start at `0/24576`.
pub const HEAD_SIZE: u64 = crate::queue::HEAD_SIZE_1K + MAX_URGENT_DATA_SIZE;

/// The xattr a reservation spills into once the head's urgent data would
/// exceed [`MAX_URGENT_DATA_SIZE`]: an encoded `BTreeMap<u32,
/// Reservation>` with no struct header. The `rgw_gc` class uses the same
/// name for its own overflow.
pub const URGENT_DATA_XATTR: &str = "cls_queue_urgent_data";

/// `cls_2pc_reservation`: `size` payload bytes and `entries` entries set
/// aside by `2pc_queue_reserve`, stamped with the OSD's clock (not the
/// client's). The dump leaves `entries` out. Version 1 (Pacific to Reef)
/// had no `entries`; it is decoded as 0 because the 18.2.0 corpus
/// archive holds such samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Reservation {
    pub size: u64,
    #[serde(serialize_with = "crate::dump::real_time")]
    pub timestamp: UTime,
    #[serde(skip)]
    pub entries: u32,
}
```

`VersionedEncode for Reservation`: `MAX_DECODE_VERSION = 2`,
`encoding_version` 2, `compat_version` 1; `encode_content` writes
`size`, `timestamp`, `entries`; `decode_content` calls
`rados::check_min_version!(struct_v, 1, "Reservation", "Pacific v16+")`,
reads `size`, `timestamp`, then `entries` only when `struct_v >= 2`
(else 0); `encoded_size_content` is `Some(20)`;
`rados::impl_denc_for_versioned!(Reservation)`.

```rust
/// `cls_2pc_urgent_data`: the class's bookkeeping, kept in the queue
/// head's urgent data. `reserved_size` is the bytes promised to open
/// reservations, entry overheads included; on a v19.2.2 OSD it only
/// grows by ten bytes per reserved entry that is committed, aborted or
/// expired (see the module doc), and a client cannot repair it.
/// `last_id` is the last id handed out. `has_xattrs` says reservations
/// have spilled into [`URGENT_DATA_XATTR`]; it is never cleared.
/// `committed_entries` counts reserved entries committed less entries
/// removed; the dump leaves it out.
///
/// Ceph keeps `reservations` in an `unordered_map` and writes and dumps
/// it in libstdc++ hash order; this map re-encodes and dumps in
/// ascending id order, so two or more reservations differ from Ceph's
/// bytes and dump in order only. Encodes version 2, what a v19.2.2 OSD
/// writes; decodes version 3 (main, v19.2.4+, same layout, marking a
/// writer without the leak) and version 1 (no `committed_entries`), the
/// latter because the 18.2.0 corpus archive holds such samples.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UrgentData {
    pub reserved_size: u64,
    pub last_id: u32,
    #[serde(serialize_with = "reservations_dump")]
    pub reservations: BTreeMap<u32, Reservation>,
    pub has_xattrs: bool,
    #[serde(skip)]
    pub committed_entries: u32,
}
```

`VersionedEncode for UrgentData`: `MAX_DECODE_VERSION = 3`,
`encoding_version` 2, `compat_version` 1; the five fields in order;
`decode_content` calls `rados::check_min_version!(struct_v, 1,
"UrgentData", "Pacific v16+")` and reads `committed_entries` only when
`struct_v >= 2`; `encoded_size_content` `None`;
`rados::impl_denc_for_versioned!(UrgentData)`.

```rust
/// `encode_json` of a `cls_2pc_reservations`: an array of `{"id", "size",
/// "timestamp"}`, here in ascending id order.
fn reservations_dump<S: serde::Serializer>(
    m: &BTreeMap<u32, Reservation>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Item {
        id: u32,
        size: u64,
        #[serde(serialize_with = "crate::dump::real_time")]
        timestamp: UTime,
    }
    s.collect_seq(m.iter().map(|(&id, r)| Item {
        id,
        size: r.size,
        timestamp: r.timestamp,
    }))
}

/// `cls_2pc_queue_reserve_op`: set aside `size` payload bytes for
/// `entries` entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReserveOp {
    pub size: u64,
    pub entries: u32,
}

/// `cls_2pc_queue_reserve_ret`: the new reservation's id. The reply of a
/// write, so it arrives only with `RETURNVEC`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReserveRet {
    pub id: u32,
}

/// `cls_2pc_queue_commit_op`: the payloads to enqueue into reservation
/// `id`, one entry each. The dump prints them as base64 `bl_data_vec`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct CommitOp {
    pub id: u32,
    #[serde(rename = "bl_data_vec", serialize_with = "crate::dump::base64_vec")]
    pub data: Vec<Bytes>,
}

/// `cls_2pc_queue_abort_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AbortOp {
    pub id: u32,
}

/// `cls_2pc_queue_expire_op`: reservations stamped strictly before
/// `stale_time` (OSD clock) go.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ExpireOp {
    #[serde(serialize_with = "crate::dump::real_time")]
    pub stale_time: UTime,
}

/// `cls_2pc_queue_reservations_ret`: the open reservations, head and
/// xattr merged. Ordered by id; see [`UrgentData`] on Ceph's order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReservationsRet {
    #[serde(serialize_with = "reservations_dump")]
    pub reservations: BTreeMap<u32, Reservation>,
}

/// `cls_2pc_queue_remove_op`: drop the entries before `end_marker` and
/// take `entries_to_remove` off `committed_entries`; 0 makes the class
/// count them itself. Version 2 (Squid) added the count; what Reef sent
/// is `cls_queue_remove_op`, `queue::RemoveOp`, which the class still
/// accepts. Ceph gives this type no dump and no `ceph-dencoder`
/// registration.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(
    crate = "rados",
    version = 2,
    compat = 1,
    min_version = 2,
    ceph_release = "Squid v19+"
)]
pub struct RemoveOp {
    pub end_marker: String,
    pub entries_to_remove: u32,
}
```

`Cargo.toml`: `two_pc_queue = ["queue"]`, appended to `default`;
`base64 = { workspace = true }` under `[dependencies]`.
`ci.yml`: `for features in "" ... rgw_gc two_pc_queue user version; do`
(keeping whatever plan 13 added). `README.md`: add `two_pc_queue` to the
parenthesised class list in the `rados-cls` row.

`rados-dencoder/src/main.rs`: import `rados_cls::two_pc_queue::{AbortOp
as TwoPcAbortOp, CommitOp as TwoPcCommitOp, ExpireOp as TwoPcExpireOp,
Reservation as TwoPcReservation, ReservationsRet as
TwoPcReservationsRet, ReserveOp as TwoPcReserveOp, ReserveRet as
TwoPcReserveRet, UrgentData as TwoPcUrgentData}`; arms under
`// Object classes (rados-cls)`:

```rust
        "cls_2pc_reservation" => Some(type_info_denc::<TwoPcReservation>()),
        "cls_2pc_urgent_data" => Some(type_info_denc::<TwoPcUrgentData>()),
        "cls_2pc_queue_reserve_op" => Some(type_info_denc::<TwoPcReserveOp>()),
        "cls_2pc_queue_reserve_ret" => Some(type_info_denc::<TwoPcReserveRet>()),
        "cls_2pc_queue_commit_op" => Some(type_info_denc::<TwoPcCommitOp>()),
        "cls_2pc_queue_abort_op" => Some(type_info_denc::<TwoPcAbortOp>()),
        "cls_2pc_queue_expire_op" => Some(type_info_denc::<TwoPcExpireOp>()),
        "cls_2pc_queue_reservations_ret" => Some(type_info_denc::<TwoPcReservationsRet>()),
```

and in the OBJECT CLASSES `list_types` block, matching the neighbours:
`"  cls_2pc_{{reservation,urgent_data}} / cls_2pc_queue_{{reserve,commit,abort,expire}}_op / cls_2pc_queue_{{reserve,reservations}}_ret [versioned]"`.

Corpus table, after the `cls_rgw_gc_*` rows:

```rust
    // cls_2pc_queue_remove_op and cls_queue_get_stats_ret have 19.2.0
    // corpus directories but v19.2.2's ceph-dencoder does not register
    // them; their unit tests pin the corpus bytes instead.
    TypeSpec::new("cls_2pc_reservation", None, false),
    // ceph-dencoder dumps the reservation map in libstdc++ unordered_map
    // order and the Rust BTreeMap dumps ascending, so samples with two or
    // more reservations differ in order only.
    TypeSpec::new("cls_2pc_urgent_data", None, true),
    TypeSpec::new("cls_2pc_queue_reserve_op", None, false),
    TypeSpec::new("cls_2pc_queue_reserve_ret", None, false),
    TypeSpec::new("cls_2pc_queue_commit_op", None, false),
    TypeSpec::new("cls_2pc_queue_abort_op", None, false),
    TypeSpec::new("cls_2pc_queue_expire_op", None, false),
    TypeSpec::new("cls_2pc_queue_reservations_ret", None, true), // as cls_2pc_urgent_data
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline && cargo check --workspace --all-targets --offline && cargo check -p rados-cls --no-default-features --features two_pc_queue --offline`
Expected: the six `two_pc_queue` tests, the `queue` and `dump` additions
pass; no warnings in either check.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/Cargo.toml rados-cls/src/lib.rs rados-cls/src/dump.rs rados-cls/src/queue.rs rados-cls/src/two_pc_queue.rs .github/workflows/ci.yml README.md rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
two_pc_queue: add the 2pc_queue class types

The request, reply and bookkeeping structs of cls_2pc_queue, the class
behind RGW's persistent notifications, plus cls_queue_get_stats_ret,
which only this class returns. Reservations and the urgent data are
version 2 on Squid and decode Reef's version 1, which the 18.2.0 corpus
holds; the urgent data also decodes main's version 3. Ceph keeps the
reservation map unordered and dumps it in libstdc++ hash order, so the
two dump types with a map are corpus exceptions: multi-entry samples
differ in order only. The commit op dumps its payloads as base64 through
the workspace's `base64` crate.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 2: The client, the head reader, and the module doc

**Files:**
- Modify: `rados-cls/src/two_pc_queue.rs`, `rados-cls/src/call.rs`
  (`exec_returnvec` gate and doc), `rados-cls/src/lib.rs` (`mod call`
  gate).

**Interfaces:**
- Consumes: `crate::call::{op, raw_op, exec, exec_raw, exec_returnvec,
  decode, decode_bytes}`; `crate::queue::{InitOp, ListOp, ListRet, Head,
  GetStatsRet, GetCapacityRet, decode_get_capacity, decode_list}`;
  `IoCtx::read`.
- Produces (all `pub`, `Result` = `rados::osdclient::error::Result`):
  - re-exports `pub use crate::queue::{decode_get_capacity, decode_list};`
    (the replies are the `queue` class's);
  - op constructors: `init_op(size: u64) -> Result<OSDOp>`,
    `get_capacity_op()`, `get_topic_stats_op()`,
    `reserve_op(size: u64, entries: u32)`, `commit_op(id: u32, data:
    Vec<Bytes>)`, `abort_op(id: u32)`, `list_reservations_op()`,
    `list_entries_op(marker: &str, max: u32)`,
    `remove_entries_op(end_marker: &str, entries_to_remove: u32)`,
    `expire_reservations_op(stale_time: UTime)`;
  - decoders: `decode_reserve(&OpReply) -> Result<u32>`,
    `decode_get_topic_stats(&OpReply) -> Result<GetStatsRet>`,
    `decode_list_reservations(&OpReply) -> Result<BTreeMap<u32, Reservation>>`;
  - async: `init(ioctx, oid, size: u64) -> Result<()>`,
    `get_capacity(ioctx, oid) -> Result<u64>`,
    `get_topic_stats(ioctx, oid) -> Result<GetStatsRet>`,
    `reserve(ioctx, oid, size: u64, entries: u32) -> Result<u32>`,
    `commit(ioctx, oid, id: u32, data: Vec<Bytes>) -> Result<()>`,
    `abort(ioctx, oid, id: u32) -> Result<()>`,
    `list_reservations(ioctx, oid) -> Result<BTreeMap<u32, Reservation>>`,
    `list_entries(ioctx, oid, marker: &str, max: u32) -> Result<ListRet>`,
    `remove_entries(ioctx, oid, end_marker: &str, entries_to_remove: u32) -> Result<()>`,
    `expire_reservations(ioctx, oid, stale_time: UTime) -> Result<()>`,
    with `ioctx: &IoCtx, oid: &str`;
  - `HeadState { head: queue::Head, urgent_data: UrgentData,
    urgent_data_version: u8 }`, `parse_head(data: &[u8]) ->
    Result<HeadState>`, `read_head(ioctx, oid) -> Result<HeadState>`.
    Tasks 3 and 4 use all of these.

- [ ] **Step 1: Write the failing tests**

Append to the test module (add `use rados::osdclient::types::OpData;`):

```rust
    #[test]
    fn ops_name_the_class_and_method() {
        for (op, method) in [
            (init_op(1000), "2pc_queue_init"),
            (get_capacity_op(), "2pc_queue_get_capacity"),
            (get_topic_stats_op(), "2pc_queue_get_topic_stats"),
            (reserve_op(4096, 1), "2pc_queue_reserve"),
            (commit_op(1, vec![]), "2pc_queue_commit"),
            (abort_op(1), "2pc_queue_abort"),
            (list_reservations_op(), "2pc_queue_list_reservations"),
            (list_entries_op("", 1024), "2pc_queue_list_entries"),
            (remove_entries_op("0/24576", 0), "2pc_queue_remove_entries"),
            (expire_reservations_op(UTime::default()), "2pc_queue_expire_reservations"),
        ] {
            let op = op.expect("op");
            assert!(op.indata.starts_with(format!("{CLASS}{method}").as_bytes()), "{method}");
            assert!(
                matches!(op.op_data, OpData::Call { class_len: 9, method_len, .. } if method_len as usize == method.len()),
                "{method}"
            );
        }
        // init sends cls_queue_init_op with only queue_size set.
        assert!(init_op(1000)
            .expect("op")
            .indata
            .ends_with(&unhex("010114000000e803000000000000000000000000000000000000")));
        assert!(reserve_op(4096, 1)
            .expect("op")
            .indata
            .ends_with(&unhex("01010c000000001000000000000001000000")));
        for op in [get_capacity_op(), get_topic_stats_op(), list_reservations_op()] {
            assert!(matches!(op.expect("op").op_data, OpData::Call { indata_len: 0, .. }));
        }
    }

    #[test]
    fn decoders_unwrap_the_replies() {
        let reply = |hex: &str| rados::OpReply {
            return_code: 0,
            outdata: Bytes::from(unhex(hex)),
        };
        // The 10-byte RETURNVEC reply; without the flag it is empty.
        assert_eq!(decode_reserve(&reply("0101040000007b000000")).expect("decode"), 123);
        assert!(decode_reserve(&reply("")).is_err());
        assert!(decode_list_reservations(&reply("01010400000000000000")).expect("decode").is_empty());
        let s = decode_get_topic_stats(&reply("01010c000000324d0000000000000e030000")).expect("decode");
        assert_eq!((s.queue_size, s.queue_entries), (19_762, 782));
    }

    /// What a `HEAD_SIZE` read of offset zero returns: the `0xDEAD` magic,
    /// the head's length, the head, then zeros.
    fn framed(head: &crate::queue::Head) -> Vec<u8> {
        let body = bytes(head);
        let mut out = 0xDEADu16.to_le_bytes().to_vec();
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&body);
        out.resize(HEAD_SIZE as usize, 0);
        out
    }

    #[test]
    fn parse_head_reads_what_2pc_init_writes() {
        let start = crate::queue::Marker { offset: HEAD_SIZE, generation: 0 };
        let head = crate::queue::Head {
            max_head_size: HEAD_SIZE,
            front: start,
            tail: start,
            queue_size: 1000 + HEAD_SIZE,
            max_urgent_data_size: MAX_URGENT_DATA_SIZE,
            urgent_data: encode_with_capacity(&UrgentData::default(), 0).expect("encode"),
        };
        assert_eq!(bytes(&head).len(), 105, "78 bytes of head, 27 of urgent data");
        let state = parse_head(&framed(&head)).expect("parse");
        assert_eq!(state.head, head);
        assert_eq!(state.urgent_data, UrgentData::default());
        assert_eq!(state.urgent_data_version, 2);
    }

    #[test]
    fn parse_head_rejects_what_is_not_a_2pc_head() {
        assert!(parse_head(&[]).is_err(), "no framing");
        assert!(parse_head(&[0xad, 0xde, 0xff, 0, 0, 0, 0, 0, 0, 0]).is_err(), "length past the data");
        let mut wire = framed(&crate::queue::Head::default());
        assert!(parse_head(&wire).is_err(), "a plain queue head has no urgent data");
        wire[0] = 0;
        assert!(parse_head(&wire).is_err(), "bad magic");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline two_pc_queue`
Expected: compile errors, the functions do not exist.

- [ ] **Step 3: Implement**

`call.rs`: `#[cfg(any(feature = "user", feature = "two_pc_queue"))]` on
`exec_returnvec`, and its last doc sentence becomes "The `user` class's
`reset_user_stats2` and the `2pc_queue` class's `2pc_queue_reserve` are
such methods." `lib.rs`: `feature = "two_pc_queue"` joins the `mod call`
gate.

Replace the one-line module doc with:

```rust
//! The `2pc_queue` object class (`cls_2pc_queue_client.h`): a `queue`
//! ring written in two phases. A writer reserves payload bytes and an
//! entry count, gets an id back, and later commits payloads into that
//! reservation or aborts it; stale reservations are expired. The head,
//! the entry format, init, capacity and listing are the `queue` class's
//! (its types and decoders are reused); the class keeps [`UrgentData`]
//! in the head's urgent data, spilling reservations into the
//! [`URGENT_DATA_XATTR`] xattr past [`MAX_URGENT_DATA_SIZE`]. The
//! writing methods are init, reserve, commit, abort, remove entries and
//! expire reservations; reserve is the only one that replies, so it
//! needs `RETURNVEC`. Everything else is RD.
//!
//! # Squid's capacity leak
//!
//! A v19.2.2 OSD adds `size + 10·entries` to `reserved_size` per
//! reservation but takes back only `size` on commit, abort or expire,
//! so every reserved entry permanently costs ten bytes of capacity
//! until `2pc_queue_reserve` answers `ENOSPC` whatever the ring holds
//! (about 12.8 million single-entry reservations at RGW's queue size).
//! v19.2.4, v20.2.3 and main subtract the overhead and write
//! [`UrgentData`] as version 3, recomputing `reserved_size` once on a
//! head they find at version 2 or older. A client cannot repair the
//! count on a Squid OSD: no request carries it. [`read_head`] shows it.
//!
//! # How RGW uses the class
//!
//! Facts about `rgw_notify.cc` a Rust driver mirrors; none of it is in
//! this crate. A persistent topic's queue is the object
//! `<account or tenant>:<topic>` in the zone's notification pool
//! (`<zone>.rgw.log`, namespace `notif`), created with `create(true)`
//! and init at 128,000,000 bytes in one op (`EEXIST` is success), then
//! registered as one empty-valued omap key on the object
//! `queues_list_object` in the same pool, a name RGW refuses for a
//! queue. RGW takes no lock on the registry; it pages it 1024 keys at a
//! time. Each daemon, every 30 s plus jitter, asserts the queue exists
//! and takes `cls_lock` `<queue>_lock` on the queue object: exclusive,
//! a 16-character random cookie per daemon, `LOCK_FLAG_MAY_RENEW`, for
//! the failover time (90 s on Squid; three times
//! `rgw_topic_ownership_update_period`, 30 s by default, on main);
//! `EBUSY` means another daemon owns the queue. Before an object write
//! RGW reserves 4096 bytes and one entry per persistent topic (with
//! `RETURNVEC`; `ENOSPC` becomes `SlowDown`); after the write it
//! commits the encoded event, first aborting and re-reserving at the
//! event's size when it exceeds 4096, and a failed write aborts. The
//! owner's deliverer runs `assert_exists`, `assert_locked` and a list
//! of 1024 entries in one read op, then `assert_exists`,
//! `assert_locked` and a remove in one write op; the owner alone, every
//! 30 s, runs `assert_exists`, `assert_locked` and an expire at now
//! minus 120 s. The expiry needs no lock in the class, but a gateway
//! that takes `<queue>_lock` to expire becomes the owner, and radosgw's
//! deliverer then skips that queue.
//!
//! Two driver-level facts a Rust gateway cannot reproduce: main creates
//! `rgw_bucket_persistent_notif_num_shards` (11) queues per topic, `<q>`
//! and `<q>.1` to `<q>.10`, each registered, and picks one per event
//! with libstdc++'s `std::hash<std::string>` of `<bucket>:<key>` modulo
//! the count, which is not portable (nothing checks it: the deliverer
//! drains every shard); and Squid's registry listing never advances
//! past its first 1024 keys, so a registry that main's shards push past
//! 1024 (about 94 topics) makes a Squid radosgw sharing it loop on the
//! first page.
```

Then the client (below the types, above the tests; imports gain
`rados::osdclient::error::{OSDClientError, Result}`,
`rados::osdclient::{IoCtx, OSDOp, OpReply}`, `crate::call`,
`crate::queue::{GetStatsRet, Head, InitOp, ListOp, ListRet}`):

```rust
pub use crate::queue::{decode_get_capacity, decode_list};

/// `QUEUE_HEAD_START`: the magic ahead of the encoded head at offset zero.
const QUEUE_HEAD_START: u16 = 0xDEAD;

/// `cls_2pc_queue_init`: lay out a ring of `size` usable bytes behind a
/// [`HEAD_SIZE`] head holding an empty [`UrgentData`]. The object must
/// exist (a missing one is `ENOENT`): RGW puts `create(true)` ahead of
/// this op in the same compound. `EEXIST` if the object holds a head.
pub fn init_op(size: u64) -> Result<OSDOp> {
    call::op(CLASS, "2pc_queue_init", &InitOp { queue_size: size, ..InitOp::default() })
}

/// `cls_2pc_queue_get_capacity`: the usable bytes; decode with
/// [`decode_get_capacity`].
pub fn get_capacity_op() -> Result<OSDOp> {
    call::raw_op(CLASS, "2pc_queue_get_capacity", Bytes::new())
}

/// `cls_2pc_queue_get_topic_stats`: the ring's used bytes (entry
/// overheads included) and `committed_entries`; decode with
/// [`decode_get_topic_stats`].
pub fn get_topic_stats_op() -> Result<OSDOp> {
    call::raw_op(CLASS, "2pc_queue_get_topic_stats", Bytes::new())
}

/// Decode the reply to [`get_topic_stats_op`].
pub fn decode_get_topic_stats(reply: &OpReply) -> Result<GetStatsRet> {
    call::decode(reply)
}

/// `cls_2pc_queue_reserve`: set aside `size` payload bytes for `entries`
/// entries. `EINVAL` if either is 0 or the object is not a 2pc queue
/// (a plain `queue` ring, an empty object); `ENOENT` if it is missing;
/// `ENOSPC` iff `size + reserved_size + 10·entries` exceeds the ring's
/// free bytes. Ids start at 1 and count up as `u32`, wrapping to
/// [`NO_ID`]; a collision after a wrap is `EAGAIN`. The reservation is
/// stamped with the OSD's clock. On a v19.2.2 OSD the `10·entries`
/// overhead is never given back (see the module doc).
///
/// The id comes back in the out-data of a write, which the OSD keeps
/// only when the operation carries `OpBuilder::returnvec()`; without it
/// the reservation is made and its id lost. The C++ client warns this
/// op cannot be batched with other read/write ops: keep it alone in its
/// compound. Decode the reply with [`decode_reserve`].
pub fn reserve_op(size: u64, entries: u32) -> Result<OSDOp> {
    call::op(CLASS, "2pc_queue_reserve", &ReserveOp { size, entries })
}

/// Decode the reply to [`reserve_op`]: the id.
pub fn decode_reserve(reply: &OpReply) -> Result<u32> {
    Ok(call::decode::<ReserveRet>(reply)?.id)
}

/// `cls_2pc_queue_commit`: enqueue `data`, one entry per payload, into
/// reservation `id` and close it. `EINVAL` if the payloads' total length
/// exceeds the reservation's `size` (neither their count nor the entry
/// overhead is checked, and the reservation stays open); `ENOENT` if no
/// such reservation is open, except that on a v19.2.2 OSD whose
/// reservations have spilled into the xattr that answer is not reliable
/// (fixed on main). `committed_entries` grows by the reservation's
/// `entries`, not by `data.len()`; RGW reserves one entry and commits one
/// payload.
pub fn commit_op(id: u32, data: Vec<Bytes>) -> Result<OSDOp> {
    call::op(CLASS, "2pc_queue_commit", &CommitOp { id, data })
}

/// `cls_2pc_queue_abort`: close reservation `id` without enqueueing. An
/// id that is not open is success, a no-op.
pub fn abort_op(id: u32) -> Result<OSDOp> {
    call::op(CLASS, "2pc_queue_abort", &AbortOp { id })
}

/// `cls_2pc_queue_list_reservations`: every open reservation, head and
/// xattr merged; decode with [`decode_list_reservations`].
pub fn list_reservations_op() -> Result<OSDOp> {
    call::raw_op(CLASS, "2pc_queue_list_reservations", Bytes::new())
}

/// Decode the reply to [`list_reservations_op`].
pub fn decode_list_reservations(reply: &OpReply) -> Result<BTreeMap<u32, Reservation>> {
    Ok(call::decode::<ReservationsRet>(reply)?.reservations)
}

/// `cls_2pc_queue_list_entries`: up to `max` committed entries from
/// `marker` (empty for the front; the first entry is `0/24576`), as
/// `queue_list_entries` answers; decode with [`decode_list`]. Like the
/// C++ client, no end marker.
pub fn list_entries_op(marker: &str, max: u32) -> Result<OSDOp> {
    call::op(
        CLASS,
        "2pc_queue_list_entries",
        &ListOp { max: u64::from(max), start_marker: marker.to_owned(), end_marker: String::new() },
    )
}

/// `cls_2pc_queue_remove_entries`: move the front to `end_marker` (as
/// `queue_remove_entries`: a no-op on an empty ring, `EINVAL` behind the
/// front or more than a wrap ahead), then take `entries_to_remove` off
/// `committed_entries`, a `u32` that wraps if overstated. With 0 the
/// class counts the entries before `end_marker` itself.
pub fn remove_entries_op(end_marker: &str, entries_to_remove: u32) -> Result<OSDOp> {
    call::op(
        CLASS,
        "2pc_queue_remove_entries",
        &RemoveOp { end_marker: end_marker.to_owned(), entries_to_remove },
    )
}

/// `cls_2pc_queue_expire_reservations`: close every reservation stamped
/// strictly before `stale_time`, head and xattr alike. The stamps are
/// the OSD's clock, so client clock skew moves the cut. Writes nothing
/// when nothing is stale. RGW's owner passes now minus 120 s.
pub fn expire_reservations_op(stale_time: UTime) -> Result<OSDOp> {
    call::op(CLASS, "2pc_queue_expire_reservations", &ExpireOp { stale_time })
}

/// Init the ring on an existing `oid`; see [`init_op`].
pub async fn init(ioctx: &IoCtx, oid: &str, size: u64) -> Result<()>
// call::exec(.., "2pc_queue_init", &InitOp { queue_size: size, ..InitOp::default() }), then Ok(())

/// See [`get_capacity_op`].
pub async fn get_capacity(ioctx: &IoCtx, oid: &str) -> Result<u64>
// exec_raw(.., "2pc_queue_get_capacity", Bytes::new()) → decode_bytes::<GetCapacityRet>(out)?.queue_capacity

/// See [`get_topic_stats_op`].
pub async fn get_topic_stats(ioctx: &IoCtx, oid: &str) -> Result<GetStatsRet>
// exec_raw(.., "2pc_queue_get_topic_stats", Bytes::new()) → decode_bytes

/// See [`reserve_op`]; runs it with `RETURNVEC`.
pub async fn reserve(ioctx: &IoCtx, oid: &str, size: u64, entries: u32) -> Result<u32> {
    let out = call::exec_returnvec(ioctx, oid, CLASS, "2pc_queue_reserve", &ReserveOp { size, entries }).await?;
    Ok(call::decode_bytes::<ReserveRet>(out)?.id)
}

/// See [`commit_op`].
pub async fn commit(ioctx: &IoCtx, oid: &str, id: u32, data: Vec<Bytes>) -> Result<()>
/// See [`abort_op`].
pub async fn abort(ioctx: &IoCtx, oid: &str, id: u32) -> Result<()>
/// See [`list_reservations_op`].
pub async fn list_reservations(ioctx: &IoCtx, oid: &str) -> Result<BTreeMap<u32, Reservation>>
/// See [`list_entries_op`].
pub async fn list_entries(ioctx: &IoCtx, oid: &str, marker: &str, max: u32) -> Result<ListRet>
/// See [`remove_entries_op`].
pub async fn remove_entries(ioctx: &IoCtx, oid: &str, end_marker: &str, entries_to_remove: u32) -> Result<()>
/// See [`expire_reservations_op`].
pub async fn expire_reservations(ioctx: &IoCtx, oid: &str, stale_time: UTime) -> Result<()>
// each: call::exec / exec_raw with the same request as its op constructor,
// then .map(drop) or decode_bytes, as queue.rs does.

/// A 2pc queue's head as the class stores it at offset zero: the
/// `queue` head, the [`UrgentData`] in it, and that data's encoding
/// version (2 from a v19.2.2 OSD, whose `reserved_size` leaks; 3 from a
/// fixed one). The only view of `reserved_size`, `last_id`,
/// `has_xattrs` and `committed_entries`, for tests and operator tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadState {
    pub head: Head,
    pub urgent_data: UrgentData,
    pub urgent_data_version: u8,
}

/// Parse the first [`HEAD_SIZE`] bytes of a 2pc queue object: the
/// `u16` magic `0xDEAD`, the head's `u64` length, the head, and the
/// urgent data inside it. A plain `queue` ring's head has no urgent
/// data and is refused.
pub fn parse_head(data: &[u8]) -> Result<HeadState> {
    let bad = |what: &str| OSDClientError::Other(format!("2pc_queue head: {what}"));
    let mut buf = data;
    if buf.len() < 10 {
        return Err(bad("shorter than its framing"));
    }
    if buf.get_u16_le() != QUEUE_HEAD_START {
        return Err(bad("bad magic"));
    }
    let len = usize::try_from(buf.get_u64_le()).map_err(|_| bad("length"))?;
    let mut body = buf.get(..len).ok_or_else(|| bad("length past the data"))?;
    let head = Head::decode(&mut body, 0)?;
    let urgent_data_version = *head
        .urgent_data
        .first()
        .ok_or_else(|| bad("no urgent data, not a 2pc_queue"))?;
    let urgent_data = UrgentData::decode(&mut head.urgent_data.clone(), 0)?;
    Ok(HeadState { head, urgent_data, urgent_data_version })
}

/// Read and parse the head of the 2pc queue on `oid`; see [`parse_head`].
/// Reservations spilled into [`URGENT_DATA_XATTR`] are not in it.
pub async fn read_head(ioctx: &IoCtx, oid: &str) -> Result<HeadState> {
    parse_head(&ioctx.read(oid, 0, HEAD_SIZE).await?.data)
}
```

The lines ending in a `//` comment are signatures whose bodies follow
the pattern of `queue.rs`'s async functions with the request named;
write them out in full (rustfmt).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline && cargo check --workspace --all-targets --offline && cargo check -p rados-cls --no-default-features --features two_pc_queue --offline && cargo check -p rados-cls --no-default-features --features user --offline`
Expected: the four new tests pass (ten in `two_pc_queue`); no warnings in
any check.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/two_pc_queue.rs rados-cls/src/call.rs rados-cls/src/lib.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
two_pc_queue: add the 2pc_queue client

Mirrors cls_2pc_queue_client.h: init, capacity, topic stats, reserve,
commit, abort, list reservations, list and remove entries, and expire,
on the "2pc_queue" class, reusing the queue class's types and decoders
where the class forwards to it. Reserve is a write that replies with
the new id, so it runs with RETURNVEC through call::exec_returnvec,
whose gate widens. A head reader parses the queue head and the class's
urgent data, the only view of reserved_size, which a Squid OSD leaks by
ten bytes per reserved entry. The module doc records what radosgw does
with the class, for a driver that must coexist with it.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 3: Cluster tests: the class semantics

**Files:**
- Create: `rados-cls/tests/cls_2pc_queue.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both `rados-cls` loops
  gain `cls_2pc_queue` at the end).

**Interfaces:**
- Consumes: Tasks 1 and 2; `rados/tests/common/mod.rs` via `#[path]`;
  `OpBuilder::{create, op, returnvec, build}`, `IoCtx::{execute_op,
  exec, create, remove, get_xattr}`.
- Produces: nine `#[ignore]` tests; Task 4 adds two to the same file.

- [ ] **Step 1: Write the tests**

The file head, helpers as `cls_queue_gc.rs` has them (`unique`,
`is_osd_error`, `payloads`, `markers`), plus:

```rust
//! Cluster tests for the 2pc_queue class, against Ceph v19.2.2. Run with:
//!   CEPH_CONF=... cargo test -p rados-cls --test cls_2pc_queue -- --ignored --nocapture

#[path = "../../rados/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use common::create_ioctx;
use rados::{IoCtx, OSDClientError, OpBuilder, UTime};
use rados_cls::queue;
use rados_cls::two_pc_queue::{self as q, Reservation, UrgentData};

const ENOENT: i32 = 2;
const EEXIST: i32 = 17;
const EINVAL: i32 = 22;
const ENOSPC: i32 = 28;
const ENODATA: i32 = 61;

/// 256 KiB: upstream ReserveError's queue.
const CAPACITY: u64 = 256 * 1024;

/// Create and init in one op, as RGW does.
async fn new_queue(ioctx: &IoCtx, prefix: &str, size: u64) -> String {
    let oid = unique(prefix);
    let op = OpBuilder::new()
        .create(true)
        .op(q::init_op(size).expect("op"))
        .build();
    ioctx.execute_op(&oid, op).await.expect("create and init");
    oid
}

/// The next instant after `t`, for a strict `<` cut just past it.
fn after(t: UTime) -> UTime {
    if t.nsec == 999_999_999 {
        UTime { sec: t.sec + 1, nsec: 0 }
    } else {
        UTime { nsec: t.nsec + 1, ..t }
    }
}

fn ids(r: &BTreeMap<u32, Reservation>) -> Vec<u32> {
    r.keys().copied().collect()
}

fn one(b: &'static [u8]) -> Vec<Bytes> {
    vec![Bytes::from_static(b)]
}
```

Tests (each `#[tokio::test] #[ignore]`, `common::init_tracing()`,
`create_ioctx()`, and `ioctx.remove` of every object it made at the
end); `q` is the `two_pc_queue` import, `oid = new_queue(&ioctx, prefix, CAPACITY)`
unless stated:

1. `init_and_capacity`:
   - `q::get_capacity == CAPACITY`, and `queue::get_capacity` on the
     same object (the plain class's method) also `== CAPACITY`.
   - `q::init(&oid, CAPACITY)` again is `EEXIST`; `q::init` on a fresh
     `unique` oid never created is `ENOENT`.
   - `q::read_head`: `max_head_size == 24576`, `front == tail ==
     queue::Marker { offset: 24576, generation: 0 }`, `queue_size ==
     CAPACITY + 24576`, `max_urgent_data_size == 23552`, `urgent_data ==
     UrgentData::default()`, and `urgent_data_version` is 2 or 3 (2 on
     the local v19.2.2; print it).
   - `get_topic_stats` is `{queue_size: 0, queue_entries: 0}`;
     `list_reservations` is empty; `list_entries("", 10)` is empty, not
     truncated, `next_marker == "0/24576"`.
2. `reserve_ids_start_at_one` (upstream `Reserve`):
   - For `i` in 1..=10, `reserve(100, i) == i`.
   - `list_reservations` has ids 1..=10, each `size == 100`, `entries ==
     id`, `timestamp.sec != 0`.
   - `read_head`: `last_id == 10`, `reserved_size == 1550` (10·100 plus
     10·(1+…+10)), the reservations equal the listing, `has_xattrs`
     false.
3. `reserve_errors`:
   - `reserve(0, 1)` and `reserve(1, 0)` are `EINVAL`.
   - A plain ring (`create(true)` + `queue::init_op(CAPACITY)` in one op)
     answers `reserve(1, 1)` with `EINVAL` (its urgent data does not
     decode).
   - An object from `ioctx.create(&oid, true)` with no init (zero
     length) is `EINVAL`; a missing object is `ENOENT`.
   - The failures persisted nothing: the next `reserve(1, 1)` on the
     2pc queue returns 1.
4. `reserve_enospc_at_the_exact_boundary` (upstream `ReserveError`):
   - 253 `reserve(1024, 1)` succeed (253 × 1034 = 261,602 ≤ 262,144).
   - The 254th is `ENOSPC` (262,636).
   - `list_reservations` has 253; `read_head.urgent_data.reserved_size
     == 261_602`.
5. `reserve_commit_list`:
   - `id = reserve(1000, 3)`; `commit(id, [b"a", b"bb"])`.
   - `list_entries("", 10)`: payloads `["a", "bb"]`, markers
     `["0/24576", "0/24587"]`, `next_marker == "0/24599"`, not truncated.
   - `list_reservations` is empty.
   - `get_topic_stats == {queue_size: 23, queue_entries: 3}`: 1+10 plus
     2+10 bytes, and the reserved `entries` (3), not the payload count.
   - `commit(id, one(b"x"))` again is `ENOENT` (queue not spilled).
   - `id2 = reserve(4, 1)`; `commit(id2, one(b"12345"))` is `EINVAL` and
     `list_reservations` still holds `id2`; `commit(id2, one(b"1234"))`
     succeeds.
6. `abort_of_an_unknown_id_succeeds` (upstream `AbortError`):
   - `id = reserve(100, 1)`; `abort(id)` leaves `list_reservations`
     empty.
   - `abort(id)` again and `abort(9999)` both succeed.
7. `expire_is_strictly_before_the_stale_time`:
   - `a = reserve(10, 1)`; sleep 50 ms; `b = reserve(10, 1)`.
   - From `list_reservations` take `ta`, `tb` (OSD clock) and assert
     `ta < tb`.
   - `expire_reservations(UTime::default())` removes nothing (ids
     `[a, b]`).
   - `expire_reservations(tb)` removes only `a`: `tb` is not strictly
     before itself.
   - `expire_reservations(after(tb))` removes `b`.
8. `remove_entries_and_committed_entries` (upstream `UpgradeFromReef`
   for the last part):
   - Three `reserve(1, 1)` + `commit(id, one(b"x"))`.
   - `list_entries` markers are `["0/24576", "0/24587", "0/24598"]`.
   - `remove_entries("0/24598", 2)` leaves one entry and `queue_entries
     == 1`.
   - Re-list, then `remove_entries(&list.next_marker, 0)`: the class
     counts the one entry itself; the list is empty and `queue_entries == 0`.
   - Two more single commits; re-list, then a v1 `queue::RemoveOp {
     end_marker: list.next_marker }` sent raw:
     `ioctx.exec(&oid, q::CLASS, "2pc_queue_remove_entries",
     rados::encode_with_capacity(&op, 0).expect("encode"))`. Empty list, `queue_entries
     == 0`.
   - The asymmetry: `id = reserve(100, 3)`, `commit(id, [b"x", b"y"])`,
     `queue_entries == 3`; `remove_entries(&next, 0)` counts the two
     payloads, so `queue_entries == 1`.
9. `reserve_without_returnvec_loses_the_id`:
   - `ioctx.execute_op(&oid, OpBuilder::new().op(q::reserve_op(100,
     1).expect("op")).build())` succeeds; its `first_outdata()` is
     empty and `q::decode_reserve(result.first_reply().expect("reply"))`
     is an error.
   - `list_reservations` has id 1; `q::reserve(100, 1)` returns 2.

`test-with-ceph.yml`: both `rados-cls` loops append `cls_2pc_queue`.

- [ ] **Step 2: Run against the cluster**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_2pc_queue -- --ignored --nocapture`
Expected: 9 passed.

- [ ] **Step 3: Commit**

```bash
git add rados-cls/tests/cls_2pc_queue.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
two_pc_queue: cluster tests for the class semantics

Pins against Ceph v19: init lays out a 24 KiB head so the first entry
is 0/24576, a second init is EEXIST and one on a missing object ENOENT;
ids start at 1; reserve refuses a zero size or count and a non-2pc
object with EINVAL and answers ENOSPC exactly when size plus ten bytes
per entry exceeds what is free; commit adds the reserved entry count,
not the payload count, to the topic stats and refuses payloads past the
reservation; aborting an unknown id succeeds; expiry is strictly before
the stale time on the OSD's clock; removal with no count makes the
class count, including for a Reef client's request; and without
RETURNVEC the reservation is made but its id is lost.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 4: Cluster tests: Squid's leak and the xattr spill-over

**Files:**
- Modify: `rados-cls/tests/cls_2pc_queue.rs`.

**Interfaces:**
- Consumes: Task 3's helpers; `q::read_head`, `q::URGENT_DATA_XATTR`,
  `IoCtx::get_xattr`, `rados::Denc` (to decode the xattr map).
- Produces: two `#[ignore]` tests.

- [ ] **Step 1: Write the tests**

10. `squid_leaks_the_entry_overhead`:
    - `oid = new_queue(CAPACITY)`; `v = read_head.urgent_data_version`;
      `leaks = v < 3`; print `v`.
    - `id = reserve(100, 3)`: `reserved_size == 130` (either version).
    - `commit(id, [30 bytes; 3])`: `reserved_size == if leaks { 30 } else
      { 0 }`.
    - `id = reserve(50, 2)`; `abort(id)`: `reserved_size == if leaks { 50
      } else { 0 }`.
    - `id = reserve(40, 1)`; take its stamp `t` from `list_reservations`;
      `expire_reservations(after(t))`: `reserved_size == if leaks { 60 }
      else { 0 }`, and no reservation open.
    - Empty the ring: `remove_entries(&list_entries("", 10).next_marker,
      0)`; `get_topic_stats == {0, 0}`, so every byte is free again.
    - If `leaks`: `reserve(CAPACITY - 60 - 10 + 1, 1)` is `ENOSPC` and
      `reserve(CAPACITY - 60 - 10, 1)` succeeds: sixty bytes of an empty
      ring can no longer be reserved. Otherwise `reserve(CAPACITY - 10,
      1)` succeeds.
    - A comment on the test: v19.2.2 is expected to leak, v19.2.4+ and
      main not; a client cannot repair it.
11. `reservations_spill_into_the_xattr_at_785`:
    - `oid = new_queue(CAPACITY)`; 784 `reserve(1, 1)`.
    - `read_head`: 784 reservations in the head, `has_xattrs` false.
    - `ioctx.get_xattr(&oid, URGENT_DATA_XATTR)` is an error (`ENODATA`;
      the class accepts `ENOENT` or `ENODATA` at `cls_2pc_queue.cc:165`).
    - The 785th reservation returns 785: the head still holds 784,
      `has_xattrs` is true.
    - The xattr decodes (`BTreeMap::<u32, Reservation>::decode(&mut
      xattr, 0)`, no struct header) to ids `[785]`;
      `list_reservations` has 785 ids (1..=785).
    - `commit(785, one(b"x"))` succeeds; `list_reservations` has 784; the
      xattr now decodes to an empty map; `has_xattrs` stays true.
    - `reserve(1, 1)` returns 786 and spills again (xattr ids `[786]`);
      `abort(786)` succeeds and leaves 784; `abort(9999)` succeeds.
    - Do not commit an unknown id here: on a spilled v19.2.2 queue that
      compares iterators of two maps, undefined behaviour, so its answer
      is not pinned (the `commit_op` doc says so).

- [ ] **Step 2: Run against the cluster**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_2pc_queue -- --ignored --nocapture`
Expected: 11 passed; `squid_leaks_the_entry_overhead` prints version 2.

- [ ] **Step 3: Commit**

```bash
git add rados-cls/tests/cls_2pc_queue.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
two_pc_queue: pin Squid's capacity leak and the xattr spill-over

On v19.2.2 reserve counts ten bytes per entry into reserved_size that
commit, abort and expire never give back, so an emptied queue can no
longer be reserved in full; the test asserts that when the head's
urgent data is version 2 and asserts no leak at version 3, the fixed
writers of v19.2.4 and main. The head holds 784 reservations; the
785th spills into the cls_queue_urgent_data xattr, where list, commit
and abort still find it and has_xattrs stays set.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6, with: the per-commit proof from the branch base
(each of the four commits builds and passes its own unit tests); the
container clippy run over the workspace and over every single class
feature, `two_pc_queue` alone included; the corpus loop over the eight
names (`cls_2pc_reservation`, `cls_2pc_urgent_data`,
`cls_2pc_queue_{reserve,commit,abort,expire}_op`,
`cls_2pc_queue_{reserve,reservations}_ret`) with zero decode failures in
both archives, 18.2.0's version-1 samples included, and format
differences only on the two exception rows' multi-entry samples; the
cluster suites through `cls_2pc_queue`; the PR body:

```
**Motivation.** RGW's persistent bucket notifications live in the `2pc_queue` object class: a write reserves queue space before the object write and commits the event after; a Rust RGW needs `cls_2pc_queue_client.h`.

**What changed.** A `two_pc_queue` module in `rados-cls`: the ten methods, `reserve` through `RETURNVEC`; the eight `ceph-dencoder` types plus `cls_2pc_queue_remove_op` and `cls_queue_get_stats_ret`; a head reader; and cluster tests pinning v19.2.2's semantics, including its reserved-size leak.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

Then merge when green and open the upstream PR from the same branch.

---

## Roadmap for later plans

Plan 15 (`cls-otp`), then plan 12 (`watch-notify`). Deferred, all
outside this crate's class-mirror scope: the RGW notification layer
(queue naming, main's shard fan-out, the registry, the ownership lock
and its renewal, the 4 KiB reservation, the expiry loop), which is the
driver's; a `reserved_size` drift report from `read_head` plus
`list_reservations` (no repair is possible on Squid, so no helper);
main's `ObjectWriteOperation` overload of the list client
(`M:cls_2pc_queue_client.cc:198-206`), which a compound can already
build from `list_entries_op`. For the rgw-go coexistence document, the
report's §0.5 corrections stand: radosgw takes no lock on
`queues_list_object`, and it expires reservations only on queues it
owns.
