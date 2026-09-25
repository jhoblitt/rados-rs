# rados-rs RGW MVP, plan 4 of N: `cls-user`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `user` object class to `rados-cls`: the bucket list, set
and remove, header and stats reset, and account-resource functions of
`cls_user_client.h`, with wire bytes pinned in unit tests, all nineteen
corpus-covered types compared against `ceph-dencoder`, and cluster tests
against Ceph v19.2.2.

**Architecture:** One module `user` behind feature `user`, following the
`version`/`refcount` pattern: structs encoded as Ceph does (two of them
hand-written because their wire order or version depends on the value),
`OSDOp` constructors for compound use, async free functions over `IoCtx`.
A shared `dump` module renders timestamps the way `encode_json` renders a
`utime_t`, because every `real_time` in this class dumps as that string.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-user`".

## Global Constraints

Plan 3's Global Constraints apply unchanged (Squid floor with
`rados::check_min_version!` in hand-written `decode_content`; every derive
carries `#[denc(crate = "rados")]`; no `unwrap`/`expect` on production
paths; gates; container fmt/clippy by the controller; push after every
gate; merge when green then an upstream PR; commit style with the
`Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer; offline
builds with the scratchpad `CARGO_HOME`; no new dependencies). Plus:

- A feature enters `rados-cls`'s `default` list in the commit that adds its
  module, so every commit builds alone.
- Branch `cls-user` is based on the fork's `main` after plan 3 merges.
- `ceph::real_time` is `rados::UTime` on the wire (u32 seconds, u32
  nanoseconds, no version header), and dumps as `utime_t::gmtime` prints
  it: seconds below 315,360,000 (ten years) as `<sec>.<usec>` with six
  microsecond digits, later times as `YYYY-MM-DDTHH:MM:SS.uuuuuuZ`.

## Review Focus

1. `cls_user_bucket` encodes as version 7 (compat 3, with the three
   explicit-placement pools) when `placement_id` is empty and as version 9
   (compat 8) otherwise, and RGW never sets `placement_id`, so the corpus
   samples are all version 7; a Rust re-encode must be byte-identical.
   Pinned in Task 2 (unit) and the corpus run in Task 5.
2. `cls_user_bucket_entry` puts `bucket` in the middle of its wire order
   and carries a legacy 32-bit `mt` copy of `creation_time` before it;
   the JSON order is different again. Pinned in Task 2.
3. `list_buckets` pages by `marker` (exclusive start) with `truncated`, and
   `end_marker` stops before the first name at or past it while forcing
   `truncated` false; the reply's `marker` is set only when truncated.
   Pinned in Task 4 (`user_list_buckets_pages`).
4. `remove_bucket` of an absent bucket is a success and leaves the header
   alone; `set_buckets` with `add` false skips absent buckets; the header
   stats follow the entries (`total_entries`, `total_bytes`,
   `total_bytes_rounded`). Pinned in Task 4 (`user_set_list_remove_buckets`).
5. Account resources are keyed by the lower-cased name (so `get("alpha")`
   finds `"Alpha"`), a new resource past `limit` is `EUSERS`, an existing
   one with `exclusive` is `EEXIST`, `get`/`rm` of a missing one is
   `ENOENT`, and `list` filters by path prefix on the class side. Pinned
   in Task 4 (`user_account_resources`).

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cls-user` off the fork's `main` (which contains plan
  3's merge) checked out in `$R`; the SDD ledger; the cluster up.

- [ ] **Step 1: Branch, cluster, ledger**

```bash
cd $R && git fetch origin main && git checkout -b cls-user origin/main && git log --oneline -1
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
```
Expected: HEAD is the merge of the `cls-crate` PR; three containers up.
Then the workspace and ledger as in plan 3's Task 0.

---

### Task 1: The `dump` module: timestamps as `encode_json` prints them

**Files:**
- Create: `rados-cls/src/dump.rs`.
- Modify: `rados-cls/src/lib.rs` (`pub(crate) mod dump;`).

**Interfaces:**
- Consumes: `rados::UTime { sec: u32, nsec: u32 }`.
- Produces: `pub(crate) fn utime<S: serde::Serializer>(t: &UTime, s: S)
  -> Result<S::Ok, S::Error>` for `#[serde(serialize_with =
  "crate::dump::utime")]`; `pub(crate) fn gmtime(t: &UTime) -> String`.
  Task 2 uses `utime` on every timestamp field.

- [ ] **Step 1: Write the failing tests**

Create `rados-cls/src/dump.rs`:

```rust
//! Renderings that match `ceph-dencoder`'s `dump_json` where `serde`'s
//! defaults do not.

use rados::UTime;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_times_print_as_seconds_and_microseconds() {
        // utime_t::gmtime: below ten years of seconds it prints the raw count.
        assert_eq!(gmtime(&UTime { sec: 1, nsec: 0 }), "1.000000");
        assert_eq!(gmtime(&UTime { sec: 12345, nsec: 0 }), "12345.000000");
        assert_eq!(gmtime(&UTime { sec: 0, nsec: 999_999_999 }), "0.999999");
    }

    #[test]
    fn absolute_times_print_as_iso_8601_with_microseconds() {
        // A cls_user_bucket_entry corpus sample: 0x66f91c3f seconds,
        // 753627000 nanoseconds, which ceph-dencoder dumps as below.
        assert_eq!(
            gmtime(&UTime {
                sec: 0x66f9_1c3f,
                nsec: 753_627_000
            }),
            "2024-09-29T09:22:07.753627Z"
        );
        assert_eq!(
            gmtime(&UTime {
                sec: 315_360_000,
                nsec: 0
            }),
            "1979-12-30T00:00:00.000000Z"
        );
        assert_eq!(
            gmtime(&UTime {
                sec: u32::MAX,
                nsec: 0
            }),
            "2106-02-07T06:28:15.000000Z"
        );
    }

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(19_995), (2024, 9, 29));
    }

    #[test]
    fn serializes_as_a_json_string() {
        #[derive(serde::Serialize)]
        struct T {
            #[serde(serialize_with = "utime")]
            t: UTime,
        }
        assert_eq!(
            serde_json::to_value(T {
                t: UTime { sec: 12345, nsec: 0 }
            })
            .expect("json"),
            serde_json::json!({"t": "12345.000000"})
        );
    }
}
```

Add `pub(crate) mod dump;` to `rados-cls/src/lib.rs` after `mod call;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline dump`
Expected: FAIL to compile: `gmtime`, `civil_from_days`, `utime` not found.

- [ ] **Step 3: Implement**

Insert between the `use` line and the test module:

