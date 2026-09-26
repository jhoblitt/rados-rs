//! The `lock` class: advisory locks on an object, kept in one xattr per
//! lock (`lock.<name>`). Mirrors `cls_lock_types.h` and `cls_lock_ops.h`.

use std::collections::BTreeMap;

use rados::{UTime, VersionedDenc};
use serde::Serialize;
use serde::ser::{SerializeSeq, SerializeStruct};

use crate::dump;

pub use rados::osdclient::types::PackedEntityName;
pub use rados::{EntityAddr, LockFlags, LockType};

/// The class name.
pub const CLASS: &str = "lock";

/// `cls_lock_type_str`: `none`, `exclusive`, `shared`,
/// `exclusive-ephemeral`.
pub fn lock_type_str(t: LockType) -> &'static str {
    match t {
        LockType::None => "none",
        LockType::Exclusive => "exclusive",
        LockType::Shared => "shared",
        LockType::ExclusiveEphemeral => "exclusive-ephemeral",
    }
}

fn type_as_str<S: serde::Serializer>(t: &LockType, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(lock_type_str(*t))
}

fn type_as_int<S: serde::Serializer>(t: &LockType, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u8(*t as u8)
}

fn flags_as_int<S: serde::Serializer>(f: &LockFlags, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u8(f.bits())
}

fn entity_name<S: serde::Serializer>(n: &PackedEntityName, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(n)
}

fn legacy_addr<S: serde::Serializer>(a: &EntityAddr, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&a.legacy_str())
}

/// `locker_id_t`: a holder, the client's entity plus the cookie it
/// locked with. Ordered as the C++ key is, entity first.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct LockerId {
    #[serde(serialize_with = "entity_name")]
    pub locker: PackedEntityName,
    pub cookie: String,
}

/// `locker_info_t`: what the class records about a holder. `expiration`
/// is on the OSD's clock, zero for a lock that never expires.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct LockerInfo {
    #[serde(serialize_with = "dump::utime_localtime")]
    pub expiration: UTime,
    #[serde(serialize_with = "legacy_addr")]
    pub addr: EntityAddr,
    pub description: String,
}

/// `lock_info_t`: the value of the object's `lock.<name>` xattr.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct LockInfo {
    pub lockers: BTreeMap<LockerId, LockerInfo>,
    pub lock_type: LockType,
    pub tag: String,
}

/// The dump prints the type as an int and nests each holder as
/// `{"id", "info"}`.
impl Serialize for LockInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Locker<'a> {
            id: &'a LockerId,
            info: &'a LockerInfo,
        }

        struct Lockers<'a>(&'a BTreeMap<LockerId, LockerInfo>);

        impl Serialize for Lockers<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut seq = s.serialize_seq(Some(self.0.len()))?;
                for (id, info) in self.0 {
                    seq.serialize_element(&Locker { id, info })?;
                }
                seq.end()
            }
        }

        #[derive(Serialize)]
        struct Dump<'a> {
            #[serde(serialize_with = "type_as_int")]
            lock_type: &'a LockType,
            tag: &'a str,
            lockers: Lockers<'a>,
        }

        Dump {
            lock_type: &self.lock_type,
            tag: &self.tag,
            lockers: Lockers(&self.lockers),
        }
        .serialize(serializer)
    }
}

/// `cls_lock_lock_op`. `duration` is sent as given (RGW's multisite path
/// puts milliseconds in `nsec`); zero never expires.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct LockOp {
    pub name: String,
    #[serde(rename = "type", serialize_with = "type_as_str")]
    pub lock_type: LockType,
    pub cookie: String,
    pub tag: String,
    pub description: String,
    #[serde(serialize_with = "dump::utime_localtime")]
    pub duration: UTime,
    #[serde(serialize_with = "flags_as_int")]
    pub flags: LockFlags,
}

/// `cls_lock_unlock_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct UnlockOp {
    pub name: String,
    pub cookie: String,
}

