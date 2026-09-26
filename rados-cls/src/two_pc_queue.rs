//! The `2pc_queue` object class; see the client below.

use std::collections::BTreeMap;

use bytes::{Buf, BufMut, Bytes};
use rados::{Denc, RadosError, UTime, VersionedDenc, VersionedEncode};
use serde::Serialize;

pub const CLASS: &str = "2pc_queue";

/// `cls_2pc_reservation::NO_ID`: RGW's no-reservation sentinel; the
/// class itself hands out 0 once `last_id` wraps, and RGW then skips
/// committing it.
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

impl VersionedEncode for Reservation {
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
        self.size.encode(buf, features)?;
        self.timestamp.encode(buf, features)?;
        self.entries.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        rados::check_min_version!(struct_v, 1, "Reservation", "Pacific v16+");

        let size = u64::decode(buf, features)?;
        let timestamp = UTime::decode(buf, features)?;
        let entries = if struct_v >= 2 {
            u32::decode(buf, features)?
        } else {
            0
        };
        Ok(Self {
            size,
            timestamp,
            entries,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        Some(20)
    }
}

rados::impl_denc_for_versioned!(Reservation);

/// `cls_2pc_urgent_data`: the class's bookkeeping, kept in the queue
/// head's urgent data. `reserved_size` is the bytes promised to open
/// reservations, entry overheads included; on an OSD before v19.2.4 or
/// v20.2.3 each committed, aborted or expired entry leaves ten bytes
/// behind (see the module doc), and a client cannot repair it.
/// `last_id` is the last id handed out. `has_xattrs` says reservations
/// have spilled into [`URGENT_DATA_XATTR`]; it is never cleared.
/// `committed_entries` counts reserved entries committed less entries
/// removed; the dump leaves it out.
///
/// Ceph keeps `reservations` in an `unordered_map` and writes and dumps
/// it in libstdc++ hash order; this map re-encodes and dumps in
/// ascending id order, so two or more reservations differ from Ceph's
/// bytes and dump in order only. Encodes version 2, what a leaking OSD
/// writes; decodes version 3 (v19.2.4, v20.2.3 and main, same layout,
/// written by an OSD without the leak, though a v3 head need not have
/// an exact `reserved_size`: only reserve recomputes it on a v2 head,
/// and commit, abort, expire and remove entries rewrite a v2 head as v3
/// with its drift intact) and version 1 (no `committed_entries`), the
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

impl VersionedEncode for UrgentData {
    const MAX_DECODE_VERSION: u8 = 3;

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
        self.reserved_size.encode(buf, features)?;
        self.last_id.encode(buf, features)?;
        self.reservations.encode(buf, features)?;
        self.has_xattrs.encode(buf, features)?;
        self.committed_entries.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        rados::check_min_version!(struct_v, 1, "UrgentData", "Pacific v16+");

        let reserved_size = u64::decode(buf, features)?;
        let last_id = u32::decode(buf, features)?;
        let reservations = BTreeMap::<u32, Reservation>::decode(buf, features)?;
        let has_xattrs = bool::decode(buf, features)?;
        let committed_entries = if struct_v >= 2 {
            u32::decode(buf, features)?
        } else {
            0
        };
        Ok(Self {
            reserved_size,
            last_id,
            reservations,
            has_xattrs,
            committed_entries,
        })
    }

    fn encoded_size_content(&self, _features: u64, _version: u8) -> Option<usize> {
        None
    }
}

rados::impl_denc_for_versioned!(UrgentData);

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

#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use std::fmt::Debug;

    fn unhex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex")
            })
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
        pin(
            &ReserveOp::default(),
            "01010c000000000000000000000000000000",
            r#"{"size":0,"entries":0}"#,
        );
        pin(
            &ReserveOp {
                size: 123,
                entries: 456,
            },
            "01010c0000007b00000000000000c8010000",
            r#"{"size":123,"entries":456}"#,
        );
        pin(
            &ReserveRet { id: 123 },
            "0101040000007b000000",
            r#"{"id":123}"#,
        );
        // Corpus 18.2.0 cls_2pc_queue_expire_op/7ace2f35....
        let e: ExpireOp = decode("010108000000077bcf64889b822b");
        assert_eq!(
            e.stale_time,
            UTime {
                sec: 1_691_319_047,
                nsec: 729_979_784
            }
        );
        assert_eq!(
            json(&e),
            r#"{"stale_time":"2023-08-06T10:50:47.729979+0000"}"#
        );
    }

    #[test]
    fn reservation_is_v2_dumps_without_entries_and_reads_reef_v1() {
        pin(
            &Reservation::default(),
            "0201140000000000000000000000000000000000000000000000",
            r#"{"size":0,"timestamp":"1970-01-01T00:00:00.000000+0000"}"#,
        );
        pin(
            &Reservation {
                size: 123,
                ..Reservation::default()
            },
            "0201140000007b00000000000000000000000000000000000000",
            r#"{"size":123,"timestamp":"1970-01-01T00:00:00.000000+0000"}"#,
        );
        // Corpus 19.2.0 cls_2pc_reservation/ff1b6965...: entries 23, not dumped.
        pin(
            &Reservation {
                size: 358,
                timestamp: UTime {
                    sec: 1_727_592_400,
                    nsec: 841_226_706,
                },
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
                timestamp: UTime {
                    sec: 1_691_319_017,
                    nsec: 462_952_248
                },
                entries: 0,
            }
        );
        assert_eq!(
            bytes(&v1),
            unhex("020114000000fa00000000000000e97acf643817981b00000000")
        );
        assert_eq!(
            json(&v1),
            r#"{"size":250,"timestamp":"2023-08-06T10:50:17.462952+0000"}"#
        );
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
            Reservation {
                size: 1,
                timestamp: UTime::default(),
                entries: 2,
            },
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
        assert_eq!(
            v1,
            UrgentData {
                reserved_size: 5520,
                last_id: 24,
                ..UrgentData::default()
            }
        );
        assert_eq!(
            bytes(&v1),
            unhex("020115000000901500000000000018000000000000000000000000")
        );
        // main and v19.2.4+ write version 3 with the same layout.
        let v3: UrgentData = decode("030115000000e2090000000000000b0000000000000000fd000000");
        assert_eq!((v3.reserved_size, v3.committed_entries), (2530, 253));
        assert_eq!(bytes(&v3)[0], 2);
    }

    #[test]
    fn reservations_ret_dumps_and_encodes_in_ascending_id_order() {
        pin(
            &ReservationsRet::default(),
            "01010400000000000000",
            r#"{"reservations":[]}"#,
        );
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
            unhex(
                "01014000000002000000010000000201140000000000000000000000000000000000000000000000020000000201140000000000000000000000000000000000000000000000"
            )
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
        assert_eq!(
            bytes(&r),
            unhex("02010f00000007000000302f33313739362a000000")
        );
        assert_eq!(
            decode::<RemoveOp>("02010f00000007000000302f33313739362a000000"),
            r
        );
        // Version 1 on the wire is cls_queue_remove_op, queue::RemoveOp.
        assert!(
            RemoveOp::decode(&mut &unhex("01010b00000007000000302f3331373936")[..], 0).is_err()
        );
    }
}
