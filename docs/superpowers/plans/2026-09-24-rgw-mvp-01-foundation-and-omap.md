# rados-rs RGW MVP, plan 1 of N: foundation and `omap-ops`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the fork, the local Ceph v19 test cluster and the branch
workflow, then land the first transport package, omap operations, as an
upstream-shaped branch with unit, encoding and cluster tests.

**Architecture:** The omap package touches four places in the `rados`
crate: a `Denc` impl for `BTreeMap` in the codec, ten `OpCode` variants,
a new `osdclient/omap.rs` module holding op constructors, assertion types
and reply decoders, and `OpBuilder`/`IoCtx` methods that use them. Read
ops carry their arguments in `indata` with an empty op union; write ops
carry `indata` plus an extent union of `(0, indata.len())`, which is what
`ObjectOperation::add_data` in `src/osdc/Objecter.h` does.

**Tech Stack:** Rust 2024 (upstream `rust-version` 1.88; the machine has
1.98), tokio, `bytes`, the crate's own `Denc`; podman with
`podman-compose` for the cluster; `gh` for the fork.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`

## Global Constraints

- Ceph floor is Squid (v19): new `decode_content` bodies call
  `check_min_version!` at the version Squid emits; no branches for older
  formats; cluster tests run only against v19.2.2.
- Encoding goes through `Denc`; raw `put_*`/`get_*` only inside `Denc`
  impls or `encode_content`/`decode_content`.
- No `unwrap` or `expect` on production paths; tests may use `expect`.
- Gates on every task: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets --all-features -- --no-deps -D warnings`, `cargo test
  --workspace --lib`; cluster tests with `-- --ignored` against the local
  cluster before the branch is pushed.
- One branch per package off upstream `main`; a draft PR against the
  fork's `main` for CI and review, never merged there; fork `main` is
  never committed to.
- Commit subjects follow upstream's `<module>: <imperative summary>`
  style (`osdclient: add omap operations`), one logical change per commit,
  and end with the `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`
  trailer after a blank line.
- Builds use a project-local `CARGO_HOME` with `--offline` after one
  unsandboxed `cargo fetch`; `.cargo/config.toml` is not committed.

## Review Focus

1. omap keys that are not UTF-8 (RGW's bucket index prefixes entries with
   `0x80`): the key type must be bytes, and a listing containing such a
   key must round-trip unchanged. Pinned in Task 3 (unit) and Task 6
   (cluster, `keys_are_bytes_not_strings`).
2. A page boundary: `max_return` smaller than the entry count must return
   exactly that many entries with `more == true`, and resuming from the
   last key must return the rest with `more == false`. Pinned in Task 6
   (`get_vals_pages_with_more`).
3. A failed `omap_cmp` inside a compound op must leave every other op in
   that transaction unapplied. Pinned in Task 6 (`cmp_failure_aborts_transaction`).
4. Reads on an object that does not exist must return the OSD's `ENOENT`
   as an `OSDError`, not a decode error. Pinned in Task 6
   (`read_on_missing_object_is_enoent`).
5. `omap_rm_range` treats `end` as exclusive; removing `["b", "d")` must
   keep `d`. Pinned in Task 6 (`rm_range_end_is_exclusive`).

---

### Task 0: Fork, remotes, local build environment

**Files:**
- Create: `~/github/rados-rs` (clone of the fork), scratchpad clone
  remotes; nothing in the tree.

**Interfaces:**
- Produces: fork `jhoblitt/rados-rs` with `main` mirroring upstream; local
  clone with remotes `origin` (fork, SSH) and `upstream`
  (`https://github.com/tchaikov/rados-rs.git`); scratchpad clone with the
  same remotes and a working offline build.

- [ ] **Step 1: Create the fork (unsandboxed, needs gh credentials)**

```bash
gh repo fork tchaikov/rados-rs --clone=false
gh api -X POST repos/jhoblitt/rados-rs/rulesets --input \
  /home/jhoblitt/.claude-personal/plugins/cache/conventions-claude/github-conventions/1.2.1/skills/github-conventions/templates/ruleset.json
gh repo view jhoblitt/rados-rs --json isFork,parent --jq '"fork=\(.isFork) parent=\(.parent.nameWithOwner)"'
```
Expected: `fork=true parent=tchaikov/rados-rs`.

- [ ] **Step 2: Canonical clone with both remotes (unsandboxed)**

```bash
git clone -q git@github.com:jhoblitt/rados-rs.git /home/jhoblitt/github/rados-rs
cd /home/jhoblitt/github/rados-rs
git remote add upstream https://github.com/tchaikov/rados-rs.git
git fetch -q upstream
git branch --set-upstream-to=upstream/main main
git log --oneline -1 upstream/main
```
Expected: the same head as the fork's `main`.

- [ ] **Step 3: Point the scratchpad clone at the same remotes (sandboxed)**

The scratchpad clone at
`/tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs`
is where every worker edits and builds; it is shallow, so unshallow it
from the local clone.

```bash
cd /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs
git remote set-url origin /home/jhoblitt/github/rados-rs
git fetch -q --unshallow origin
git branch -f main origin/main
```

- [ ] **Step 4: Offline build environment (fetch unsandboxed, rest sandboxed)**