```rust
/// `encode_json` of a `utime_t` streams `utime_t::gmtime`: a count of
/// seconds below ten years prints as `<sec>.<usec>`, anything later as
/// ISO 8601 with six microsecond digits and a `Z`.
pub(crate) fn utime<S: serde::Serializer>(
    t: &UTime,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&gmtime(t))
}

/// `utime_t::gmtime` with `legacy_form` false.
pub(crate) fn gmtime(t: &UTime) -> String {
    let usec = t.nsec / 1000;
    if t.sec < 315_360_000 {
        return format!("{}.{usec:06}", t.sec);
    }
    let days = i64::from(t.sec / 86_400);
    let secs = t.sec % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{usec:06}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`;
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline dump`
Expected: 4 passed, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/dump.rs rados-cls/src/lib.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: dump timestamps as encode_json prints a utime_t

Every real_time in the user class dumps through encode_json's utime_t
overload, which streams utime_t::gmtime: a raw second count below ten
years, otherwise ISO 8601 with microseconds and a Z. The corpus harness
compares that string.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 2: The `user` class types

**Files:**
- Create: `rados-cls/src/user.rs` (types, tests; the client functions come
  in Task 3).
- Modify: `rados-cls/Cargo.toml` (feature `user`, added to `default`),
  `rados-cls/src/lib.rs` (`#[cfg(feature = "user")] pub mod user;`),
  `rados-dencoder/src/main.rs`, `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`.

**Interfaces:**
- Consumes: `rados::{Denc, RadosError, UTime, VersionedDenc,
  VersionedEncode}`, `rados::check_min_version!`,
  `rados::impl_denc_for_versioned!`, `crate::dump::utime`, `bytes::Bytes`.
- Produces: `user::CLASS = "user"`; types `Bucket`, `ExplicitPlacement`,
  `BucketEntry`, `Stats`, `Header`, `AccountHeader`, `AccountResource`,
  `SetBucketsOp`, `RemoveBucketOp`, `ListBucketsOp`, `ListBucketsRet`,
  `GetHeaderOp`, `GetHeaderRet`, `CompleteStatsSyncOp`, `ResetStatsOp`,
  `ResetStats2Op`, `ResetStats2Ret`, `AccountResourceAddOp`,
  `AccountResourceGetOp`, `AccountResourceGetRet`, `AccountResourceRmOp`,
  `AccountResourceListOp`, `AccountResourceListRet`, all `Denc +
  Serialize + Default + PartialEq + Clone + Debug`. Task 3 builds the
  client on them.

- [ ] **Step 1: Write the failing unit tests**

Create `rados-cls/src/user.rs` with the module doc and tests:

```rust
//! The `user` class: RGW's per-user bucket index (an omap of
//! `cls_user_bucket_entry` keyed by bucket name, with running stats in the
//! omap header) and, since Squid, the account-resource index (an omap
//! keyed by lower-cased resource name). Mirrors `cls_user_client.h`.
//!
//! Server facts the API rests on (`src/cls/user/cls_user.cc`): `set_buckets`
//! with `add` skips nothing and only refreshes `bucket_id` and
//! `creation_time` of an existing entry, without `add` it overwrites stats
//! and skips absent buckets; `remove_bucket` of an absent bucket succeeds;
//! `list_buckets` caps `max_entries` at 1000, treats `marker` as exclusive
//! and stops before the first name at or past `end_marker`; account
//! resources past `limit` are `EUSERS`, an `exclusive` add of an existing
//! one is `EEXIST`, `get` and `rm` of a missing one are `ENOENT`.

#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;

    fn s(v: &str) -> String {
        v.to_owned()
    }

    /// A Ceph string: u32 length then bytes.
    fn str_wire(v: &str) -> Vec<u8> {
        let mut w = (v.len() as u32).to_le_bytes().to_vec();
        w.extend_from_slice(v.as_bytes());
        w
    }

    /// An ENCODE_START(v, compat) frame around `content`.
    fn frame(v: u8, compat: u8, content: &[u8]) -> Vec<u8> {
        let mut w = vec![v, compat];
        w.extend_from_slice(&(content.len() as u32).to_le_bytes());
        w.extend_from_slice(content);
        w
    }

    /// cls_user_gen_test_bucket(i): buck.i / mark.i / bucket.id.i
    fn test_bucket(i: u32) -> Bucket {
        Bucket {
            name: format!("buck.{i}"),
            marker: format!("mark.{i}"),
            bucket_id: format!("bucket.id.{i}"),
            ..Bucket::default()
        }
    }

    /// cls_user_bucket with an empty placement_id: version 7, compat 3,
    /// name, data_pool, marker, bucket_id, index_pool, data_extra_pool.
    fn test_bucket_wire(i: u32) -> Vec<u8> {
        let mut c = str_wire(&format!("buck.{i}"));
        c.extend_from_slice(&str_wire(""));
        c.extend_from_slice(&str_wire(&format!("mark.{i}")));
        c.extend_from_slice(&str_wire(&format!("bucket.id.{i}")));
        c.extend_from_slice(&str_wire(""));
        c.extend_from_slice(&str_wire(""));
        frame(7, 3, &c)
    }

    /// cls_user_gen_test_bucket_entry(i).
    fn test_entry(i: u32) -> BucketEntry {
        BucketEntry {
            bucket: test_bucket(i),
            size: u64::from(i + 1),
            size_rounded: u64::from(i + 2),
            creation_time: UTime {
                sec: i + 3,
                nsec: 0,
            },
            count: u64::from(i + 4),
            user_stats_sync: true,
        }
    }

    #[test]
    fn bucket_without_placement_encodes_the_legacy_version() {
        let bytes = encode_with_capacity(&test_bucket(0), 0).expect("encode");
        assert_eq!(bytes.as_ref(), &test_bucket_wire(0)[..]);
        assert_eq!(&bytes[..2], &[7, 3]);
        assert_eq!(Bucket::decode(&mut bytes.clone(), 0).expect("decode"), test_bucket(0));
    }

    #[test]
    fn bucket_with_placement_encodes_version_nine() {
        let bucket = Bucket {
            placement_id: s("default-placement"),
            ..test_bucket(1)
        };
        let bytes = encode_with_capacity(&bucket, 0).expect("encode");
        let mut c = str_wire("buck.1");
        c.extend_from_slice(&str_wire("mark.1"));
        c.extend_from_slice(&str_wire("bucket.id.1"));
        c.extend_from_slice(&str_wire("default-placement"));
        assert_eq!(bytes.as_ref(), &frame(9, 8, &c)[..]);
        assert_eq!(Bucket::decode(&mut bytes.clone(), 0).expect("decode"), bucket);
    }

    #[test]
    fn bucket_version_eight_with_empty_placement_carries_pools() {
        let mut c = str_wire("b");
        c.extend_from_slice(&str_wire("m"));
        c.extend_from_slice(&str_wire("id"));
        c.extend_from_slice(&str_wire(""));
        c.extend_from_slice(&str_wire("dp"));
        c.extend_from_slice(&str_wire("ip"));
        c.extend_from_slice(&str_wire("xp"));
        let wire = frame(8, 3, &c);
        let bucket = Bucket::decode(&mut &wire[..], 0).expect("decode");
        assert_eq!(bucket.explicit_placement.data_pool, "dp");
        assert_eq!(bucket.explicit_placement.index_pool, "ip");
        assert_eq!(bucket.explicit_placement.data_extra_pool, "xp");
    }

    #[test]
    fn bucket_below_the_floor_is_rejected() {
        let c = str_wire("b");
        assert!(Bucket::decode(&mut &frame(6, 3, &c)[..], 0).is_err());
    }

    #[test]
    fn bucket_entry_wire_order_differs_from_its_fields() {
        // cls_user_bucket_entry::encode, version 9 compat 5: an empty
        // string, size, a u32 copy of the creation seconds, count, the
        // bucket, size_rounded, user_stats_sync, creation_time.
        let entry = test_entry(0);
        let bytes = encode_with_capacity(&entry, 0).expect("encode");
        let mut c = str_wire("");
        c.extend_from_slice(&1u64.to_le_bytes());
        c.extend_from_slice(&3u32.to_le_bytes());
        c.extend_from_slice(&4u64.to_le_bytes());
        c.extend_from_slice(&test_bucket_wire(0));
        c.extend_from_slice(&2u64.to_le_bytes());
        c.push(1);
        c.extend_from_slice(&3u32.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(c.len(), 94);
        assert_eq!(bytes.as_ref(), &frame(9, 5, &c)[..]);
        assert_eq!(BucketEntry::decode(&mut bytes.clone(), 0).expect("decode"), entry);
    }

    #[test]
    fn stats_header_and_ops_are_flat_version_one_structs() {
        let stats = Stats {
            total_entries: 1,
            total_bytes: 2,
            total_bytes_rounded: 3,
        };
        let mut c = 1u64.to_le_bytes().to_vec();
        c.extend_from_slice(&2u64.to_le_bytes());
        c.extend_from_slice(&3u64.to_le_bytes());
        let stats_wire = frame(1, 1, &c);
        assert_eq!(encode_with_capacity(&stats, 0).expect("encode").as_ref(), &stats_wire[..]);

        let header = Header {
            stats: stats.clone(),
            last_stats_sync: UTime { sec: 1, nsec: 0 },
            last_stats_update: UTime { sec: 2, nsec: 0 },
        };
        let mut c = stats_wire.clone();
        c.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(encode_with_capacity(&header, 0).expect("encode").as_ref(), &frame(1, 1, &c)[..]);

        let op = ListBucketsOp {
            marker: s("marker"),
            max_entries: 1000,
            end_marker: String::new(),
        };
        let mut c = str_wire("marker");
        c.extend_from_slice(&1000i32.to_le_bytes());
        c.extend_from_slice(&str_wire(""));
        assert_eq!(encode_with_capacity(&op, 0).expect("encode").as_ref(), &frame(2, 1, &c)[..]);

        assert_eq!(
            encode_with_capacity(&GetHeaderOp {}, 0).expect("encode").as_ref(),
            &[1, 1, 0, 0, 0, 0][..]
        );

        let add = AccountResourceAddOp {
            entry: AccountResource {
                name: s("name"),
                path: s("path"),
                metadata: Bytes::new(),
            },
            exclusive: false,
            limit: 0,
        };
        let mut c = str_wire("name");
        c.extend_from_slice(&str_wire("path"));
        c.extend_from_slice(&str_wire(""));
        let mut outer = frame(1, 1, &c);
        outer.push(0);
        outer.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(encode_with_capacity(&add, 0).expect("encode").as_ref(), &frame(1, 1, &outer)[..]);
    }

    #[test]
    fn json_matches_ceph_dencoder_dump() {
        // cls_user_bucket dumps name, marker, bucket_id only; the entry
        // dumps bucket, size, size_rounded, creation_time (gmtime), count,
        // user_stats_sync; stats and header as numbers and gmtime strings.
        assert_eq!(
            serde_json::to_value(test_entry(0)).expect("json"),
            serde_json::json!({
                "bucket": {"name": "buck.0", "marker": "mark.0", "bucket_id": "bucket.id.0"},
                "size": 1,
                "size_rounded": 2,
                "creation_time": "3.000000",
                "count": 4,
                "user_stats_sync": true
            })
        );
        assert_eq!(
            serde_json::to_value(ListBucketsOp {
                marker: s("m"),
                max_entries: 5,
                end_marker: s("hidden"),
            })
            .expect("json"),
            serde_json::json!({"marker": "m", "max_entries": 5})
        );
        assert_eq!(
            serde_json::to_value(GetHeaderOp {}).expect("json"),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(AccountResourceAddOp {
                entry: AccountResource {
                    name: s("name"),
                    path: s("path"),
                    metadata: Bytes::from_static(b"hidden"),
                },
                exclusive: true,
                limit: 7,
            })
            .expect("json"),
            serde_json::json!({"name": "name", "path": "path", "limit": 7})
        );
        assert_eq!(
            serde_json::to_value(AccountResourceListRet {
                entries: vec![AccountResource {
                    name: s("n"),
                    path: s("p"),
                    metadata: Bytes::new(),
                }],
                truncated: true,
                marker: s("n"),
            })
            .expect("json"),
            serde_json::json!({"entries": [{"name": "n", "path": "p"}], "truncated": true, "marker": "n"})
        );
        assert_eq!(
            serde_json::to_value(GetHeaderRet {
                header: Header {
                    stats: Stats::default(),
                    last_stats_sync: UTime { sec: 1, nsec: 0 },
                    last_stats_update: UTime { sec: 2, nsec: 0 },
                }
            })
            .expect("json"),
            serde_json::json!({"header": {
                "stats": {"total_entries": 0, "total_bytes": 0, "total_bytes_rounded": 0},
                "last_stats_sync": "1.000000",
                "last_stats_update": "2.000000"
            }})
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add to `rados-cls/Cargo.toml`: `user = []` under `[features]` and `"user"`
to `default`. Add `#[cfg(feature = "user")] pub mod user;` to `lib.rs`
(keep the `pub mod` lines alphabetical: refcount, user, version).

