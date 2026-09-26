# rados-rs RGW MVP, plan 13 of N: `cls-lock`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `lock` object class to `rados-cls`: the seven methods of
`cls_lock_client.h` (lock, unlock, break, get info, list locks, assert
locked, set cookie), all eleven structs (fourteen `ceph-dencoder` names)
pinned against `ceph-dencoder` v19.2.2 and the corpus, and cluster tests
that pin the class's semantics on Ceph v19.2.2. The canon for which locks
matter is rgw-go's `docs/exclusions.md` ("Lock names and cookies are
radosgw's"): a Rust RGW that coexists with radosgw takes the GC,
lifecycle, reshard and notification-queue locks by radosgw's names with
radosgw's discipline. The research found that list slightly wrong
(`bucket_instance_lock` is never taken, the queue registry has no lock,
and the multipart-completion lock on the S3 path is missing); the module
documents what the code does.

**Architecture:** One module `lock` behind a new feature `lock`, following
the `user`/`rgw::gc` pattern: structs encoded as Ceph does, `OSDOp`
constructors for compound use (RGW batches `assert_exists`, `lock` and
`assert_locked` with other ops), async free functions over `IoCtx`. The
`rados` crate first gets four small fixes the structs depend on:
`EntityAddr`'s `AF_UNSPEC` length, ordering and display for
`PackedEntityName` (C++ `entity_name_t`), `EntityAddr::legacy_str`, and
derives plus `Denc` on the existing `LockType`/`LockFlags` so the module
reuses them.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md` (in the clone, the copy at `.superpowers/plan-copies/2026-09-24-rados-rs-rgw-mvp-design.md`),
"`cls-lock`: lock, unlock, break, get info, set cookie, assert locked,
with radosgw's lock names, cookies and durations documented per worker".
Placement as the spec's crate-layout section states: the seven-method
mirror is the `cls-lock` package of `rados-cls`; the core crate's existing
`IoCtx::lock_exclusive`/`lock_shared`/`unlock` (upstream's API) stay
untouched, and routing them through the module is a follow-up. Research:
`.superpowers/research/cls-lock.md` (cited below as §n).

## Global Constraints

Plans 3 to 11's Global Constraints apply unchanged (Squid floor, derives
with `#[denc(crate = "rados")]`, no `unwrap`/`expect` on production paths,
per-commit gates, container fmt/clippy by the controller, push after every
gate, offline builds with the scratchpad `CARGO_HOME`, no new
dependencies). Plus:

- Branch `cls-lock` is based on the fork's `main` after plan 11
  (`cls-rgw-olh`) merges. Execution order of the remaining plans: 11, 13,
  14 (`cls-2pc-queue`), 15 (`cls-otp`), then 12 (`watch-notify`).