```bash
cd /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs
mkdir -p .cargo-home .cargo
printf '[env]\nCCACHE_DISABLE = "1"\n' > .cargo/config.toml
echo '.cargo-home/' >> .git/info/exclude
echo '.cargo/config.toml' >> .git/info/exclude
export CARGO_HOME=$PWD/.cargo-home
cargo fetch            # unsandboxed: crates.io is off the allowlist
cargo check --workspace --all-targets --offline
cargo test --workspace --lib --offline 2>&1 | grep 'test result' | tail -1
```
Expected: check clean; the lib tests pass (upstream's own count).

- [ ] **Step 5: Push the design branch to the fork (unsandboxed)**

```bash
cd /home/jhoblitt/github/rados-rs
git fetch -q /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs design/rgw-mvp:design/rgw-mvp
git push -q origin design/rgw-mvp
```

### Task 1: Local Ceph v19 cluster and harness check

**Files:**
- Create: `$TMPDIR/ceph/ceph.conf`, `$TMPDIR/ceph/ceph.client.admin.keyring` (outside the tree).

**Interfaces:**
- Produces: a running mon, mgr and memstore OSD on `127.0.0.1` (ports 6789
  and 6800 to 7000), a `test-pool`, and `CEPH_CONF` pointing at a config
  whose `[global]` carries the keyring path, exactly as CI's
  `test-with-ceph.yml` arranges it.

- [ ] **Step 1: Install podman-compose in a session venv (unsandboxed, needs PyPI)**

```bash
python3 -m venv "$TMPDIR/pc-venv" && "$TMPDIR/pc-venv/bin/pip" -q install podman-compose
"$TMPDIR/pc-venv/bin/podman-compose" version | head -1
```

- [ ] **Step 2: Start the cluster (unsandboxed, needs the podman socket)**

```bash
cd /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs/docker
"$TMPDIR/pc-venv/bin/podman-compose" -f docker-compose.ceph.yml up -d
for i in $(seq 1 60); do
  podman exec ceph-mon ceph -s 2>/dev/null | grep -qE 'HEALTH_OK|HEALTH_WARN' && break
  sleep 3
done
podman exec ceph-mon ceph -s | head -12
podman exec ceph-mon ceph osd pool create test-pool 8
```
Expected: `osd: 1 osds: 1 up`, and the pool created.

- [ ] **Step 3: Export the client config (unsandboxed for the exec, then sandboxed)**

```bash
mkdir -p "$TMPDIR/ceph"
podman exec ceph-mon cat /etc/ceph/ceph.conf > "$TMPDIR/ceph/ceph.conf"
podman exec ceph-mon cat /etc/ceph/ceph.client.admin.keyring > "$TMPDIR/ceph/ceph.client.admin.keyring"
sed -i '/^\[global\]/a keyring = '"$TMPDIR"'/ceph/ceph.client.admin.keyring' "$TMPDIR/ceph/ceph.conf"
```

- [ ] **Step 4: Prove the harness with an existing cluster test (sandboxed)**

```bash
cd /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs
CARGO_HOME=$PWD/.cargo-home CEPH_CONF=$TMPDIR/ceph/ceph.conf \
  cargo test -p rados --offline --test osdclient_rados_operations -- --ignored --nocapture 2>&1 | grep 'test result'
```
Expected: `test result: ok.` with a non-zero pass count. If the sandbox
refuses the loopback connection, run the same command with the sandbox
off and record that in the task notes; do not change the code.

### Task 2: `Denc` for `BTreeMap`

**Files:**
- Modify: `rados/src/denc/codec.rs` (after the `BTreeSet` impl, near line 808)
- Test: same file's `#[cfg(test)]` module

**Interfaces:**
- Produces: `impl<K: Denc + Ord, V: Denc> Denc for BTreeMap<K, V>`, wire
  format `u32 count` then `key, value` pairs, the same as
  `std::map` in `include/encoding.h`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `rados/src/denc/codec.rs`:

```rust
#[test]
fn btreemap_encodes_as_count_then_pairs() {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, Bytes> = BTreeMap::new();
    map.insert("a".to_owned(), Bytes::from_static(b"x"));
    let mut buf = BytesMut::new();
    map.encode(&mut buf, 0).expect("encode");
    assert_eq!(
        buf.as_ref(),
        &[1, 0, 0, 0, 1, 0, 0, 0, b'a', 1, 0, 0, 0, b'x'][..]
    );
    let back: BTreeMap<String, Bytes> = Denc::decode(&mut buf.freeze(), 0).expect("decode");
    assert_eq!(back, map);
    assert_eq!(map.encoded_size(0), Some(14));
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib btreemap_encodes -- --nocapture`
Expected: compile error, `Denc` is not implemented for `BTreeMap`.

- [ ] **Step 3: Implement**

Insert after the `BTreeSet` impl:

```rust
impl<K: Denc + Ord, V: Denc> Denc for std::collections::BTreeMap<K, V> {
    fn encode<B: BufMut>(&self, buf: &mut B, features: u64) -> Result<(), RadosError> {
        Denc::encode(&(self.len() as u32), buf, features)?;
        for (key, value) in self {
            Denc::encode(key, buf, features)?;
            Denc::encode(value, buf, features)?;
        }
        Ok(())
    }

    fn decode<B: Buf>(buf: &mut B, features: u64) -> Result<Self, RadosError> {
        let len = <u32 as Denc>::decode(buf, features)? as usize;
        let mut map = std::collections::BTreeMap::new();
        for _ in 0..len {
            let key = <K as Denc>::decode(buf, features)?;
            let value = <V as Denc>::decode(buf, features)?;
            map.insert(key, value);
        }
        Ok(map)
    }

    fn encoded_size(&self, features: u64) -> Option<usize> {
        let mut size = 4;
        for (key, value) in self {
            size += Denc::encoded_size(key, features)?;
            size += Denc::encoded_size(value, features)?;
        }
        Some(size)
    }
}
```

- [ ] **Step 4: Run the test and the gates**

Run: `CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib btreemap_encodes && cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --offline -- --no-deps -D warnings`
Expected: PASS, no diagnostics.

- [ ] **Step 5: Commit**

```bash
git switch -c omap-ops main
git add rados/src/denc/codec.rs
git commit -m "denc: encode and decode BTreeMap as a Ceph map

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 3: omap op codes, constructors and reply decoders

**Files:**
- Modify: `rados/src/osdclient/types.rs` (the `OpCode` enum, after `Rollback`)
- Create: `rados/src/osdclient/omap.rs`
- Modify: `rados/src/osdclient/mod.rs` (`pub mod omap;` and a `pub use`)
- Test: `#[cfg(test)]` module inside `omap.rs`

**Interfaces:**
- Consumes: `OSDOp { op, flags, op_data, indata }`, `OpData::{None, Extent}`,
  `OpReply { return_code, outdata }`, `Denc` for `Bytes`, `u64`, `i32`,
  `BTreeSet`, `BTreeMap` (Task 2).
- Produces:
  - `OpCode::{OmapGetKeys, OmapGetVals, OmapGetHeader, OmapGetValsByKeys, OmapSetVals, OmapSetHeader, OmapClear, OmapRmKeys, OmapCmp, OmapRmKeyRange}`
  - `pub type OmapKey = Bytes; pub type OmapMap = BTreeMap<OmapKey, Bytes>; pub type OmapKeySet = BTreeSet<OmapKey>;`
  - `pub struct OmapKeys { pub keys: OmapKeySet, pub more: bool }`
  - `pub struct OmapVals { pub vals: OmapMap, pub more: bool }`
  - `pub enum CmpOp { Eq = 1, Ne = 2, Gt = 3, Gte = 4, Lt = 5, Lte = 6 }`
  - `pub struct OmapAssertion { pub value: Bytes, pub op: CmpOp }` (`Denc`)
  - `impl OSDOp { pub fn omap_get_keys(start_after: &[u8], max_return: u64) -> Result<Self>; pub fn omap_get_vals(start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<Self>; pub fn omap_get_vals_by_keys(keys: &OmapKeySet) -> Result<Self>; pub fn omap_get_header() -> Self; pub fn omap_set(vals: &OmapMap) -> Result<Self>; pub fn omap_set_header(header: Bytes) -> Self; pub fn omap_clear() -> Self; pub fn omap_rm_keys(keys: &OmapKeySet) -> Result<Self>; pub fn omap_rm_range(begin: &[u8], end: &[u8]) -> Result<Self>; pub fn omap_cmp(assertions: &BTreeMap<OmapKey, OmapAssertion>) -> Result<Self> }`
  - `pub fn decode_omap_keys(reply: &OpReply) -> Result<OmapKeys>; pub fn decode_omap_vals(reply: &OpReply) -> Result<OmapVals>; pub fn decode_omap_vals_by_keys(reply: &OpReply) -> Result<OmapMap>`
  - `Result` here is `crate::osdclient::error::Result`.

- [ ] **Step 1: Write the failing tests** (`omap.rs` is created with the tests first; the module will not compile until Step 3)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::denc::Denc;

    #[test]
    fn get_vals_encodes_start_after_max_then_prefix() {
        let op = OSDOp::omap_get_vals(b"k", 7, b"p").expect("op");
        assert_eq!(op.op, OpCode::OmapGetVals);
        assert!(matches!(op.op_data, OpData::None));
        assert_eq!(
            op.indata.as_ref(),
            &[1, 0, 0, 0, b'k', 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, b'p'][..]
        );
    }

    #[test]
    fn set_carries_extent_of_indata_length() {
        let mut vals = OmapMap::new();
        vals.insert(Bytes::from_static(b"a"), Bytes::from_static(b"xy"));
        let op = OSDOp::omap_set(&vals).expect("op");
        assert_eq!(op.op, OpCode::OmapSetVals);
        assert_eq!(op.indata.as_ref(), &[1, 0, 0, 0, 1, 0, 0, 0, b'a', 2, 0, 0, 0, b'x', b'y'][..]);
        assert!(matches!(
            op.op_data,
            OpData::Extent { offset: 0, length: 15, truncate_size: 0, truncate_seq: 0 }
        ));
    }

    #[test]
    fn keys_are_bytes_and_survive_non_utf8() {
        let key = Bytes::from_static(&[0x80, b'v', b'1']);
        let mut keys = OmapKeySet::new();
        keys.insert(key.clone());
        let mut buf = bytes::BytesMut::new();
        keys.encode(&mut buf, 0).expect("encode");
        let reply = OpReply { return_code: 0, outdata: {
            let mut b = bytes::BytesMut::from(buf.as_ref());
            b.extend_from_slice(&[1]); // more = true
            b.freeze()
        } };
        let decoded = decode_omap_keys(&reply).expect("decode");
        assert!(decoded.more);
        assert!(decoded.keys.contains(&key));
    }

    #[test]
    fn assertion_encodes_value_then_op() {
        let a = OmapAssertion { value: Bytes::from_static(b"v"), op: CmpOp::Gte };
        let mut buf = bytes::BytesMut::new();
        a.encode(&mut buf, 0).expect("encode");
        assert_eq!(buf.as_ref(), &[1, 0, 0, 0, b'v', 4, 0, 0, 0][..]);
        let back = OmapAssertion::decode(&mut buf.freeze(), 0).expect("decode");
        assert_eq!(back, a);
    }

    #[test]
    fn decode_vals_reads_map_then_more() {
        let reply = OpReply {
            return_code: 0,
            outdata: Bytes::from_static(&[1, 0, 0, 0, 1, 0, 0, 0, b'a', 1, 0, 0, 0, b'x', 0]),
        };
        let vals = decode_omap_vals(&reply).expect("decode");
        assert!(!vals.more);
        assert_eq!(vals.vals.get(&Bytes::from_static(b"a")), Some(&Bytes::from_static(b"x")));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib omap:: 2>&1 | head -5`
Expected: compile errors for the missing items.

- [ ] **Step 3: Implement**

Add to the `OpCode` enum in `rados/src/osdclient/types.rs`, after `Rollback`, keeping the file's comment style (each line names the `rados.h` entry):

```rust
    /// CEPH_OSD_OP_OMAPGETKEYS
    OmapGetKeys = osd_op!(RD, DATA, 17),
    /// CEPH_OSD_OP_OMAPGETVALS
    OmapGetVals = osd_op!(RD, DATA, 18),
    /// CEPH_OSD_OP_OMAPGETHEADER
    OmapGetHeader = osd_op!(RD, DATA, 19),
    /// CEPH_OSD_OP_OMAPGETVALSBYKEYS
    OmapGetValsByKeys = osd_op!(RD, DATA, 20),
    /// CEPH_OSD_OP_OMAPSETVALS
    OmapSetVals = osd_op!(WR, DATA, 21),
    /// CEPH_OSD_OP_OMAPSETHEADER
    OmapSetHeader = osd_op!(WR, DATA, 22),
    /// CEPH_OSD_OP_OMAPCLEAR
    OmapClear = osd_op!(WR, DATA, 23),
    /// CEPH_OSD_OP_OMAPRMKEYS
    OmapRmKeys = osd_op!(WR, DATA, 24),
    /// CEPH_OSD_OP_OMAP_CMP
    OmapCmp = osd_op!(RD, DATA, 25),
    /// CEPH_OSD_OP_OMAPRMKEYRANGE
    OmapRmKeyRange = osd_op!(WR, DATA, 44),
```

Create `rados/src/osdclient/omap.rs` above the tests module:

```rust
//! omap operations: the key/value store every RADOS object carries.
//!
//! Wire formats follow `ObjectOperation` in `src/osdc/Objecter.h`: read
//! ops carry their arguments in `indata` with an empty op union, write ops
//! carry `indata` and an extent union of `(0, indata.len())` as
//! `add_data` builds it. Replies for the listing ops carry the entries and
//! then a `more` flag.

use std::collections::{BTreeMap, BTreeSet};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::denc::{Denc, RadosError};
use crate::osdclient::error::Result;
use crate::osdclient::types::{OSDOp, OpCode, OpData, OpReply};

/// An omap key. Ceph treats keys as byte strings and RGW's bucket index
/// uses bytes outside UTF-8, so keys are not `String`.
pub type OmapKey = Bytes;
pub type OmapMap = BTreeMap<OmapKey, Bytes>;
pub type OmapKeySet = BTreeSet<OmapKey>;

/// Reply to `omap_get_keys`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OmapKeys {
    pub keys: OmapKeySet,
    /// More keys remain after the last one returned.
    pub more: bool,
}

/// Reply to `omap_get_vals`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OmapVals {
    pub vals: OmapMap,
    /// More entries remain after the last one returned.
    pub more: bool,
}