Run: `cargo test -p rados-cls --lib --offline user`
Expected: FAIL to compile: `Bucket`, `BucketEntry`, ... not found.

- [ ] **Step 3: Implement the types**

Insert between the module doc and the test module:

```rust
use bytes::{Buf, BufMut, Bytes};
use rados::{Denc, RadosError, UTime, VersionedDenc, VersionedEncode};
use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::dump;

/// The class name.
pub const CLASS: &str = "user";

/// Pools named directly on a bucket, from before placement rules; empty
/// in everything RGW writes today.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplicitPlacement {
    pub data_pool: String,
    pub index_pool: String,
    pub data_extra_pool: String,
}

/// `cls_user_bucket`. Encodes as version 9 (compat 8) when `placement_id`
/// is set and as version 7 (compat 3) with the explicit pools otherwise,
/// exactly as the C++ does; RGW leaves `placement_id` empty. The dump
/// carries only `name`, `marker` and `bucket_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Bucket {
    pub name: String,
    pub marker: String,
    pub bucket_id: String,
    #[serde(skip)]
    pub placement_id: String,
    #[serde(skip)]
    pub explicit_placement: ExplicitPlacement,
}

impl VersionedEncode for Bucket {
    const MAX_DECODE_VERSION: u8 = 9;

    fn encoding_version(&self, _features: u64) -> u8 {
        if self.placement_id.is_empty() { 7 } else { 9 }
    }

