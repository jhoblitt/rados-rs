# rados-rs RGW MVP, plan 15 of N: `cls-otp`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the `otp` object class to `rados-cls`: the six methods of
`cls_otp_client.h` (`otp_set`, `otp_get`, `otp_check`, `otp_get_result`,
`otp_remove`, `get_current_time`), their request and reply structs with
wire bytes pinned in unit tests from hand-derived hex (there is no
`ceph-dencoder` type and no corpus for this class), op constructors and
`IoCtx` functions, and cluster tests that are the only behavioural oracle
for v19's verdicts, replay guard, rate limit and result expiry.

**Architecture:** One module `otp` behind a new feature `otp`, following
the `version`/`user` pattern: derived `VersionedDenc` structs (one
hand-written empty request), one-byte enums that keep unknown values,
`OSDOp` constructors for compound use, async free functions over `IoCtx`.
The one-byte enum macro moves from `rgw::types` to the crate root so
`otp` does not depend on `rgw`. The cluster suite computes TOTP codes
with a hand-written SHA-1/HMAC helper local to the test, against the OSD's
clock.

**Tech Stack:** as plan 3.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cls-otp`: create, remove, set, check and get, and the MFA seed types".

Research: `.superpowers/research/cls-otp.md` (cited below as "report
§N"; `S:` = `git show v19.2.2:<path>`, `M:` = Ceph `main` at 7ed73efc1be).

## Global Constraints

Plans 3 to 14's Global Constraints apply unchanged (Squid floor with
`rados::check_min_version!` in every hand-written `decode_content`; every
derive carries `#[denc(crate = "rados")]`; no `unwrap`/`expect` on
production paths; gates; container fmt/clippy by the controller; push
after every gate; merge when green then an upstream PR; offline builds
with the scratchpad `CARGO_HOME`; no new dependencies). Plus:

- Execution order is plans 11, 13 (`cls-lock`), 14 (`cls-2pc-queue`),
  15 (this), then 12 (`watch-notify`). Branch `cls-otp` is based on the
  fork's `main` after plan 14 merges. Every list this plan extends
  (`default`, the `mod call` gate, the CI clippy loop, the suite loops,
  the README class list) already carries what plans 11 to 14 added; add
  `otp` to it, do not rewrite it.