/// `CEPH_OSD_CMPXATTR_OP_*`, shared by `omap_cmp` and `cmpxattr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CmpOp {
    Eq = 1,
    Ne = 2,
    Gt = 3,
    Gte = 4,
    Lt = 5,
    Lte = 6,
}

impl TryFrom<i32> for CmpOp {
    type Error = RadosError;

    fn try_from(value: i32) -> std::result::Result<Self, RadosError> {
        Ok(match value {
            1 => Self::Eq,
            2 => Self::Ne,
            3 => Self::Gt,
            4 => Self::Gte,
            5 => Self::Lt,
            6 => Self::Lte,
            other => return Err(RadosError::Protocol(format!("invalid cmp op {other}"))),
        })
    }
}

/// One entry of an `omap_cmp` assertion map: `std::pair<bufferlist, int>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmapAssertion {
    pub value: Bytes,
    pub op: CmpOp,
}

impl Denc for OmapAssertion {
    fn encode<B: BufMut>(&self, buf: &mut B, features: u64) -> std::result::Result<(), RadosError> {
        self.value.encode(buf, features)?;
        (self.op as i32).encode(buf, features)
    }

    fn decode<B: Buf>(buf: &mut B, features: u64) -> std::result::Result<Self, RadosError> {
        let value = Bytes::decode(buf, features)?;
        let op = CmpOp::try_from(i32::decode(buf, features)?)?;
        Ok(Self { value, op })
    }