/// `cls_lock_break_op`; the dump prints `cookie` before `locker`.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct BreakOp {
    pub name: String,
    pub locker: PackedEntityName,
    pub cookie: String,
}

impl Serialize for BreakOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("BreakOp", 3)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("cookie", &self.cookie)?;
        state.serialize_field("locker", &self.locker.to_string())?;
        state.end()
    }
}

/// `cls_lock_get_info_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetInfoOp {
    pub name: String,
}

/// `cls_lock_get_info_reply`: [`LockInfo`]'s wire, with expired holders
/// already dropped. The dump prints the type as a string and flattens
/// each holder.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetInfoReply {
    pub lockers: BTreeMap<LockerId, LockerInfo>,
    pub lock_type: LockType,
    pub tag: String,
}

impl Serialize for GetInfoReply {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Locker<'a> {
            locker: String,
            description: &'a str,
            cookie: &'a str,
            expiration: String,
            addr: String,
        }

        let lockers: Vec<Locker<'_>> = self
            .lockers
            .iter()
            .map(|(id, info)| Locker {
                locker: id.locker.to_string(),
                description: &info.description,
                cookie: &id.cookie,
                expiration: dump::localtime(&info.expiration),
                addr: info.addr.legacy_str(),
            })
            .collect();
        let mut state = serializer.serialize_struct("GetInfoReply", 3)?;
        state.serialize_field("lock_type", lock_type_str(self.lock_type))?;
        state.serialize_field("tag", &self.tag)?;
        state.serialize_field("lockers", &lockers)?;
        state.end()
    }
}

impl From<GetInfoReply> for LockInfo {
    fn from(r: GetInfoReply) -> Self {
        Self {
            lockers: r.lockers,
            lock_type: r.lock_type,
            tag: r.tag,
        }
    }
}

impl From<LockInfo> for GetInfoReply {
    fn from(i: LockInfo) -> Self {
        Self {
            lockers: i.lockers,
            lock_type: i.lock_type,
            tag: i.tag,
        }
    }
}

/// `cls_lock_list_locks_reply`; the dump wraps each name in its own
/// array.
#[derive(Debug, Clone, Default, PartialEq, Eq, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ListLocksReply {
    pub locks: Vec<String>,
}

impl Serialize for ListLocksReply {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wrapped: Vec<[&str; 1]> = self.locks.iter().map(|l| [l.as_str()]).collect();
        let mut state = serializer.serialize_struct("ListLocksReply", 1)?;
        state.serialize_field("locks", &wrapped)?;
        state.end()
    }
}

/// `cls_lock_assert_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct AssertOp {
    pub name: String,
    #[serde(rename = "type", serialize_with = "type_as_str")]
    pub lock_type: LockType,
    pub cookie: String,
    pub tag: String,
}