- Commit trailers for this plan are two lines:
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` then
  `Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s`.
  Subjects start `otp: `.
- Methods (report §1; `S:src/cls/otp/cls_otp.cc:557-575`; names and
  flags identical on `main`, `M:src/cls/otp/cls_otp_ops.h:171-181`):

  | method | flags | request | reply |
  |---|---|---|---|
  | `otp_set` | RD\|WR | `cls_otp_set_otp_op` | none |
  | `otp_get` | RD | `cls_otp_get_otp_op` | `cls_otp_get_otp_reply` |
  | `otp_check` | RD\|WR | `cls_otp_check_otp_op` | none |
  | `otp_get_result` | RD | **`cls_otp_check_otp_op`** | `cls_otp_get_result_reply` |
  | `otp_remove` | RD\|WR | `cls_otp_remove_otp_op` | none |
  | `get_current_time` | RD | `cls_otp_get_current_time_op` | `cls_otp_get_current_time_reply` |

  No writing method writes `*out` (report §0.4), so every call goes
  through `call::exec`/`call::exec_raw`; nothing uses `exec_returnvec`.
  `otp_check` goes out read-flagged as `call::exec` sends every call,
  which is what Squid's C++ `check` does (`ioctx.exec`, a read-path exec,
  `S:src/cls/otp/cls_otp_client.cc:71`); the OSD derives `may_write` from
  the method's flags (report §1, "rados-rs note"). `main`'s switch of
  `check` to a write op (`M:src/cls/otp/cls_otp_client.cc:72-74`) is the
  crate-wide WRITE-flag follow-up plan 4's roadmap already carries.
- `otp_get_result` decodes a `cls_otp_check_otp_op` and looks the token
  up under its `id` (`S:src/cls/otp/cls_otp.cc:487-507`); the C++ client
  builds a `cls_otp_get_result_op` and never sends it, sending the check
  request's `in` instead (`S:src/cls/otp/cls_otp_client.cc:76-81`,
  unchanged at `M:...:84`). This module has no `GetResultOp` type.
- Wire (report §2): every struct is `ENCODE_START(1, 1)`; `string` and
  `bufferlist` are u32 length + bytes; lists u32 count + items; `bool` u8;
  `ceph::real_time` is `rados::UTime` (u32 sec, u32 nsec). Squid to `main`
  changed no encoding (report §0.7, §5), so no version gate: every derive
  is `version = 1, compat = 1`, and the one hand-written decode
  (`GetCurrentTimeOp`) checks `struct_v >= 1` with the hint naming the
  release that first shipped the class (`git log --diff-filter=A --
  src/cls/otp/cls_otp.cc` then `git tag --contains`; the campaign's hint
  format is `"<Release> v<N>+"`).
- Enums are one byte and C++ decodes them unchecked
  (`S:src/cls/otp/cls_otp_types.h:61-68`, `:102-104`), so they are
  newtypes over `u8` that keep unknown values.
- No oracle, no corpus (report §0.1): `ceph-dencoder list_types` on
  v19.2.2 has no otp type, `ceph-object-corpus` has no otp directory in
  either archive, and no otp type has `generate_test_instances`. This
  plan therefore adds **no `rados-dencoder` arm and no `TypeSpec` row**:
  an arm exists to be compared with `ceph-dencoder` and a `TypeSpec` names
  a corpus directory; neither has anything to compare against. Every hex
  pin below is derived by hand from the `ENCODE_*` code (report §2) and
  labelled so in the test; the cluster suite is the only behavioural
  oracle, and there is no upstream `test_cls_otp.cc` to borrow from
  (report §0.2).
- The crate generates no randomness: the check token is the caller's.
- No dependency is added, not even a dev-dependency: the workspace has
  `hmac` and `sha2` but no `sha1` (report §6), so the test helper
  hand-writes SHA-1.

## Review Focus

1. `otp_get_result` is sent a `CheckOp` body, not a get-result body;
   `check_and_get_result` sends byte-identical request data to both
   methods, as the C++ client does. Pinned in Task 1 (framing hex of both
   ops) and Task 2 (`otp_get_result_decodes_the_check_request`: a
   `cls_otp_get_result_op` body is `EINVAL`).
2. The verdict exists only in `otp_get_result`: `otp_check` returns 0 for
   right and wrong codes; a sixth check inside one step window is dropped
   silently and reads back `UNKNOWN`; a recorded result expires after
   `step_size` seconds. Pinned in Task 2 (`otp_check_totp`,
   `otp_rate_limit`, `otp_result_expires`).
3. The replay guard's index: a match one step back records
   `last_success` one past the current counter, so the current and next
   steps' codes then fail and the one after succeeds; an upsert through
   `otp_set` keeps the replay state. Pinned in Task 2
   (`otp_past_step_quirk`, `otp_upsert_keeps_replay_state`).
4. `step_size == 0` never leaves the client (the OSD would divide by
   zero, report §0.6); enums keep unknown bytes; seeds are cleartext in
   replies and in `OtpInfo`'s JSON, and the docs say so. Pinned in Task 1
   (unit tests); the zero step is never sent to a cluster.

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cls-otp` off the fork's `main` (containing plan 14's
  merge) in `$R`; the SDD ledger; the cluster up.

- [ ] **Step 1: Branch, cluster, ledger**

```bash
cd $R && git fetch origin main && git checkout -b cls-otp origin/main && git log --oneline -1
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
```
Expected: HEAD is the merge of the `cls-2pc-queue` PR; three containers
up. Then the workspace and ledger as in plan 3's Task 0.

---

### Task 1: The crate-root `byte_enum!` and the `otp` module