    fn compat_version(&self, _features: u64) -> u8 {
        if self.placement_id.is_empty() { 3 } else { 8 }
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        version: u8,
    ) -> std::result::Result<(), RadosError> {
        self.name.encode(buf, features)?;
        if version >= 8 {
            self.marker.encode(buf, features)?;
            self.bucket_id.encode(buf, features)?;
            self.placement_id.encode(buf, features)
        } else {
            self.explicit_placement.data_pool.encode(buf, features)?;
            self.marker.encode(buf, features)?;
            self.bucket_id.encode(buf, features)?;
            self.explicit_placement.index_pool.encode(buf, features)?;
            self.explicit_placement.data_extra_pool.encode(buf, features)
        }
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        // Squid writes 7 or 9; 8 was a transitional form worth reading.
        rados::check_min_version!(struct_v, 7, "Bucket", "Squid v19+");

        let mut bucket = Self {
            name: String::decode(buf, features)?,
            ..Self::default()
        };
        if struct_v >= 8 {
            bucket.marker = String::decode(buf, features)?;
            bucket.bucket_id = String::decode(buf, features)?;
            bucket.placement_id = String::decode(buf, features)?;
            if struct_v == 8 && bucket.placement_id.is_empty() {
                bucket.explicit_placement.data_pool = String::decode(buf, features)?;
                bucket.explicit_placement.index_pool = String::decode(buf, features)?;
                bucket.explicit_placement.data_extra_pool = String::decode(buf, features)?;
            }
        } else {
            bucket.explicit_placement.data_pool = String::decode(buf, features)?;
            bucket.marker = String::decode(buf, features)?;
            bucket.bucket_id = String::decode(buf, features)?;
            bucket.explicit_placement.index_pool = String::decode(buf, features)?;
            bucket.explicit_placement.data_extra_pool = String::decode(buf, features)?;
        }
        Ok(bucket)
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(Bucket);

/// `cls_user_bucket_entry`: a bucket and its stats as the user's index
/// records them. Version 9 (compat 5); the wire order is an empty legacy
/// string, `size`, a 32-bit copy of the creation seconds, `count`, the
/// bucket, `size_rounded`, `user_stats_sync`, `creation_time`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BucketEntry {
    pub bucket: Bucket,
    pub size: u64,
    pub size_rounded: u64,
    #[serde(serialize_with = "dump::utime")]
    pub creation_time: UTime,
    pub count: u64,
    pub user_stats_sync: bool,
}

impl VersionedEncode for BucketEntry {
    const MAX_DECODE_VERSION: u8 = 9;

    fn encoding_version(&self, _features: u64) -> u8 {
        9
    }

    fn compat_version(&self, _features: u64) -> u8 {
        5
    }

    fn encode_content<B: BufMut>(
        &self,
        buf: &mut B,
        features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        String::new().encode(buf, features)?;
        self.size.encode(buf, features)?;
        self.creation_time.sec.encode(buf, features)?;
        self.count.encode(buf, features)?;
        self.bucket.encode(buf, features)?;
        self.size_rounded.encode(buf, features)?;
        self.user_stats_sync.encode(buf, features)?;
        self.creation_time.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        rados::check_min_version!(struct_v, 9, "BucketEntry", "Squid v19+");

        let _legacy_name = String::decode(buf, features)?;
        let size = u64::decode(buf, features)?;
        let _legacy_mtime = u32::decode(buf, features)?;
        let count = u64::decode(buf, features)?;
        let bucket = Bucket::decode(buf, features)?;
        let size_rounded = u64::decode(buf, features)?;
        let user_stats_sync = bool::decode(buf, features)?;
        let creation_time = UTime::decode(buf, features)?;
        Ok(Self {
            bucket,
            size,
            size_rounded,
            creation_time,
            count,
            user_stats_sync,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(BucketEntry);

/// `cls_user_stats`: the running totals in the index header.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct Stats {
    pub total_entries: u64,
    pub total_bytes: u64,
    pub total_bytes_rounded: u64,
}

/// `cls_user_header`: the omap header of a user's bucket index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct Header {
    pub stats: Stats,
    #[serde(serialize_with = "dump::utime")]
    pub last_stats_sync: UTime,
    #[serde(serialize_with = "dump::utime")]
    pub last_stats_update: UTime,
}

/// `cls_user_account_header`: the omap header of an account's resource
/// index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountHeader {
    pub count: u32,
}

/// `cls_user_account_resource`: one resource of an account, indexed by
/// its lower-cased name; `metadata` is opaque and not dumped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResource {
    pub name: String,
    pub path: String,
    #[serde(skip)]
    pub metadata: Bytes,
}

/// `cls_user_set_buckets_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct SetBucketsOp {
    pub entries: Vec<BucketEntry>,
    pub add: bool,
    #[serde(serialize_with = "dump::utime")]
    pub time: UTime,
}

/// `cls_user_remove_bucket_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct RemoveBucketOp {
    pub bucket: Bucket,
}

/// `cls_user_list_buckets_op`: version 2 added `end_marker` after
/// `max_entries`; the dump leaves it out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 2, compat = 1)]
pub struct ListBucketsOp {
    pub marker: String,
    pub max_entries: i32,
    #[serde(skip)]
    pub end_marker: String,
}

/// `cls_user_list_buckets_ret`: `marker` is set only when `truncated`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ListBucketsRet {
    pub entries: Vec<BucketEntry>,
    pub marker: String,
    pub truncated: bool,
}

/// `cls_user_get_header_op`: an empty versioned struct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GetHeaderOp {}

impl VersionedEncode for GetHeaderOp {
    const MAX_DECODE_VERSION: u8 = 1;

    fn encoding_version(&self, _features: u64) -> u8 {
        1
    }

    fn compat_version(&self, _features: u64) -> u8 {
        1
    }

    fn encode_content<B: BufMut>(
        &self,
        _buf: &mut B,
        _features: u64,
        _version: u8,
    ) -> std::result::Result<(), RadosError> {
        Ok(())
    }

    fn decode_content<B: Buf>(
        _buf: &mut B,
        _features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        rados::check_min_version!(struct_v, 1, "GetHeaderOp", "Squid v19+");
        Ok(Self {})
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        Some(0)
    }
}

rados::impl_denc_for_versioned!(GetHeaderOp);

/// `cls_user_get_header_ret`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetHeaderRet {
    pub header: Header,
}

/// `cls_user_complete_stats_sync_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct CompleteStatsSyncOp {
    #[serde(serialize_with = "dump::utime")]
    pub time: UTime,
}

/// `cls_user_reset_stats_op`: recompute the header from every entry in
/// one call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ResetStatsOp {
    #[serde(serialize_with = "dump::utime")]
    pub time: UTime,
}

/// `cls_user_reset_stats2_op`: one page per call; the server ignores
/// `acc_stats` and only writes the header on the final page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ResetStats2Op {
    #[serde(serialize_with = "dump::utime")]
    pub time: UTime,
    pub marker: String,
    pub acc_stats: Stats,
}

/// `cls_user_reset_stats2_ret`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ResetStats2Ret {
    pub marker: String,
    pub acc_stats: Stats,
    pub truncated: bool,
}

/// `cls_user_account_resource_add_op`; the dump flattens the entry's
/// `name` and `path` next to `limit` and omits `exclusive`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceAddOp {
    pub entry: AccountResource,
    pub exclusive: bool,
    pub limit: u32,
}

impl Serialize for AccountResourceAddOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AccountResourceAddOp", 3)?;
        state.serialize_field("name", &self.entry.name)?;
        state.serialize_field("path", &self.entry.path)?;
        state.serialize_field("limit", &self.limit)?;
        state.end()
    }
}

/// `cls_user_account_resource_get_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceGetOp {
    pub name: String,
}

/// `cls_user_account_resource_get_ret`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceGetRet {
    pub entry: AccountResource,
}

