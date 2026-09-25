# rados-rs RGW MVP, plan 6 of N: `cls-rgw-types`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the value types of `cls_rgw_types.h` that the `rgw` object
class and RGW share: the bucket-index entry, its meta, version and
pending info, the category stats, the dir header and dir, the bi and
bilog entries, the zone set, the reshard and bucket-instance entries, the
OLH entry and log entry, the usage types and the lifecycle head and
entry, with wire bytes pinned against `ceph-dencoder` v19.2.2 and every
registered type compared against the corpus. No class methods: the
bucket-index, GC, usage, lifecycle and OLH packages add those to the
files this plan lays out.

**Architecture:** The `rgw` module (feature `rgw`, from plan 5) grows
four files that later packages extend in place: `types.rs` (enums,
version, pending info, category stats, zone set), `index.rs` (the
bucket-index structs, bilog, reshard and bucket-instance entries),
`olh.rs`, `usage.rs`, `lc.rs`, plus a crate-private `packed.rs` for
Ceph's packed integers. Enums are newtypes over `u8` with named
constants, because Ceph decodes any byte into them and newer releases
add values; dumps that print names go through an `as_str`. Every
encoder writes what Ceph v19.2.2 writes; every decoder also accepts the
version Tentacle writes, whose extra tail the crate's versioned decoder
skips.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-rgw-types`" (`cls_rgw_obj_key` and the GC chain types landed in
plan 5).

## Global Constraints

Plans 3 to 5's Global Constraints apply unchanged. Plus:

- Branch `cls-rgw-types` is based on the fork's `main` after plan 5
  merges; `rgw::types` holds `ObjKey`, `Obj`, `ObjChain`, `GcObjInfo`;
  `rgw::gc` the `cls_rgw_gc_*` structs; `dump` has `utime`/`gmtime`
  (gated `user`), `real_time`/`iso_utc` (gated `rgw`), `bool_as_int`
  (gated `any(refcount, rgw)`), `civil_from_days` (gated `any(user,
  rgw)`).
- The oracle is `ceph-dencoder` from `quay.io/ceph/ceph:v19.2.2`. Where
  Ceph `main` differs, v19.2.2 wins on encode and on the dump:
  `rgw_bucket_dir_entry_meta` writes version 7 (no restore fields) and
  dumps `storage_class` raw (`main` canonicalises `""` to `"STANDARD"`);
  `rgw_bucket_dir_header` writes version 7 (no `reshardlog_entries`);
  `cls_rgw_reshard_entry` writes version 2 (no `initiator`);
  `rgw_bucket_olh_entry`'s dump has no `epoch_timestamp`;
  `rgw_usage_log_entry` writes version 4 with the `s3select` section
  (v19.2.0 wrote 3; the corpus samples are 3). Verified by encoding every
  `generate_test_instances` instance with the oracle (first bytes
  `0703`, `0702`, `0201`, `0401`).
- `MAX_DECODE_VERSION` is the newest version any Ceph release writes
  (`main` at e234256339f): meta 8, header 8, reshard entry 3, usage
  entry 4. The extra fields are not modelled; `decode_content` reads
  the v19.2.2 fields and the versioned decoder skips the rest.
- Legacy one-byte headers (`DECODE_START_LEGACY_COMPAT_LEN` with
  `struct_v` below the compat threshold: meta below 3, dir entry below
  3, header below 2, pending info below 2, category stats below 2, dir
  below 2) are not decoded. The OSD class re-encodes every entry it
  returns at the current version, so a client never sees them.
- `ceph::real_time` fields are `rados::UTime`; every dump in this header
  prints them through `encode_json`'s `utime_t` overload
  (`crate::dump::utime`), except `rgw_bi_log_entry::timestamp`, which
  streams `utime_t::gmtime_nsec` (nine fraction digits in the calendar
  form, six in the raw-seconds form).
- Packed integers (`encode_packed_val`): a value below `0x80` is one
  byte; otherwise a tag `0x80 | n` then the value in `n` little-endian
  bytes with `n` = 1 below `0x100`, 2 up to and including `0x10000`, 4 up
  to and including `0x1000000`, else 8. The `<=` on the 2-byte branch
  means `0x10000` is written as two zero bytes and reads back as 0;
  mirror it (byte identity) and document it. A negative `int64_t` is
  cast to `u64` and takes the 8-byte form.
- Ceph's `std::multimap`/`std::map`/`flat_map` encode as `u32` count then
  pairs in key order; `pending_map` keeps duplicates, so it is a
  `Vec<(String, PendingInfo)>` encoded by hand.
- `rgw_user` appears only as its string form (`tenant$ns$id`, `$ns$id`,
  or `id`); `owner`/`payer` stay `String`s, which is lossless where a
  parsed triple would not be.
- Every dump quirk below was read from the oracle's output, not only
  from `dump()`.
- No class calls in this plan; the standing rule for later ones: a RD|WR
  method whose C++ client reads its reply needs `OsdOpFlags::RETURNVEC`
  (`call::exec_returnvec`) and a reply under `osd_max_write_op_reply_len`
  (64 bytes by default; more is `EOVERFLOW`).

## Review Focus

1. `rgw_bucket_dir_entry` writes `ver.epoch` twice (a legacy `u64` after
   the name, then inside `rgw_bucket_entry_ver` with `pool`) and splits
   the key (`name` second field, `instance` tenth); `index_ver` is packed
   and not dumped. Pinned in Task 3 against the oracle's 159-byte
   instance.
2. Packed integers, including the `0x10000` quirk and the 8-byte form of
   `pool = -1`. Pinned in Task 2.
3. The dumps that are not their wire shape: `rgw_bucket_dir_header.stats`
   and `rgw_bucket_dir.map` are arrays alternating a bare key and an
   object; `pending_map` and `pending_log` are arrays of `{key, val}`;
   `rgw_bi_log_entry` renames `id`/`tag` to `op_id`/`op_tag`, prints `op`
   and `state` by name, adds a derived `versioned`, and prints
   `zones_trace` as a bare array while `rgw_zone_set` alone prints
   `{entries: [...]}`; `cls_rgw_reshard_entry` renames `new_num_shards`;
   `rgw_usage_log_entry` flattens `total_usage` and lists categories with
   the name inside each object; `rgw_bucket_dir_header` omits
   `tag_timeout`, `max_marker`, `syncstopped`; `cls_rgw_lc_obj_head` omits
   `shard_rollover_date`. Pinned per type.
4. Version handling: encode v19.2.2's version, accept `main`'s
   (`MAX_DECODE_VERSION`), and read older corpus forms where the corpus
   has them (`rgw_usage_log_entry` v3). Pinned in Tasks 3 and 5 and by
   the corpus run.
5. `rgw_cls_bi_entry` dumps its payload decoded by `type` (`plain`/
   `instance` as a dir entry, `olh` as an OLH entry, nothing for
   `invalid`), so its `Serialize` decodes `data`; `get_info` mirrors the
   C++ (accounted stats from a dir entry). Pinned in Task 4.

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

- [ ] **Step 1: Branch, ledger**

```bash
cd $R && git fetch origin main && git checkout -b cls-rgw-types origin/main && git log --oneline -1
ls rados-cls/src/rgw/ && grep -n 'cfg' rados-cls/src/dump.rs | head
```
Expected: HEAD is the `cls-queue-gc` merge; `rgw/{mod,types,gc}.rs`
exist. Then the workspace and ledger as in plan 3's Task 0. No cluster is
needed for this plan.

---

### Task 1: Test helper and dump additions land with their first users

There is no separate commit: the hex helper below goes into each test
module that needs it, and the `dump` additions (`utime_nsec`,
`map_entries`) land in Task 3 with the bilog entry and dir entry. Task 2
widens `dump::utime`/`gmtime`/`civil_from_days` to `any(feature = "user",
feature = "rgw")` because `PendingInfo` dumps through `utime`.

Test helper (copy into each test module that pins oracle bytes):

```rust
    /// Bytes from a hex string, as `ceph-dencoder ... encode export` wrote them.
    fn unhex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex"))
            .collect()
    }