- Commits: `osdclient: ...` subjects for Task 1's `rados` fixes, `lock: ...`
  for the rest; bodies say what and why; trailers, in this order:
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s`.
  One commit per Task 1 fix.
- Wire and server semantics are identical on v19.2.2 and `main`
  7ed73efc1be (§0.1, §7): `git diff v19.2.2 7ed73efc1be -- src/cls/lock/`
  changes registration style and test-generator signatures only. Every
  struct is `ENCODE_START(1, 1)`, decoded with
  `DECODE_START_LEGACY_COMPAT_LEN(1, 1, 1)`.
- Class `lock`, `CLS_VER(1,0)`. Methods and flags (§1, identical on main):

  | Method | Flags | Request | Reply |
  |---|---|---|---|
  | `lock` | RD\|WR\|PROMOTE | `cls_lock_lock_op` | none |
  | `unlock` | RD\|WR\|PROMOTE | `cls_lock_unlock_op` | none |
  | `break_lock` | RD\|WR (no PROMOTE) | `cls_lock_break_op` | none |
  | `get_info` | RD | `cls_lock_get_info_op` | `cls_lock_get_info_reply` |
  | `list_locks` | RD | ignored; the C++ client sends an empty buffer | `cls_lock_list_locks_reply` |
  | `assert_locked` | RD\|PROMOTE | `cls_lock_assert_op` | none |
  | `set_cookie` | RD\|WR\|PROMOTE | `cls_lock_set_cookie_op` | none |

  No writing method produces output and the two replying methods are RD
  only, so **no method needs the return-vector flag**
  (`call::exec_returnvec` is not used; §6).
- Oracle: `ceph-dencoder` from `quay.io/ceph/ceph:v19.2.2`
  (`/tmp/claude/ceph-dencoder`); raw captures and the script that made
  them are in `/tmp/claude/cls-lock-oracle/` (`run.sh`, `<type>.<N>.bin`,
  `ea.<N>.bin` (N from 4 to 8, so `ea.1`-`ea.3` stay; `run.sh` writes no `ea.*` file) for `entity_addr_t`). **`select_test` counts from 1** and
  `0` wraps to the last instance: `select_test 1` is the populated
  instance, `select_test 2` the default-constructed one (§2.12). Every
  pin below says which.
- `ClsLockType` is a `u8` on the wire: `NONE=0`, `EXCLUSIVE=1`,
  `SHARED=2`, `EXCLUSIVE_EPHEMERAL=3`; string form (`cls_lock_type_str`)
  `none`, `exclusive`, `shared`, `exclusive-ephemeral`. Flags:
  `LOCK_FLAG_MAY_RENEW=0x1`, `LOCK_FLAG_MUST_RENEW=0x2`. C++ decoders cast
  any byte to the enum; the Rust decoder rejects an unknown type byte with
  `RadosError::InvalidData`. Only a request the server would refuse with
  `EINVAL` can carry one; `lock` validates the type before storing it, so
  a stored xattr never does. Documented on `LockType`'s `Denc` impl.
- `entity_name_t` is `rados::osdclient::types::PackedEntityName` (`u8`
  type, then a little-endian 64-bit number the C++ reads as `int64_t`);
  `rados::EntityName` is the unrelated auth-layer name (§0.4).
- `entity_addr_t` inside `locker_info_t` is encoded with the connection's
  features in C++ (`TYPE_FEATUREFUL`); rados-rs always writes the
  marker-1 (`MSG_ADDR2`) form, which is what the oracle writes with
  `CEPH_FEATURES_SUPPORTED_DEFAULT`, and decodes both markers (§2.2). The
  OSD encodes `get_info`'s reply and the stored xattr with the
  *requesting* connection's features (§2.14).
- Squid floor: every struct is version 1, and every struct's declaration
  order is its wire order, so all eleven derive `VersionedDenc` (version
  1, compat 1); the ones whose dump differs from the wire get a
  hand-written `Serialize`. A hand-written `decode_content`, should one
  prove necessary, calls
  `rados::check_min_version!(struct_v, 1, "<Type>", "Bobtail v0.56+")`:
  `cls_lock` landed in `2f8de8943ef` ("cls_lock: objclass for advisory
  locking", 2012-07-20, `git describe --contains` = `v0.51~76^2~6`), and
  its first stable release is Bobtail.
- Dump conventions (§2.0): a `utime_t` streamed into a dump prints as
  `utime_t::localtime`: below ten years of seconds (`sec < 315360000`)
  `<sec>.<usec:06>`; otherwise `YYYY-MM-DDTHH:MM:SS.<usec:06>` plus `%z`,
  which is `+0000` where `ceph-dencoder` runs. That is neither
  `dump::gmtime` (ends `Z`) nor `dump::real_time` (always calendar form).
  An `entity_name_t` prints `<type>.<num>`, or `<type>.?` when the number
  is negative as `int64_t`; type 0 is `unknown`. An address prints as
  `get_legacy_str()` = `<sockaddr>/<nonce>`.
- Cluster: local Ceph v19.2.2, `CEPH_CONF=/tmp/ceph/ceph.conf`, pool
  `test-pool` (`common::test_pool_name()`).

## Review Focus

1. `EntityAddr` encodes an address whose family is neither `AF_INET` nor
   `AF_INET6` with `elen` 28 (`sizeof(u)`) as C++ does, not 0 (family 0)
   or 128 (anything else). Outside `cls_lock` only unit tests encode an
   unset address; every handshake address comes from a TCP socket or the
   OSDMap, so no production frame changes. Pinned in Task 1(a) against
   `entity_addr_t` oracle instances 1-2 and in Task 2 by `locker_info_t`
   instance 2.
2. Ordering: `PackedEntityName`'s `Ord` compares type, then the number as
   `i64`; `LockerId`'s compares locker, then cookie bytes; so
   `LockInfo.lockers` (a `BTreeMap`) encodes a caller-built map in C++'s
   order whatever the insertion order. Pinned by `cls_lock_get_info_reply`
   instance 1 (client.1 before client.2), a reverse-insertion test, and a
   negative-number test.
3. Dumps that differ from the wire: `lock_info_t` prints `lock_type` as an
   int and nests `{"id", "info"}`; `cls_lock_get_info_reply` has the same
   wire but prints the type as a string and flattens each locker into
   `{locker, description, cookie, expiration, addr}`; `cls_lock_break_op`
   prints `cookie` before `locker`; `cls_lock_list_locks_reply` wraps each
   name in its own array; timestamps go through `dump::localtime`. Pinned
   in Task 2 against the oracle JSON and one real corpus object.
4. Server semantics on the cluster (Task 4): a holder is `(client.<gid>,
   cookie)`, so a second client can neither unlock nor renew the first's
   lock; renewal needs a flag and `MUST_RENEW` without a lock is
   `ENOENT`; unlock leaves the xattr with its type and tag; unlocking an
   ephemeral lock deletes the object; an RD method that finds an expired
   ephemeral lock is `EIO` (derived from code, not yet observed: Task 4
   reports BLOCKED if the class behaves otherwise).

---

### Task 0: Branch and workspace (controller)

As plan 3's Task 0: branch `cls-lock` off the fork's `main` after plan 11
merges; the SDD ledger; the cluster up (three containers).

---

### Task 1: `rados`: address length, entity-name order and display, legacy address string, lock enums

**Files:**
- Modify: `rados/src/denc/entity_addr.rs` (a, c), `rados/src/osdclient/types.rs`
  (b), `rados/src/osdclient/watchers.rs` (b), `rados/src/osdclient/lock.rs` (d).

Four commits, each with its unit tests; `cargo test -p rados --lib
--offline` green after each. `IoCtx::lock_exclusive`, `lock_shared`,
`unlock`, `OSDOp::lock_*`/`unlock`, `LockRequest` and `UnlockRequest`
are not touched.

**(a) `EntityAddr::sockaddr_len` as C++ `get_sockaddr_len`.**

- `AF_INET` → 16, `AF_INET6` → 28, any other family (including 0,
  `AF_UNSPEC`) → 28, C++'s `sizeof(u)` (§0.5; `S:src/msg/msg_types.h:311-319`).
  Today family 0 gives 0 and other families 128 (`STORAGE_SIZE`). The doc
  comment says it mirrors `entity_addr_t::get_sockaddr_len`.
  `encoded_size` follows.
- Pins (from `/tmp/claude/cls-lock-oracle/ea.<N>.bin` (N from 4 to 8, so `ea.1`-`ea.3` stay; `run.sh` writes no `ea.*` file)``, 47 bytes each):
  `entity_addr_t` `select_test 1` = `EntityAddr { addr_type: None, nonce:
  0, sockaddr_data: zeros }` =
  `0101012800000000000000000000001c000000` + 56 hex zeros;
  `select_test 2` = the same with `nonce: 1` =
  `0101012800000000000000010000001c000000` + 56 hex zeros (content: type
  `00000000`, nonce `01000000`, elen `1c000000`, 28 zero bytes). Each
  encodes to exactly those bytes and decodes back equal. Regression pin
  (already passing): `ea.3.bin`, a `Legacy` address 127.0.1.2:2 nonce 5,
  = `0101011c000000010000000500000010000000020000027f0001020000000000000000`.
- `test_entity_addr_encoded_size`'s unset case becomes 47 (was 19).
- Run the whole `rados` unit suite. The msgr2 frame tests that encode
  `EntityAddr::default()` assert lengths with `>=` and keep passing; any
  other failing pin of an unset address is updated to the 28-byte form
  only if it was produced by rados-rs's own encoder. If a failing pin was
  captured from C++ and disagrees, stop and report BLOCKED.
- Corpus: `entity_addr_t` is already a `CORPUS_TYPES` row. The harness
  (`test_type` in `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`)
  compares JSON only (decode, both roundtrips, both cross-decodes), never
  bytes, so this fix changes no corpus outcome and the harness will not
  start comparing `entity_addr_t` bytes; byte parity is guarded by these
  unit pins and Task 2's `locker_info_t` instance 2.

```
osdclient: encode an unspecified address with C++'s sockaddr length

entity_addr_t::get_sockaddr_len returns sizeof(sin) for AF_INET,
sizeof(sin6) for AF_INET6 and sizeof(u), 28, for anything else. The
encoder wrote 0 bytes for AF_UNSPEC and 128 for other families, so an
unset address, as in ceph-dencoder's entity_addr_t and locker_info_t
instances, did not match C++ byte for byte. Decoding was unaffected.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

**(b) `PackedEntityName`: `Default`, `PartialOrd`/`Ord`, `Hash`, `Display`.**

- `Default` = `new(0, 0)` (written by hand; prints `unknown.0`).
- `Ord`/`PartialOrd` by hand, as C++ `operator<` for `entity_name_t`
  (`S:src/msg/msg_types.h:108-109`): `entity_type`, then `num.get() as
  i64`. `Hash` consistent with the derived `PartialEq`.