    fn encoded_size(&self, features: u64) -> Option<usize> {
        Some(self.value.encoded_size(features)? + 4)
    }
}

fn encoded<T: Denc + ?Sized>(value: &T) -> Result<Bytes> {
    let mut buf = BytesMut::new();
    value.encode(&mut buf, 0)?;
    Ok(buf.freeze())
}

/// A read-class op: arguments in `indata`, empty union (`add_op`).
fn read_op(op: OpCode, indata: Bytes) -> OSDOp {
    OSDOp { op, flags: 0, op_data: OpData::None, indata }
}

/// A write-class op: `indata` plus an extent of its length (`add_data`).
fn data_op(op: OpCode, indata: Bytes) -> OSDOp {
    let length = indata.len() as u64;
    OSDOp {
        op,
        flags: 0,
        op_data: OpData::Extent { offset: 0, length, truncate_size: 0, truncate_seq: 0 },
        indata,
    }
}

impl OSDOp {
    /// Keys after `start_after`, at most `max_return` (0 lets the OSD choose).
    pub fn omap_get_keys(start_after: &[u8], max_return: u64) -> Result<Self> {
        let mut buf = BytesMut::new();
        Bytes::copy_from_slice(start_after).encode(&mut buf, 0)?;
        max_return.encode(&mut buf, 0)?;
        Ok(read_op(OpCode::OmapGetKeys, buf.freeze()))
    }