```

---

### Task 2: `rgw::types`: enums, packed integers, version, pending info, stats, zone set

**Files:**
- Create: `rados-cls/src/rgw/packed.rs`.
- Modify: `rados-cls/src/rgw/mod.rs` (`mod packed;`),
  `rados-cls/src/rgw/types.rs`, `rados-cls/src/dump.rs` (gate widening),
  `rados-dencoder/src/main.rs` (four arms), the corpus test (four
  `TypeSpec`s: `rgw_bucket_entry_ver`, `rgw_bucket_pending_info`,
  `rgw_bucket_category_stats`, `rgw_zone_set`).

**Interfaces:**
- Produces in `packed.rs`: `pub(crate) fn encode<B: BufMut>(v: u64, buf:
  &mut B)`, `pub(crate) fn decode<B: Buf>(buf: &mut B) ->
  Result<u64, RadosError>` (malformed tag is
  `RadosError::InvalidData`), `pub(crate) fn encoded_size(v: u64) ->
  usize`.
- Produces in `types.rs`: `ObjCategory(u8)` with `NONE`, `MAIN`,
  `SHADOW`, `MULTI_META`, `CLOUD_TIERED`; `ModifyOp(u8)` with `ADD`,
  `DEL`, `CANCEL`, `UNKNOWN`, `LINK_OLH`, `LINK_OLH_DM`,
  `UNLINK_INSTANCE`, `SYNCSTOP`, `RESYNC` and `as_str()` (`write`, `del`,
  `cancel`, `link_olh`, `link_olh_del`, `unlink_instance`, `syncstop`,
  `resync`, else `unknown`); `PendingState(u8)` with `PENDING_MODIFY`,
  `COMPLETE`, `UNKNOWN`; `BiIndexType(u8)` with `INVALID`, `PLAIN`,
  `INSTANCE`, `OLH` and `as_str()` (`plain`, `instance`, `olh`, else
  `invalid`); `ReshardStatus(u8)` with `NOT_RESHARDING`, `IN_PROGRESS`,
  `DONE` and `as_str()` (`not-resharding`, `in-progress`, `done`, else
  `Unknown reshard status`); each newtype `Denc` as one byte, `Serialize`
  as a number (`#[serde(transparent)]`), `Ord`; the dir-entry flag
  constants `FLAG_VER = 0x1`, `FLAG_CURRENT = 0x2`, `FLAG_DELETE_MARKER =
  0x4`, `FLAG_VER_MARKER = 0x8`, `FLAG_COMMON_PREFIX = 0x8000`; `pub fn
  rounded_size(size: u64) -> u64` (`cls_rgw_get_rounded_size`, 4 KiB
  blocks); structs `EntryVer`, `PendingInfo`, `CategoryStats`,
  `ZoneSetEntry`, `ZoneSet`.

- [ ] **Step 1: Write the failing tests**

`packed.rs` test module:

```rust
    #[test]
    fn packed_forms_and_the_u16_quirk() {
        let cases: &[(u64, &[u8])] = &[
            (0, &[0x00]),
            (0x7f, &[0x7f]),
            (0x80, &[0x81, 0x80]),
            (0xff, &[0x81, 0xff]),
            (0x100, &[0x82, 0x00, 0x01]),
            (12_322, &[0x82, 0x22, 0x30]),
            (0xffff, &[0x82, 0xff, 0xff]),
            // encode_packed_val's `<= 0x10000` writes the value as a u16: zero.
            (0x10000, &[0x82, 0x00, 0x00]),
            (0x10001, &[0x84, 0x01, 0x00, 0x01, 0x00]),
            (0x1000000, &[0x84, 0x00, 0x00, 0x00, 0x01]),
            (0x1000001, &[0x88, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0]),
            (u64::MAX, &[0x88, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
        ];
        for (value, wire) in cases {
            let mut buf = Vec::new();
            encode(*value, &mut buf);
            assert_eq!(&buf, wire, "{value:#x}");
            assert_eq!(encoded_size(*value), wire.len());
            let expect = if *value == 0x10000 { 0 } else { *value };
            assert_eq!(decode(&mut &wire[..]).expect("decode"), expect, "{value:#x}");
        }
        assert!(decode(&mut &[0x80u8][..]).is_err());
        assert!(decode(&mut &[0x83u8, 0, 0, 0][..]).is_err());
        // A non-minimal form still decodes.
        assert_eq!(decode(&mut &[0x81u8, 0x05][..]).expect("decode"), 5);
    }
```