- `Display` as C++ `operator<<` (`:112-117`): the type name from
  `crate::EntityType::from_bits_truncate(u32::from(entity_type))`'s
  `Display` (`mon`, `mds`, `osd`, `client`, `mgr`, `auth`, else
  `unknown`, which is `ceph_entity_type_name`), then `.<num as i64>`, or
  `.?` when `num as i64 < 0` (this also covers `NEW = -1`).
- `WatchItem::watcher_name` becomes `self.name.to_string()`; its existing
  tests (`client.4242`, `client.?` for `u64::MAX`) keep passing.
- Unit tests: `new(0x08, 1)` → `client.1`; `default()` → `unknown.0`;
  `new(0x04, 3)` → `osd.3`; `new(0x08, u64::MAX)` → `client.?`;
  `new(0x03, 1)` → `unknown.1`; `new(0x08, 1) < new(0x08, 2)`;
  `new(0x01, 9) < new(0x08, 0)` (type first); `new(0x08, u64::MAX) <
  new(0x08, 0)` (signed: -1 < 0).

```
osdclient: order and print entity names as entity_name_t does

cls_lock keys its lockers by entity_name_t and cookie, and dumps the
name as client.<num>. PackedEntityName gains the C++ ordering, type then
the number as a signed 64-bit value, and the C++ printing, with
<type>.? for a negative number, so a lock map built in Rust encodes in
the OSD's order. WatchItem's formatter now uses it.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

**(c) `EntityAddr::legacy_str(&self) -> String`, as `get_legacy_str`.**

- `<sockaddr> "/" <nonce>` (`S:src/msg/msg_types.h:435-439`), where
  `<sockaddr>` follows `operator<<(ostream&, const sockaddr*)`
  (`S:src/msg/msg_types.cc:247-269`) on the family read little-endian from
  `sockaddr_data[0..2]`: `AF_INET` → `a.b.c.d:<port>` (port big-endian at
  `[2..4]`, always printed, `0` included); `AF_INET6` →
  `[<inet_ntop>]:<port>`, where `inet_ntop` is `std::net::Ipv6Addr`'s
  compressed `Display` except glibc's IPv4-compatible form (first 96 bits
  zero and bits 96-111 non-zero print as `::a.b.c.d`); any other family →
  `(unrecognized address family <N>)` with the real number. The address
  type (`v1`/`v2`/...) is not printed. The private `format_addr` (the
  `entity_addr_t` dump) is left alone.
- Observe the IPv6 and family-1 strings first: write these marker-1
  `entity_addr_t` encodings (Legacy, `sockaddr_in6` = family 10 LE, port
  BE, flowinfo, 16 address bytes, scope id) to
  `/tmp/claude/cls-lock-oracle/ea.<N>.bin` (N from 4 to 8, so `ea.1`-`ea.3` stay; `run.sh` writes no `ea.*` file)`` and run `ceph-dencoder type
  entity_addr_t import ea.<N>.bin` (N from 4 to 8, so `ea.1`-`ea.3` stay; `run.sh` writes no `ea.*` file)` decode dump_json` (unsandboxed); the
  `addr` field is the pin's `<sockaddr>` part, with `/<nonce>` appended:
  - `::1.2.3.4` port 1 nonce 0:
    `0101012800000001000000000000001c0000000a000001000000000000000000000000000000000102030400000000`
  - `::ffff:10.0.0.1` port 3 nonce 0:
    `0101012800000001000000000000001c0000000a0000030000000000000000000000000000ffff0a00000100000000`
  - `::1` port 6789 nonce 7:
    `0101012800000001000000070000001c0000000a001a85000000000000000000000000000000000000000100000000`
  - `2001:db8::1` port 6800 nonce 42:
    `01010128000000010000002a0000001c0000000a001a900000000020010db800000000000000000000000100000000`
  - family 1, zeros:
    `0101012800000001000000000000001c00000001000000000000000000000000000000000000000000000000000000`

  If any observed string differs from the pin below, the pin follows the
  oracle.
- Pins: `127.0.1.2:2` Legacy nonce 1 → `127.0.1.2:2/1`; the default
  address → `(unrecognized address family 0)/0`; `127.0.1.2:20` nonce 10
  → `127.0.1.2:20/10`; `172.21.5.153:0` nonce 1725310796 →
  `172.21.5.153:0/1725310796` (the corpus object in Task 2); `[::1]:6789`
  nonce 7 → `[::1]:6789/7`; `[2001:db8::1]:6800` nonce 42 →
  `[2001:db8::1]:6800/42`; `::1.2.3.4` port 1 → `[::1.2.3.4]:1/0`;
  `::ffff:10.0.0.1` port 3 → `[::ffff:10.0.0.1]:3/0`; family 1 →
  `(unrecognized address family 1)/0`.

```
osdclient: an address's legacy string as get_legacy_str prints it

cls_lock dumps a locker's address with entity_addr_t::get_legacy_str:
ip:port or [ipv6]:port, then /nonce, with no type prefix, compressed
IPv6 and the real family number for anything else. Display prefixes
v1:/v2: and omits the nonce, and the dump formatter prints IPv6 in full,
so neither can serve.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

**(d) `LockType` and `LockFlags` usable in encoded structs.**

`rados::osdclient::lock::{LockType, LockFlags}` are public (re-exported
as `rados::{LockType, LockFlags}`) with the C++ values, so the module
reuses them, but they cannot sit in a derived struct today: `LockFlags`
(bitflags 2) has no derives, `LockType` has no `Default`, and neither is
`Denc`, which `rados-cls` cannot add (orphan rule). Add:

- `LockType`: `Default` (`None`), `Hash`, `TryFrom<u8>` (0-3; anything
  else `RadosError::InvalidData`), `Denc` as one byte (`encoded_size`
  `Some(1)`), with the doc note from Global Constraints on unknown bytes.
- `LockFlags`: `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
  Default)]` inside the `bitflags!` invocation; `Denc` as one byte,
  decoding with `from_bits_retain` (C++ keeps any byte).
- Unit tests: each type round-trips as its byte; `LockType::try_from(4)`
  fails; `LockFlags::from_bits_retain(0x04)` round-trips as `04`;
  defaults are `None` and empty.