    /// Entries after `start_after` whose key starts with `filter_prefix`.
    pub fn omap_get_vals(start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<Self> {
        let mut buf = BytesMut::new();
        Bytes::copy_from_slice(start_after).encode(&mut buf, 0)?;
        max_return.encode(&mut buf, 0)?;
        Bytes::copy_from_slice(filter_prefix).encode(&mut buf, 0)?;
        Ok(read_op(OpCode::OmapGetVals, buf.freeze()))
    }

    pub fn omap_get_vals_by_keys(keys: &OmapKeySet) -> Result<Self> {
        Ok(read_op(OpCode::OmapGetValsByKeys, encoded(keys)?))
    }

    pub fn omap_get_header() -> Self {
        read_op(OpCode::OmapGetHeader, Bytes::new())
    }

    pub fn omap_set(vals: &OmapMap) -> Result<Self> {
        Ok(data_op(OpCode::OmapSetVals, encoded(vals)?))
    }

    /// The header is raw bytes, not length-prefixed.
    pub fn omap_set_header(header: Bytes) -> Self {
        data_op(OpCode::OmapSetHeader, header)
    }

    pub fn omap_clear() -> Self {
        read_op(OpCode::OmapClear, Bytes::new())
    }

    pub fn omap_rm_keys(keys: &OmapKeySet) -> Result<Self> {
        Ok(data_op(OpCode::OmapRmKeys, encoded(keys)?))
    }

    /// Removes keys in `[begin, end)`.
    pub fn omap_rm_range(begin: &[u8], end: &[u8]) -> Result<Self> {
        let mut buf = BytesMut::new();
        Bytes::copy_from_slice(begin).encode(&mut buf, 0)?;
        Bytes::copy_from_slice(end).encode(&mut buf, 0)?;
        Ok(data_op(OpCode::OmapRmKeyRange, buf.freeze()))
    }

    /// Fails the whole transaction with `ECANCELED` when any assertion fails.
    pub fn omap_cmp(assertions: &BTreeMap<OmapKey, OmapAssertion>) -> Result<Self> {
        Ok(read_op(OpCode::OmapCmp, encoded(assertions)?))
    }
}

pub fn decode_omap_keys(reply: &OpReply) -> Result<OmapKeys> {
    let mut buf = reply.outdata.clone();
    let keys = OmapKeySet::decode(&mut buf, 0)?;
    let more = bool::decode(&mut buf, 0)?;
    Ok(OmapKeys { keys, more })
}

pub fn decode_omap_vals(reply: &OpReply) -> Result<OmapVals> {
    let mut buf = reply.outdata.clone();
    let vals = OmapMap::decode(&mut buf, 0)?;
    let more = bool::decode(&mut buf, 0)?;
    Ok(OmapVals { vals, more })
}

pub fn decode_omap_vals_by_keys(reply: &OpReply) -> Result<OmapMap> {
    let mut buf = reply.outdata.clone();
    Ok(OmapMap::decode(&mut buf, 0)?)
}
```

`bool` must implement `Denc` as one byte; it is in the codec's impl list.
If `OpCode` derives a primitive conversion via `num_enum`, the new
variants are covered automatically; the reply decoder's `_ => OpData::None`
arm in `denc_types.rs` handles their union bytes.

Register the module in `rados/src/osdclient/mod.rs`:

```rust
pub mod omap;
pub use omap::{CmpOp, OmapAssertion, OmapKey, OmapKeySet, OmapKeys, OmapMap, OmapVals};
```

and add the same names to the `pub use osdclient::{...}` list in `rados/src/lib.rs`.

- [ ] **Step 4: Run the tests and the gates**

Run: `CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib omap:: && cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --offline -- --no-deps -D warnings`
Expected: 5 passed, no diagnostics.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/types.rs rados/src/osdclient/omap.rs rados/src/osdclient/mod.rs rados/src/lib.rs
git commit -m "osdclient: add omap op codes, constructors and reply decoders

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 4: `OpBuilder` methods

**Files:**
- Modify: `rados/src/osdclient/operation.rs` (the `impl OpBuilder` block)
- Test: the file's `#[cfg(test)]` module

**Interfaces:**
- Consumes: Task 3's constructors.
- Produces: `impl OpBuilder { pub fn omap_get_keys(self, start_after: &[u8], max_return: u64) -> Result<Self>; pub fn omap_get_vals(self, start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<Self>; pub fn omap_get_vals_by_keys(self, keys: &OmapKeySet) -> Result<Self>; pub fn omap_get_header(self) -> Self; pub fn omap_set(self, vals: &OmapMap) -> Result<Self>; pub fn omap_set_header(self, header: Bytes) -> Self; pub fn omap_clear(self) -> Self; pub fn omap_rm_keys(self, keys: &OmapKeySet) -> Result<Self>; pub fn omap_rm_range(self, begin: &[u8], end: &[u8]) -> Result<Self>; pub fn omap_cmp(self, assertions: &BTreeMap<OmapKey, OmapAssertion>) -> Result<Self> }`. Read methods set `OsdOpFlags::READ`, write methods `OsdOpFlags::WRITE`, as `stat` and `create` do; `omap_cmp` sets `READ` because a transaction of only assertions is a read.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn omap_builder_sets_flags_and_keeps_order() {
    let mut vals = crate::osdclient::omap::OmapMap::new();
    vals.insert(Bytes::from_static(b"k"), Bytes::from_static(b"v"));
    let built = OpBuilder::new()
        .omap_set(&vals)
        .expect("set")
        .omap_get_header()
        .build();
    assert!(built.is_write());
    assert!(built.is_read());
    let ops = built.into_ops();
    assert_eq!(ops[0].op, OpCode::OmapSetVals);
    assert_eq!(ops[1].op, OpCode::OmapGetHeader);
}
```

- [ ] **Step 2: Run to see it fail**

Run: `CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib omap_builder`
Expected: compile error, no method `omap_set`.

- [ ] **Step 3: Implement**, inside `impl OpBuilder`, after `rollback`:

```rust
    pub fn omap_get_keys(mut self, start_after: &[u8], max_return: u64) -> Result<Self> {
        self.ops.push(OSDOp::omap_get_keys(start_after, max_return)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }

    pub fn omap_get_vals(mut self, start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<Self> {
        self.ops.push(OSDOp::omap_get_vals(start_after, max_return, filter_prefix)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }

    pub fn omap_get_vals_by_keys(mut self, keys: &OmapKeySet) -> Result<Self> {
        self.ops.push(OSDOp::omap_get_vals_by_keys(keys)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }

    pub fn omap_get_header(mut self) -> Self {
        self.ops.push(OSDOp::omap_get_header());
        self.flags |= OsdOpFlags::READ;
        self
    }

    pub fn omap_set(mut self, vals: &OmapMap) -> Result<Self> {
        self.ops.push(OSDOp::omap_set(vals)?);
        self.flags |= OsdOpFlags::WRITE;
        Ok(self)
    }

    pub fn omap_set_header(mut self, header: Bytes) -> Self {
        self.ops.push(OSDOp::omap_set_header(header));
        self.flags |= OsdOpFlags::WRITE;
        self
    }

    pub fn omap_clear(mut self) -> Self {
        self.ops.push(OSDOp::omap_clear());
        self.flags |= OsdOpFlags::WRITE;
        self
    }

    pub fn omap_rm_keys(mut self, keys: &OmapKeySet) -> Result<Self> {
        self.ops.push(OSDOp::omap_rm_keys(keys)?);
        self.flags |= OsdOpFlags::WRITE;
        Ok(self)
    }

    pub fn omap_rm_range(mut self, begin: &[u8], end: &[u8]) -> Result<Self> {
        self.ops.push(OSDOp::omap_rm_range(begin, end)?);
        self.flags |= OsdOpFlags::WRITE;
        Ok(self)
    }

    pub fn omap_cmp(mut self, assertions: &BTreeMap<OmapKey, OmapAssertion>) -> Result<Self> {
        self.ops.push(OSDOp::omap_cmp(assertions)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }
```

with `use crate::osdclient::omap::{OmapAssertion, OmapKey, OmapKeySet, OmapMap};`, `use crate::osdclient::error::Result;` and `use std::collections::BTreeMap;` at the top of the file if not present.

- [ ] **Step 4: Run the test and the gates** (same commands as Task 3 step 4, filter `omap_builder`)

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/operation.rs
git commit -m "osdclient: omap methods on OpBuilder

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 5: `IoCtx` conveniences

**Files:**
- Modify: `rados/src/osdclient/ioctx.rs` (after `list_xattrs`)

**Interfaces:**
- Consumes: Tasks 3 and 4; `IoCtx::execute`, `OSDClient::check_op_result`.
- Produces: `impl IoCtx { pub async fn omap_get_keys(&self, oid: impl Into<String>, start_after: &[u8], max_return: u64) -> Result<OmapKeys>; pub async fn omap_get_vals(&self, oid, start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<OmapVals>; pub async fn omap_get_vals_by_keys(&self, oid, keys: &OmapKeySet) -> Result<OmapMap>; pub async fn omap_get_header(&self, oid) -> Result<Bytes>; pub async fn omap_set(&self, oid, vals: &OmapMap) -> Result<()>; pub async fn omap_set_header(&self, oid, header: Bytes) -> Result<()>; pub async fn omap_clear(&self, oid) -> Result<()>; pub async fn omap_rm_keys(&self, oid, keys: &OmapKeySet) -> Result<()>; pub async fn omap_rm_range(&self, oid, begin: &[u8], end: &[u8]) -> Result<()> }`. No `omap_cmp` convenience: an assertion is only meaningful inside a compound op.

- [ ] **Step 1: Implement** (no unit test is possible without a cluster; Task 6 covers these):

```rust
    pub async fn omap_get_keys(&self, oid: impl Into<String>, start_after: &[u8], max_return: u64) -> Result<OmapKeys> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_get_keys(start_after, max_return)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_get_keys")?;
        decode_omap_keys(first_reply(&result)?)
    }

    pub async fn omap_get_vals(&self, oid: impl Into<String>, start_after: &[u8], max_return: u64, filter_prefix: &[u8]) -> Result<OmapVals> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_get_vals(start_after, max_return, filter_prefix)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_get_vals")?;
        decode_omap_vals(first_reply(&result)?)
    }

    pub async fn omap_get_vals_by_keys(&self, oid: impl Into<String>, keys: &OmapKeySet) -> Result<OmapMap> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_get_vals_by_keys(keys)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_get_vals_by_keys")?;
        decode_omap_vals_by_keys(first_reply(&result)?)
    }

    pub async fn omap_get_header(&self, oid: impl Into<String>) -> Result<Bytes> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_get_header().build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_get_header")?;
        Ok(result.first_outdata()?.clone())
    }

    pub async fn omap_set(&self, oid: impl Into<String>, vals: &OmapMap) -> Result<()> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_set(vals)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_set")
    }

    pub async fn omap_set_header(&self, oid: impl Into<String>, header: Bytes) -> Result<()> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_set_header(header).build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_set_header")
    }