`types.rs` test additions:

```rust
    #[test]
    fn entry_ver_packs_both_fields() {
        // Oracle rgw_bucket_entry_ver test 0: pool 123, epoch 12322.
        let v = EntryVer { pool: 123, epoch: 12_322 };
        assert_eq!(bytes(&v), unhex("0101040000007b822230"));
        assert_eq!(json(&v), r#"{"pool":123,"epoch":12322}"#);
        // Default: pool -1 takes the 8-byte form.
        assert_eq!(bytes(&EntryVer::default()), unhex("01010a00000088ffffffffffffffff00"));
        assert_eq!(EntryVer::decode(&mut &bytes(&EntryVer::default())[..], 0).expect("decode"), EntryVer::default());
    }

    #[test]
    fn pending_info_is_state_time_op() {
        // Oracle rgw_bucket_pending_info test 1: complete, epoch, op del.
        let p = PendingInfo { state: PendingState::COMPLETE, timestamp: UTime::default(), op: 1 };
        assert_eq!(bytes(&p), unhex("02020a00000001000000000000000001"));
        assert_eq!(json(&p), r#"{"state":1,"timestamp":"0.000000","op":1}"#);
    }

    #[test]
    fn category_stats_and_its_v2_default() {
        let s = CategoryStats { total_size: 1024, total_size_rounded: 4096, num_entries: 2, actual_size: 1024 };
        assert_eq!(bytes(&s), unhex("0302200000000004000000000000001000000000000002000000000000000004000000000000"));
        assert_eq!(json(&s), r#"{"total_size":1024,"total_size_rounded":4096,"num_entries":2,"actual_size":1024}"#);
        // Version 2 had no actual_size; it reads as total_size.
        let v2 = unhex("0202180000000004000000000000001000000000000002000000000000");
        assert_eq!(CategoryStats::decode(&mut &v2[..], 0).expect("decode").actual_size, 1024);
    }

    #[test]
    fn zone_set_is_bare_strings_in_entry_order() {
        // Oracle rgw_zone_set test 0.
        let mut z = ZoneSet::default();
        for zone in ["zone1", "zone2", "zone3"] {
            z.insert(zone, Some("loc_key"));
        }
        assert_eq!(bytes(&z), unhex("030000000d0000007a6f6e65313a6c6f635f6b65790d0000007a6f6e65323a6c6f635f6b65790d0000007a6f6e65333a6c6f635f6b6579"));
        assert_eq!(json(&z), r#"{"entries":[{"entry":"zone1:loc_key"},{"entry":"zone2:loc_key"},{"entry":"zone3:loc_key"}]}"#);
        assert_eq!(ZoneSet::decode(&mut &bytes(&z)[..], 0).expect("decode"), z);
        // A key-less zone sorts before the same zone with a key; the split is at the first colon.
        let e = ZoneSetEntry::from_str("z:a:b");
        assert_eq!((e.zone.as_str(), e.location_key.as_deref()), ("z", Some("a:b")));
        assert!(ZoneSetEntry::from_str("z") < e);
        assert!(z.exists("zone2", Some("loc_key")) && !z.exists("zone2", None));
    }

    #[test]
    fn enums_keep_unknown_bytes_and_name_the_known_ones() {
        assert_eq!(ModifyOp(9).as_str(), "unknown");
        assert_eq!(ModifyOp::LINK_OLH_DM.as_str(), "link_olh_del");
        assert_eq!(BiIndexType(7).as_str(), "invalid");
        assert_eq!(ReshardStatus::IN_PROGRESS.as_str(), "in-progress");
        assert_eq!(json(&ObjCategory::MAIN), "1");
        assert_eq!(bytes(&ObjCategory::CLOUD_TIERED), [4]);
        assert_eq!(rounded_size(1), 4096);
        assert_eq!(rounded_size(4096), 4096);
        assert_eq!(rounded_size(0), 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline rgw::`
Expected: compile errors.

- [ ] **Step 3: Implement**

`packed.rs`:

```rust
//! Ceph's `encode_packed_val`/`decode_packed_val` (`cls_rgw_types.h`): a
//! value below 0x80 is one byte; otherwise a tag `0x80 | n` and the value
//! in `n` little-endian bytes. The C++ picks two bytes up to and
//! including 0x10000, so that one value is written as zero; this mirrors
//! it for byte identity.

use bytes::{Buf, BufMut};
use rados::RadosError;

pub(crate) fn encode<B: BufMut>(v: u64, buf: &mut B) {
    if v < 0x80 {
        buf.put_u8(v as u8);
    } else if v < 0x100 {
        buf.put_u8(0x81);
        buf.put_u8(v as u8);
    } else if v <= 0x10000 {
        buf.put_u8(0x82);
        buf.put_u16_le(v as u16);
    } else if v <= 0x100_0000 {
        buf.put_u8(0x84);
        buf.put_u32_le(v as u32);
    } else {
        buf.put_u8(0x88);
        buf.put_u64_le(v);
    }
}

pub(crate) fn encoded_size(v: u64) -> usize {
    if v < 0x80 {
        1
    } else if v < 0x100 {
        2
    } else if v <= 0x10000 {
        3
    } else if v <= 0x100_0000 {
        5
    } else {
        9
    }
}

pub(crate) fn decode<B: Buf>(buf: &mut B) -> Result<u64, RadosError> {
    fn need<B: Buf>(buf: &B, n: usize) -> Result<(), RadosError> {
        if buf.remaining() < n {
            return Err(RadosError::InvalidData("packed value truncated".into()));
        }
        Ok(())
    }
    need(buf, 1)?;
    let tag = buf.get_u8();
    if tag < 0x80 {
        return Ok(u64::from(tag));
    }
    match tag & !0x80 {
        1 => { need(buf, 1)?; Ok(u64::from(buf.get_u8())) }
        2 => { need(buf, 2)?; Ok(u64::from(buf.get_u16_le())) }
        4 => { need(buf, 4)?; Ok(u64::from(buf.get_u32_le())) }
        8 => { need(buf, 8)?; Ok(buf.get_u64_le()) }
        n => Err(RadosError::InvalidData(format!("packed value tag {n:#x}"))),
    }
}
```
(Format the `match` arms on separate lines as rustfmt will.)