**Files:**
- Create: `rados-cls/src/otp.rs` (module doc, types, client, unit tests).
- Modify: `rados-cls/src/lib.rs` (the macro; `#[cfg(feature = "otp")] pub
  mod otp;`; `feature = "otp"` in the `mod call` gate),
  `rados-cls/src/rgw/types.rs` (macro removed, `use crate::byte_enum;`),
  every other user of `byte_enum` (`grep -rn byte_enum rados-cls/src`;
  today `rgw/olh.rs`, plus whatever plans 13 and 14 added) to import
  `crate::byte_enum`, `rados-cls/Cargo.toml` (`otp = []`, `"otp"` in
  `default`), `.github/workflows/ci.yml` (`otp` in the "Run clippy on each
  rados-cls class alone" loop), `README.md` (`otp` in the `rados-cls`
  row's class list).
- Not modified: the `dump` gate (`otp` uses no `dump` helper;
  `OtpInfo`'s JSON needs only serde attributes and a local function);
  `rados-dencoder` (see Global Constraints).

**Interfaces:**
- Consumes: `rados::{Denc, RadosError, UTime, VersionedDenc,
  VersionedEncode}`, `rados::check_min_version!`,
  `rados::impl_denc_for_versioned!`, `crate::call::{op, raw_op, exec,
  exec_raw, decode, decode_bytes}`, `rados::osdclient::{IoCtx, OSDOp,
  OpReply}`, `rados::OSDClientError`, `bytes::Bytes`.
- Produces (all `pub` in `rados_cls::otp`): `CLASS = "otp"`; the enums,
  structs, constructors, decoders and async functions below. Task 2 uses
  them.

**The macro move.** Move `macro_rules! byte_enum` from
`rgw/types.rs:112-141` into `lib.rs` above the `mod` declarations,
unchanged in behaviour, gated `#[cfg(any(feature = "rgw", feature =
"otp"))]` (widen it if plan 13 or 14 already uses the macro), followed by
`pub(crate) use byte_enum;` under the same gate. Inside the macro, make
every path absolute so a caller needs no imports: `::serde::Serialize`
in the derive, `::rados::Denc`, `::rados::RadosError`, `::bytes::Buf`,
`::bytes::BufMut`. If plan 13 or 14 already moved it to the crate root,
use it as it is and only widen its gate. Then drop imports clippy reports
unused in `rgw/types.rs`.

**Types** (wire order is field order; `#[derive(Debug, Clone, PartialEq,
Eq, VersionedDenc)]` + `#[denc(crate = "rados", version = 1, compat =
1)]` unless stated; `Default` derived unless stated):

```rust
byte_enum! {
    /// `rados::cls::otp::OTPType`. The class never reads it: every device
    /// is validated as TOTP, so `HOTP` is stored but never honoured.
    OtpType { UNKNOWN = 0, HOTP = 1, TOTP = 2 }
}
byte_enum! {
    /// `SeedType`. `otp_set` parses `BASE32` as base32 and anything else,
    /// `UNKNOWN` included, as hex.
    SeedType { UNKNOWN = 0, HEX = 1, BASE32 = 2 }
}
byte_enum! {
    /// `OTPCheckResult`: `UNKNOWN` is also the answer for a token the
    /// class holds no check for (never recorded, rate-limited, expired).
    CheckResult { UNKNOWN = 0, SUCCESS = 1, FAIL = 2 }
}
```
`SeedType::as_str(self) -> &'static str`: `HEX` → `"hex"`, `BASE32` →
`"base32"`, anything else `"unknown"` (`otp_info_t::dump`'s `default:`
arm, `S:src/cls/otp/cls_otp_types.cc:30-50`).

- `OtpInfo` (`otp_info_t`, derives also `Serialize`; manual `Default`
  = `{TOTP, "", "", UNKNOWN, empty, 0, 30, 2}`, the C++ defaults,
  `S:src/cls/otp/cls_otp_types.h:32-41`):
  `#[serde(rename = "type")] otp_type: OtpType`, `id: String`,
  `seed: String`, `#[serde(serialize_with = "seed_type_str")] seed_type:
  SeedType`, `#[serde(skip)] seed_bin: Bytes`, `time_ofs: i32`,
  `step_size: u32`, `window: u32`. JSON follows `dump`: `type` as an int,
  `seed` in cleartext, no `seed_bin`. Doc: `seed_bin` is filled (and
  overwritten) by `otp_set`; a client sends it empty. Constructor
  `OtpInfo::totp(id: &str, seed: &str, seed_type: SeedType) -> Self`
  (the `TOTPConfig` builder's fields, rest default).
- `OtpCheck` (`otp_check_t`): `token: String`, `timestamp: UTime`,
  `result: CheckResult`. No `Serialize` (no Ceph `dump`; report §2).
- `SetOp` (`cls_otp_set_otp_op`): `entries: Vec<OtpInfo>`.
- `CheckOp` (`cls_otp_check_otp_op`): `id: String`, `val: String`,
  `token: String`. Doc: also the request of `otp_get_result`.
- `GetResultReply` (`cls_otp_get_result_reply`): `result: OtpCheck`.
- `RemoveOp` (`cls_otp_remove_otp_op`): `ids: Vec<String>`.
- `GetOp` (`cls_otp_get_otp_op`): `get_all: bool`, `ids: Vec<String>`.
- `GetReply` (`cls_otp_get_otp_reply`): `found_entries: Vec<OtpInfo>`.
- `GetCurrentTimeOp {}` (`cls_otp_get_current_time_op`): hand-written
  exactly as `user::GetHeaderOp` (`VersionedEncode` with
  `MAX_DECODE_VERSION = 1`, empty content,
  `rados::check_min_version!(struct_v, 1, "GetCurrentTimeOp", "<Release vN+>")` (the hint from Global Constraints), `rados::impl_denc_for_versioned!`).
- `GetCurrentTimeReply` (`cls_otp_get_current_time_reply`): `time: UTime`.

The op structs have no `Serialize` (none has a `dump`, report §2).

**Op constructors, decoders, functions** (`Result` is
`rados::osdclient::error::Result`):

```rust
pub fn set_op(entries: &[OtpInfo]) -> Result<OSDOp>;          // "otp_set"
pub fn create_op(info: &OtpInfo) -> Result<OSDOp>;             // set_op of one entry
pub fn remove_op(ids: &[&str]) -> Result<OSDOp>;               // "otp_remove"
pub fn check_op(id: &str, val: &str, token: &str) -> Result<OSDOp>; // "otp_check"
pub fn get_result_op(id: &str, token: &str) -> Result<OSDOp>;  // CheckOp{id, "", token}
pub fn get_op(ids: &[&str]) -> Result<OSDOp>;                  // GetOp{false, ids}
pub fn get_all_op() -> Result<OSDOp>;                          // GetOp{true, []}
pub fn get_current_time_op() -> Result<OSDOp>;
pub fn decode_get(reply: &OpReply) -> Result<Vec<OtpInfo>>;
pub fn decode_get_result(reply: &OpReply) -> Result<OtpCheck>;
pub fn decode_current_time(reply: &OpReply) -> Result<UTime>;

pub async fn set(ioctx: &IoCtx, oid: &str, entries: &[OtpInfo]) -> Result<()>;
pub async fn create(ioctx: &IoCtx, oid: &str, info: &OtpInfo) -> Result<()>;
pub async fn remove(ioctx: &IoCtx, oid: &str, ids: &[&str]) -> Result<()>;
pub async fn get(ioctx: &IoCtx, oid: &str, ids: &[&str]) -> Result<Vec<OtpInfo>>;
pub async fn get_one(ioctx: &IoCtx, oid: &str, id: &str) -> Result<OtpInfo>;
pub async fn get_all(ioctx: &IoCtx, oid: &str) -> Result<Vec<OtpInfo>>;
pub async fn get_current_time(ioctx: &IoCtx, oid: &str) -> Result<UTime>;
pub async fn check(ioctx: &IoCtx, oid: &str, id: &str, val: &str, token: &str) -> Result<()>;
pub async fn get_result(ioctx: &IoCtx, oid: &str, id: &str, token: &str) -> Result<OtpCheck>;
pub async fn check_and_get_result(
    ioctx: &IoCtx, oid: &str, id: &str, val: &str, token: &str,
) -> Result<OtpCheck>;
```

- `set_op`, `create_op`, `set`, `create` first run a private
  `refuse_zero_step(entries: &[OtpInfo]) -> Result<()>`: the first entry
  with `step_size == 0` gives
  `OSDClientError::Other(format!("otp device {id:?}: step_size must be non-zero"))`
  and nothing is sent. Doc on `set`: the class validates neither
  `step_size` nor `type`, and with a zero step the OSD divides by zero on
  the device's next check (`S:src/cls/otp/cls_otp.cc:143`; report §0.6).
- `get_one`: `get` of `[id]`; an empty reply is
  `OSDClientError::OSDError { code: -2, message: format!("otp device {id:?} not found") }`,
  mirroring the C++ wrapper (`S:src/cls/otp/cls_otp_client.cc:147-149`).
- `check` sends `otp_check` and returns `Ok(())` for a right and a wrong
  code alike; its doc sends the reader to `get_result`.
- `get_result` encodes `CheckOp { id, val: "", token }` (the server reads
  `id` and `token` only).
- `check_and_get_result` encodes `CheckOp { id, val, token }` once and
  sends the same `Bytes` with `call::exec_raw` to `otp_check` then
  `otp_get_result`, decoding `GetResultReply` — the bytes C++'s
  `OTP::check` sends to both.
- Token doc (on `check` and `check_and_get_result`): the caller supplies
  it; tokens should be unique per check, since `otp_get_result` returns
  the newest recorded check with that token. The C++ client sends 16
  characters from `gen_rand_alphanumeric` (report §1;
  `S:src/cls/otp/cls_otp_client.cc:65-66`), whose table is
  `A-Za-z0-9` plus `-` and `_`, 64 symbols
  (`S:src/common/random_string.cc:45-49`; the report names the function
  but not its table, which the plan writer read at v19.2.2). A driver that
  mirrors radosgw draws the same.

**Module doc** (put at the top of `otp.rs`; every fact is v19):

```rust
//! The `otp` class: the TOTP devices radosgw checks S3 MFA against.
//! Mirrors `cls_otp_client.h`.
//!
//! radosgw keeps one object per user, `user:<uid>`, in the zone's otp
//! pool (`<zone>.rgw.otp` by default), and reaches it through
//! `RGWSI_Cls::MFA`. A request carrying `x-amz-mfa: <serial> <pin>` runs
//! one `otp_check` and one `otp_get_result`, and only a `SUCCESS` marks it
//! MFA-verified; changing `MfaDelete`, changing versioning on an
//! MFA-enabled bucket, and deleting an object version there (singly or in
//! a multi-object delete) require that mark.
//!
//! Server facts (`src/cls/otp/cls_otp.cc`):
//! - The object's omap holds a `header` key listing the device ids and
//!   one `otp/<id>` key per device; nothing is kept in xattrs.
//! - `otp_set` upserts: it parses `seed` into `seed_bin` (base32 when
//!   `seed_type` is `BASE32`, hex otherwise; a seed that does not parse is
//!   `EINVAL`), replaces the device's settings and keeps its recorded
//!   checks and replay index. It validates neither `type` nor
//!   `step_size`; this module refuses a zero `step_size`, on which the OSD
//!   would divide by zero.
//! - `otp_remove` skips ids it does not hold and succeeds.
//! - `otp_get` returns whole entries, the seed and `seed_bin` in
//!   cleartext; with `get_all` they come sorted by id; ids it does not
//!   hold are skipped.
//! - `otp_check` returns 0 whether the code is right or wrong (`ENOENT`
//!   for an unknown id). It validates against the primary OSD's clock,
//!   which `get_current_time` reports: TOTP (HMAC-SHA1) over `seed_bin`
//!   at counter `(now - time_ofs) / step_size`, within `window` steps
//!   either side, as many digits as the code has (six and eight work;
//!   five or none fail). It records `{token, now, SUCCESS|FAIL}`.
//! - Rate limit: recorded checks older than `step_size` seconds are
//!   dropped; while five remain, a check records nothing and still returns
//!   0, so its token reads back `UNKNOWN`.
//! - Replay guard: a match records `last_success = distance + counter`,
//!   the distance being the absolute number of steps between the code and
//!   now, and any later match at or below it fails. A code from one step
//!   back therefore records one past the current counter, and the current
//!   and next steps' codes then fail.
//! - `otp_get_result` takes a `cls_otp_check_otp_op` (the C++ client's
//!   `cls_otp_get_result_op` never reaches the wire) and returns the
//!   newest recorded check for the token, or `{token, now, UNKNOWN}`;
//!   a recorded check expires after `step_size` seconds.
//! - `otp_get`, `otp_get_result` and `get_current_time` on a missing
//!   object are `ENOENT`.
//!
//! Seeds are secrets and Ceph never redacts them: they travel in
//! cleartext in `otp_set` requests and `otp_get` replies, and
//! [`OtpInfo`]'s JSON prints `seed` as `otp_info_t::dump` does.
```
The last server fact (missing object) is pinned by Task 2's
`otp_missing_object`; if that test observes otherwise, correct this line
in Task 2's commit to what was observed.

- [ ] **Step 1: Move the macro**

Move it as above; `cargo test -p rados-cls --offline --lib` passes
unchanged and `cargo check -p rados-cls --offline --no-default-features
--features rgw` builds.

- [ ] **Step 2: Write the failing unit tests**

In `otp.rs`'s test module, with a local `unhex` as `rgw/usage.rs` has.
Every hex constant carries the comment `// Hand-derived from the
ENCODE_* code (report §2); no ceph-dencoder type exists.` Instances:
`A = OtpInfo { TOTP, "dev1", "3132", HEX, seed_bin empty, 0, 30, 2 }`,
`A2` = `A` with `seed_bin = b"12"` (what `otp_get` returns), `B = {TOTP,
"dev2", "GEZDGNBV", BASE32, empty, -30, 60, 1}`, `CHK = OtpCheck { "tok",
UTime { sec: 1, nsec: 2 }, SUCCESS }`.

| test | pins |
|---|---|
| `otp_info_encodes_as_derived` | `A` = `01012200000002040000006465763104000000333133320100000000000000001e00000002000000` (40 B); `A2` = `010124000000020400000064657631040000003331333201020000003132000000001e00000002000000` (42 B); `B` = `0101260000000204000000646576320800000047455a44474e42560200000000e2ffffff3c00000001000000` (44 B, `time_ofs` -30 as `e2ffffff`); each decodes back equal |
| `otp_info_json_matches_dump` | `A` and `A2` both → `{"type":2,"id":"dev1","seed":"3132","seed_type":"hex","time_ofs":0,"step_size":30,"window":2}` (seed cleartext, no `seed_bin`) |
| `enums_keep_unknown_bytes` | `A`'s bytes with the `seed_type` byte (offset 23) set to 7 decode to `SeedType(7)`, re-encode identically, JSON `"seed_type":"unknown"`; the `type` byte (offset 6) set to 9 → `OtpType(9)` → JSON `"type":9`; `OtpInfo::default()` is `{TOTP, UNKNOWN, 0, 30, 2}` |
| `otp_check_encodes_as_derived` | `CHK` = `01011000000003000000746f6b010000000200000001` (22 B) |
| `requests_encode_as_derived` | `SetOp{[A]}` = `01012c0000000100000001012200000002040000006465763104000000333133320100000000000000001e00000002000000`; `CheckOp{dev1,123456,tok}` = `01011900000004000000646576310600000031323334353603000000746f6b`; `RemoveOp{[dev1]}` = `01010c000000010000000400000064657631`; `GetOp{true,[]}` = `0101050000000100000000`; `GetOp{false,[dev1]}` = `01010d00000000010000000400000064657631`; `GetCurrentTimeOp{}` = `010100000000`; round trips |
| `replies_decode_as_derived` | `GetResultReply{CHK}` = `01011600000001011000000003000000746f6b010000000200000001`; `GetReply{[A2]}` = `01012e00000001000000010124000000020400000064657631040000003331333201020000003132000000001e00000002000000`; `GetReply{[]}` = `01010400000000000000`; `GetCurrentTimeReply{1700000000.123456789}` = `01010800000000f1536515cd5b07`; round trips |
| `get_current_time_op_refuses_version_zero` | `000000000000` → `Err` (the floor check) |
| `ops_frame_the_class_method_and_request` | `check_op("dev1","123456","tok").indata` = `b"otpotp_check"` ++ the `CheckOp` hex above, `OpData::Call { class_len: 3, method_len: 9, indata_len: 31 }`; `get_result_op("dev1","tok").indata` = `b"otpotp_get_result"` ++ `01011300000004000000646576310000000003000000746f6b` (`CheckOp{dev1,"",tok}`, derived here by the same rules; 25 B); `get_all_op()` starts `b"otpotp_get"`; `remove_op(&["dev1"])` starts `b"otpotp_remove"`; `get_current_time_op()` = `b"otpget_current_time"` ++ `010100000000` |
| `decoders_unwrap_the_replies` | `decode_get`, `decode_get_result`, `decode_current_time` over `OpReply { return_code: 0, outdata }` of the reply hex above |
| `zero_step_size_is_refused_before_sending` | `set_op(&[A, {B with step_size 0}])` and `create_op(&{A with step_size 0})` are `Err(OSDClientError::Other(msg))` with `msg` containing `"step_size"` and the device id |

- [ ] **Step 3: Implement to green**

Run: `cargo test -p rados-cls --offline --lib otp` → all pass;
`cargo check -p rados-cls --offline --no-default-features --features otp`
and `--features rgw` both build (the controller's container clippy runs
the full loop).

- [ ] **Step 4: Commit**

```bash
git add rados-cls/src/lib.rs rados-cls/src/otp.rs rados-cls/src/rgw rados-cls/Cargo.toml .github/workflows/ci.yml README.md
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
otp: the otp class's types and client

Mirrors cls_otp_client.h: set, get, check, get_result, remove and
get_current_time behind a new otp feature. otp_get_result decodes the
check request rather than the get-result op the C++ client builds and
never sends, so check_and_get_result sends the check's bytes to both
methods. The check token is the caller's. A zero step_size is refused
before sending because the class stores it unchecked and the OSD then
divides by it. No ceph-dencoder type or corpus exists for this class,
so the wire pins are derived from the ENCODE code.

byte_enum! moves to the crate root so otp's one-byte enums do not
depend on the rgw feature.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 2: TOTP helper and cluster tests

**Files:**
- Create: `rados-cls/tests/totp/mod.rs` (a subdirectory module, so Cargo
  does not build it as its own test target), `rados-cls/tests/cls_otp.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (both rados-cls loops
  gain `cls_otp`).

**Interfaces:**
- Consumes: Task 1's functions; `rados/tests/common/mod.rs` via
  `#[path]`; `IoCtx::exec`.
- Produces: `totp::{sha1, hmac_sha1, hotp}` with four plain `#[test]`s
  (run by CI's `cargo test --workspace --all-targets`); eleven
  `#[tokio::test] #[ignore]` cluster tests.

**`tests/totp/mod.rs`** (about 75 lines, no dependency):
- `pub fn sha1(msg: &[u8]) -> [u8; 20]`: FIPS 180-4. Initial state
  `67452301 efcdab89 98badcfe 10325476 c3d2e1f0`; pad with `0x80`, zeros
  to 56 mod 64, the bit length as u64 big-endian; per 64-byte block a
  80-word schedule (`w[i] = rotl1(w[i-3]^w[i-8]^w[i-14]^w[i-16])`), rounds
  with `f`/`k` = `(b&c)|(!b&d)`/`5a827999`, `b^c^d`/`6ed9eba1`,
  `(b&c)|(b&d)|(c&d)`/`8f1bbcdc`, `b^c^d`/`ca62c1d6`; wrapping adds.
- `pub fn hmac_sha1(key: &[u8], msg: &[u8]) -> [u8; 20]`: RFC 2104, block
  64; a key over 64 bytes is hashed first; `ipad 0x36`, `opad 0x5c`.
- `pub fn hotp(key: &[u8], counter: u64, digits: u32) -> String`: HMAC of
  `counter` big-endian; `off = mac[19] & 0x0f`; `bin =
  u32::from_be_bytes(mac[off..off+4]) & 0x7fff_ffff`; `bin % 10^digits`
  zero-padded to `digits`. TOTP is `hotp(key, (unix - time_ofs) / step, d)`.
- Tests: `sha1(b"abc")` = `a9993e364706816aba3e25717850c26c9cd0d89d`;
  `sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")` (two
  blocks) = `84983e441c3bd26ebaae4aa1f95129e5e54670f1`; RFC 6238
  Appendix B, key ASCII `"12345678901234567890"`, step 30: T=59
  (counter 1) → `94287082` at 8 digits and `287082` at 6 (the liboath
  probe accepted `287082` at t=59, report "liboath probe");
  T=1111111109 (counter 37037036) → `07081804` at 8 (pins the zero pad).

**`tests/cls_otp.rs` helpers** (shape of `tests/cls_user.rs`: `#[path =
"../../rados/tests/common/mod.rs"] mod common;`, `mod totp;`, `unique`,
`is_osd_error`):
- `ENOENT = 2`, `EINVAL = 22`, `STEP: u32 = 300`,
  `KEY = b"12345678901234567890"`,
  `SEED_HEX = "3132333435363738393031323334353637383930"` (hex of `KEY`, so
  no base32 code is needed).
- `device(id) -> OtpInfo`: `OtpInfo { step_size: STEP, window: 2,
  ..OtpInfo::totp(id, SEED_HEX, SeedType::HEX) }`.
- `async fn counter(ioctx, oid) -> (UTime, u64)`: `now =
  otp::get_current_time`; if `now.sec % STEP >= 285`, sleep until one
  second past the boundary and read again (so no test straddles a step);
  return `(now, u64::from(now.sec / STEP))` (every device here has
  `time_ofs` 0). Codes come from the OSD's clock, never the host's.
- `code(t: u64) -> String` = `totp::hotp(KEY, t, 6)`;
  `wrong(t) -> String`: the first of `000000`, `000001`, ... equal to no
  `code(t-2..=t+2)` (deterministically wrong within the window).
- `verdict(ioctx, oid, id, val, token) -> CheckResult` =
  `check_and_get_result(..)?.result`.

Each test uses a fresh `unique("cls-otp-...")` oid and calls
`common::init_tracing()`.

| # | test | steps and expected results |
|---|---|---|
| 1 | `otp_set_get_list` | `set` `[device("a-hex"), {id "b-b32", seed "GEZDGNBV", BASE32, time_ofs -30, step 60, window 1}]`. `get_all` → two entries in id order `a-hex`, `b-b32`; `a-hex.seed_bin` = `KEY`, `b-b32.seed_bin` = `b"12345"` (filled by the server); `seed`, `seed_type`, `time_ofs` -30, `step_size`, `window` as sent; `serde_json` of `a-hex` has `"seed":"3132…3930"` (cleartext). `get(["b-b32","missing"])` → only `b-b32`; `get(["missing"])` → empty; `get_one("missing")` → `ENOENT` |
| 2 | `otp_set_bad_seed` | on a fresh oid, `set([{device("x") with seed "zz"}])` → `EINVAL`; then `get_all` → `ENOENT` (nothing was created) |
| 3 | `otp_remove` | `set` `a`, `b`; `remove(["a"])` → Ok; `remove(["a"])` again → Ok; `get_all` → only `b` |
| 4 | `otp_missing_object` | fresh oid: `get_all`, `get_current_time`, `get_result(.., "a", "t")` → `ENOENT` (the OSD refuses a read on a missing object before the method runs, report §3 "Errnos"); `remove(["a"])` → Ok (the method runs and returns 0; whether it creates the object is not asserted). Then `set([device("a")])` and `check(.., "nope", "123456", "t")` → `ENOENT` |
| 5 | `otp_check_totp` | `set([device("d")])`; `(now, t) = counter`; `check_and_get_result(code(t), "t1")` → `SUCCESS`, `token` `"t1"`, `timestamp.sec` within 5 of `now.sec`; `check(wrong(t), "t2")` → Ok, then `get_result("t2")` → `FAIL`; `code(t)` again with `"t3"` → `FAIL` (replay); `code(t+1)` with `"t4"` → `SUCCESS` (window 2, index `t+1 > t`) |
| 6 | `otp_past_step_quirk` | fresh device; `code(t-1)` → `SUCCESS`; `code(t)` → `FAIL`; `code(t+1)` → `FAIL`; `code(t+2)` → `SUCCESS` (the t-1 match recorded `last_success = 1 + t`, `S:src/cls/otp/cls_otp.cc:143`, report §0.5) |
| 7 | `otp_get_result_unknown` | `set([device("d")])`; `get_result("d", "never")` → `{token "never", UNKNOWN}`, `timestamp` within 5 s of `get_current_time` |
| 8 | `otp_rate_limit` | fresh device; five `check_and_get_result(wrong(t), "w1".."w5")` → each `FAIL`; then `check_and_get_result(code(t), "r")` → `UNKNOWN` (five recorded checks within `step_size`: dropped without a record, `S:src/cls/otp/cls_otp.cc:113-117`); `get_result("w1")` → still `FAIL` |
| 9 | `otp_upsert_keeps_replay_state` | fresh device; `code(t)` → `SUCCESS`; `set([device("d") with window 1])`; `get_one("d").window` = 1; `code(t)` with a new token → `FAIL` (last_success survived, `:327`); `code(t+1)` → `SUCCESS` |
| 10 | `otp_get_result_decodes_the_check_request` | `set([device("d")])`; `ioctx.exec(&oid, "otp", "otp_get_result", unhex("01010700000003000000746f6b"))` (a `cls_otp_get_result_op{tok}`) → `EINVAL` (it does not decode as a `cls_otp_check_otp_op`, report §0.3) |
| 11 | `otp_result_expires` | `set([{device("e") with step_size 1}])`; `check_and_get_result("12345", "x")` → `FAIL` (five digits fail validation, report §3); sleep 3 s; `get_result("e", "x")` → `UNKNOWN` (a check is dropped once older than `step_size`, `:98-106`; strict `<`, so 3 s clears a 1 s step) |

The expiry is pinned (test 11, 3 s of sleep); no test waits out the
300 s window, so the recovery after a rate-limited window is not pinned.

- [ ] **Step 1: Helper and its tests**

Write `tests/totp/mod.rs` and a stub `tests/cls_otp.rs` with `mod totp;`.
Run: `cargo test -p rados-cls --offline --test cls_otp` → the four helper
tests pass.

- [ ] **Step 2: Cluster tests**

Write the eleven tests. Add `cls_otp` to both loops of
`test-with-ceph.yml`.

- [ ] **Step 3: Run them (unsandboxed)**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_otp -- --ignored --nocapture`
Expected: 11 passed. On a failure, report the observed value and the
`cls_otp.cc` line that explains it; do not weaken an assertion. The
report could not exercise three paths, and each has a stop rule:
- **Zero step size:** never send it to the cluster, in this or any test;
  it is pinned client-side only (Task 1). An OSD crash during this suite
  is BLOCKED, with the OSD log.
- **Rate limit (test 8):** if the sixth check reads `SUCCESS` or `FAIL`
  rather than `UNKNOWN`, stop and report BLOCKED with the six verdicts
  and `get_current_time` before and after; the module doc's rate-limit
  fact would be wrong.
- **Missing object (test 4):** the `ENOENT` for reads on a missing object
  is the report's reading of `PrimaryLogPG.cc`, not an observation; if
  the OSD answers otherwise, report BLOCKED with the codes rather than
  changing the doc on your own.
If test 6's `t+2` code does not succeed, report the verdicts: the index
formula is the report's reading, and the Review Focus depends on it.

- [ ] **Step 4: Commit**

```bash
git add rados-cls/tests/totp/mod.rs rados-cls/tests/cls_otp.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
otp: cluster tests

No upstream test covers cls_otp, so these pin its v19 semantics:
upserts that keep the replay state, get skipping unknown ids, removes
that tolerate absent ids, ENOENT for reads on a missing object, the
verdict readable only through otp_get_result, the replay guard
including a match one step back blocking the next two steps, the
five-checks-per-window limit answering UNKNOWN, and results expiring
after step_size seconds. Codes are computed from the OSD's clock by a
hand-written SHA-1 TOTP helper pinned to RFC 6238's vectors, since
the workspace has no sha1 crate.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
EOF
```

---

### Task 3: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6, with: the per-commit proof from the branch base (both
commits build alone, and `--no-default-features --features otp` builds
at each); no corpus step (no otp type exists in `ceph-dencoder` or the
corpus); the cluster suites through `cls_otp`; the PR body:

```
**Motivation.** radosgw verifies S3 MFA (`x-amz-mfa`) through the `otp` object class, which keeps each user's TOTP devices and replay state; a Rust RGW needs its client.

**What changed.** A new `otp` module and feature in `rados-cls`: request and reply structs pinned by hand-derived bytes (no `ceph-dencoder` type or corpus exists), op constructors and `IoCtx` functions, and cluster tests pinning v19's verdicts, replay guard, rate limit and result expiry.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01UctL4Y67TY89ZjJPAmnR4s
```

Then merge when green and open the upstream PR from the same branch.

---

## Roadmap for later plans

`watch-notify` (plan 12) runs next and closes the spec's roadmap.
Deferred: the RGW-layer `mfa_oid(uid) = "user:<uid>"` helper and the
`otp` metadata section's `{"devices": [...]}` JSON belong to the driver,
not this crate; decoders for the server-private `otp_header` and
`otp_instance` omap values (report §2) would let a test read
`last_success` directly and are optional; `main`'s write-op `check` joins
the crate-wide WRITE-flag follow-up from plan 4's roadmap; HOTP is never
honoured by the class and is not modelled beyond its enum value.