/// `cls_user_account_resource_rm_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceRmOp {
    pub name: String,
}

/// `cls_user_account_resource_list_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceListOp {
    pub marker: String,
    pub path_prefix: String,
    pub max_entries: u32,
}

/// `cls_user_account_resource_list_ret`: `marker` and `truncated` describe
/// the omap page, not the entries left after the path filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AccountResourceListRet {
    pub entries: Vec<AccountResource>,
    pub truncated: bool,
    pub marker: String,
}
```

Register the nineteen corpus-covered types with the dencoder. In
`rados-dencoder/src/main.rs` add (wrapped under 100 columns):

```rust
use rados_cls::user::{
    AccountHeader as UserAccountHeader, AccountResource as UserAccountResource,
    AccountResourceAddOp as UserAccountResourceAddOp,
    AccountResourceGetOp as UserAccountResourceGetOp,
    AccountResourceGetRet as UserAccountResourceGetRet,
    AccountResourceListOp as UserAccountResourceListOp,
    AccountResourceListRet as UserAccountResourceListRet,
    AccountResourceRmOp as UserAccountResourceRmOp, Bucket as UserBucket,
    BucketEntry as UserBucketEntry, CompleteStatsSyncOp as UserCompleteStatsSyncOp,
    GetHeaderOp as UserGetHeaderOp, GetHeaderRet as UserGetHeaderRet, Header as UserHeader,
    ListBucketsOp as UserListBucketsOp, ListBucketsRet as UserListBucketsRet,
    RemoveBucketOp as UserRemoveBucketOp, SetBucketsOp as UserSetBucketsOp, Stats as UserStats,
};
```

and in `get_type_info`, after the refcount arms:

```rust
        "cls_user_bucket" => Some(type_info_denc::<UserBucket>()),
        "cls_user_bucket_entry" => Some(type_info_denc::<UserBucketEntry>()),
        "cls_user_stats" => Some(type_info_denc::<UserStats>()),
        "cls_user_header" => Some(type_info_denc::<UserHeader>()),
        "cls_user_account_header" => Some(type_info_denc::<UserAccountHeader>()),
        "cls_user_account_resource" => Some(type_info_denc::<UserAccountResource>()),
        "cls_user_set_buckets_op" => Some(type_info_denc::<UserSetBucketsOp>()),
        "cls_user_remove_bucket_op" => Some(type_info_denc::<UserRemoveBucketOp>()),
        "cls_user_list_buckets_op" => Some(type_info_denc::<UserListBucketsOp>()),
        "cls_user_list_buckets_ret" => Some(type_info_denc::<UserListBucketsRet>()),
        "cls_user_get_header_op" => Some(type_info_denc::<UserGetHeaderOp>()),
        "cls_user_get_header_ret" => Some(type_info_denc::<UserGetHeaderRet>()),
        "cls_user_complete_stats_sync_op" => Some(type_info_denc::<UserCompleteStatsSyncOp>()),
        "cls_user_account_resource_add_op" => Some(type_info_denc::<UserAccountResourceAddOp>()),
        "cls_user_account_resource_get_op" => Some(type_info_denc::<UserAccountResourceGetOp>()),
        "cls_user_account_resource_get_ret" => Some(type_info_denc::<UserAccountResourceGetRet>()),
        "cls_user_account_resource_rm_op" => Some(type_info_denc::<UserAccountResourceRmOp>()),
        "cls_user_account_resource_list_op" => Some(type_info_denc::<UserAccountResourceListOp>()),
        "cls_user_account_resource_list_ret" => Some(type_info_denc::<UserAccountResourceListRet>()),
```

and in `list_types`, after the refcount line:

```rust
    println!("  cls_user_* (19 types) - user class: buckets, header, stats, account resources [versioned]");
```

In `rados-dencoder/tests/dencoder_corpus_comparison_test.rs` add to
`CORPUS_TYPES` after the refcount entries, one `TypeSpec::new("<name>",
None, false),` line per name in the same order as the arms above (the
eight account-resource types have no samples in the 18.2.0 archive; the
harness warns and skips them there).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --lib --offline && cargo build -p rados-dencoder --offline && cargo test -p rados-dencoder --offline --test dencoder_corpus_comparison_test --no-run`
Expected: all pass, seven new tests in `rados-cls`, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/Cargo.toml rados-cls/src/lib.rs rados-cls/src/user.rs rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the user class types

Every struct of cls_user_types.h and cls_user_ops.h. cls_user_bucket
encodes as version 7 with the explicit pools unless placement_id is
set, then as version 9, as the C++ does; cls_user_bucket_entry keeps
its legacy wire order with the bucket in the middle. Timestamps dump
as encode_json prints a utime_t. All nineteen corpus-covered types
join the dencoder.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 3: The `user` class client

**Files:**
- Modify: `rados-cls/src/user.rs` (client functions after the types;
  tests).

**Interfaces:**
- Consumes: Task 2's types; `crate::call::{op, bare_op, exec, decode,
  decode_bytes}`; `rados::osdclient::{IoCtx, OSDOp, OpReply}`.
- Produces: op constructors `set_buckets_op(&[BucketEntry], bool, UTime)`,
  `complete_stats_sync_op(UTime)`, `remove_bucket_op(&Bucket)`,
  `list_buckets_op(&str, &str, i32)`, `get_header_op()`,
  `reset_stats_op(UTime)`, `reset_stats2_op(&ResetStats2Op)`,
  `account_resource_add_op(&AccountResource, bool, u32)`,
  `account_resource_get_op(&str)`, `account_resource_rm_op(&str)`,
  `account_resource_list_op(&str, &str, u32)`, all `-> Result<OSDOp>`;
  decoders `decode_list_buckets(&OpReply) -> Result<ListBucketsRet>`,
  `decode_get_header(&OpReply) -> Result<Header>`,
  `decode_reset_stats2(&OpReply) -> Result<ResetStats2Ret>`,
  `decode_account_resource_get(&OpReply) -> Result<AccountResource>`,
  `decode_account_resource_list(&OpReply) -> Result<AccountResourceListRet>`;
  async functions over `(&IoCtx, &str, ...)` with the same names minus
  `_op`, returning `Result<()>` or the decoded reply. Task 4 uses them.

- [ ] **Step 1: Write the failing unit tests**

Append to `user.rs`'s test module:

```rust
    #[test]
    fn ops_name_the_class_and_method() {
        use rados::osdclient::types::OpData;

        let op = list_buckets_op("m", "", 10).expect("op");
        assert!(op.indata.starts_with(b"userlist_buckets"));
        assert!(matches!(
            op.op_data,
            OpData::Call {
                class_len: 4,
                method_len: 12,
                ..
            }
        ));
        let op = get_header_op().expect("op");
        assert!(op.indata.starts_with(b"userget_header"));
        assert!(matches!(op.op_data, OpData::Call { indata_len: 6, .. }));
        let op = account_resource_list_op("", "/p", 5).expect("op");
        assert!(op.indata.starts_with(b"useraccount_resource_list"));
    }

    #[test]
    fn decoders_unwrap_the_replies() {
        let ret = ListBucketsRet {
            entries: vec![test_entry(2)],
            marker: s("buck.2"),
            truncated: true,
        };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&ret, 0).expect("encode"),
        };
        assert_eq!(decode_list_buckets(&reply).expect("decode"), ret);

        let header = Header {
            stats: Stats {
                total_entries: 1,
                total_bytes: 2,
                total_bytes_rounded: 3,
            },
            ..Header::default()
        };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(
                &GetHeaderRet {
                    header: header.clone(),
                },
                0,
            )
            .expect("encode"),
        };
        assert_eq!(decode_get_header(&reply).expect("decode"), header);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline user::tests::ops`
Expected: FAIL to compile: `list_buckets_op` not found.

- [ ] **Step 3: Implement the client**

Append after the types (before the test module), adding
`use rados::osdclient::error::Result;`, `use rados::osdclient::{IoCtx,
OSDOp, OpReply};` and `use crate::call;` to the imports:

```rust
/// `cls_user_set_buckets`: record `entries` in the user's index. With
/// `add`, absent buckets are inserted and existing ones keep their stats
/// but refresh `bucket_id` and `creation_time`; without it, absent
/// buckets are skipped and existing ones take the entries' stats. `time`
/// becomes the header's `last_stats_update` if later.
pub fn set_buckets_op(entries: &[BucketEntry], add: bool, time: UTime) -> Result<OSDOp> {
    call::op(
        CLASS,
        "set_buckets_info",
        &SetBucketsOp {
            entries: entries.to_vec(),
            add,
            time,
        },
    )
}