```
osdclient: lock type and flags as encodable values

cls_lock's request and state structs carry the lock type and flags as
single bytes. LockType and LockFlags already hold the C++ values but
had no Denc, no Default and, for the flags, no derives at all, so no
struct could hold them. The flags decode keeps unknown bits as the C++
does; an unknown type byte is refused, which only a request the class
would reject with EINVAL can carry.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 2: The `lock` class types

**Files:**
- Create: `rados-cls/src/lock.rs` (short module doc, types, tests; Task 3
  adds the client and the full doc).
- Modify: `rados-cls/Cargo.toml` (`lock = []`, added to `default`),
  `rados-cls/src/lib.rs` (`#[cfg(feature = "lock")] pub mod lock;`, and
  `feature = "lock"` in the `dump` gate), `rados-cls/src/dump.rs`,
  `.github/workflows/ci.yml` (`lock` in the "Run clippy on each rados-cls
  class alone" loop: `for features in "" lock queue refcount rgw rgw_gc
  user version`), `README.md` (the `rados-cls` row lists every class
  feature: `version`, `refcount`, `user`, `queue`, `rgw`, `rgw_gc`,
  `lock`), `rados-dencoder/src/main.rs` (`use rados_cls::lock::{...}` with
  `Lock*` aliases, fourteen arms, one `list_types` line),
  `rados-dencoder/tests/dencoder_corpus_comparison_test.rs` (fourteen
  `TypeSpec::new(name, None, false)` rows).

**Interfaces:**
- `dump.rs`: `pub(crate) fn localtime(t: &UTime) -> String`, mirroring
  `utime_t::localtime` (`S:src/include/utime.h:334-368`) with
  `legacy_form` false in a UTC process: `sec < 315_360_000` →
  `format!("{}.{:06}", sec, nsec / 1000)`, else `iso_utc(t)`; and the
  serde adapter `pub(crate) fn utime_localtime<S: Serializer>(t: &UTime,
  s: S)`. Gates: `use rados::UTime` and `civil_from_days` gain
  `feature = "lock"`, `iso_utc` becomes `any(feature = "rgw", feature =
  "lock")`, the new pair is `feature = "lock"`. Tests (`#[cfg(feature =
  "lock")]`): `{5, 0}` → `5.000000`; `{0, 0}` → `0.000000`;
  `{315_359_999, 999_999_999}` → `315359999.999999`; `{315_360_000, 0}` →
  `1979-12-30T00:00:00.000000+0000`; `{1_727_604_086, 460_555_954}` →
  `2024-09-29T10:01:26.460555+0000`.
- `lock.rs` re-exports `pub use rados::osdclient::types::PackedEntityName;
  pub use rados::{EntityAddr, LockFlags, LockType};` and defines
  `pub const CLASS: &str = "lock";`, `pub fn lock_type_str(t: LockType) ->
  &'static str` (`cls_lock_type_str`), and private serde adapters
  `type_as_str`, `type_as_int`, `flags_as_int` (`bits()`), `entity_name`
  (`Display`), `legacy_addr` (`legacy_str`).
- Structs, fields in wire order (all `#[derive(Debug, Clone, Default,
  PartialEq, Eq, VersionedDenc)] #[denc(crate = "rados", version = 1,
  compat = 1)]`; strings are `String`; "dump" is the JSON the oracle
  prints):

  | Rust | C++ (dencoder names) | Fields | Dump |
  |---|---|---|---|
  | `LockerId` (+ `PartialOrd, Ord, Hash`, derived: field order is the C++ key order) | `locker_id_t`, `rados::cls::lock::locker_id_t` | `locker: PackedEntityName`, `cookie` | derived `Serialize`: `locker` (`entity_name`), `cookie` |
  | `LockerInfo` | `locker_info_t`, `rados::cls::lock::locker_info_t` | `expiration: UTime`, `addr: EntityAddr`, `description` | derived: `expiration` (`utime_localtime`), `addr` (`legacy_addr`), `description` |
  | `LockInfo` | `lock_info_t`, `rados::cls::lock::lock_info_t` | `lockers: BTreeMap<LockerId, LockerInfo>`, `lock_type: LockType`, `tag` | hand-written: `lock_type` (int), `tag`, `lockers` = `[{"id": LockerId, "info": LockerInfo}]` |
  | `LockOp` | `cls_lock_lock_op` | `name`, `lock_type: LockType`, `cookie`, `tag`, `description`, `duration: UTime`, `flags: LockFlags` | derived with renames: `name`, `type` (string), `cookie`, `tag`, `description`, `duration` (`utime_localtime`), `flags` (int) |
  | `UnlockOp` | `cls_lock_unlock_op` | `name`, `cookie` | derived |
  | `BreakOp` | `cls_lock_break_op` | `name`, `locker: PackedEntityName`, `cookie` | hand-written: `name`, `cookie`, `locker` |
  | `GetInfoOp` | `cls_lock_get_info_op` | `name` | derived |
  | `GetInfoReply` | `cls_lock_get_info_reply` | same as `LockInfo` | hand-written: `lock_type` (string), `tag`, `lockers` = `[{"locker", "description", "cookie", "expiration", "addr"}]` |
  | `ListLocksReply` | `cls_lock_list_locks_reply` | `locks: Vec<String>` | hand-written: `{"locks": [["a"], ["b"]]}` |
  | `AssertOp` | `cls_lock_assert_op` | `name`, `lock_type: LockType`, `cookie`, `tag` | derived: `name`, `type` (string), `cookie`, `tag` |
  | `SetCookieOp` | `cls_lock_set_cookie_op` | `name`, `lock_type: LockType`, `cookie`, `tag`, `new_cookie` | derived: `name`, `type` (string), `cookie`, `tag`, `new_cookie` |

  `LockOp.duration` stays a raw `UTime` (RGW's multisite path puts
  milliseconds into `nsec`; §8). `LockInfo` is the value of the object's
  `lock.<name>` xattr (§3.1). `impl From<GetInfoReply> for LockInfo` and
  the reverse (same fields).

Unit tests (`bytes`/`json`/`unhex` helpers as plan 6; every pin: encode
the built value to the hex, decode the hex back to an equal value, and
`serde_json::to_string` equals the JSON exactly, key order included):

- `LockerId` 1 `{client.1, "cookie"}` =
  `01011300000008010000000000000006000000636f6f6b6965`,
  `{"locker":"client.1","cookie":"cookie"}`; 2 (default) =
  `01010d00000000000000000000000000000000`,
  `{"locker":"unknown.0","cookie":""}`.
- `LockerInfo` 1 `{expiration {5, 0}, addr Legacy 127.0.1.2:2 nonce 1,
  "description"}` =
  `01013a00000005000000000000000101011c000000010000000100000010000000020000027f00010200000000000000000b0000006465736372697074696f6e`,
  `{"expiration":"5.000000","addr":"127.0.1.2:2/1","description":"description"}`;
  2 (default; needs Task 1(a)) =
  `01013b00000000000000000000000101012800000000000000000000001c0000000000000000000000000000000000000000000000000000000000000000000000`,
  `{"expiration":"0.000000","addr":"(unrecognized address family 0)/0","description":""}`.
- `LockInfo` 1 `{ {LockerId 1: LockerInfo 1}, EXCLUSIVE, "tag" }` =
  `0101650000000100000001011300000008010000000000000006000000636f6f6b696501013a00000005000000000000000101011c000000010000000100000010000000020000027f00010200000000000000000b0000006465736372697074696f6e0103000000746167`,
  `{"lock_type":1,"tag":"tag","lockers":[{"id":{"locker":"client.1","cookie":"cookie"},"info":{"expiration":"5.000000","addr":"127.0.1.2:2/1","description":"description"}}]}`;
  2 = `010109000000000000000000000000`, `{"lock_type":0,"tag":"","lockers":[]}`.