    pub async fn omap_clear(&self, oid: impl Into<String>) -> Result<()> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_clear().build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_clear")
    }

    pub async fn omap_rm_keys(&self, oid: impl Into<String>, keys: &OmapKeySet) -> Result<()> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_rm_keys(keys)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_rm_keys")
    }

    pub async fn omap_rm_range(&self, oid: impl Into<String>, begin: &[u8], end: &[u8]) -> Result<()> {
        let oid = oid.into();
        let op = OpBuilder::new().omap_rm_range(begin, end)?.build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "omap_rm_range")
    }
```

plus, at module level in `ioctx.rs`:

```rust
fn first_reply(result: &OpResult) -> Result<&OpReply> {
    result
        .ops
        .first()
        .ok_or_else(|| OSDClientError::Other("no op reply".into()))
}
```

and the imports `use crate::osdclient::omap::{decode_omap_keys, decode_omap_vals, decode_omap_vals_by_keys, OmapKeySet, OmapKeys, OmapMap, OmapVals};` and `use crate::osdclient::types::{OpReply, OpResult};` (merge with existing `use` lines; `OSDClient` and `OSDClientError` are already imported for `exec`). If `check_op_result` also inspects the first op's return code, a non-zero per-op code surfaces there as `OSDError`, which is the behavior these methods want.

- [ ] **Step 2: Gates**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --offline -- --no-deps -D warnings && CARGO_HOME=$PWD/.cargo-home cargo test -p rados --offline --lib`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add rados/src/osdclient/ioctx.rs
git commit -m "osdclient: omap conveniences on IoCtx

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 6: Cluster tests and CI wiring