/// `cls_user_complete_stats_sync`: raise the header's `last_stats_sync`
/// to `time`.
pub fn complete_stats_sync_op(time: UTime) -> Result<OSDOp> {
    call::op(CLASS, "complete_stats_sync", &CompleteStatsSyncOp { time })
}

/// `cls_user_remove_bucket`: drop `bucket` from the index and its stats
/// from the header; a bucket that is not there is a success.
pub fn remove_bucket_op(bucket: &Bucket) -> Result<OSDOp> {
    call::op(
        CLASS,
        "remove_bucket",
        &RemoveBucketOp {
            bucket: bucket.clone(),
        },
    )
}

/// `cls_user_bucket_list`: up to `max_entries` (the class caps it at
/// 1000) entries after `marker`, stopping before the first name at or
/// past `end_marker` when it is not empty. Decode with
/// [`decode_list_buckets`].
pub fn list_buckets_op(marker: &str, end_marker: &str, max_entries: i32) -> Result<OSDOp> {
    call::op(
        CLASS,
        "list_buckets",
        &ListBucketsOp {
            marker: marker.to_owned(),
            max_entries,
            end_marker: end_marker.to_owned(),
        },
    )
}

/// Decode the reply to [`list_buckets_op`].
pub fn decode_list_buckets(reply: &OpReply) -> Result<ListBucketsRet> {
    call::decode(reply)
}

/// `cls_user_get_header`: the op; decode with [`decode_get_header`].
pub fn get_header_op() -> Result<OSDOp> {
    call::op(CLASS, "get_header", &GetHeaderOp {})
}

/// Decode the reply to [`get_header_op`].
pub fn decode_get_header(reply: &OpReply) -> Result<Header> {
    Ok(call::decode::<GetHeaderRet>(reply)?.header)
}

/// `cls_user_reset_stats`: recompute the header's stats from every entry
/// in one call and stamp `time`.
pub fn reset_stats_op(time: UTime) -> Result<OSDOp> {
    call::op(CLASS, "reset_user_stats", &ResetStatsOp { time })
}

/// `reset_user_stats2`: one page of the recompute; loop from `op.marker`
/// while the reply is truncated. Decode with [`decode_reset_stats2`].
pub fn reset_stats2_op(op: &ResetStats2Op) -> Result<OSDOp> {
    call::op(CLASS, "reset_user_stats2", op)
}

/// Decode the reply to [`reset_stats2_op`].
pub fn decode_reset_stats2(reply: &OpReply) -> Result<ResetStats2Ret> {
    call::decode(reply)
}

/// `cls_user_account_resource_add`: index `entry` by its lower-cased
/// name. A new entry past `limit` is `EUSERS`; an existing one with
/// `exclusive` is `EEXIST`, otherwise it is overwritten.
pub fn account_resource_add_op(entry: &AccountResource, exclusive: bool, limit: u32) -> Result<OSDOp> {
    call::op(
        CLASS,
        "account_resource_add",
        &AccountResourceAddOp {
            entry: entry.clone(),
            exclusive,
            limit,
        },
    )
}

/// `cls_user_account_resource_get`: the op; `ENOENT` when absent. Decode
/// with [`decode_account_resource_get`].
pub fn account_resource_get_op(name: &str) -> Result<OSDOp> {
    call::op(
        CLASS,
        "account_resource_get",
        &AccountResourceGetOp {
            name: name.to_owned(),
        },
    )
}

/// Decode the reply to [`account_resource_get_op`].
pub fn decode_account_resource_get(reply: &OpReply) -> Result<AccountResource> {
    Ok(call::decode::<AccountResourceGetRet>(reply)?.entry)
}

/// `cls_user_account_resource_rm`: `ENOENT` when absent.
pub fn account_resource_rm_op(name: &str) -> Result<OSDOp> {
    call::op(
        CLASS,
        "account_resource_rm",
        &AccountResourceRmOp {
            name: name.to_owned(),
        },
    )
}

/// `cls_user_account_resource_list`: up to `max_entries` (capped at 1000)
/// resources after `marker` whose `path` starts with `path_prefix`; the
/// reply's `marker` and `truncated` describe the omap page. Decode with
/// [`decode_account_resource_list`].
pub fn account_resource_list_op(marker: &str, path_prefix: &str, max_entries: u32) -> Result<OSDOp> {
    call::op(
        CLASS,
        "account_resource_list",
        &AccountResourceListOp {
            marker: marker.to_owned(),
            path_prefix: path_prefix.to_owned(),
            max_entries,
        },
    )
}

/// Decode the reply to [`account_resource_list_op`].
pub fn decode_account_resource_list(reply: &OpReply) -> Result<AccountResourceListRet> {
    call::decode(reply)
}

/// See [`set_buckets_op`].
pub async fn set_buckets(
    ioctx: &IoCtx,
    oid: &str,
    entries: &[BucketEntry],
    add: bool,
    time: UTime,
) -> Result<()> {
    let op = SetBucketsOp {
        entries: entries.to_vec(),
        add,
        time,
    };
    call::exec(ioctx, oid, CLASS, "set_buckets_info", &op)
        .await
        .map(drop)
}

/// See [`complete_stats_sync_op`].
pub async fn complete_stats_sync(ioctx: &IoCtx, oid: &str, time: UTime) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "complete_stats_sync", &CompleteStatsSyncOp { time })
        .await
        .map(drop)
}

/// See [`remove_bucket_op`].
pub async fn remove_bucket(ioctx: &IoCtx, oid: &str, bucket: &Bucket) -> Result<()> {
    let op = RemoveBucketOp {
        bucket: bucket.clone(),
    };
    call::exec(ioctx, oid, CLASS, "remove_bucket", &op)
        .await
        .map(drop)
}