- `LockInfo` corpus object
  `ceph-object-corpus/archive/19.2.0-404-g78ddc7f9027/objects/lock_info_t/f4cf81c9b553c2fb2c66e15ebd54a829`
  (`{ {client.4533 "j1_EFM8DDt8SI1R": {expiration {1727604086, 460555954},
  Legacy 172.21.5.153:0 nonce 1725310796, ""}}, EXCLUSIVE, "" }`) =
  `0101600000000100000001011c00000008b5110000000000000f0000006a315f45464d38444474385349315201012f0000007625f966b286731b0101011c000000010000004c27d6661000000002000000ac1505990000000000000000000000000100000000`,
  `{"lock_type":1,"tag":"","lockers":[{"id":{"locker":"client.4533","cookie":"j1_EFM8DDt8SI1R"},"info":{"expiration":"2024-09-29T10:01:26.460555+0000","addr":"172.21.5.153:0/1725310796","description":""}}]}`.
- `LockOp` 1 `{"name", SHARED, "cookie", "tag", "description", {5, 0},
  MAY_RENEW}` =
  `010132000000040000006e616d650206000000636f6f6b6965030000007461670b0000006465736372697074696f6e050000000000000001`,
  `{"name":"name","type":"shared","cookie":"cookie","tag":"tag","description":"description","duration":"5.000000","flags":1}`;
  2 = `01011a0000000000000000000000000000000000000000000000000000000000`,
  `{"name":"","type":"none","cookie":"","tag":"","description":"","duration":"0.000000","flags":0}`.
- `UnlockOp` 1 `{"name", "cookie"}` =
  `010112000000040000006e616d6506000000636f6f6b6965`,
  `{"name":"name","cookie":"cookie"}`; 2 = `0101080000000000000000000000`,
  `{"name":"","cookie":""}`.
- `BreakOp` 1 `{"name", client.1, "cookie"}` =
  `01011b000000040000006e616d6508010000000000000006000000636f6f6b6965`,
  `{"name":"name","cookie":"cookie","locker":"client.1"}`; 2 =
  `0101110000000000000000000000000000000000000000`,
  `{"name":"","cookie":"","locker":"unknown.0"}`.
- `GetInfoOp` 1 `{"name"}` = `010108000000040000006e616d65`,
  `{"name":"name"}`; 2 = `01010400000000000000`, `{"name":""}`.
- `GetInfoReply` 1 `{ {client.1 "cookie1": {{10, 0}, Legacy 127.0.1.2:20
  nonce 10, "description1"}, client.2 "cookie2": {{20, 0}, Legacy
  127.0.1.2:40 nonce 30, "description2"}}, SHARED, "tag" }` =
  `0101c20000000200000001011400000008010000000000000007000000636f6f6b69653101013b0000000a000000000000000101011c000000010000000a00000010000000020000147f00010200000000000000000c0000006465736372697074696f6e3101011400000008020000000000000007000000636f6f6b69653201013b00000014000000000000000101011c000000010000001e00000010000000020000287f00010200000000000000000c0000006465736372697074696f6e320203000000746167`,
  `{"lock_type":"shared","tag":"tag","lockers":[{"locker":"client.1","description":"description1","cookie":"cookie1","expiration":"10.000000","addr":"127.0.1.2:20/10"},{"locker":"client.2","description":"description2","cookie":"cookie2","expiration":"20.000000","addr":"127.0.1.2:40/30"}]}`;
  2 = `010109000000000000000000000000`,
  `{"lock_type":"none","tag":"","lockers":[]}`. Plus: the same map built
  by inserting client.2 first encodes to the same bytes; a map holding
  `client.1 "b"`, `client.1 "a"` and `client.?` (`u64::MAX`) iterates as
  `client.?`, `client.1 "a"`, `client.1 "b"`.
- `ListLocksReply` 1 `{["lock1", "lock2", "lock3"]}` =
  `01011f00000003000000050000006c6f636b31050000006c6f636b32050000006c6f636b33`,
  `{"locks":[["lock1"],["lock2"],["lock3"]]}`; 2 = `01010400000000000000`,
  `{"locks":[]}`.
- `AssertOp` 1 `{"name", SHARED, "cookie", "tag"}` =
  `01011a000000040000006e616d650206000000636f6f6b696503000000746167`,
  `{"name":"name","type":"shared","cookie":"cookie","tag":"tag"}`; 2 =
  `01010d00000000000000000000000000000000`,
  `{"name":"","type":"none","cookie":"","tag":""}`.
- `SetCookieOp` 1 `{"name", SHARED, "cookie", "tag", "new cookie"}` =
  `010128000000040000006e616d650206000000636f6f6b6965030000007461670a0000006e657720636f6f6b6965`,
  `{"name":"name","type":"shared","cookie":"cookie","tag":"tag","new_cookie":"new cookie"}`;
  2 = `0101110000000000000000000000000000000000000000`,
  `{"name":"","type":"none","cookie":"","tag":"","new_cookie":""}`.
- `lock_type_str` for all four types; decoding `LockOp` 1 with the type
  byte patched to `04` fails.

Dencoder arms (`type_info_denc::<T>()`), fourteen names:
`locker_id_t` and `rados::cls::lock::locker_id_t` → `LockerId`;
`locker_info_t` and `rados::cls::lock::locker_info_t` → `LockerInfo`;
`lock_info_t` and `rados::cls::lock::lock_info_t` → `LockInfo`;
`cls_lock_lock_op`, `cls_lock_unlock_op`, `cls_lock_break_op`,
`cls_lock_get_info_op`, `cls_lock_get_info_reply`,
`cls_lock_list_locks_reply`, `cls_lock_assert_op`,
`cls_lock_set_cookie_op` → their types. The corpus table gets the same
fourteen names; all have directories in both archives (10 samples each
for the three bare type names, `lock_op`, `unlock_op`, `get_info_op` and
`get_info_reply`, 2 each for the
`rados::cls::lock::` names, `break_op` 7, `list_locks_reply` 4,
`assert_op` 6, `set_cookie_op` 6; §2.13). The harness's `features: None`
suffices: the Rust encoders ignore features.

Run: `cargo test -p rados-cls --lib --offline lock dump && cargo build -p
rados-dencoder --offline && cargo test -p rados-dencoder --offline --test
dencoder_corpus_comparison_test --no-run && cargo test --workspace --lib
--offline`.

