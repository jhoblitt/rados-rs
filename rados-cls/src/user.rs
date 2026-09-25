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
        assert_eq!(
            encode_with_capacity(&stats, 0).expect("encode").as_ref(),
            &stats_wire[..]
        );

        let header = Header {
            stats: stats.clone(),
            last_stats_sync: UTime { sec: 1, nsec: 0 },
            last_stats_update: UTime { sec: 2, nsec: 0 },
        };
        let mut c = stats_wire.clone();
        c.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            encode_with_capacity(&header, 0).expect("encode").as_ref(),
            &frame(1, 1, &c)[..]
        );

        let op = ListBucketsOp {
            marker: s("marker"),
            max_entries: 1000,
            end_marker: String::new(),
        };
        let mut c = str_wire("marker");
        c.extend_from_slice(&1000i32.to_le_bytes());
        c.extend_from_slice(&str_wire(""));
        assert_eq!(
            encode_with_capacity(&op, 0).expect("encode").as_ref(),
            &frame(2, 1, &c)[..]
        );

        let op = ResetStats2Op {
            time: UTime { sec: 4, nsec: 0 },
            marker: s("m"),
            acc_stats: stats.clone(),
        };
        let mut c = vec![4, 0, 0, 0, 0, 0, 0, 0];
        c.extend_from_slice(&str_wire("m"));
        c.extend_from_slice(&stats_wire);
        assert_eq!(
            encode_with_capacity(&op, 0).expect("encode").as_ref(),
            &frame(1, 1, &c)[..]
        );
        let ret = ResetStats2Ret {
            marker: s("m"),
            acc_stats: stats.clone(),
            truncated: true,
        };
        let mut c = str_wire("m");
        c.extend_from_slice(&stats_wire);
        c.push(1);
        assert_eq!(
            encode_with_capacity(&ret, 0).expect("encode").as_ref(),
            &frame(1, 1, &c)[..]
        );

        assert_eq!(
            encode_with_capacity(&GetHeaderOp {}, 0)
                .expect("encode")
                .as_ref(),
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
        assert_eq!(
            encode_with_capacity(&add, 0).expect("encode").as_ref(),
            &frame(1, 1, &outer)[..]
        );
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