/// See [`list_buckets_op`].
pub async fn list_buckets(
    ioctx: &IoCtx,
    oid: &str,
    marker: &str,
    end_marker: &str,
    max_entries: i32,
) -> Result<ListBucketsRet> {
    let op = ListBucketsOp {
        marker: marker.to_owned(),
        max_entries,
        end_marker: end_marker.to_owned(),
    };
    let out = call::exec(ioctx, oid, CLASS, "list_buckets", &op).await?;
    call::decode_bytes(out)
}

/// See [`get_header_op`].
pub async fn get_header(ioctx: &IoCtx, oid: &str) -> Result<Header> {
    let out = call::exec(ioctx, oid, CLASS, "get_header", &GetHeaderOp {}).await?;
    Ok(call::decode_bytes::<GetHeaderRet>(out)?.header)
}

/// See [`reset_stats_op`].
pub async fn reset_stats(ioctx: &IoCtx, oid: &str, time: UTime) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "reset_user_stats", &ResetStatsOp { time })
        .await
        .map(drop)
}

/// See [`reset_stats2_op`].
pub async fn reset_stats2(ioctx: &IoCtx, oid: &str, op: &ResetStats2Op) -> Result<ResetStats2Ret> {
    let out = call::exec(ioctx, oid, CLASS, "reset_user_stats2", op).await?;
    call::decode_bytes(out)
}

/// See [`account_resource_add_op`].
pub async fn account_resource_add(
    ioctx: &IoCtx,
    oid: &str,
    entry: &AccountResource,
    exclusive: bool,
    limit: u32,
) -> Result<()> {
    let op = AccountResourceAddOp {
        entry: entry.clone(),
        exclusive,
        limit,
    };
    call::exec(ioctx, oid, CLASS, "account_resource_add", &op)
        .await
        .map(drop)
}

/// See [`account_resource_get_op`].
pub async fn account_resource_get(ioctx: &IoCtx, oid: &str, name: &str) -> Result<AccountResource> {
    let op = AccountResourceGetOp {
        name: name.to_owned(),
    };
    let out = call::exec(ioctx, oid, CLASS, "account_resource_get", &op).await?;
    Ok(call::decode_bytes::<AccountResourceGetRet>(out)?.entry)
}

/// See [`account_resource_rm_op`].
pub async fn account_resource_rm(ioctx: &IoCtx, oid: &str, name: &str) -> Result<()> {
    let op = AccountResourceRmOp {
        name: name.to_owned(),
    };
    call::exec(ioctx, oid, CLASS, "account_resource_rm", &op)
        .await
        .map(drop)
}

/// See [`account_resource_list_op`].
pub async fn account_resource_list(
    ioctx: &IoCtx,
    oid: &str,
    marker: &str,
    path_prefix: &str,
    max_entries: u32,
) -> Result<AccountResourceListRet> {
    let op = AccountResourceListOp {
        marker: marker.to_owned(),
        path_prefix: path_prefix.to_owned(),
        max_entries,
    };
    let out = call::exec(ioctx, oid, CLASS, "account_resource_list", &op).await?;
    call::decode_bytes(out)
}
```

(`call::decode` and `call::decode_bytes` are generic over the reply type,
so `decode_list_buckets`, `decode_reset_stats2` and
`decode_account_resource_list` name it through their return type.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --lib --offline`
Expected: all pass, two new tests, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/user.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the user class client

Mirrors cls_user_client.h: set_buckets_info, complete_stats_sync,
remove_bucket, list_buckets, get_header, reset_user_stats,
reset_user_stats2 and the four account_resource methods, each as an op
constructor for compound use and an IoCtx function.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 4: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_user.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both `rados-cls` steps
  become loops over `cls_version_refcount cls_user`).

**Interfaces:**
- Consumes: Task 3's functions; `rados/tests/common/mod.rs` via `#[path]`;
  `IoCtx::{create, stat}`.
- Produces: four `#[ignore]` tests.

- [ ] **Step 1: Write the tests**

Create `rados-cls/tests/cls_user.rs`:

```rust
//! Cluster tests for the user class. Run with:
//!   CEPH_CONF=... cargo test -p rados-cls --test cls_user -- --ignored --nocapture

#[path = "../../rados/tests/common/mod.rs"]
mod common;

use bytes::Bytes;
use common::create_ioctx;
use rados::{OSDClientError, UTime};
use rados_cls::user::{self, AccountResource, Bucket, BucketEntry, ResetStats2Op, Stats};

const ENOENT: i32 = 2;
const EEXIST: i32 = 17;
const EUSERS: i32 = 87;

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

fn at(sec: u32) -> UTime {
    UTime { sec, nsec: 0 }
}

fn bucket(name: &str) -> Bucket {
    Bucket {
        name: name.to_owned(),
        marker: format!("{name}-marker"),
        bucket_id: format!("{name}-id"),
        ..Bucket::default()
    }
}

fn entry(name: &str, size: u64, count: u64) -> BucketEntry {
    BucketEntry {
        bucket: bucket(name),
        size,
        size_rounded: size + 2,
        creation_time: at(1_700_000_000),
        count,
        user_stats_sync: false,
    }
}

fn names(ret: &user::ListBucketsRet) -> Vec<String> {
    ret.entries.iter().map(|e| e.bucket.name.clone()).collect()
}

fn resource(name: &str, path: &str) -> AccountResource {
    AccountResource {
        name: name.to_owned(),
        path: path.to_owned(),
        metadata: Bytes::from_static(b"opaque"),
    }
}

#[tokio::test]
#[ignore]
async fn user_set_list_remove_buckets() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-user-buckets");
    ioctx.create(&oid, true).await.expect("create");

    let entries = [entry("b1", 10, 1), entry("b2", 20, 2)];
    user::set_buckets(&ioctx, &oid, &entries, true, at(400_000_000))
        .await
        .expect("set_buckets add");
    let listed = user::list_buckets(&ioctx, &oid, "", "", 1000)
        .await
        .expect("list");
    assert_eq!(names(&listed), vec!["b1".to_owned(), "b2".to_owned()]);
    assert!(!listed.truncated);
    assert!(listed.entries.iter().all(|e| e.user_stats_sync));
    assert_eq!(listed.entries[0].size, 10);
    assert_eq!(listed.entries[1].bucket.bucket_id, "b2-id");

    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(
        header.stats,
        Stats {
            total_entries: 3,
            total_bytes: 30,
            total_bytes_rounded: 34,
        }
    );
    assert_eq!(header.last_stats_update, at(400_000_000));

    // Without `add`, an absent bucket is skipped and stats are overwritten.
    user::set_buckets(&ioctx, &oid, &[entry("b1", 100, 5), entry("b9", 1, 1)], false, at(400_000_001))
        .await
        .expect("set_buckets overwrite");
    let listed = user::list_buckets(&ioctx, &oid, "", "", 1000)
        .await
        .expect("list");
    assert_eq!(names(&listed), vec!["b1".to_owned(), "b2".to_owned()]);
    assert_eq!(listed.entries[0].size, 100);
    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(header.stats.total_entries, 7);
    assert_eq!(header.stats.total_bytes, 120);

    user::remove_bucket(&ioctx, &oid, &bucket("b1"))
        .await
        .expect("remove b1");
    user::remove_bucket(&ioctx, &oid, &bucket("b1"))
        .await
        .expect("removing an absent bucket succeeds");
    let listed = user::list_buckets(&ioctx, &oid, "", "", 1000)
        .await
        .expect("list");
    assert_eq!(names(&listed), vec!["b2".to_owned()]);
    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(
        header.stats,
        Stats {
            total_entries: 2,
            total_bytes: 20,
            total_bytes_rounded: 22,
        }
    );

    user::complete_stats_sync(&ioctx, &oid, at(400_000_050))
        .await
        .expect("complete_stats_sync");
    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(header.last_stats_sync, at(400_000_050));

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn user_list_buckets_pages() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-user-pages");
    ioctx.create(&oid, true).await.expect("create");
    let entries = [entry("a", 1, 1), entry("b", 1, 1), entry("c", 1, 1)];
    user::set_buckets(&ioctx, &oid, &entries, true, at(400_000_000))
        .await
        .expect("set_buckets");

    let first = user::list_buckets(&ioctx, &oid, "", "", 2)
        .await
        .expect("page 1");
    assert_eq!(names(&first), vec!["a".to_owned(), "b".to_owned()]);
    assert!(first.truncated);
    assert_eq!(first.marker, "b");
    let second = user::list_buckets(&ioctx, &oid, &first.marker, "", 2)
        .await
        .expect("page 2");
    assert_eq!(names(&second), vec!["c".to_owned()]);
    assert!(!second.truncated);
    assert_eq!(second.marker, "", "marker is set only when truncated");

    let bounded = user::list_buckets(&ioctx, &oid, "", "b", 1000)
        .await
        .expect("end_marker");
    assert_eq!(names(&bounded), vec!["a".to_owned()], "stops before end_marker");
    assert!(!bounded.truncated);

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn user_reset_stats() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-user-reset");
    ioctx.create(&oid, true).await.expect("create");
    let entries = [entry("x", 5, 1), entry("y", 7, 3)];
    user::set_buckets(&ioctx, &oid, &entries, true, at(400_000_000))
        .await
        .expect("set_buckets");
    let want = Stats {
        total_entries: 4,
        total_bytes: 12,
        total_bytes_rounded: 16,
    };

    user::reset_stats(&ioctx, &oid, at(400_000_100))
        .await
        .expect("reset_stats");
    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(header.stats, want);
    assert_eq!(header.last_stats_update, at(400_000_100));

    let mut op = ResetStats2Op {
        time: at(400_000_200),
        ..ResetStats2Op::default()
    };
    let ret = loop {
        let ret = user::reset_stats2(&ioctx, &oid, &op).await.expect("reset_stats2");
        if !ret.truncated {
            break ret;
        }
        op.marker = ret.marker;
        op.acc_stats = ret.acc_stats;
    };
    assert_eq!(ret.acc_stats, want);
    let header = user::get_header(&ioctx, &oid).await.expect("header");
    assert_eq!(header.stats, want);
    assert_eq!(header.last_stats_update, at(400_000_200));

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn user_account_resources() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-user-account");
    ioctx.create(&oid, true).await.expect("create");

    user::account_resource_add(&ioctx, &oid, &resource("Alpha", "/a/x"), false, 2)
        .await
        .expect("add Alpha");
    user::account_resource_add(&ioctx, &oid, &resource("beta", "/b/y"), false, 2)
        .await
        .expect("add beta");
    let err = user::account_resource_add(&ioctx, &oid, &resource("gamma", "/a/z"), false, 2)
        .await
        .expect_err("past the limit");
    assert!(is_osd_error(&err, EUSERS), "{err:?}");
    let err = user::account_resource_add(&ioctx, &oid, &resource("Alpha", "/a/x"), true, 2)
        .await
        .expect_err("exclusive add of an existing resource");
    assert!(is_osd_error(&err, EEXIST), "{err:?}");
    user::account_resource_add(&ioctx, &oid, &resource("Alpha", "/a/x2"), false, 2)
        .await
        .expect("a non-exclusive add overwrites");

    let got = user::account_resource_get(&ioctx, &oid, "alpha")
        .await
        .expect("get by lower-cased name");
    assert_eq!(got.name, "Alpha");
    assert_eq!(got.path, "/a/x2");
    assert_eq!(got.metadata, Bytes::from_static(b"opaque"));
    let err = user::account_resource_get(&ioctx, &oid, "nope")
        .await
        .expect_err("missing");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");

    let listed = user::account_resource_list(&ioctx, &oid, "", "/a/", 100)
        .await
        .expect("list");
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].name, "Alpha");
    assert!(!listed.truncated);
    let all = user::account_resource_list(&ioctx, &oid, "", "", 100)
        .await
        .expect("list all");
    assert_eq!(all.entries.len(), 2);

    user::account_resource_rm(&ioctx, &oid, "ALPHA")
        .await
        .expect("rm by any case");
    let err = user::account_resource_rm(&ioctx, &oid, "alpha")
        .await
        .expect_err("already removed");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
    user::account_resource_add(&ioctx, &oid, &resource("gamma", "/a/z"), false, 2)
        .await
        .expect("the count went down, so gamma fits");

    ioctx.remove(&oid).await.expect("remove");
}
```

In `.github/workflows/test-with-ceph.yml`, change both `rados-cls` steps'
`run:` to:

```yaml
        run: |
          for test in cls_version_refcount cls_user; do
            cargo test -p rados-cls --test "$test" -- --ignored --nocapture
          done
```

- [ ] **Step 2: Build the test binary**

Run: `cargo test -p rados-cls --offline --test cls_user --no-run`
Expected: compiles with no warnings.

- [ ] **Step 3: Run against the cluster (controller, unsandboxed)**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_user -- --ignored --nocapture`
Expected: 4 passed. On a failure, report the observed code and the
`cls_user.cc` line that explains it; do not weaken the assertion.

- [ ] **Step 4: Commit**

```bash
git add rados-cls/tests/cls_user.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: user cluster tests

Pins against Ceph v19: set_buckets with and without add, header stats
following the entries, idempotent remove_bucket, list_buckets paging
with marker and end_marker, both stats resets, and the account resource
methods' lower-cased keys, EUSERS, EEXIST and ENOENT.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6, with: the per-commit proof from the branch base; the
corpus loop over the nineteen `cls_user_*` names (expect the eight
account-resource types to report only for the 19.2.0 archive); the
cluster suites `cls_version_refcount` and `cls_user`; the PR body:

```
**Motivation.** RGW keeps each user's bucket list, its running stats and, since Squid, an account's resources in the `user` object class; a Rust RGW needs every method of `cls_user_client.h`.

**What changed.** The `user` module of `rados-cls`: all nineteen corpus-covered types (`cls_user_bucket` with its value-dependent version, `cls_user_bucket_entry` with its legacy wire order), the three reset-stats types, op constructors and `IoCtx` functions for every method, timestamps dumped as `encode_json` prints them, unit, corpus and cluster tests.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Then merge when green and open the upstream PR from the same branch.

---

## Roadmap for later plans

`cls-queue-gc`; `cls-rgw-types`; `cls-rgw-bucket-index`; `cls-rgw-gc`;
`cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh`; `watch-notify`. Deferred
minors carried forward: `cls_user_reset_stats*` have no `ceph-dencoder`
registration upstream; the `reset_user_stats2` server never reads
`acc_stats` (documented on the type); plan 3's deferred minors.