```
lock: the lock class types

All eleven cls_lock structs, version 1 each, as ceph-dencoder v19.2.2
encodes and dumps them. lock_info_t, the lock's xattr value, dumps its
type as an int and nests each locker; get_info_reply has the same wire
but dumps the type as a string and flattens the lockers; break_op dumps
its cookie before the locker; list_locks_reply wraps each name in an
array. Timestamps print as utime_t::localtime does, with +0000 where
dencoder runs. The locker map orders as locker_id_t does, so a map built
in Rust encodes as the OSD's.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 3: The `lock` class client

**Files:**
- Modify: `rados-cls/src/lock.rs` (client, full module doc, tests),
  `rados-cls/src/lib.rs` (`feature = "lock"` joins the `mod call` gate in
  this commit, the class's first call).

**Interfaces** (`Result` is `rados::osdclient::error::Result`; `oid:
&str`; every constructor uses `call::op`, `list_locks_op` uses
`call::raw_op(CLASS, "list_locks", Bytes::new())`):

```rust
pub fn xattr_name(lock_name: &str) -> String; // "lock." + lock_name
pub fn lock_op(op: &LockOp) -> Result<OSDOp>;
pub fn unlock_op(name: &str, cookie: &str) -> Result<OSDOp>;
pub fn break_lock_op(name: &str, cookie: &str, locker: &PackedEntityName) -> Result<OSDOp>;
pub fn get_info_op(name: &str) -> Result<OSDOp>;
pub fn list_locks_op() -> Result<OSDOp>;
pub fn assert_locked_op(name: &str, lock_type: LockType, cookie: &str, tag: &str) -> Result<OSDOp>;
pub fn set_cookie_op(name: &str, lock_type: LockType, cookie: &str, tag: &str, new_cookie: &str) -> Result<OSDOp>;
pub fn decode_get_info(reply: &OpReply) -> Result<GetInfoReply>;
pub fn decode_list_locks(reply: &OpReply) -> Result<Vec<String>>;

pub async fn lock(ioctx: &IoCtx, oid: &str, op: &LockOp) -> Result<()>;
pub async fn unlock(ioctx: &IoCtx, oid: &str, name: &str, cookie: &str) -> Result<()>;
pub async fn break_lock(ioctx: &IoCtx, oid: &str, name: &str, cookie: &str, locker: &PackedEntityName) -> Result<()>;
pub async fn get_info(ioctx: &IoCtx, oid: &str, name: &str) -> Result<GetInfoReply>;
pub async fn list_locks(ioctx: &IoCtx, oid: &str) -> Result<Vec<String>>;
pub async fn assert_locked(ioctx: &IoCtx, oid: &str, name: &str, lock_type: LockType, cookie: &str, tag: &str) -> Result<()>;
pub async fn set_cookie(ioctx: &IoCtx, oid: &str, name: &str, lock_type: LockType, cookie: &str, tag: &str, new_cookie: &str) -> Result<()>;
```

`lock_op` sends what it is given, both renew flags included (the server
answers `EINVAL`; the C++ `Lock` never sends both). Each constructor's doc
names its errors; each async function's doc is "See [`x_op`]".
`xattr_name`'s doc: the xattr that holds the lock's state, for a reader
that fetches it among the object's other xattrs; decode it with
`LockInfo::decode` (`rados::Denc`); unlike `get_info`, the raw value is not
trimmed of expired lockers, whose `expiration` is on the OSD's clock.

Module doc (replaces Task 2's short one). Server facts
(`S:src/cls/lock/cls_lock.cc`, identical on main; §3):

- State is one xattr per lock, `lock.<name>`, holding `LockInfo`; no
  omap, no data. `lock` on a missing object creates it (RGW prepends
  `assert_exists` where that matters); every other method on a missing
  object is `ENOENT`.
- A holder is `(client.<global_id>, cookie)`: the entity of the RADOS
  client session, shared by every `IoCtx` of one client. The address is
  recorded (retyped `LEGACY`) and reported, never compared. So a new
  client instance (restart, reconnect under a new global id) cannot
  renew, unlock or assert its predecessor's lock (renew → `EBUSY`, or a
  second entry for a shared lock; unlock → `ENOENT`) and must wait for
  expiry or `break_lock`; and two workers in one client with one cookie
  are one holder: flags 0 → `EEXIST`, `MAY_RENEW` → a silent takeover.
  RGW's lifecycle code therefore treats `EBUSY || EEXIST` as busy (GC
  checks only `EBUSY`). Cookies need only be unique within one client;
  RGW's random ones are 15 characters (`COOKIE_LEN 16` less the NUL) or,
  for the notification manager, 16.
- `lock`: `EINVAL` for type `NONE` or unknown, an empty name, or both
  renew flags; then `EBUSY` if any unexpired holder exists under a
  different `tag` (checked before renewal, so even the holder cannot
  renew under another tag); the same holder again: flags 0 → `EEXIST`,
  either renew flag → renewed (expiration, address and description
  refreshed); `MUST_RENEW` without that holder → `ENOENT`; other holders
  remain: an exclusive request → `EBUSY`, a shared request over a
  different stored type → `EBUSY`; shared locks under one tag coexist.
  Expiration is the OSD's clock plus `duration`; a zero duration never
  expires. Expired holders are dropped on every read of the state.
- `unlock` (the caller's entity) and `break_lock` (the given `locker`;
  anyone with write access who knows name, entity and cookie; no
  PROMOTE flag) are `ENOENT` for an absent or expired holder; after the
  last holder leaves, the xattr stays with its type and tag, and
  `get_info` reports them with no holders. Unlocking or breaking an
  **ephemeral** lock deletes the object.
- `get_info` on an existing object without the lock succeeds with type
  `none`. `list_locks` lists every `lock.`-prefixed xattr and never drops
  expired or fully unlocked locks.
- `assert_locked` is `EBUSY` when there is no holder, the stored type is
  not exactly the requested one (`EXCLUSIVE` does not match
  `EXCLUSIVE_EPHEMERAL`), the tag differs, or the caller's `(entity,
  cookie)` is not a holder; it does not renew and composes into read and
  write ops (a failing assert fails the whole op). `set_cookie` makes the
  same checks, is `EBUSY` if `new_cookie` is already a holder of the
  caller, and moves the entry keeping expiration, address and
  description.
- An ephemeral lock whose last holder has expired is deleted with its
  object by the next method that reads it; in `get_info` or
  `assert_locked` (RD only) that delete is refused and the call fails with
  `EIO` (Task 4 test 11).

RGW's locks (Squid v19.2.2; `main` differences noted; §4):

| Worker | Lock name | Object (pool) | Cookie | Duration (option, default) | Type, flags | Renewal | On `EBUSY` |
|---|---|---|---|---|---|---|---|
| GC (`RGWGC::process`) | `gc_process` | `gc.<i>`, `i < rgw_gc_max_objs` (32) (GC pool) | empty | `rgw_gc_processor_max_time`, 1 h (≤ 0: `EAGAIN`, no lock) | exclusive, 0, empty tag | none; unlock at the end | skip the shard (only `EBUSY` is checked) |
| LC shard walk (`RGWLC::process`) | `lc_process` | `lc.<i>`, `i < rgw_lc_max_objs` (32) (LC pool) | `lc_thrd: <ix>` | `rgw_lc_lock_max_time`, 90 s | exclusive, 0 | none; dropped before `bucket_lc_process` | `EBUSY`/`EEXIST`: retry, backoff 5 × 50 ms |
| LC single bucket (`process_bucket`), `bucket_lc_post`, `guard_lc_modify` (S3 Put/Delete lifecycle) | `lc_process` | `lc.<i>` | `lc_thrd: <ix>`; `RGWLC::cookie` (15 random, per process); that or a fresh 15-char cookie | 90 s | exclusive, 0 | none | `EBUSY`/`EEXIST`: return `EBUSY`; sleep 5 s and retry forever; retry every 100 ms, 500 times |
| Reshard logshard (`RGWBucketReshardLock`) | `reshard_process` | `reshard.%010u` (reshard pool) | 15 random | `rgw_reshard_bucket_lock_duration`, 360 s (min 30) | exclusive, 0 | `MUST_RENEW` after `duration / 2`, per entry; `ENOENT` = expired | logged, returned |
| Reshard per bucket (same class) | `reshard_process` | `[tenant:]name[:bucket_id]` (reshard pool) | 15 random | 360 s | **exclusive-ephemeral**, 0 | as above, renewing the logshard lock first | logged, returned |
| Multipart completion (`RGWCompleteMultipart`, S3 path) | `RGWCompleteMultipart` | the upload's meta object (bucket data pool) | empty | `rgw_mp_lock_max_time`, 10 min | `assert_exists` + exclusive, 0 | none on Squid; `main` renews every `duration / 2` with `MUST_RENEW` | `ENOENT` with a completed upload = success; else "already in progress" |
| Notification queue (`rgw_notify.cc`) | `<queue>_lock` | the queue object `<queue>` (notification pool) | 16 random, per manager | 90 s (Squid; `main`: 3 × `rgw_topic_ownership_update_period`, 30 s) | `assert_exists` + exclusive, `MAY_RENEW` | re-lock every 30 s + 100-500 ms jitter | owned elsewhere, skip; `ENOENT`: queue deleted |

Queue ownership is checked with `assert_exists` + `assert_locked(EXCLUSIVE,
cookie, "")` batched with each 2pc-queue op (`EBUSY`: ownership moved,
stop); release is `assert_exists` + `unlock`, with `ENOENT` and `EBUSY`
treated as done. `bucket_instance_lock` is constructed and never taken;
the notification registry object (`queues_list_object`) has no lock. (The
Swift object expirer, excluded, also locks `gc_process`, on
`obj_delete_at_hint.%010u`.)

Unit tests: every constructor's `indata` starts with `lock` + the method
name and its `OpData::Call` has `class_len: 4` and the method's length
(`list_locks_op`'s `OpData::Call` has `indata_len: 0` and its `indata` is
exactly `b"locklist_locks"`, 14 bytes: no request bytes);
`decode_get_info` and `decode_list_locks` unwrap Task 2's instance-1
bytes; `xattr_name("gc_process") == "lock.gc_process"`.

```
lock: the lock class client