**Files:**
- Create: `rados/tests/osdclient_omap_operations.rs`
- Modify: `.github/workflows/test-with-ceph.yml` (both `for test in ...` lists)

**Interfaces:**
- Consumes: `common::{init_tracing, create_ioctx}` from `rados/tests/common/mod.rs`, Tasks 3 to 5.

- [ ] **Step 1: Write the tests**

```rust
//! omap cluster tests. Run with:
//!   CEPH_CONF=... cargo test -p rados --test osdclient_omap_operations -- --ignored --nocapture

mod common;

use std::collections::BTreeMap;

use bytes::Bytes;
use common::create_ioctx;
use rados::osdclient::omap::{CmpOp, OmapAssertion, OmapKeySet, OmapMap};
use rados::osdclient::OSDClientError;
use rados::OpBuilder;

const ECANCELED: i32 = 125;
const ENOENT: i32 = 2;

fn key(s: &str) -> Bytes {
    Bytes::copy_from_slice(s.as_bytes())
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_nanos())
}

fn sample() -> OmapMap {
    let mut m = OmapMap::new();
    for (k, v) in [("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")] {
        m.insert(key(k), key(v));
    }
    m
}

#[tokio::test]
#[ignore]
async fn set_get_rm_roundtrip() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-roundtrip");

    ioctx.omap_set(&oid, &sample()).await.expect("omap_set");
    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert_eq!(vals.vals, sample());
    assert!(!vals.more);

    let keys = ioctx.omap_get_keys(&oid, b"", 0).await.expect("get_keys");
    assert_eq!(keys.keys, sample().into_keys().collect::<OmapKeySet>());

    let mut want = OmapKeySet::new();
    want.insert(key("b"));
    want.insert(key("zzz"));
    let by = ioctx.omap_get_vals_by_keys(&oid, &want).await.expect("by_keys");
    assert_eq!(by.len(), 1);
    assert_eq!(by.get(&key("b")), Some(&key("2")));

    let mut rm = OmapKeySet::new();
    rm.insert(key("a"));
    rm.insert(key("missing"));
    ioctx.omap_rm_keys(&oid, &rm).await.expect("rm_keys tolerates missing keys");
    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert!(!vals.vals.contains_key(&key("a")));
    assert_eq!(vals.vals.len(), 3);

    ioctx.omap_clear(&oid).await.expect("clear");
    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert!(vals.vals.is_empty());
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn get_vals_pages_with_more() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-paging");
    ioctx.omap_set(&oid, &sample()).await.expect("omap_set");

    let page = ioctx.omap_get_vals(&oid, b"", 2, b"").await.expect("page 1");
    assert_eq!(page.vals.len(), 2);
    assert!(page.more);
    let last = page.vals.keys().next_back().expect("last key").clone();

    let rest = ioctx.omap_get_vals(&oid, &last, 0, b"").await.expect("page 2");
    assert_eq!(rest.vals.len(), 2);
    assert!(!rest.more);
    assert!(rest.vals.keys().all(|k| k > &last));

    let filtered = ioctx.omap_get_vals(&oid, b"", 0, b"c").await.expect("prefix");
    assert_eq!(filtered.vals.keys().cloned().collect::<Vec<_>>(), vec![key("c")]);
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn keys_are_bytes_not_strings() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-bytes");
    let raw = Bytes::from_static(&[0x80, b'0', b'_', b'v']);
    let mut m = OmapMap::new();
    m.insert(raw.clone(), key("versioned"));
    ioctx.omap_set(&oid, &m).await.expect("omap_set");
    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert_eq!(vals.vals.get(&raw), Some(&key("versioned")));
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn header_roundtrip() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-header");
    ioctx.omap_set_header(&oid, Bytes::from_static(b"hdr")).await.expect("set_header");
    assert_eq!(ioctx.omap_get_header(&oid).await.expect("get_header"), Bytes::from_static(b"hdr"));
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn rm_range_end_is_exclusive() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-range");
    ioctx.omap_set(&oid, &sample()).await.expect("omap_set");
    ioctx.omap_rm_range(&oid, b"b", b"d").await.expect("rm_range");
    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert_eq!(vals.vals.keys().cloned().collect::<Vec<_>>(), vec![key("a"), key("d")]);
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn cmp_failure_aborts_transaction() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-cmp");
    ioctx.omap_set(&oid, &sample()).await.expect("omap_set");

    let mut assertions = BTreeMap::new();
    assertions.insert(key("a"), OmapAssertion { value: key("1"), op: CmpOp::Eq });
    let mut update = OmapMap::new();
    update.insert(key("e"), key("5"));
    let ok = OpBuilder::new().omap_cmp(&assertions).expect("cmp").omap_set(&update).expect("set").build();
    ioctx.execute_op(&oid, ok).await.expect("assertion holds, set applied");

    let mut bad = BTreeMap::new();
    bad.insert(key("a"), OmapAssertion { value: key("wrong"), op: CmpOp::Eq });
    let mut update = OmapMap::new();
    update.insert(key("f"), key("6"));
    let failing = OpBuilder::new().omap_cmp(&bad).expect("cmp").omap_set(&update).expect("set").build();
    let err = ioctx.execute_op(&oid, failing).await.expect_err("assertion fails");
    assert!(matches!(err, OSDClientError::OSDError { code, .. } if code == -ECANCELED), "{err:?}");

    let vals = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("get_vals");
    assert!(vals.vals.contains_key(&key("e")));
    assert!(!vals.vals.contains_key(&key("f")), "a failed assertion must abort the whole transaction");
    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn read_on_missing_object_is_enoent() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-missing");
    let err = ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect_err("missing object");
    assert!(matches!(err, OSDClientError::OSDError { code, .. } if code == -ENOENT), "{err:?}");
}

#[tokio::test]
#[ignore]
async fn compound_with_data_and_xattr() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("omap-compound");
    let mut m = OmapMap::new();
    m.insert(key("k"), key("v"));
    let op = OpBuilder::new()
        .write_full(Bytes::from_static(b"payload"))
        .op(rados::osdclient::types::OSDOp::set_xattr("user.x", Bytes::from_static(b"1")).expect("xattr"))
        .omap_set(&m)
        .expect("set")
        .build();
    ioctx.execute_op(&oid, op).await.expect("compound write");
    assert_eq!(ioctx.get_xattr(&oid, "user.x").await.expect("xattr"), Bytes::from_static(b"1"));
    assert_eq!(ioctx.omap_get_vals(&oid, b"", 0, b"").await.expect("vals").vals, m);
    ioctx.remove(&oid).await.expect("remove");
}
```