`types.rs` additions (after `GcObjInfo`):

```rust
/// A one-byte Ceph enum. Ceph decodes any byte and later releases add
/// values, so the newtype keeps the byte; the constants name the values
/// v19 knows.
macro_rules! byte_enum {
    ($(#[$doc:meta])* $name:ident { $($k:ident = $v:expr),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub u8);

        impl $name {
            $(pub const $k: Self = Self($v);)*
        }

        impl Denc for $name {
            fn encode<B: BufMut>(&self, buf: &mut B, features: u64) -> std::result::Result<(), RadosError> {
                self.0.encode(buf, features)
            }

            fn decode<B: Buf>(buf: &mut B, features: u64) -> std::result::Result<Self, RadosError> {
                Ok(Self(u8::decode(buf, features)?))
            }

            fn encoded_size(&self, _features: u64) -> Option<usize> {
                Some(1)
            }
        }
    };
}

byte_enum! {
    /// `RGWObjCategory`: what a bucket-index entry accounts for. Dumps as a number.
    ObjCategory { NONE = 0, MAIN = 1, SHADOW = 2, MULTI_META = 3, CLOUD_TIERED = 4 }
}

byte_enum! {
    /// `RGWModifyOp`: the operation a bilog entry or pending change records.
    ModifyOp {
        ADD = 0, DEL = 1, CANCEL = 2, UNKNOWN = 3, LINK_OLH = 4, LINK_OLH_DM = 5,
        UNLINK_INSTANCE = 6, SYNCSTOP = 7, RESYNC = 8,
    }
}

impl ModifyOp {
    /// `to_string(RGWModifyOp)`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ADD => "write",
            Self::DEL => "del",
            Self::CANCEL => "cancel",
            Self::LINK_OLH => "link_olh",
            Self::LINK_OLH_DM => "link_olh_del",
            Self::UNLINK_INSTANCE => "unlink_instance",
            Self::SYNCSTOP => "syncstop",
            Self::RESYNC => "resync",
            _ => "unknown",
        }
    }
}

byte_enum! {
    /// `RGWPendingState`.
    PendingState { PENDING_MODIFY = 0, COMPLETE = 1, UNKNOWN = 2 }
}

byte_enum! {
    /// `BIIndexType`: which struct a bucket-index entry's payload holds.
    BiIndexType { INVALID = 0, PLAIN = 1, INSTANCE = 2, OLH = 3 }
}

impl BiIndexType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PLAIN => "plain",
            Self::INSTANCE => "instance",
            Self::OLH => "olh",
            _ => "invalid",
        }
    }
}

byte_enum! {
    /// `cls_rgw_reshard_status`.
    ReshardStatus { NOT_RESHARDING = 0, IN_PROGRESS = 1, DONE = 2 }
}

impl ReshardStatus {
    /// `to_string(cls_rgw_reshard_status)`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NOT_RESHARDING => "not-resharding",
            Self::IN_PROGRESS => "in-progress",
            Self::DONE => "done",
            _ => "Unknown reshard status",
        }
    }
}

/// `rgw_bucket_dir_entry` flag bits.
pub const FLAG_VER: u16 = 0x1;
pub const FLAG_CURRENT: u16 = 0x2;
pub const FLAG_DELETE_MARKER: u16 = 0x4;
pub const FLAG_VER_MARKER: u16 = 0x8;
pub const FLAG_COMMON_PREFIX: u16 = 0x8000;

/// `cls_rgw_get_rounded_size`: up to the next 4 KiB block.
pub fn rounded_size(size: u64) -> u64 {
    (size + 4095) & !4095
}

/// `rgw_bucket_entry_ver`: both fields packed; `pool` is `-1` when unset
/// and, cast to unsigned, takes the eight-byte form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntryVer {
    pub pool: i64,
    pub epoch: u64,
}

impl Default for EntryVer {
    fn default() -> Self {
        Self { pool: -1, epoch: 0 }
    }
}

impl VersionedEncode for EntryVer {
    const MAX_DECODE_VERSION: u8 = 1;
    fn encoding_version(&self, _features: u64) -> u8 { 1 }
    fn compat_version(&self, _features: u64) -> u8 { 1 }
    fn encode_content<B: BufMut>(&self, buf: &mut B, _features: u64, _version: u8) -> std::result::Result<(), RadosError> {
        packed::encode(self.pool as u64, buf);
        packed::encode(self.epoch, buf);
        Ok(())
    }
    fn decode_content<B: Buf>(buf: &mut B, _features: u64, _struct_v: u8, _compat_version: u8) -> std::result::Result<Self, RadosError> {
        let pool = packed::decode(buf)? as i64;
        let epoch = packed::decode(buf)?;
        Ok(Self { pool, epoch })
    }
    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        Some(packed::encoded_size(self.pool as u64) + packed::encoded_size(self.epoch))
    }
}

rados::impl_denc_for_versioned!(EntryVer);

/// `rgw_bucket_pending_info`: a change the class has prepared but not
/// completed. Version 2 (compat 2); `op` holds an `RGWModifyOp` value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 2, compat = 2)]
pub struct PendingInfo {
    pub state: PendingState,
    #[serde(serialize_with = "crate::dump::utime")]
    pub timestamp: UTime,
    pub op: u8,
}

/// `rgw_bucket_category_stats`. Version 3 (compat 2) added `actual_size`;
/// a version-2 value reads it as `total_size`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CategoryStats {
    pub total_size: u64,
    pub total_size_rounded: u64,
    pub num_entries: u64,
    pub actual_size: u64,
}
// VersionedEncode: MAX_DECODE_VERSION 3, version 3, compat 2; encode the four u64s;
// decode three, then actual_size if struct_v >= 3 else total_size; size Some(32).

/// `rgw_zone_set_entry`: `zone[:location_key]` on the wire as one string,
/// split at the first colon; ordered by zone then key with no key first.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ZoneSetEntry {
    pub zone: String,
    pub location_key: Option<String>,
}

impl ZoneSetEntry {
    pub fn from_str(s: &str) -> Self { /* split_once(':') */ }
    pub fn to_str(&self) -> String { /* zone or zone:key */ }
}
// Denc: encode to_str() as a String; decode a String and from_str; size = 4 + len. No header.
// Serialize: {"entry": to_str()}.

/// `rgw_zone_set`: the zones a change has passed through. No header; a
/// `u32` count then each entry's string, in entry order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZoneSet {
    pub entries: BTreeSet<ZoneSetEntry>,
}

impl ZoneSet {
    pub fn insert(&mut self, zone: &str, location_key: Option<&str>) { ... }
    pub fn exists(&self, zone: &str, location_key: Option<&str>) -> bool { ... }
}
// Denc: delegate to BTreeSet<ZoneSetEntry>. Serialize: {"entries": [ {entry}, ... ]}.
// The bare-array form (a bilog entry's `zones_trace`) is `ZoneSet::serialize_bare`,
// a `pub(crate) fn serialize_bare<S>(&self, s: S)` usable with serialize_with.
```
(`Ord` on `Option<String>` puts `None` first, as `std::optional` does.)
Derive-able pieces are left as comments where the pattern is plan 5's;
write them out. Widen the `dump` gates as Task 1 says. Register the four
types in the dencoder and the corpus table (names `rgw_bucket_entry_ver`,
`rgw_bucket_pending_info`, `rgw_bucket_category_stats`, `rgw_zone_set`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados-cls --lib --offline rgw:: && cargo check --workspace --all-targets --offline && cargo check -p rados-cls --no-default-features --features rgw --offline`
Expected: green, no warnings in any.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/rgw rados-cls/src/dump.rs rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: rgw enums, packed integers, entry version, pending info, stats, zone set

The one-byte enums of cls_rgw_types.h keep their byte, since Ceph
decodes any value and later releases add some. encode_packed_val's
two-byte branch takes 0x10000 inclusive and so writes that value as
zero; the port mirrors it. rgw_zone_set has no struct header and orders
its entries by zone then optional key, which is not string order.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 3: `rgw::index`: the bucket-index structs

**Files:**
- Create: `rados-cls/src/rgw/index.rs`.
- Modify: `rados-cls/src/rgw/mod.rs` (`pub mod index;`),
  `rados-cls/src/dump.rs` (`utime_nsec`, `gmtime_nsec`, `map_entries`,
  all gated `rgw`), dencoder (eight arms), corpus table (eight:
  `rgw_bucket_dir_entry_meta`, `rgw_bucket_dir_entry`,
  `rgw_bucket_dir_header`, `rgw_bucket_dir`, `rgw_bi_log_entry`,
  `cls_rgw_bucket_instance_entry`, `cls_rgw_reshard_entry`,
  `rgw_cls_bi_entry` — the last after Task 4 adds the OLH entry it dumps;
  put its arm and `TypeSpec` in Task 4's commit).

**Interfaces:**
- Produces: `DirEntryMeta`, `DirEntry`, `DirHeader`, `Dir`, `BiLogEntry`,
  `BucketInstanceEntry`, `ReshardEntry` (and `BiEntry` in Task 4);
  `dump::utime_nsec` (bilog timestamps), `dump::map_entries` (a
  `Vec<(K, V)>` or `BTreeMap<K, V>` as `[{"key", "val"}]`).

Wire and dump facts, all verified against the oracle:

- `DirEntryMeta` (`rgw_bucket_dir_entry_meta`): encode version 7, compat
  3, `MAX_DECODE_VERSION` 8. Wire: `category` (ObjCategory), `size` u64,
  `mtime` UTime, `etag`, `owner`, `owner_display_name`, `content_type`
  (v2+), `accounted_size` u64 (v4+, else `= size`), `user_data` (v5+),
  `storage_class` (v6+), `appendable` bool (v7+); v8 adds two restore
  fields, skipped. Dump order: `category` (number), `size`, `mtime`
  (utime), `etag`, `storage_class` (raw), `owner`, `owner_display_name`,
  `content_type`, `accounted_size`, `user_data`, `appendable` (bool).
  Oracle instance 1 (89 bytes):
  `07035300000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f747970650000000000000000000000000000000000`
  = `{"category":1,"size":100,"mtime":"0.000000","etag":"etag","storage_class":"","owner":"owner","owner_display_name":"display name","content_type":"content/type","accounted_size":0,"user_data":"","appendable":false}`.
- `DirEntry` (`rgw_bucket_dir_entry`): version 8, compat 3,
  `MAX_DECODE_VERSION` 8. Fields: `key: ObjKey`, `ver: EntryVer`,
  `locator`, `exists: bool`, `meta: DirEntryMeta`, `pending_map:
  Vec<(String, PendingInfo)>`, `index_ver: u64`, `tag`, `flags: u16`,
  `versioned_epoch: u64`. Wire: `key.name`, `ver.epoch` as a plain u64,
  `exists`, `meta`, `pending_map` (u32 count, then string + PendingInfo
  pairs), `locator` (v2+), `ver` (v4+, else `pool = -1`; this decode
  overwrites the epoch read earlier), packed `index_ver` (v5+), `tag`
  (v5+), `key.instance` (v6+), `flags` (v7+), `versioned_epoch` (v8+).
  Dump: `name`, `instance`, `ver`, `locator`, `exists`, `meta`, `tag`,
  `flags` (number), `pending_map` (`map_entries`), `versioned_epoch`; no
  `index_ver`. Helpers `is_current`, `is_delete_marker`, `is_visible`,
  `is_valid`, `is_common_prefix` as the C++. Oracle instance 1 (159
  bytes):
  `080399000000040000006e616d65d2040000000000000107035300000001640000000000000000000000000000000400000065746167050000006f776e65720c000000646973706c6179206e616d650c000000636f6e74656e742f74797065000000000000000000000000000000000000000000070000006c6f6361746f720101040000000182d20400030000007461670000000000000000000000000000`
  with `key.name = "name"`, `ver = {1, 1234}`, `locator = "locator"`,
  `exists = true`, `meta` = the instance above, `tag = "tag"`; JSON
  `{"name":"name","instance":"","ver":{"pool":1,"epoch":1234},"locator":"locator","exists":true,"meta":{...},"tag":"tag","flags":0,"pending_map":[],"versioned_epoch":0}`.
  Add a unit test with one `pending_map` entry and a non-zero `flags`
  round-tripping through decode, and one asserting `is_current`/
  `is_visible` on `FLAG_VER | FLAG_CURRENT` versus `FLAG_VER` alone.
- `DirHeader` (`rgw_bucket_dir_header`): encode version 7, compat 2,
  `MAX_DECODE_VERSION` 8. Fields: `stats: BTreeMap<ObjCategory,
  CategoryStats>`, `tag_timeout: u64`, `ver: u64`, `master_ver: u64`,
  `max_marker: String`, `new_instance: BucketInstanceEntry`,
  `syncstopped: bool`. Wire in that order with the version gates the C++
  has (`tag_timeout` v3+, `ver`/`master_ver` v4+, `max_marker` v5+,
  `new_instance` v6+, `syncstopped` v7+; v8's `reshardlog_entries` is
  skipped). Dump: `ver` (number), `master_ver`, `stats` as an array
  alternating the category number and the stats object, `new_instance`;
  nothing else. Helpers `resharding()`, `resharding_in_progress()`.
  Oracle instance 1 (93 bytes):
  `07025700000001000000000302200000000004000000000000001000000000000002000000000000000004000000000000000000000000000000000000000000000000000000000000000000000301090000000000000000ffffffff00`
  = `{"ver":0,"master_ver":0,"stats":[0,{"total_size":1024,"total_size_rounded":4096,"num_entries":2,"actual_size":1024}],"new_instance":{"reshard_status":"not-resharding"}}`.
  The alternating array needs a custom `Serialize` that opens a sequence
  and pushes the key then the value.
- `Dir` (`rgw_bucket_dir`): version 2, compat 2. Fields `header:
  DirHeader`, `entries: BTreeMap<String, DirEntry>` (`m`). Dump:
  `header`, then `map` alternating key and `dir_entry` objects. Oracle
  instance 1 (103 bytes) is the header above wrapped:
  `02026100000007025700000001000000000302200000000004000000000000001000000000000002000000000000000004000000000000000000000000000000000000000000000000000000000000000000000301090000000000000000ffffffff0000000000`.
- `BiLogEntry` (`rgw_bi_log_entry`): version 4, compat 1. Fields `id`,
  `object`, `instance`, `timestamp: UTime`, `ver: EntryVer`, `op:
  ModifyOp`, `state: PendingState`, `index_ver: u64`, `tag`, `bilog_flags:
  u16`, `owner`, `owner_display_name`, `zones_trace: ZoneSet`. Wire: `id`,
  `object`, `timestamp`, `ver`, `tag`, `op` u8, `state` u8, packed
  `index_ver`, `instance` (v2+), `bilog_flags` (v2+), `owner` and
  `owner_display_name` (v3+), `zones_trace` (v4+). Dump: `op_id` (id),
  `op_tag` (tag), `op` (name), `object`, `instance`, `state` (`pending`,
  `complete`, else `invalid`), `index_ver` (number), `timestamp`
  (`utime_nsec`), `ver`, `bilog_flags` (number), `versioned` (bool:
  `bilog_flags & 1`), `owner`, `owner_display_name`, `zones_trace` (bare
  array). Constants `BILOG_FLAG_VERSIONED_OP = 0x1`, `BILOG_NULL_VERSION
  = 0x2`; helpers `is_versioned`, `is_null_verid`. Oracle instance 0 (81
  bytes):
  `04014b000000040000006d696466030000006f626a020000000300000001010a00000088ffffffffffffffff0009000000746167617364666473010082e310000000000000000000000000000000000000`
  = `{"op_id":"midf","op_tag":"tagasdfds","op":"del","object":"obj","instance":"","state":"pending","index_ver":4323,"timestamp":"2.000000","ver":{"pool":-1,"epoch":0},"bilog_flags":0,"versioned":false,"owner":"","owner_display_name":"","zones_trace":[]}`
  (timestamp `{2 s, 3 ns}`; `gmtime_nsec` prints `"2.000000"` below ten
  years and nine fraction digits in the calendar form; pin
  `iso_nsec(&UTime { sec: 1_727_611_205, nsec: 747_275_123 }) ==
  "2024-09-29T12:00:05.747275123Z"` in `dump.rs`).
- `BucketInstanceEntry` (`cls_rgw_bucket_instance_entry`): version 3,
  compat 1. One field `reshard_status: ReshardStatus`; the wire also
  carries an empty string and an `i32` `-1` after it (fields removed in
  version 2 and put back empty in 3; a version-2 buffer lacks them).
  Dump `{"reshard_status": name}`. Oracle instance 0 (15 bytes):
  `0301090000000100000000ffffffff` = `{"reshard_status":"in-progress"}`.
  Helpers `resharding()`, `resharding_in_progress()`.
- `ReshardEntry` (`cls_rgw_reshard_entry`): encode version 2, compat 1,
  `MAX_DECODE_VERSION` 3. Fields `time: UTime`, `tenant`, `bucket_name`,
  `bucket_id`, `old_num_shards: u32`, `new_num_shards: u32`. Wire in that
  order; a version-1 buffer carries a string `new_instance_id` between
  `bucket_id` and the shard counts (read and dropped); v3's `initiator`
  byte is skipped. Dump: `time` (utime), `tenant`, `bucket_name`,
  `bucket_id`, `old_num_shards`, `tentative_new_num_shards`. `key()` =
  `tenant + ":" + bucket_name`. Oracle instance 0 (56 bytes):
  `02013200000002000000030000000600000074656e616e74070000006275636b657431090000006275636b65745f69640800000040000000`
  = `{"time":"2.000000","tenant":"tenant","bucket_name":"bucket1","bucket_id":"bucket_id","old_num_shards":8,"tentative_new_num_shards":64}`.

`dump.rs` additions (gated `rgw`):

```rust
/// `utime_t::gmtime_nsec`: as [`gmtime`], with nine fraction digits in
/// the calendar form.
pub(crate) fn gmtime_nsec(t: &UTime) -> String { ... }
pub(crate) fn utime_nsec<S: serde::Serializer>(t: &UTime, s: S) -> Result<S::Ok, S::Error> { ... }

/// `encode_json` of a map: an array of `{"key": k, "val": v}` objects.
pub(crate) fn map_entries<'a, K: Serialize + 'a, V: Serialize + 'a, I, S>(entries: I, s: S) -> Result<S::Ok, S::Error>
where I: IntoIterator<Item = (&'a K, &'a V)>, S: serde::Serializer { ... }
```
with `serialize_with` wrappers where the field type needs them
(`fn pending_map<S>(m: &[(String, PendingInfo)], s: S)` in `index.rs`
calling `map_entries(m.iter().map(|(k, v)| (k, v)), s)`).

- [ ] **Step 1 to 5** as Task 2: failing tests first (each type: oracle
  bytes both ways, JSON, plus the version-gated decode cases named
  above), implement, run `cargo test -p rados-cls --lib --offline rgw::`
  and the two `cargo check`s, commit:

```
cls: the rgw bucket-index types

rgw_bucket_dir_entry writes ver.epoch twice and splits its key across
the wire; the header's stats map is keyed by a one-byte category; the
bucket-instance entry carries two empty legacy fields; the reshard
entry is written as version 2, the meta as 7 and the header as 7,
which is what Ceph v19.2.2 writes, while newer versions decode with
their tails skipped.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 4: `rgw::olh` and `BiEntry`

**Files:**
- Create: `rados-cls/src/rgw/olh.rs`.
- Modify: `rados-cls/src/rgw/index.rs` (`BiEntry`),
  `rados-cls/src/rgw/mod.rs`, dencoder (three arms), corpus table
  (`rgw_bucket_olh_log_entry`, `rgw_bucket_olh_entry`,
  `rgw_cls_bi_entry`).

**Interfaces:**
- Produces `olh::OlhLogOp(u8)` (`UNKNOWN`, `LINK_OLH`, `UNLINK_OLH`,
  `REMOVE_INSTANCE`; `as_str()`: `link_olh`, `unlink_olh`,
  `remove_instance`, else `unknown`), `olh::OlhLogEntry`,
  `olh::OlhEntry`; `index::BiEntry` with `get_info`.

Facts:

- `OlhLogEntry` (`rgw_bucket_olh_log_entry`): version 1. Fields `epoch:
  u64`, `op: OlhLogOp`, `op_tag`, `key: ObjKey`, `delete_marker: bool`,
  in wire order. Dump: `epoch`, `op` (name), `op_tag`, `key` (object),
  `delete_marker`. Oracle instance 1 (60 bytes):
  `010136000000d20400000000000001060000006f705f74616701011c000000080000006b65792e6e616d650c0000006b65792e696e7374616e636501`
  = `{"epoch":1234,"op":"link_olh","op_tag":"op_tag","key":{"name":"key.name","instance":"key.instance"},"delete_marker":true}`.
- `OlhEntry` (`rgw_bucket_olh_entry`): version 1. Fields `key: ObjKey`,
  `delete_marker: bool`, `epoch: u64`, `pending_log: BTreeMap<u64,
  Vec<OlhLogEntry>>`, `tag`, `exists: bool`, `pending_removal: bool`.
  Dump in that order with `pending_log` as `map_entries`. Oracle instance
  1 (62 bytes):
  `01013800000001011c000000080000006b65792e6e616d650c0000006b65792e696e7374616e636501d20400000000000000000000030000007461670101`
  = `{"key":{"name":"key.name","instance":"key.instance"},"delete_marker":true,"epoch":1234,"pending_log":[],"tag":"tag","exists":true,"pending_removal":true}`.
  Add a test with one `pending_log` entry: JSON
  `"pending_log":[{"key":5,"val":[{...}]}]`.
- `BiEntry` (`rgw_cls_bi_entry`): version 1. Fields `kind: BiIndexType`
  (`type`), `idx: String`, `data: Bytes`. Dump: `type` (name), `idx`,
  then `entry`: `data` decoded as a `DirEntry` for `PLAIN`/`INSTANCE`, an
  `OlhEntry` for `OLH`, and no `entry` key otherwise; a payload that
  fails to decode is a serialization error. `get_info(&self) ->
  Result<BiInfo>` mirrors the C++: for `OLH` the key and `false`; else
  decode a dir entry, return its key, category, the stats delta
  (`num_entries` 1, `total_size` `accounted_size`, `total_size_rounded`
  `rounded_size(accounted_size)`, `actual_size` `size`) and whether it
  counts (`PLAIN`: `exists && flags == 0`; `INSTANCE`: `exists`; else
  false). Oracle instance 1 (80 bytes):
  `01014a00000003030000006964783e00000001013800000001011c000000080000006b65792e6e616d650c0000006b65792e696e7374616e636501d20400000000000000000000030000007461670101`
  = `{"type":"olh","idx":"idx","entry":{...the OLH entry above...}}`;
  instance 0 = `{"type":"invalid","idx":""}`.

Commit:

```
cls: the rgw OLH entry, OLH log entry and bucket-index entry wrapper

rgw_cls_bi_entry carries an encoded dir entry or OLH entry by type,
and its dump decodes the payload; get_info accounts a dir entry the
way reshard does.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 5: `rgw::usage`

**Files:**
- Create: `rados-cls/src/rgw/usage.rs`. Modify `mod.rs`, dencoder (five
  arms), corpus table (`rgw_usage_data`, `rgw_s3select_usage_data`,
  `rgw_usage_log_entry`, `rgw_usage_log_info`, `rgw_user_bucket`; the
  s3select type has no corpus directory in either archive).

**Interfaces:**
- Produces `UsageData`, `S3selectUsageData`, `UsageLogEntry`,
  `UsageLogInfo`, `UserBucket`.

Facts:

- `UsageData` (`rgw_usage_data`): version 1; four u64s `bytes_sent`,
  `bytes_received`, `ops`, `successful_ops`; dump the same (numbers);
  `aggregate(&mut self, &Self)`. Oracle instance 1:
  `0101200000000004000000000000000400000000000002000000000000000100000000000000`.
- `S3selectUsageData`: version 1; `bytes_processed`, `bytes_returned`;
  `aggregate`.
- `UsageLogEntry` (`rgw_usage_log_entry`): encode version 4, compat 1,
  `MAX_DECODE_VERSION` 4. Fields `owner: String`, `payer: String`,
  `bucket`, `epoch: u64`, `total_usage: UsageData`, `usage_map:
  BTreeMap<String, UsageData>`, `s3select_usage: S3selectUsageData`.
  Wire: `owner`, `bucket`, `epoch`, the four `total_usage` fields inline
  (no header), `usage_map` (v2+; a v1 buffer sets `usage_map[""] =
  total_usage`), `payer` (v3+), `s3select_usage` (v4+). Dump: `owner`,
  `payer`, `bucket`, `epoch`, `total_usage` object, `categories` as an
  array of `{category, bytes_sent, bytes_received, ops, successful_ops}`,
  `s3select` object. Helpers `aggregate(&mut self, &Self, categories:
  Option<&BTreeSet<String>>)`, `sum(&self, categories) -> UsageData`,
  `add_usage(&mut self, category, &UsageData)` as the C++. Oracle
  instance 1 (149 bytes):
  `04018f000000050000006f776e6572060000006275636b6574d204000000000000000400000000000000080000000000000000000000000000000000000000000001000000070000006765745f6f626a010120000000000400000000000000080000000000000000000000000000000000000000000005000000706179657201011000000000200000000000000010000000000000`
  = `{"owner":"owner","payer":"payer","bucket":"bucket","epoch":1234,"total_usage":{"bytes_sent":1024,"bytes_received":2048,"ops":0,"successful_ops":0},"categories":[{"category":"get_obj","bytes_sent":1024,"bytes_received":2048,"ops":0,"successful_ops":0}],"s3select":{"bytes_processed":8192,"bytes_returned":4096}}`.
  Also pin a version-3 decode (the corpus form): the same bytes with
  header `0301 79000000` and without the trailing 22-byte s3select
  block, decoding to zero s3select counters.
- `UsageLogInfo`: version 1; `entries: Vec<UsageLogEntry>`; dump
  `entries`.
- `UserBucket` (`rgw_user_bucket`): version 1; `user`, `bucket`; derive
  `Ord` (lexicographic, as the C++). Oracle instance 1:
  `0101120000000400000075736572060000006275636b6574`.

Commit:

```
cls: the rgw usage log types

rgw_usage_log_entry writes its total inline without a header, its
owner and payer as rgw_user's string form, and, since v19.2.2, the
s3select counters as version 4; the corpus holds version 3.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```

---

### Task 6: `rgw::lc`

**Files:**
- Create: `rados-cls/src/rgw/lc.rs`. Modify `mod.rs`, dencoder (two
  arms), corpus table (`cls_rgw_lc_entry`, `cls_rgw_lc_obj_head`; the head
  has no corpus directory and no test instances).

Facts:

- `LcEntry` (`cls_rgw_lc_entry`): version 1; `bucket`, `start_time: u64`,
  `status: u32`; dump the same. Oracle instance 1:
  `010116000000060000006275636b65740a0000000000000001000000`.
- `LcObjHead` (`cls_rgw_lc_obj_head`): version 2, compat 2; `start_date:
  i64` (a `time_t`, written as u64), `marker`, `shard_rollover_date: i64`
  (v2+, written as i64); dump `start_date` (number), `marker` only. Pin
  the bytes of `{10, "m", 20}`:
  `0202 15000000 0a00000000000000 01000000 6d 1400000000000000`.

Commit:

```
cls: the rgw lifecycle head and entry

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
```
(with a body sentence on the head's two `time_t` fields written as
eight-byte integers and the dump dropping the rollover date).

---

### Task 7: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6 without the cluster suites (no class calls in this
branch; the existing three suites still run in CI): per-commit proof;
container fmt/clippy including each class alone; corpus loop over the
twenty-two registered names (expect `rgw_s3select_usage_data` and
`cls_rgw_lc_obj_head` to report no directory in both archives); the PR
body:

```
**Motivation.** Every method of the `rgw` object class encodes or decodes the bucket-index entry, its header, the bilog, OLH, usage, lifecycle and reshard types; a Rust RGW needs them once, in one place, before the class's methods can follow.

**What changed.** `rados-cls`'s `rgw` module gains the value types of `cls_rgw_types.h` in the files their classes will extend (`types`, `index`, `olh`, `usage`, `lc`), Ceph's packed integers, one-byte enums that keep unknown values, and every dump quirk pinned against `ceph-dencoder` v19.2.2 (twenty-two registered types compared against the corpus). Encoders write what v19.2.2 writes; decoders also accept the versions Ceph `main` writes.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

## Roadmap for later plans

`cls-rgw-bucket-index` (ops and client in `rgw::index`);
`cls-rgw-gc` (`rgw::gc`); `cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh`;
`watch-notify`. Deferred: the Tentacle fields (`restore_*`,
`reshardlog_entries`, `initiator`, `rgw_bucket_deleted_entry`,
`BIIndexType::ReshardDeleted`, `OLHLogOp::STALE`,
`ReshardStatus::IN_LOGRECORD`, `ObjCategory::MULTI_PART`) are decoded by
skipping or kept as raw bytes, not modelled; legacy one-byte headers are
not decoded; `rgw_zone_set_entry` has corpus files but no
`ceph-dencoder` registration.