Mirrors cls_lock_client.h as op constructors and IoCtx functions for
all seven methods; none needs RETURNVEC. The module documents the
server's rules, a holder being the client's entity plus a cookie, and
the locks radosgw takes: name, object, cookie, duration, flags, renewal
and what it does on EBUSY, for GC, lifecycle, reshard, multipart
completion and the notification queues.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 4: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_lock.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both `rados-cls` loops
  gain `cls_lock` at the end (after `cls_rgw_lc` and plan 11's
  `cls_rgw_olh`)).

Helpers: `unique`, `is_osd_error` (as `cls_rgw_usage.rs`); errnos
`ENOENT 2`, `EIO 5`, `EBUSY 16`, `EEXIST 17`, `EINVAL 22`;
`secs(n) -> UTime`; `lock_req(name, type, cookie, tag, dur, flags) ->
LockOp` (description `"test"`); `async fn client() -> (rados::Client,
IoCtx, PackedEntityName)` = `common::build_test_client()`, `open_pool(&
common::test_pool_name())`, and `PackedEntityName::new(0x08,
client.mon_client().get_global_id().await)` (each call is a new client,
so a new global id); `holders(&GetInfoReply) -> Vec<(String, String)>` =
`(locker.to_string(), cookie)` in map order. Each test uses its own object
(`ioctx.create(&oid, true)` unless stated) and lock name, and removes the
object at the end when it still exists. All `#[tokio::test] #[ignore]`.

1. `lock_exclusive_get_info_list_locks`: `lock(EXCLUSIVE, "c1", 30 s)` ok;
   `get_info`: type `EXCLUSIVE`, tag `""`, `holders == [("client.<gid>",
   "c1")]`, description `"test"`, `addr.addr_type == Legacy`,
   `expiration.sec > 315_360_000`; `list_locks == [name]`. On a missing
   object: `lock` succeeds and `stat` shows it exists, size 0; on another
   missing object the compound `assert_exists` + `lock_op` fails `ENOENT`
   and the object still does not exist.
2. `second_cookie_is_busy`: A holds `EXCLUSIVE "c1"`; `EXCLUSIVE "c2"`
   (same client) → `EBUSY`; `SHARED "c2"` → `EBUSY`; `EXCLUSIVE "c1"`
   flags 0 again → `EEXIST`.
3. `renew_may_and_must`: `lock(EXCLUSIVE, "c1", 30 s)`, read expiration
   `e1`; `MAY_RENEW` → ok and `(sec, nsec) > e1`; `MUST_RENEW` → ok and
   later again; `MUST_RENEW` with `"c2"` → `ENOENT`; flags `MAY_RENEW |
   MUST_RENEW` → `EINVAL`; still one holder, `"c1"`.
4. `lock_expires`: `lock(EXCLUSIVE, "c1", 1 s)`; sleep 2 s; `lock(EXCLUSIVE,
   "c2", 30 s)` ok; `unlock("c1")` → `ENOENT`; `holders == [(.., "c2")]`.
5. `shared_locks_and_the_tag_rule`: `SHARED "c1"` and `SHARED "c2"` with tag
   `"t"` coexist, `holders` in cookie order `c1`, `c2`, type `SHARED`, tag
   `"t"`; `SHARED "c3"` tag `"u"` → `EBUSY`; `SHARED "c1"` tag `"u"` with
   `MAY_RENEW` → `EBUSY` (tag checked before renewal); `EXCLUSIVE "c3"` tag
   `"t"` → `EBUSY`.
6. `unlock_keeps_the_xattr`: `lock(EXCLUSIVE, "c1", tag "t")`; `unlock` ok;
   `get_info`: type `EXCLUSIVE`, tag `"t"`, no holders; `list_locks` still
   `[name]`; `ioctx.get_xattr(&oid, xattr_name(name))` decodes as a
   `LockInfo` equal to that (type, tag, empty map); `unlock` again →
   `ENOENT`.