`ioctx.execute_op(oid, BuiltOp)` is the public form of the private
`execute`; if `IoCtx` does not expose one, add in Task 5:

```rust
    /// Execute a built (possibly compound) operation on `oid`.
    pub async fn execute_op(&self, oid: impl Into<String>, op: BuiltOp) -> Result<OpResult> {
        let oid = oid.into();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "execute_op")?;
        Ok(result)
    }
```

and include it in that task's commit. The negative code convention
(`-ECANCELED`) matches the crate's existing `OSDError` handling of
`ENOENT`; if `check_op_result` reports the per-op code positive, adjust
the two assertions to the crate's convention and say so in the task notes.

- [ ] **Step 2: Run them against the local cluster**

Run: `CARGO_HOME=$PWD/.cargo-home CEPH_CONF=$TMPDIR/ceph/ceph.conf cargo test -p rados --offline --test osdclient_omap_operations -- --ignored --nocapture 2>&1 | grep -E '^test |test result'`
Expected: 8 passed. A failing wire assumption shows up here as a decode
error or an `EINVAL` from the OSD; fix the constructor, not the test.

- [ ] **Step 3: Wire into CI**

In `.github/workflows/test-with-ceph.yml`, add `osdclient_omap_operations` to
both `for test in ...` lists (the CRC-enabled and CRC-disabled runs).

- [ ] **Step 4: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features --offline -- --no-deps -D warnings
git add rados/tests/osdclient_omap_operations.rs .github/workflows/test-with-ceph.yml
git commit -m "tests: omap cluster tests

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 7: Branch review, push, draft PR

**Files:** none new.

- [ ] **Step 1: History check** — `git log --oneline main..omap-ops` shows five commits (map denc, op codes and module, builder, IoCtx, tests); no fixups; each builds on its own (`git rebase -x 'cargo check -p rados --offline' main` is the proof, run with `CARGO_HOME` set).

- [ ] **Step 2: Push and open the draft PR (unsandboxed)**

```bash
cd /home/jhoblitt/github/rados-rs
git fetch -q /tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs omap-ops:omap-ops
git push -q origin omap-ops
gh pr create --repo jhoblitt/rados-rs --draft --assignee @me --base main --head omap-ops \
  --title "osdclient: omap operations" \
  --body "$(cat <<'MSG'
**Motivation.** RGW composes omap writes with data and xattr ops in one transaction and pages omap listings; the client had no omap ops.

**What changed.** The ten `OMAP*` op codes with `OpBuilder` and `IoCtx` methods, `Denc` for `BTreeMap`, reply decoders with the `more` flag, and cluster tests covering paging, byte keys, range removal and a failed `omap_cmp` aborting its transaction.

**Notable decisions.** Keys are `Bytes`, not `String`: Ceph treats omap keys as byte strings and RGW's bucket index uses bytes outside UTF-8.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
MSG
)"
```

- [ ] **Step 3: Watch CI** on a three-minute poll until every check, including the Ceph integration job, is green; a red check from this branch is fixed on the branch and force-pushed with lease.

---

## Roadmap for later plans

Each later package gets its own plan file, written just before it runs and
reviewed the same way, in this order: `cmpxattr-small-ops`; `watch-notify`;
`cls-crate` (with `version` and `refcount`); `cls-user`; `cls-queue-gc`;
`cls-rgw-types`; `cls-rgw-bucket-index`; `cls-rgw-gc`; `cls-rgw-usage`;
`cls-rgw-lc`; `cls-rgw-olh`. The `integration` branch is created after the
first two transport packages exist and is re-merged after every package.
Check-in with the owner after every three PRs.