/// `cls_lock_set_cookie_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct SetCookieOp {
    pub name: String,
    #[serde(rename = "type", serialize_with = "type_as_str")]
    pub lock_type: LockType,
    pub cookie: String,
    pub tag: String,
    pub new_cookie: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rados::{Denc, EntityAddrType, encode_with_capacity};

    fn bytes<T: Denc>(v: &T) -> Vec<u8> {
        encode_with_capacity(v, 0).expect("encode").to_vec()
    }

    fn json<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).expect("json")
    }

    /// Bytes from a hex string, as `ceph-dencoder ... encode export` wrote them.
    fn unhex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex")
            })
            .collect()
    }

    /// Encodes to `hex`, decodes back to `v`, and dumps as `expected`.
    fn pin<T: Denc + Serialize + PartialEq + std::fmt::Debug>(v: &T, hex: &str, expected: &str) {
        assert_eq!(bytes(v), unhex(hex));
        assert_eq!(&T::decode(&mut &unhex(hex)[..], 0).expect("decode"), v);
        assert_eq!(json(v), expected);
    }

    fn s(v: &str) -> String {
        v.to_owned()
    }

    fn client(n: u64) -> PackedEntityName {
        PackedEntityName::new(0x08, n)
    }

    fn legacy(addr: &str, nonce: u32) -> EntityAddr {
        EntityAddr {
            nonce,
            ..EntityAddr::from_socket_addr(EntityAddrType::Legacy, addr.parse().expect("addr"))
        }
    }

    fn id(n: u64, cookie: &str) -> LockerId {
        LockerId {
            locker: client(n),
            cookie: s(cookie),
        }
    }

    fn info(sec: u32, addr: EntityAddr, description: &str) -> LockerInfo {
        LockerInfo {
            expiration: UTime::new(sec, 0),
            addr,
            description: s(description),
        }
    }

    const LOCKER_INFO_1: &str = "01013a00000005000000000000000101011c000000010000000100000010000000020000027f00010200000000000000000b0000006465736372697074696f6e";
    const LOCK_OP_1: &str = "010132000000040000006e616d650206000000636f6f6b6965030000007461670b0000006465736372697074696f6e050000000000000001";

    #[test]
    fn locker_id_matches_the_oracle() {
        pin(
            &id(1, "cookie"),
            "01011300000008010000000000000006000000636f6f6b6965",
            r#"{"locker":"client.1","cookie":"cookie"}"#,
        );
        pin(
            &LockerId::default(),
            "01010d00000000000000000000000000000000",
            r#"{"locker":"unknown.0","cookie":""}"#,
        );
    }

    #[test]
    fn locker_info_matches_the_oracle() {
        pin(
            &info(5, legacy("127.0.1.2:2", 1), "description"),
            LOCKER_INFO_1,
            r#"{"expiration":"5.000000","addr":"127.0.1.2:2/1","description":"description"}"#,
        );
        pin(
            &LockerInfo::default(),
            "01013b00000000000000000000000101012800000000000000000000001c0000000000000000000000000000000000000000000000000000000000000000000000",
            r#"{"expiration":"0.000000","addr":"(unrecognized address family 0)/0","description":""}"#,
        );
    }

    #[test]
    fn lock_info_matches_the_oracle() {
        let i = LockInfo {
            lockers: BTreeMap::from([(
                id(1, "cookie"),
                info(5, legacy("127.0.1.2:2", 1), "description"),
            )]),
            lock_type: LockType::Exclusive,
            tag: s("tag"),
        };
        pin(
            &i,
            "0101650000000100000001011300000008010000000000000006000000636f6f6b696501013a00000005000000000000000101011c000000010000000100000010000000020000027f00010200000000000000000b0000006465736372697074696f6e0103000000746167",
            r#"{"lock_type":1,"tag":"tag","lockers":[{"id":{"locker":"client.1","cookie":"cookie"},"info":{"expiration":"5.000000","addr":"127.0.1.2:2/1","description":"description"}}]}"#,
        );
        pin(
            &LockInfo::default(),
            "010109000000000000000000000000",
            r#"{"lock_type":0,"tag":"","lockers":[]}"#,
        );
    }

    #[test]
    fn lock_info_matches_a_corpus_object() {
        // ceph-object-corpus 19.2.0-404-g78ddc7f9027 lock_info_t
        // f4cf81c9b553c2fb2c66e15ebd54a829.
        let i = LockInfo {
            lockers: BTreeMap::from([(
                LockerId {
                    locker: client(4533),
                    cookie: s("j1_EFM8DDt8SI1R"),
                },
                LockerInfo {
                    expiration: UTime::new(1_727_604_086, 460_555_954),
                    addr: legacy("172.21.5.153:0", 1_725_310_796),
                    description: String::new(),
                },
            )]),
            lock_type: LockType::Exclusive,
            tag: String::new(),
        };
        pin(
            &i,
            "0101600000000100000001011c00000008b5110000000000000f0000006a315f45464d38444474385349315201012f0000007625f966b286731b0101011c000000010000004c27d6661000000002000000ac1505990000000000000000000000000100000000",
            r#"{"lock_type":1,"tag":"","lockers":[{"id":{"locker":"client.4533","cookie":"j1_EFM8DDt8SI1R"},"info":{"expiration":"2024-09-29T10:01:26.460555+0000","addr":"172.21.5.153:0/1725310796","description":""}}]}"#,
        );
    }

    #[test]
    fn lock_op_matches_the_oracle() {
        pin(
            &LockOp {
                name: s("name"),
                lock_type: LockType::Shared,
                cookie: s("cookie"),
                tag: s("tag"),
                description: s("description"),
                duration: UTime::new(5, 0),
                flags: LockFlags::MAY_RENEW,
            },
            LOCK_OP_1,
            r#"{"name":"name","type":"shared","cookie":"cookie","tag":"tag","description":"description","duration":"5.000000","flags":1}"#,
        );
        pin(
            &LockOp::default(),
            "01011a0000000000000000000000000000000000000000000000000000000000",
            r#"{"name":"","type":"none","cookie":"","tag":"","description":"","duration":"0.000000","flags":0}"#,
        );
    }

    #[test]
    fn lock_op_with_an_unknown_type_is_refused() {
        let mut wire = unhex(LOCK_OP_1);
        assert_eq!(wire[14], 2);
        wire[14] = 4;
        assert!(LockOp::decode(&mut &wire[..], 0).is_err());
    }

    #[test]
    fn unlock_op_matches_the_oracle() {
        pin(
            &UnlockOp {
                name: s("name"),
                cookie: s("cookie"),
            },
            "010112000000040000006e616d6506000000636f6f6b6965",
            r#"{"name":"name","cookie":"cookie"}"#,
        );
        pin(
            &UnlockOp::default(),
            "0101080000000000000000000000",
            r#"{"name":"","cookie":""}"#,
        );
    }

    #[test]
    fn break_op_matches_the_oracle() {
        pin(
            &BreakOp {
                name: s("name"),
                locker: client(1),
                cookie: s("cookie"),
            },
            "01011b000000040000006e616d6508010000000000000006000000636f6f6b6965",
            r#"{"name":"name","cookie":"cookie","locker":"client.1"}"#,
        );
        pin(
            &BreakOp::default(),
            "0101110000000000000000000000000000000000000000",
            r#"{"name":"","cookie":"","locker":"unknown.0"}"#,
        );
    }

    #[test]
    fn get_info_op_matches_the_oracle() {
        pin(
            &GetInfoOp { name: s("name") },
            "010108000000040000006e616d65",
            r#"{"name":"name"}"#,
        );
        pin(
            &GetInfoOp::default(),
            "01010400000000000000",
            r#"{"name":""}"#,
        );
    }

    const GET_INFO_REPLY_1: &str = "0101c20000000200000001011400000008010000000000000007000000636f6f6b69653101013b0000000a000000000000000101011c000000010000000a00000010000000020000147f00010200000000000000000c0000006465736372697074696f6e3101011400000008020000000000000007000000636f6f6b69653201013b00000014000000000000000101011c000000010000001e00000010000000020000287f00010200000000000000000c0000006465736372697074696f6e320203000000746167";

    fn get_info_reply_1(reverse: bool) -> GetInfoReply {
        let mut entries = vec![
            (
                id(1, "cookie1"),
                info(10, legacy("127.0.1.2:20", 10), "description1"),
            ),
            (
                id(2, "cookie2"),
                info(20, legacy("127.0.1.2:40", 30), "description2"),
            ),
        ];
        if reverse {
            entries.reverse();
        }
        let mut lockers = BTreeMap::new();
        for (k, v) in entries {
            lockers.insert(k, v);
        }
        GetInfoReply {
            lockers,
            lock_type: LockType::Shared,
            tag: s("tag"),
        }
    }

    #[test]
    fn get_info_reply_matches_the_oracle() {
        pin(
            &get_info_reply_1(false),
            GET_INFO_REPLY_1,
            r#"{"lock_type":"shared","tag":"tag","lockers":[{"locker":"client.1","description":"description1","cookie":"cookie1","expiration":"10.000000","addr":"127.0.1.2:20/10"},{"locker":"client.2","description":"description2","cookie":"cookie2","expiration":"20.000000","addr":"127.0.1.2:40/30"}]}"#,
        );
        pin(
            &GetInfoReply::default(),
            "010109000000000000000000000000",
            r#"{"lock_type":"none","tag":"","lockers":[]}"#,
        );
    }

    #[test]
    fn lockers_encode_in_the_osds_order_whatever_the_insertion_order() {
        assert_eq!(bytes(&get_info_reply_1(true)), unhex(GET_INFO_REPLY_1));

        let mut lockers = BTreeMap::new();
        for key in [id(1, "b"), id(1, "a"), id(u64::MAX, "z")] {
            lockers.insert(key, LockerInfo::default());
        }
        let order: Vec<(String, &str)> = lockers
            .keys()
            .map(|k| (k.locker.to_string(), k.cookie.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                (s("client.?"), "z"),
                (s("client.1"), "a"),
                (s("client.1"), "b")
            ]
        );
    }

    #[test]
    fn lock_info_and_get_info_reply_share_the_wire() {
        let reply = get_info_reply_1(false);
        let info = LockInfo::from(reply.clone());
        assert_eq!(bytes(&info), bytes(&reply));
        assert_eq!(GetInfoReply::from(info), reply);
    }

    #[test]
    fn list_locks_reply_matches_the_oracle() {
        pin(
            &ListLocksReply {
                locks: vec![s("lock1"), s("lock2"), s("lock3")],
            },
            "01011f00000003000000050000006c6f636b31050000006c6f636b32050000006c6f636b33",
            r#"{"locks":[["lock1"],["lock2"],["lock3"]]}"#,
        );
        pin(
            &ListLocksReply::default(),
            "01010400000000000000",
            r#"{"locks":[]}"#,
        );
    }

    #[test]
    fn assert_op_matches_the_oracle() {
        pin(
            &AssertOp {
                name: s("name"),
                lock_type: LockType::Shared,
                cookie: s("cookie"),
                tag: s("tag"),
            },
            "01011a000000040000006e616d650206000000636f6f6b696503000000746167",
            r#"{"name":"name","type":"shared","cookie":"cookie","tag":"tag"}"#,
        );
        pin(
            &AssertOp::default(),
            "01010d00000000000000000000000000000000",
            r#"{"name":"","type":"none","cookie":"","tag":""}"#,
        );
    }

    #[test]
    fn set_cookie_op_matches_the_oracle() {
        pin(
            &SetCookieOp {
                name: s("name"),
                lock_type: LockType::Shared,
                cookie: s("cookie"),
                tag: s("tag"),
                new_cookie: s("new cookie"),
            },
            "010128000000040000006e616d650206000000636f6f6b6965030000007461670a0000006e657720636f6f6b6965",
            r#"{"name":"name","type":"shared","cookie":"cookie","tag":"tag","new_cookie":"new cookie"}"#,
        );
        pin(
            &SetCookieOp::default(),
            "0101110000000000000000000000000000000000000000",
            r#"{"name":"","type":"none","cookie":"","tag":"","new_cookie":""}"#,
        );
    }

    #[test]
    fn lock_type_strings() {
        assert_eq!(lock_type_str(LockType::None), "none");
        assert_eq!(lock_type_str(LockType::Exclusive), "exclusive");
        assert_eq!(lock_type_str(LockType::Shared), "shared");
        assert_eq!(
            lock_type_str(LockType::ExclusiveEphemeral),
            "exclusive-ephemeral"
        );
    }
}