7. `break_lock_by_locker`: clients A and B; A holds `EXCLUSIVE "c1"`; B
   `break_lock(name, "c2", A)` → `ENOENT`; B `break_lock(name, "c1", B's
   own name)` → `ENOENT`; B `break_lock(name, "c1", A)` ok; B
   `lock(EXCLUSIVE, "c1")` ok and `get_info` shows B only.
8. `set_cookie_moves_the_holder`: `EXCLUSIVE "c1"` 30 s, expiration `e1`;
   `set_cookie(EXCLUSIVE, "c1", "", "c2")` ok; `holders == [(.., "c2")]` with
   expiration `== e1`; `unlock("c1")` → `ENOENT`; `unlock("c2")` ok. Shared
   variant: `SHARED "c1"`, `"c2"` tag `"t"`; `set_cookie(SHARED, "c1", "t",
   "c2")` → `EBUSY`; `set_cookie(EXCLUSIVE, "c1", "t", "c3")` → `EBUSY`
   (type mismatch).
9. `assert_locked_standalone_and_compound`: A holds `EXCLUSIVE "c1"` tag
   `""`; `assert_locked(EXCLUSIVE, "c1", "")` ok; wrong cookie, tag `"x"`,
   or type `SHARED` → `EBUSY`; the write op `assert_locked_op(.. "c1" ..)`
   + `write_full(b"ok")` succeeds and the object reads `ok`; the same with
   cookie `"c2"` and `write_full(b"no")` fails `EBUSY` and the object still
   reads `ok`; a read op `assert_locked_op` + `stat` succeeds; after
   `unlock`, `assert_locked` → `EBUSY`.
10. `ephemeral_unlock_and_break_delete_the_object`: `write_full` data;
    `lock(EXCLUSIVE_EPHEMERAL, "c1")`; `assert_locked(EXCLUSIVE, "c1", "")`
    → `EBUSY` (exact type); `unlock` ok; `stat` → `ENOENT`. Again on a new
    object with clients A (holder) and B: B `break_lock(name, "c1", A)` ok;
    `stat` → `ENOENT`.
11. `expired_ephemeral_read_is_eio`: `write_full` data;
    `lock(EXCLUSIVE_EPHEMERAL, "c1", 1 s)`; sleep 2 s; `get_info` → `EIO`
    and `stat` still shows the object; `lock(EXCLUSIVE, "c2", 30 s)` ok and
    `stat` shows the object with size 0 (deleted and recreated inside the
    write). This pins a claim derived from code, never observed (§3.9):
    if any step of this test behaves otherwise (`get_info` not `EIO`, the
    object gone after it, the relock failing, or the recreated object not
    size 0), **stop and report BLOCKED** with what the class did; do not
    adapt the test or the module doc.
12. `two_clients_are_two_lockers`: two `client()` calls; their global ids
    differ; A holds `EXCLUSIVE "c1"`; B `unlock(name, "c1")` → `ENOENT`; B
    `lock(EXCLUSIVE, "c1")` flags 0 → `EBUSY`, `MAY_RENEW` → `EBUSY`; B
    `assert_locked(EXCLUSIVE, "c1", "")` → `EBUSY`; `holders ==
    [("client.<A gid>", "c1")]`; A `unlock` ok.

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline
--test cls_lock -- --ignored --nocapture` (12 passed).

```
lock: cluster tests

Pins against Ceph v19: exclusive and shared locks, EBUSY from a
second cookie and EEXIST from the same one, renewal with MAY_RENEW
and MUST_RENEW, expiry after one second, the tag rule, break by
locker, set_cookie, assert_locked alone and inside read and write ops,
the xattr that outlives an unlock, ephemeral locks deleting their
object, EIO from reading an expired ephemeral lock, and a second
client that cannot unlock or renew the first's lock.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

---

### Task 5: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6: per-commit proof from the branch base; container
fmt/clippy including each class alone (the loop now has `lock`); the
whole `rados` unit suite (Task 1(a) changes an encoder); corpus over the
fourteen `cls_lock` names plus `entity_addr_t` (unchanged result
expected); the cluster suites of every `rados-cls` test file plus
`cls_lock`; the PR body:

```
**Motivation.** radosgw's GC, lifecycle, reshard, multipart-completion and notification-queue workers exclude each other with `cls_lock`; a Rust RGW sharing a cluster with radosgw must take the same locks by the same rules.

**What changed.** `rados-cls` gains a `lock` module: the eleven structs pinned against `ceph-dencoder` v19.2.2 and the corpus, all seven methods, and radosgw's lock table in the docs. `rados` fixes `EntityAddr`'s unset-address length and orders and prints entity names as C++ does. Twelve cluster tests pin the class's semantics on v19.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

Then merge when green and open the upstream PR from the same branch.

---

## Roadmap for later plans

`cls-2pc-queue` (plan 14: reserve needs `RETURNVEC`; `rgw_notify.cc`
batches its ops with this plan's `assert_exists` + `assert_locked`),
`cls-otp` (15), then `watch-notify` (12). Follow-ups from this plan:
route the core crate's `IoCtx::lock_exclusive`/`lock_shared`/`unlock`
through `rados_cls::lock` (the `rados` crate cannot depend on
`rados-cls`, so that means deprecating them in its favor or moving the
shared encoding into `rados`; upstream's call), and meanwhile their
known gaps: `flags` always 0, the duration's seconds truncated with `as
u32`, no ephemeral, break, info, list, assert or set-cookie (§0.3);
`EntityAddr`'s `entity_addr_t` dump formatter could share `legacy_str`'s
sockaddr printer (it prints IPv6 uncompressed and a fixed family 0);
`main`'s multipart-lock renewal and notification period are RGW-level
behavior for the driver. Outside rados-rs: rgw-go's
`docs/exclusions.md` lock bullet should drop `bucket_instance_lock` and
the registry lock and add `RGWCompleteMultipart` (§4.9).

## Pre-flight (2026-09-25, against 3fe03d3)

- Every tree claim verified (`.superpowers/sdd/2026-09-25-rgw-mvp-13-cls-lock/preflight.md`, 47 rows): `sockaddr_len` returns 0 for family 0 and 128 for other families (`rados/src/denc/entity_addr.rs:189-197`), the existing test asserts 19; `PackedEntityName` derives no `Ord`/`Display`/`Hash` (derive `Hash`; write `Ord`/`PartialOrd` by hand since zerocopy's `U64` orders unsigned); `LockType`/`LockFlags` are public with the C++ values and re-exported, without `Denc`; the oracle directory holds `ea.1`-`ea.3` and the 22 struct captures, all matching the pins; the five IPv6/family-1 captures are the implementer's to make. Note for test 1: a compound `assert_exists` + class call goes out with only the READ flag (`OpCode::Call` is `RD|EXEC`), as every class write call in the crate already does; the OSD classifies the op by the method's flags.
