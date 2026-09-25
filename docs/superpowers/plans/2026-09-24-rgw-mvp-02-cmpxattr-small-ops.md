# rados-rs RGW MVP, plan 2 of N: `cmpxattr-small-ops`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the second transport package as an upstream-shaped branch:
`cmpxattr` (the guard RGW puts on every overwrite), `assert_exists`,
`zero`, `set_alloc_hint` and `list_watchers`, with unit, encoding and
cluster tests, and merge it into the fork's `main` once its CI is green.

**Architecture:** Four new `OpCode` variants and one new `OpData` variant
in `types.rs`, with their union bytes in `denc_types.rs`; `OSDOp`
constructors that follow `ObjectOperation` in `src/osdc/Objecter.h`
(`add_xattr_cmp`, `add_data` with an empty buffer, `add_alloc_hint` plus
the `FAILOK` per-op flag, and a bare `add_op` for `STAT` and
`LIST_WATCHERS`); `OpBuilder` and `IoCtx` methods on top; a new
`watchers.rs` holding the `watch_item_t` and `obj_list_watch_response_t`
encodings. One behavioural fix comes first: the OSD returns the truth
value of a `cmpxattr` as a positive result, and the crate's result check
treated any nonzero code as an error.

**Tech Stack:** Rust 2024 (upstream `rust-version` 1.88; the machine has
1.98), tokio, `bytes`, `bitflags`, the crate's own `Denc` and
`VersionedEncode`; podman for the local Ceph v19.2.2 cluster; `gh` for the
fork.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
component "cmpxattr and small ops". Its "bulk `getxattrs` decodes a map"
item already landed on `xattr-raw-name` as `IoCtx::get_xattrs`, so it is
not repeated here.

## Global Constraints

- Ceph floor is Squid (v19): new `decode_content` bodies call
  `check_min_version!` at the version Squid emits; no branches for older
  formats; cluster tests run only against v19.2.2.
- Encoding goes through `Denc`; raw `put_*`/`get_*` only inside `Denc`
  impls or `encode_content`/`decode_content`.
- No `unwrap` or `expect` on production paths; tests may use `expect` and
  `unwrap`.
- Gates on every task: `cargo test -p rados --lib --offline` green;
  before the push, in the tooling container: `cargo fmt --all --check`
  clean and `cargo clippy --workspace --all-targets --offline -- -D
  warnings -D clippy::uninlined_format_args` with zero diagnostics (the
  branch carries the `clippy-1.98` fix, so there is no baseline to
  subtract); cluster tests with `-- --ignored` against the local cluster.
- rustfmt and clippy are not installed on the host: implementers keep
  lines under 100 columns and match the surrounding style; the controller
  runs the container gate and folds any reflow into the owning commit.
- Branch `cmpxattr-small-ops` is based on `omap-ops` (`bec893e`), because
  its cluster tests need `IoCtx::execute_op` and the xattr fixes; the PR
  describes the stack. A draft PR against the fork's `main` for CI and
  review, merged there once green; the upstream PR waits for the owner
  (four are open).
- Commit subjects follow upstream's `<module>: <imperative summary>`
  style, one logical change per commit, body explaining the wire fact the
  change rests on, and end with the
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer after a
  blank line. Commits are made with
  `git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit`.
- Builds use the scratchpad clone's project-local `CARGO_HOME` with
  `--offline`; every implementer starts with the two lines
  `export R=<scratchpad>/rados-rs` and `export CARGO_HOME=$R/.cargo-home`
  (two statements, not one), then `cd $R`.

## Review Focus

1. A `cmpxattr` that holds returns 1, as the op's return code and, in a
   read-only request, as the overall result (`PrimaryLogPG.cc`
   `do_cmp_xattr` returns the comparison's bool; nothing resets it). The
   crate must treat it as success. Pinned in Task 1 (unit) and Task 7
   (`cmpxattr_match_returns_one`).
2. A missing attribute compares as the empty string, not as `ENODATA`:
   `cmpxattr(name, Eq, "")` asserts absence, which is how RGW guards OLH
   initialisation. Pinned in Task 7 (`cmpxattr_missing_xattr_compares_as_empty`).
3. The supplied value is the left-hand side of every comparison
   (`value <op> stored`), the reverse of `omap_cmp`; and U64 mode reads its
   two sides differently: the operand travels as a little-endian u64, the
   stored attribute is parsed as decimal text, and text that does not start
   with a number fails the request with `EINVAL`. Pinned in
   Task 2 (unit, operand bytes) and Task 7 (`cmpxattr_u64_compares_decimal_text`).
4. `zero` on a missing object is a silent no-op: no `ENOENT`, no object
   created. A caller using it as an existence probe is wrong. Pinned in
   Task 7 (`zero_clears_a_range_and_ignores_missing_objects`).
5. `set_alloc_hint` creates the object when it does not exist and carries
   the `FAILOK` per-op flag. Pinned in Task 4 (unit, flag and union bytes)
   and Task 7 (`set_alloc_hint_creates_the_object`).

Also pinned: `assert_exists` in a write request on a missing object fails
with `ENOENT` and creates nothing (Task 7). Not pinned here: a
`list_watchers` reply with a live watcher, which needs the watch op the
`watch-notify` package adds; its cluster test lists watchers there.

---

### Task 0: Branch, workspace, cluster check (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cmpxattr-small-ops` at `bec893e` checked out in the
  scratchpad clone `$R`; the SDD ledger for this plan; the three Ceph
  containers confirmed up.

- [ ] **Step 1: Check out the branch**

```bash
export R=/tmp/claude-1000/-home-jhoblitt-github-ceph/25f4e5fb-a25a-47b9-ae27-bd24e00c9744/scratchpad/rados-rs
cd $R && git status --short && git checkout -q -b cmpxattr-small-ops bec893e && git log --oneline -1
```
Expected: a clean tree and `bec893e tests: omap cluster tests`.

- [ ] **Step 2: Confirm the cluster**

```bash
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
ls /tmp/ceph/ceph.conf
```
Expected: `ceph-mon`, `ceph-mgr`, `ceph-osd` all `Up`. If they are gone,
bring them back with the podman-compose project plan 1's Task 1 created
(`/tmp/pc-venv/bin/podman-compose`), then re-create `test-pool` and
regenerate `/tmp/ceph/ceph.conf` as that task did.

- [ ] **Step 3: Create the SDD workspace and ledger**

Run the `sdd-workspace` script against the plan copy in
`$R/.superpowers/plan-copies/` (the docs live on `design/rgw-mvp`, not on
this branch), and record in the ledger: the base `bec893e`, the branch
name, and the rulings inherited from plan 1 that still apply (controller
runs environment tasks; container fmt/clippy gate; cluster tests run
unsandboxed; `execute_op` is the compound entry point).

---

### Task 1: Treat positive op results as success

**Files:**
- Modify: `rados/src/osdclient/client.rs:994-1016` (`check_op_result`)
  and append a test module at the end of the file (after line 2615).
- Modify: `rados/src/osdclient/ioctx.rs:840-845` (the `execute_op` doc).

**Interfaces:**
- Consumes: `OpResult { result: i32, version: u64, ops: Vec<OpReply>,
  redirect: Option<RequestRedirect> }`, `OpReply { return_code: i32,
  outdata: Bytes }`, `OSDClientError::OSDError { code: i32, message: String }`.
- Produces: `OSDClient::check_op_result(&OpResult, &str) -> Result<()>`
  returning `Ok` for any overall result `>= 0` and any first-op return
  code `>= 0`. Every later task's cluster tests depend on this.

- [ ] **Step 1: Write the failing tests**

Append to `rados/src/osdclient/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::OSDClient;
    use crate::osdclient::error::OSDClientError;
    use crate::osdclient::types::{OpReply, OpResult};
    use bytes::Bytes;

    fn result(overall: i32, first: i32) -> OpResult {
        OpResult {
            result: overall,
            version: 0,
            ops: vec![OpReply {
                return_code: first,
                outdata: Bytes::new(),
            }],
            redirect: None,
        }
    }

    #[test]
    fn check_op_result_treats_positive_codes_as_success() {
        // Ceph is errno-shaped: only a negative code is a failure. A
        // holding CMPXATTR returns 1.
        assert!(OSDClient::check_op_result(&result(0, 0), "t").is_ok());
        assert!(OSDClient::check_op_result(&result(1, 1), "t").is_ok());
        assert!(OSDClient::check_op_result(&result(0, 1), "t").is_ok());
    }

    #[test]
    fn check_op_result_reports_negative_codes() {
        let err = OSDClient::check_op_result(&result(-2, 0), "t").expect_err("overall");
        assert!(matches!(err, OSDClientError::OSDError { code: -2, .. }));
        let err = OSDClient::check_op_result(&result(0, -125), "t").expect_err("per-op");
        assert!(matches!(err, OSDClientError::OSDError { code: -125, .. }));
    }
}
```

- [ ] **Step 2: Run the tests to verify the first one fails**

Run: `cargo test -p rados --lib --offline check_op_result`
Expected: `check_op_result_treats_positive_codes_as_success` FAILS on the
`result(1, 1)` assertion; the other test passes.

- [ ] **Step 3: Change the check**

Replace the body of `check_op_result` in `client.rs` (lines 994-1016) with:

```rust
    /// Check an OpResult for errors.
    ///
    /// Ceph's result codes are errno-shaped: a negative code is a failure,
    /// zero or positive is success. Some ops report success with a positive
    /// code (`CMPXATTR` returns 1 when the comparison holds), so this tests
    /// `< 0`, not `!= 0`, on the overall result and on the first op's
    /// return code.
    pub(crate) fn check_op_result(
        result: &crate::osdclient::types::OpResult,
        op_name: &str,
    ) -> Result<()> {
        if result.result < 0 {
            return Err(OSDClientError::OSDError {
                code: result.result,
                message: format!("{op_name} failed"),
            });
        }
        if let Some(op) = result.ops.first()
            && op.return_code < 0
        {
            return Err(OSDClientError::OSDError {
                code: op.return_code,
                message: format!("{op_name} failed"),
            });
        }
        Ok(())
    }
```

In `ioctx.rs`, change the `execute_op` doc's second paragraph to:

```rust
    /// Only checks the overall result and the first op's return code (via
    /// `check_op_result`, where a negative code is the failure); return
    /// codes of later ops in a compound operation are the caller's to
    /// inspect in the returned [`OpResult`].
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass, including both new ones.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/client.rs rados/src/osdclient/ioctx.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: treat positive op results as success

Ceph's op results are errno-shaped: negative is failure, zero or positive
is success. PrimaryLogPG returns the truth value of a CMPXATTR as the op's
result, so a holding comparison comes back as 1, in a read-only request
as the overall result too. check_op_result tested for nonzero and turned
that success into an OSDError.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 2: `cmpxattr`

**Files:**
- Modify: `rados/src/osdclient/types.rs` (constants after line 466, the
  `OpCode` enum after `ListXattrs` at line 534, `xattr_op` at lines
  943-970 and its three callers, new constructors after `remove_xattr`,
  tests after `set_xattr_indata_is_raw_name_then_value` at line 1547).
- Modify: `rados/src/osdclient/denc_types.rs:347` (the xattr decode arm)
  and its test module.
- Modify: `rados/src/osdclient/operation.rs` (imports at line 30, new
  methods after `omap_cmp` at line 349, test module).

**Interfaces:**
- Consumes: `CmpOp` from `crate::osdclient::omap` (`#[repr(i32)]`, `Eq=1
  .. Lte=6`), `OSDOp::xattr_op`, `OpData::Xattr { name_len, value_len,
  cmp_op, cmp_mode }`, `crate::encode_with_capacity`.
- Produces: `OpCode::CmpXattr` (0x1303); `OSDOp::cmpxattr(name: impl
  Into<String>, op: CmpOp, value: Bytes) -> Result<OSDOp, OSDClientError>`;
  `OSDOp::cmpxattr_u64(name, op: CmpOp, value: u64) -> Result<OSDOp,
  OSDClientError>`; `OpBuilder::cmpxattr(self, name, op, value: Bytes) ->
  Result<Self>` and `OpBuilder::cmpxattr_u64(self, name, op, value: u64)
  -> Result<Self>`, both READ. Task 7 uses the builder methods.

- [ ] **Step 1: Write the failing unit tests**

In `types.rs`'s `mod tests`, after `set_xattr_indata_is_raw_name_then_value`
(`use super::*;` brings in `CmpOp` through the module-level import added in
Step 3, so add no import to the test module):

```rust
    #[test]
    fn cmpxattr_sets_cmp_fields_and_raw_indata() {
        let op = OSDOp::cmpxattr("user.tag", CmpOp::Eq, Bytes::from_static(b"t1")).unwrap();
        assert_eq!(op.op, OpCode::CmpXattr);
        assert_eq!(&op.indata[..], b"user.tagt1");
        assert!(matches!(
            op.op_data,
            OpData::Xattr {
                name_len: 8,
                value_len: 2,
                cmp_op: 1,
                cmp_mode: 1
            }
        ));
    }

    #[test]
    fn cmpxattr_u64_sends_a_little_endian_value() {
        let op = OSDOp::cmpxattr_u64("user.ver", CmpOp::Gte, 5).unwrap();
        assert_eq!(&op.indata[..], b"user.ver\x05\0\0\0\0\0\0\0");
        assert!(matches!(
            op.op_data,
            OpData::Xattr {
                name_len: 8,
                value_len: 8,
                cmp_op: 4,
                cmp_mode: 2
            }
        ));
    }

    #[test]
    fn small_op_opcodes_match_rados_h() {
        // __CEPH_OSD_OP(mode, type, nr) from ceph/src/include/rados.h:
        // mode RD 0x1000 / WR 0x2000, type DATA 0x0200 / ATTR 0x0300.
        assert_eq!(OpCode::CmpXattr as u16, 0x1303); // (RD, ATTR, 3)
    }
```

In `denc_types.rs`'s `mod tests`:

```rust
    #[test]
    fn cmpxattr_union_roundtrips() {
        use crate::osdclient::omap::CmpOp;
        use crate::osdclient::types::{OSDOp, OpCode, OpData};
        use bytes::Bytes;

        let op = OSDOp::cmpxattr("user.tag", CmpOp::Eq, Bytes::from_static(b"t1")).unwrap();
        let mut buf = BytesMut::new();
        op.encode(&mut buf, 0).unwrap();
        assert_eq!(buf.len(), CEPH_OSD_OP_SIZE);
        assert_eq!(&buf[..6], &[0x03, 0x13, 0, 0, 0, 0]); // op 0x1303, flags 0
        assert_eq!(&buf[6..16], &[8, 0, 0, 0, 2, 0, 0, 0, 1, 1]); // xattr union
        assert!(buf[16..34].iter().all(|b| *b == 0)); // union padding
        assert_eq!(&buf[34..], &[10, 0, 0, 0]); // payload_len

        let decoded = OSDOp::decode(&mut buf, 0).unwrap();
        assert_eq!(decoded.op, OpCode::CmpXattr);
        assert!(matches!(
            decoded.op_data,
            OpData::Xattr {
                name_len: 8,
                value_len: 2,
                cmp_op: 1,
                cmp_mode: 1
            }
        ));
    }
```

In `operation.rs`'s `mod tests` (add `use crate::osdclient::omap::CmpOp;`
and `use bytes::Bytes;` to its imports if `use super::*;` does not already
bring them in):

```rust
    #[test]
    fn cmpxattr_builder_is_a_read() {
        let built = OpBuilder::new()
            .cmpxattr("user.tag", CmpOp::Eq, Bytes::from_static(b"t"))
            .expect("cmp")
            .write_full(Bytes::from_static(b"x"))
            .build();
        assert!(built.is_read());
        assert!(built.is_write());
        let ops = built.into_ops();
        assert_eq!(ops[0].op, OpCode::CmpXattr);
        assert_eq!(ops[1].op, OpCode::WriteFull);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados --lib --offline cmpxattr`
Expected: FAIL to compile: no variant `CmpXattr`, no function `cmpxattr`.

- [ ] **Step 3: Implement the op code, constants and constructors**

In `types.rs`, after `const CEPH_OSD_OP_TYPE_PG` (line 466):

```rust
/// `CEPH_OSD_CMPXATTR_MODE_STRING`: the OSD compares the stored attribute
/// and the supplied value as byte strings.
const CEPH_OSD_CMPXATTR_MODE_STRING: u8 = 1;
/// `CEPH_OSD_CMPXATTR_MODE_U64`: the OSD parses the stored attribute as
/// decimal text and reads the supplied value as a little-endian u64.
const CEPH_OSD_CMPXATTR_MODE_U64: u8 = 2;
```

In the `OpCode` enum, after `ListXattrs`:

```rust
    /// Compare an extended attribute: __CEPH_OSD_OP(RD, ATTR, 3) = CMPXATTR
    CmpXattr = osd_op!(RD, ATTR, 3),
```

Add `use crate::osdclient::omap::CmpOp;` to the `use` block at the top of
`types.rs`.

Change `xattr_op` to take the comparison fields (Objecter's
`add_xattr_cmp` is `add_xattr` plus these two assignments):

```rust
    fn xattr_op(
        op: OpCode,
        name: String,
        value: Option<Bytes>,
        cmp_op: u8,
        cmp_mode: u8,
    ) -> Result<Self, crate::osdclient::error::OSDClientError> {
        use bytes::BytesMut;

        let mut buf = BytesMut::with_capacity(name.len() + value.as_ref().map_or(0, |v| v.len()));
        buf.extend_from_slice(name.as_bytes());

        let value_len = value.as_ref().map_or(0, |v| v.len() as u32);

        if let Some(ref v) = value {
            buf.extend_from_slice(v);
        }

        Ok(Self {
            op,
            flags: 0,
            op_data: OpData::Xattr {
                name_len: name.len() as u32,
                value_len,
                cmp_op,
                cmp_mode,
            },
            indata: buf.freeze(),
        })
    }
```

and its three callers pass `0, 0`:

```rust
        Self::xattr_op(OpCode::GetXattr, name.into(), None, 0, 0)
```
```rust
        Self::xattr_op(OpCode::SetXattr, name.into(), Some(value), 0, 0)
```
```rust
        Self::xattr_op(OpCode::RemoveXattr, name.into(), None, 0, 0)
```

After `remove_xattr`, add:

```rust
    /// Compare an extended attribute with `value` as byte strings, as
    /// `ObjectOperation::cmpxattr(name, op, bufferlist)` does.
    ///
    /// The OSD evaluates `value <op> current`: the supplied operand is the
    /// left-hand side, so `CmpOp::Gt` holds when `value` exceeds the stored
    /// attribute (`do_cmp_xattr` in `PrimaryLogPG.cc`). A missing attribute
    /// compares as the empty string, so `CmpOp::Eq` with an empty `value`
    /// asserts absence. When the comparison holds the op's return code is
    /// 1; when it does not, the whole request fails with `ECANCELED` and
    /// none of its writes apply.
    pub fn cmpxattr(
        name: impl Into<String>,
        op: CmpOp,
        value: Bytes,
    ) -> Result<Self, crate::osdclient::error::OSDClientError> {
        Self::xattr_op(
            OpCode::CmpXattr,
            name.into(),
            Some(value),
            op as u8,
            CEPH_OSD_CMPXATTR_MODE_STRING,
        )
    }

    /// Compare an extended attribute with `value` as unsigned integers, as
    /// `ObjectOperation::cmpxattr(name, op, uint64_t)` does.
    ///
    /// The OSD parses the stored attribute as decimal text (missing or
    /// empty is 0; text that does not start with a decimal number, or that
    /// overflows u64, fails the request with `EINVAL`) and receives `value`
    /// as a little-endian u64. Operand order and result codes are those of
    /// [`Self::cmpxattr`].
    pub fn cmpxattr_u64(
        name: impl Into<String>,
        op: CmpOp,
        value: u64,
    ) -> Result<Self, crate::osdclient::error::OSDClientError> {
        let value = crate::encode_with_capacity(&value, 0)?;
        Self::xattr_op(
            OpCode::CmpXattr,
            name.into(),
            Some(value),
            op as u8,
            CEPH_OSD_CMPXATTR_MODE_U64,
        )
    }
```

In `denc_types.rs`, widen the xattr decode arm so every xattr op code
decodes its union (the reply path decodes each op's header):

```rust
            OpCode::GetXattr
            | OpCode::SetXattr
            | OpCode::RemoveXattr
            | OpCode::ListXattrs
            | OpCode::CmpXattr => {
```

In `operation.rs`, change the omap import to
`use crate::osdclient::omap::{CmpOp, OmapAssertion, OmapKey, OmapKeySet, OmapMap};`
and add after `omap_cmp`:

```rust
    /// Add a cmpxattr assertion on a byte-string attribute; the request
    /// fails with `ECANCELED` when it does not hold. RGW guards every
    /// overwrite with one on the object's id tag.
    pub fn cmpxattr(mut self, name: impl Into<String>, op: CmpOp, value: Bytes) -> Result<Self> {
        self.ops.push(OSDOp::cmpxattr(name, op, value)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }

    /// Add a cmpxattr assertion on an integer attribute; see
    /// [`OSDOp::cmpxattr_u64`] for how the OSD reads each side.
    pub fn cmpxattr_u64(mut self, name: impl Into<String>, op: CmpOp, value: u64) -> Result<Self> {
        self.ops.push(OSDOp::cmpxattr_u64(name, op, value)?);
        self.flags |= OsdOpFlags::READ;
        Ok(self)
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass, including the four new ones.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/types.rs rados/src/osdclient/denc_types.rs rados/src/osdclient/operation.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: add cmpxattr

CMPXATTR is a GETXATTR-shaped op whose union carries the comparison
operator and mode, as Objecter's add_xattr_cmp sets them. The string
form mirrors librados's bufferlist overload. The u64 form encodes the
operand as a little-endian u64, which the OSD compares against the
stored attribute parsed as decimal text (PrimaryLogPG::do_xattr_cmp_u64).

The reply decoder's xattr arm now covers every xattr op code, not only
GETXATTR and SETXATTR.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 3: `zero`

**Files:**
- Modify: `rados/src/osdclient/types.rs` (the `OpCode` enum after
  `Truncate` at line 516, a constructor after `delete` at line 743, the
  test module).
- Modify: `rados/src/osdclient/denc_types.rs:318-323` (the extent decode
  arm) and its test module.
- Modify: `rados/src/osdclient/operation.rs` (a method after `create` at
  line 184, test module).
- Modify: `rados/src/osdclient/ioctx.rs` (a method after `write`, which
  ends at line 276).

**Interfaces:**
- Consumes: `OpData::Extent { offset, length, truncate_size, truncate_seq }`,
  `WriteResult { version: u64 }`, `IoCtx::execute`.
- Produces: `OpCode::Zero` (0x2204); `OSDOp::zero(offset: u64, length:
  u64) -> OSDOp`; `OpBuilder::zero(self, offset: u64, length: u64) -> Self`
  (WRITE); `IoCtx::zero(&self, oid: &str, offset: u64, length: u64) ->
  Result<WriteResult>`. Task 7 uses `IoCtx::zero`.

- [ ] **Step 1: Write the failing unit tests**

In `types.rs`'s `mod tests`:

```rust
    #[test]
    fn zero_carries_an_extent_and_no_indata() {
        let op = OSDOp::zero(2, 3);
        assert_eq!(op.op, OpCode::Zero);
        assert!(op.indata.is_empty());
        assert!(matches!(
            op.op_data,
            OpData::Extent {
                offset: 2,
                length: 3,
                truncate_size: 0,
                truncate_seq: 0
            }
        ));
    }
```

and in `small_op_opcodes_match_rados_h` add:

```rust
        assert_eq!(OpCode::Zero as u16, 0x2204); // (WR, DATA, 4)
```

In `denc_types.rs`'s `mod tests`:

```rust
    #[test]
    fn zero_union_roundtrips() {
        use crate::osdclient::types::{OSDOp, OpCode, OpData};

        let op = OSDOp::zero(2, 3);
        let mut buf = BytesMut::new();
        op.encode(&mut buf, 0).unwrap();
        assert_eq!(buf.len(), CEPH_OSD_OP_SIZE);
        assert_eq!(&buf[..6], &[0x04, 0x22, 0, 0, 0, 0]); // op 0x2204, flags 0
        assert_eq!(&buf[6..14], &2u64.to_le_bytes());
        assert_eq!(&buf[14..22], &3u64.to_le_bytes());

        let decoded = OSDOp::decode(&mut buf, 0).unwrap();
        assert_eq!(decoded.op, OpCode::Zero);
        assert!(matches!(
            decoded.op_data,
            OpData::Extent {
                offset: 2,
                length: 3,
                truncate_size: 0,
                truncate_seq: 0
            }
        ));
    }
```

In `operation.rs`'s `mod tests`:

```rust
    #[test]
    fn zero_builder_is_a_write() {
        let built = OpBuilder::new().zero(0, 4096).build();
        assert!(built.is_write());
        assert!(!built.is_read());
        assert_eq!(built.into_ops()[0].op, OpCode::Zero);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados --lib --offline zero`
Expected: FAIL to compile: no variant `Zero`, no function `zero`.

- [ ] **Step 3: Implement**

In the `OpCode` enum, after `Truncate`:

```rust
    /// Zero a byte range: __CEPH_OSD_OP(WR, DATA, 4)
    Zero = osd_op!(WR, DATA, 4),
```

After `OSDOp::delete`:

```rust
    /// Zero the bytes `[offset, offset + length)`, as `ObjectOperation::zero`
    /// does: an extent union and no data. The OSD treats a missing object
    /// as a no-op, not as an error, and does not create it.
    pub fn zero(offset: u64, length: u64) -> Self {
        Self {
            op: OpCode::Zero,
            flags: 0,
            op_data: OpData::Extent {
                offset,
                length,
                truncate_size: 0,
                truncate_seq: 0,
            },
            indata: Bytes::new(),
        }
    }
```

In `denc_types.rs`, add `| OpCode::Zero` to the extent arm:

```rust
            OpCode::Read
            | OpCode::Write
            | OpCode::WriteFull
            | OpCode::Truncate
            | OpCode::Append
            | OpCode::Zero
            | OpCode::Stat => {
```

In `operation.rs`, after `create`:

```rust
    /// Add a zero operation (clear `length` bytes from `offset`).
    pub fn zero(mut self, offset: u64, length: u64) -> Self {
        self.ops.push(OSDOp::zero(offset, length));
        self.flags |= OsdOpFlags::WRITE;
        self
    }
```

In `ioctx.rs`, after `write`:

```rust
    /// Zero the bytes `[offset, offset + length)` of an object
    ///
    /// Like [`write`](Self::write) this does not truncate. A missing object
    /// stays missing: the OSD treats the op as a no-op, not as `ENOENT`.
    pub async fn zero(&self, oid: &str, offset: u64, length: u64) -> Result<WriteResult> {
        debug!("Zeroing {} bytes of object {} at offset {}", length, oid, offset);

        let op = OpBuilder::new().zero(offset, length).build();
        let result = self.execute(oid, op).await?;
        OSDClient::check_op_result(&result, "zero")?;
        Ok(WriteResult {
            version: result.version,
        })
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/types.rs rados/src/osdclient/denc_types.rs rados/src/osdclient/operation.rs rados/src/osdclient/ioctx.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: add the ZERO op

ZERO is add_data with an empty buffer in Objecter: an extent union of the
range and no payload. PrimaryLogPG zeroes the range of an existing object
and silently does nothing for a missing one, so the op never reports
ENOENT and never creates an object.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 4: `set_alloc_hint`

**Files:**
- Modify: `rados/src/osdclient/types.rs` (a bitflags block after
  `OsdOpFlags` ends at line 166, a per-op flag constant after line 466,
  the `OpCode` enum after `Create` at line 526, the `OpData` enum after
  `AssertVer` at line 619, a constructor after `assert_version` at line
  839, the test module).
- Modify: `rados/src/osdclient/denc_types.rs` (an encode arm after
  `OpData::AssertVer` at line 290, a decode arm after `OpCode::AssertVer`
  at line 374, the test module).
- Modify: `rados/src/osdclient/operation.rs` (imports at line 31, a
  method after `zero`, test module).
- Modify: `rados/src/osdclient/ioctx.rs` (imports at lines 20-22, a
  method after `zero`).
- Modify: `rados/src/osdclient/mod.rs:46-49` and `rados/src/lib.rs:41-47`
  (re-exports).

**Interfaces:**
- Consumes: `bitflags::bitflags!` as `OsdOpFlags` uses it; the
  28-byte union padding convention in `denc_types.rs`.
- Produces: `AllocHintFlags` (bitflags u32: `SEQUENTIAL_WRITE = 1` ..
  `LOG = 1024`); `OpCode::SetAllocHint` (0x2223); `OpData::AllocHint {
  expected_object_size: u64, expected_write_size: u64, flags: u32 }`;
  `OSDOp::set_alloc_hint(expected_object_size: u64, expected_write_size:
  u64, flags: AllocHintFlags) -> OSDOp` with per-op `flags == 0x2`
  (FAILOK); `OpBuilder::set_alloc_hint(self, u64, u64, AllocHintFlags) ->
  Self` (WRITE); `IoCtx::set_alloc_hint(&self, oid: &str, u64, u64,
  AllocHintFlags) -> Result<()>`. `AllocHintFlags` is re-exported as
  `rados::AllocHintFlags`. Task 7 uses the builder and `IoCtx` methods.

- [ ] **Step 1: Write the failing unit tests**

In `types.rs`'s `mod tests`:

```rust
    #[test]
    fn set_alloc_hint_carries_failok_and_the_hint() {
        let op = OSDOp::set_alloc_hint(4096, 512, AllocHintFlags::INCOMPRESSIBLE);
        assert_eq!(op.op, OpCode::SetAllocHint);
        assert_eq!(op.flags, 0x2); // CEPH_OSD_OP_FLAG_FAILOK, as Objecter sets it
        assert!(op.indata.is_empty());
        assert!(matches!(
            op.op_data,
            OpData::AllocHint {
                expected_object_size: 4096,
                expected_write_size: 512,
                flags: 512
            }
        ));
    }
```

and in `small_op_opcodes_match_rados_h` add:

```rust
        assert_eq!(OpCode::SetAllocHint as u16, 0x2223); // (WR, DATA, 35)
```

In `denc_types.rs`'s `mod tests`:

```rust
    #[test]
    fn alloc_hint_union_roundtrips() {
        use crate::osdclient::types::{AllocHintFlags, OSDOp, OpCode, OpData};

        let op = OSDOp::set_alloc_hint(4096, 512, AllocHintFlags::INCOMPRESSIBLE);
        let mut buf = BytesMut::new();
        op.encode(&mut buf, 0).unwrap();
        assert_eq!(buf.len(), CEPH_OSD_OP_SIZE);
        assert_eq!(&buf[..6], &[0x23, 0x22, 2, 0, 0, 0]); // op 0x2223, FAILOK
        assert_eq!(&buf[6..14], &4096u64.to_le_bytes());
        assert_eq!(&buf[14..22], &512u64.to_le_bytes());
        assert_eq!(&buf[22..26], &512u32.to_le_bytes());
        assert!(buf[26..34].iter().all(|b| *b == 0)); // union padding
        assert_eq!(&buf[34..], &[0, 0, 0, 0]); // payload_len

        let decoded = OSDOp::decode(&mut buf, 0).unwrap();
        assert_eq!(decoded.op, OpCode::SetAllocHint);
        assert_eq!(decoded.flags, 2);
        assert!(matches!(
            decoded.op_data,
            OpData::AllocHint {
                expected_object_size: 4096,
                expected_write_size: 512,
                flags: 512
            }
        ));
    }
```

In `operation.rs`'s `mod tests`:

```rust
    #[test]
    fn set_alloc_hint_builder_is_a_write() {
        let built = OpBuilder::new()
            .set_alloc_hint(0, 0, AllocHintFlags::INCOMPRESSIBLE)
            .write_full(Bytes::from_static(b"x"))
            .build();
        assert!(built.is_write());
        assert!(!built.is_read());
        let ops = built.into_ops();
        assert_eq!(ops[0].op, OpCode::SetAllocHint);
        assert_eq!(ops[1].op, OpCode::WriteFull);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados --lib --offline alloc_hint`
Expected: FAIL to compile: no type `AllocHintFlags`, no variant
`SetAllocHint`.

- [ ] **Step 3: Implement**

In `types.rs`, after the `OsdOpFlags` bitflags block:

```rust
bitflags::bitflags! {
    /// `CEPH_OSD_ALLOC_HINT_FLAG_*` from `rados.h`: what the writer expects
    /// of an object's access pattern, sent with `set_alloc_hint`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct AllocHintFlags: u32 {
        const SEQUENTIAL_WRITE = 1;
        const RANDOM_WRITE = 2;
        const SEQUENTIAL_READ = 4;
        const RANDOM_READ = 8;
        const APPEND_ONLY = 16;
        const IMMUTABLE = 32;
        const SHORTLIVED = 64;
        const LONGLIVED = 128;
        const COMPRESSIBLE = 256;
        const INCOMPRESSIBLE = 512;
        const LOG = 1024;
    }
}
```

After the cmp-mode constants Task 2 added:

```rust
/// `CEPH_OSD_OP_FLAG_FAILOK`, a per-op flag: the OSD carries on past this
/// op's failure instead of failing the request.
const CEPH_OSD_OP_FLAG_FAILOK: u32 = 0x2;
```

In the `OpCode` enum, after `Create`:

```rust
    /// Allocation hint: __CEPH_OSD_OP(WR, DATA, 35) = SETALLOCHINT
    SetAllocHint = osd_op!(WR, DATA, 35),
```

In the `OpData` enum, after `AssertVer`:

```rust
    /// Allocation hint (`ceph_osd_op.alloc_hint`)
    AllocHint {
        expected_object_size: u64,
        expected_write_size: u64,
        flags: u32,
    },
```

After `OSDOp::assert_version`:

```rust
    /// Hint the OSD about the object's expected size and access pattern, as
    /// `ObjectOperation::set_alloc_hint` does.
    ///
    /// The op carries `CEPH_OSD_OP_FLAG_FAILOK`, as Objecter sets it: an
    /// OSD that rejects the hint does not fail the request. The OSD creates
    /// the object if it does not exist. `0` for either size means no
    /// expectation.
    pub fn set_alloc_hint(
        expected_object_size: u64,
        expected_write_size: u64,
        flags: AllocHintFlags,
    ) -> Self {
        Self {
            op: OpCode::SetAllocHint,
            flags: CEPH_OSD_OP_FLAG_FAILOK,
            op_data: OpData::AllocHint {
                expected_object_size,
                expected_write_size,
                flags: flags.bits(),
            },
            indata: Bytes::new(),
        }
    }
```

In `denc_types.rs`, an encode arm after `OpData::AssertVer`:

```rust
            OpData::AllocHint {
                expected_object_size,
                expected_write_size,
                flags,
            } => {
                buf.put_u64_le(*expected_object_size);
                buf.put_u64_le(*expected_write_size);
                buf.put_u32_le(*flags);
                // Pad to CEPH_OSD_OP_UNION_SIZE: 8 + 8 + 4 = 20, need 8 more
                buf.put_u64_le(0);
            }
```

and a decode arm after `OpCode::AssertVer`:

```rust
            OpCode::SetAllocHint => {
                // alloc_hint: u64 expected_object_size + u64 expected_write_size
                // + u32 flags + 8 bytes padding
                let expected_object_size = buf.get_u64_le();
                let expected_write_size = buf.get_u64_le();
                let flags = buf.get_u32_le();
                buf.advance(8);
                OpData::AllocHint {
                    expected_object_size,
                    expected_write_size,
                    flags,
                }
            }
```

In `operation.rs`, change the types import to
`use crate::osdclient::types::{AllocHintFlags, OSDOp, OsdOpFlags};` and add
after `zero`:

```rust
    /// Add a set_alloc_hint operation; see [`OSDOp::set_alloc_hint`].
    pub fn set_alloc_hint(
        mut self,
        expected_object_size: u64,
        expected_write_size: u64,
        flags: AllocHintFlags,
    ) -> Self {
        self.ops.push(OSDOp::set_alloc_hint(
            expected_object_size,
            expected_write_size,
            flags,
        ));
        self.flags |= OsdOpFlags::WRITE;
        self
    }
```

In `ioctx.rs`, add `AllocHintFlags` to the `crate::osdclient::types`
import and add after `zero`:

```rust
    /// Hint the OSD about an object's expected size and access pattern, as
    /// `rados_set_alloc_hint2` does. Creates the object if it does not exist.
    pub async fn set_alloc_hint(
        &self,
        oid: &str,
        expected_object_size: u64,
        expected_write_size: u64,
        flags: AllocHintFlags,
    ) -> Result<()> {
        debug!(
            "Setting alloc hint on object {} in pool {}: {:?}",
            oid, self.pool_id, flags
        );

        let op = OpBuilder::new()
            .set_alloc_hint(expected_object_size, expected_write_size, flags)
            .build();
        let result = self.execute(oid, op).await?;
        OSDClient::check_op_result(&result, "set_alloc_hint")?;
        Ok(())
    }
```

Re-export: add `AllocHintFlags` to `pub use types::{...}` in
`osdclient/mod.rs` and to `pub use osdclient::{...}` in `lib.rs` (keep
each list alphabetical: it goes first).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/types.rs rados/src/osdclient/denc_types.rs rados/src/osdclient/operation.rs rados/src/osdclient/ioctx.rs rados/src/osdclient/mod.rs rados/src/lib.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: add SETALLOCHINT

The alloc_hint union is two u64 sizes and a u32 of
CEPH_OSD_ALLOC_HINT_FLAG_* bits. Objecter marks the op FAILOK because the
hint is advisory, so an OSD that rejects it does not fail the request;
PrimaryLogPG creates the object if it is missing. RGW sends it with
INCOMPRESSIBLE ahead of the data of every compressed object.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 5: `assert_exists`

**Files:**
- Modify: `rados/src/osdclient/operation.rs` (a method after `stat` at
  line 170, test module).

**Interfaces:**
- Consumes: `OpBuilder::stat(self) -> Self`.
- Produces: `OpBuilder::assert_exists(self) -> Self`. Task 7 uses it.

- [ ] **Step 1: Write the failing unit test**

In `operation.rs`'s `mod tests`:

```rust
    #[test]
    fn assert_exists_is_a_stat() {
        let built = OpBuilder::new()
            .assert_exists()
            .write_full(Bytes::from_static(b"x"))
            .build();
        assert!(built.is_write());
        assert!(built.is_read());
        let ops = built.into_ops();
        assert_eq!(ops[0].op, OpCode::Stat);
        assert_eq!(ops[1].op, OpCode::WriteFull);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p rados --lib --offline assert_exists`
Expected: FAIL to compile: no method `assert_exists`.

- [ ] **Step 3: Implement**

After `OpBuilder::stat`:

```rust
    /// Assert that the object exists, as `ObjectOperation::assert_exists`
    /// does: a `STAT` whose `ENOENT` fails the whole request before any
    /// write in it applies. RGW uses it to guard bucket-index shard and OLH
    /// updates that must target an existing object.
    pub fn assert_exists(self) -> Self {
        self.stat()
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass.

- [ ] **Step 5: Commit**

```bash
git add rados/src/osdclient/operation.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: add assert_exists to OpBuilder

librados implements assert_exists as a STAT with no outputs: PrimaryLogPG
answers STAT on a missing object with ENOENT, which fails the request
before its writes apply.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 6: `list_watchers`

**Files:**
- Create: `rados/src/osdclient/watchers.rs`.
- Modify: `rados/src/osdclient/types.rs` (the `OpCode` enum after
  `ListSnaps` at line 555, a constructor after `list_snaps`, the opcode
  test).
- Modify: `rados/src/osdclient/mod.rs` (`pub mod watchers;` after `pub mod
  types;`, a re-export) and `rados/src/lib.rs:41-47`.
- Modify: `rados/src/osdclient/operation.rs` (a method after `list_snaps`
  at line 242, test module).
- Modify: `rados/src/osdclient/ioctx.rs` (imports, a method after
  `list_xattrs`).
- Modify: `rados/src/bin/dencoder.rs:22-26` (imports), `:137-145` (the
  type registry) and `:172-183` (the `list_types` text);
  `rados/tests/dencoder_corpus_comparison_test.rs:97-103` (`CORPUS_TYPES`).
  The corpus carries ten `watch_item_t` and two `obj_list_watch_response_t`
  samples in each of the 18.2.0 and 19.2.0 archives CI tests.

**Interfaces:**
- Consumes: `PackedEntityName` (`crate::osdclient::types`, 9-byte
  `Denc`, `PackedEntityName::new(entity_type: u8, num: u64)`);
  `EntityAddr`/`EntityAddrType` (`crate::denc`, `Denc` ignoring
  features, `EntityAddr::from_socket_addr(EntityAddrType, SocketAddr)`);
  `VersionedEncode` and `crate::denc::impl_denc_for_versioned!` as
  `ObjectLocator` in `rados/src/crush/placement.rs:150-216` uses them;
  `crate::denc::check_min_version!`; `OpResult::first_reply`.
- Produces: `OpCode::ListWatchers` (0x1209); `OSDOp::list_watchers() ->
  OSDOp`; `WatchItem { name: PackedEntityName, cookie: u64,
  timeout_seconds: u32, addr: EntityAddr }` and `ListWatchersReply {
  entries: Vec<WatchItem> }`, both `Denc`; `decode_list_watchers(&OpReply)
  -> Result<Vec<WatchItem>>`; `OpBuilder::list_watchers(self) -> Self`
  (READ); `IoCtx::list_watchers(&self, oid: impl Into<String>) ->
  Result<Vec<WatchItem>>`. `WatchItem` and `ListWatchersReply` are
  re-exported at the crate root, `Serialize` in the shape of their C++
  `dump`, and registered with the Rust dencoder and the corpus harness as
  `watch_item_t` and `obj_list_watch_response_t`. Task 7 uses
  `IoCtx::list_watchers`; Task 8 runs the corpus check.

- [ ] **Step 1: Write the failing unit tests**

Create `rados/src/osdclient/watchers.rs` with only the module doc and the
test module for now:

```rust
//! `LIST_WATCHERS`: the clients watching an object.
//!
//! The OSD answers with `obj_list_watch_response_t` from
//! `src/osd/osd_types.h`: a versioned struct (v1) holding a list of
//! `watch_item_t` (v2, compat 1), each the watcher's packed
//! `entity_name_t`, the watch cookie, the timeout in seconds and the
//! watcher's `entity_addr_t`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::denc::{EntityAddrType, encode_with_capacity};
    use crate::osdclient::types::{OpReply, PackedEntityName};
    use bytes::Bytes;

    fn addr() -> EntityAddr {
        EntityAddr::from_socket_addr(
            EntityAddrType::Msgr2,
            "127.0.0.1:6800".parse().expect("addr"),
        )
    }

    fn sample_item() -> WatchItem {
        WatchItem {
            name: PackedEntityName::new(0x08, 4242), // client.4242
            cookie: 7,
            timeout_seconds: 30,
            addr: addr(),
        }
    }

    /// `obj_list_watch_response_t` holding `sample_item`, built by hand
    /// from osd_types.h: each ENCODE_START is `struct_v, struct_compat,
    /// u32 length`; the addr bytes come from the crate's own EntityAddr
    /// encoding, which has its own tests.
    fn one_watcher_wire() -> Vec<u8> {
        let addr = encode_with_capacity(&addr(), 0).expect("addr");
        let mut item = vec![0x08]; // entity_name_t: type CLIENT
        item.extend_from_slice(&4242u64.to_le_bytes()); // num
        item.extend_from_slice(&7u64.to_le_bytes()); // cookie
        item.extend_from_slice(&30u32.to_le_bytes()); // timeout_seconds
        item.extend_from_slice(&addr);
        let mut entries = 1u32.to_le_bytes().to_vec(); // list length
        entries.extend_from_slice(&[2, 1]); // watch_item_t v2, compat 1
        entries.extend_from_slice(&(item.len() as u32).to_le_bytes());
        entries.extend_from_slice(&item);
        let mut wire = vec![1, 1]; // obj_list_watch_response_t v1, compat 1
        wire.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        wire.extend_from_slice(&entries);
        wire
    }

    fn reply(outdata: Vec<u8>) -> OpReply {
        OpReply {
            return_code: 0,
            outdata: Bytes::from(outdata),
        }
    }

    #[test]
    fn decodes_one_watcher() {
        let items = decode_list_watchers(&reply(one_watcher_wire())).expect("decode");
        assert_eq!(items, vec![sample_item()]);
    }

    #[test]
    fn decodes_no_watchers() {
        // v1, compat 1, length 4: an empty list.
        let items = decode_list_watchers(&reply(vec![1, 1, 4, 0, 0, 0, 0, 0, 0, 0]))
            .expect("decode");
        assert!(items.is_empty());
    }

    #[test]
    fn encodes_the_same_bytes() {
        let reply = ListWatchersReply {
            entries: vec![sample_item()],
        };
        let bytes = encode_with_capacity(&reply, 0).expect("encode");
        assert_eq!(bytes.as_ref(), &one_watcher_wire()[..]);
        let back = ListWatchersReply::decode(&mut bytes.clone(), 0).expect("decode");
        assert_eq!(back, reply);
    }

    #[test]
    fn rejects_a_pre_squid_watch_item() {
        // watch_item_t v1 has no addr; no supported OSD emits it.
        let mut item = vec![0x08];
        item.extend_from_slice(&1u64.to_le_bytes());
        item.extend_from_slice(&2u64.to_le_bytes());
        item.extend_from_slice(&3u32.to_le_bytes());
        let mut entries = 1u32.to_le_bytes().to_vec();
        entries.extend_from_slice(&[1, 1]);
        entries.extend_from_slice(&(item.len() as u32).to_le_bytes());
        entries.extend_from_slice(&item);
        let mut wire = vec![1, 1];
        wire.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        wire.extend_from_slice(&entries);
        assert!(decode_list_watchers(&reply(wire)).is_err());
    }

    #[test]
    fn serializes_like_ceph_dencoder() {
        // watch_item_t::dump in osd_types.h: "watcher" streams the
        // entity_name_t, "cookie" and "timeout" are ints, "addr" is the
        // entity_addr_t dump; obj_list_watch_response_t::dump wraps the
        // items in an "entries" array.
        let item = sample_item();
        let json = serde_json::to_value(&item).expect("json");
        assert_eq!(json["watcher"], serde_json::json!("client.4242"));
        assert_eq!(json["cookie"], serde_json::json!(7));
        assert_eq!(json["timeout"], serde_json::json!(30));
        assert_eq!(json["addr"], serde_json::to_value(addr()).expect("addr json"));
        assert_eq!(json.as_object().expect("object").len(), 4);

        let reply = ListWatchersReply {
            entries: vec![item],
        };
        let json = serde_json::to_value(&reply).expect("json");
        assert_eq!(json["entries"].as_array().expect("array").len(), 1);

        // entity_name_t prints a negative number as "?".
        let negative = WatchItem {
            name: PackedEntityName::new(0x08, u64::MAX),
            ..sample_item()
        };
        assert_eq!(negative.watcher_name(), "client.?");
    }
}
```

Add `pub mod watchers;` after `pub mod types;` in `osdclient/mod.rs`.

In `types.rs`'s `small_op_opcodes_match_rados_h` add:

```rust
        assert_eq!(OpCode::ListWatchers as u16, 0x1209); // (RD, DATA, 9)
```

In `operation.rs`'s `mod tests`:

```rust
    #[test]
    fn list_watchers_builder_is_a_read() {
        let built = OpBuilder::new().list_watchers().build();
        assert!(built.is_read());
        assert!(!built.is_write());
        assert_eq!(built.into_ops()[0].op, OpCode::ListWatchers);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados --lib --offline watchers`
Expected: FAIL to compile: no `WatchItem`, no `decode_list_watchers`, no
variant `ListWatchers`.

- [ ] **Step 3: Implement the op code and constructor**

In the `OpCode` enum, after `ListSnaps`:

```rust
    /// List the object's watchers: __CEPH_OSD_OP(RD, DATA, 9) = LIST_WATCHERS
    ListWatchers = osd_op!(RD, DATA, 9),
```

After `OSDOp::list_snaps`:

```rust
    /// List the clients watching an object (LIST_WATCHERS): a bare op, as
    /// Objecter's `add_op` builds it. Decode the reply with
    /// [`crate::osdclient::watchers::decode_list_watchers`].
    pub fn list_watchers() -> Self {
        Self {
            op: OpCode::ListWatchers,
            flags: 0,
            op_data: OpData::None,
            indata: Bytes::new(),
        }
    }
```

- [ ] **Step 4: Implement the types and decoder**

Insert between the module doc and the test module in `watchers.rs`:

```rust
use bytes::{Buf, BufMut};
use serde::Serialize;

use crate::denc::{Denc, EntityAddr, RadosError, VersionedEncode};
use crate::osdclient::error::Result;
use crate::osdclient::types::{OpReply, PackedEntityName};

/// One watcher of an object: `watch_item_t`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchItem {
    /// The watching client (`client.<gid>`).
    pub name: PackedEntityName,
    /// The cookie the watcher registered with.
    pub cookie: u64,
    /// The watch timeout the OSD applies, in seconds.
    pub timeout_seconds: u32,
    /// The watcher's address.
    pub addr: EntityAddr,
}

impl WatchItem {
    /// The watcher as `entity_name_t` prints it: `client.4242`, or
    /// `client.?` when the number is negative as an `int64_t`.
    pub fn watcher_name(&self) -> String {
        let entity_type =
            crate::EntityType::from_bits_truncate(u32::from(self.name.entity_type));
        let num = self.name.num.get();
        if (num as i64) < 0 {
            format!("{entity_type}.?")
        } else {
            format!("{entity_type}.{num}")
        }
    }
}

/// Matches `watch_item_t::dump` for the corpus harness: the field names
/// differ from the struct's, and the name is a streamed string.
impl Serialize for WatchItem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("WatchItem", 4)?;
        state.serialize_field("watcher", &self.watcher_name())?;
        state.serialize_field("cookie", &self.cookie)?;
        state.serialize_field("timeout", &self.timeout_seconds)?;
        state.serialize_field("addr", &self.addr)?;
        state.end()
    }
}

/// `watch_item_t` is `ENCODE_START(2, 1)`: name, cookie, timeout, and
/// from v2 the address. Squid emits v2.
impl VersionedEncode for WatchItem {
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
        self.name.encode(buf, features)?;
        self.cookie.encode(buf, features)?;
        self.timeout_seconds.encode(buf, features)?;
        self.addr.encode(buf, features)?;
        Ok(())
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        crate::denc::check_min_version!(struct_v, 2, "WatchItem", "Squid v19+");

        let name = PackedEntityName::decode(buf, features)?;
        let cookie = u64::decode(buf, features)?;
        let timeout_seconds = u32::decode(buf, features)?;
        let addr = EntityAddr::decode(buf, features)?;

        Ok(Self {
            name,
            cookie,
            timeout_seconds,
            addr,
        })
    }

    fn encoded_size_content(&self, features: u64, _version: u8) -> Option<usize> {
        Some(
            self.name.encoded_size(features)?
                + 8
                + 4
                + self.addr.encoded_size(features)?,
        )
    }
}

crate::denc::impl_denc_for_versioned!(WatchItem);

/// The reply to `LIST_WATCHERS`: `obj_list_watch_response_t`. Its
/// `dump` is an `entries` array of the items, which the derive matches.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ListWatchersReply {
    /// Every current watcher of the object.
    pub entries: Vec<WatchItem>,
}

/// `obj_list_watch_response_t` is `ENCODE_START(1, 1)` around a
/// `std::list<watch_item_t>`, which encodes as a u32 count then the items.
impl VersionedEncode for ListWatchersReply {
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
        self.entries.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        crate::denc::check_min_version!(struct_v, 1, "ListWatchersReply", "Squid v19+");

        Ok(Self {
            entries: Vec::<WatchItem>::decode(buf, features)?,
        })
    }

    fn encoded_size_content(&self, features: u64, _version: u8) -> Option<usize> {
        self.entries.encoded_size(features)
    }
}

crate::denc::impl_denc_for_versioned!(ListWatchersReply);

/// Decode the reply to `list_watchers`.
pub fn decode_list_watchers(reply: &OpReply) -> Result<Vec<WatchItem>> {
    let mut buf = reply.outdata.clone();
    Ok(ListWatchersReply::decode(&mut buf, 0)?.entries)
}
```

- [ ] **Step 5: Wire the builder, `IoCtx` and re-exports**

In `operation.rs`, after `list_snaps`:

```rust
    /// Add a list_watchers operation; decode its reply with
    /// [`crate::osdclient::watchers::decode_list_watchers`].
    pub fn list_watchers(mut self) -> Self {
        self.ops.push(OSDOp::list_watchers());
        self.flags |= OsdOpFlags::READ;
        self
    }
```

In `ioctx.rs`, add
`use crate::osdclient::watchers::{WatchItem, decode_list_watchers};` to the
imports and add after `list_xattrs`:

```rust
    /// List the clients watching an object, as `rados_list_watchers` does.
    pub async fn list_watchers(&self, oid: impl Into<String>) -> Result<Vec<WatchItem>> {
        let oid = oid.into();
        debug!(
            "Listing watchers of object '{}' in pool {}",
            oid, self.pool_id
        );

        let op = OpBuilder::new().list_watchers().build();
        let result = self.execute(&oid, op).await?;
        OSDClient::check_op_result(&result, "list_watchers")?;
        decode_list_watchers(result.first_reply()?)
    }
```

In `osdclient/mod.rs`, add `pub use watchers::{ListWatchersReply, WatchItem};`
after the `pub use types::{...}` block. In `lib.rs`, add `ListWatchersReply`
and `WatchItem` to the `pub use osdclient::{...}` list in alphabetical
position.

Register both types with the Rust dencoder so the corpus harness can
compare them with `ceph-dencoder`. In `rados/src/bin/dencoder.rs`, add
`ListWatchersReply` and `WatchItem` to the `use rados::{...}` import, add
to `get_type_info` after the `"pg_pool_t"` arm:

```rust
        "watch_item_t" => Some(type_info_denc::<WatchItem>()),
        "obj_list_watch_response_t" => Some(type_info_denc::<ListWatchersReply>()),
```

and to `list_types` after the `pg_pool_t` line:

```rust
    println!("  watch_item_t      - One watcher of an object [versioned]");
    println!("  obj_list_watch_response_t - LIST_WATCHERS reply [versioned]");
```

In `rados/tests/dencoder_corpus_comparison_test.rs`, add to `CORPUS_TYPES`
after the `pg_pool_t` entry:

```rust
    TypeSpec::new("watch_item_t", None, false),
    TypeSpec::new("obj_list_watch_response_t", None, false),
```

Then `cargo build -p rados --bin dencoder --offline` must succeed; the
harness itself needs `ceph-dencoder` and runs in Task 8.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p rados --lib --offline`
Expected: all lib tests pass, including the four watcher tests.

- [ ] **Step 7: Commit**

```bash
git add rados/src/osdclient/watchers.rs rados/src/osdclient/types.rs rados/src/osdclient/operation.rs rados/src/osdclient/ioctx.rs rados/src/osdclient/mod.rs rados/src/lib.rs rados/src/bin/dencoder.rs rados/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
osdclient: add list_watchers

LIST_WATCHERS is a bare read op; the OSD answers with
obj_list_watch_response_t (v1) holding watch_item_t entries (v2): the
watcher's entity_name_t, cookie, timeout and entity_addr_t, encoded with
the connection's features. The decoder floors watch_item_t at v2, which
every Squid OSD emits.

Both types serialize as their C++ dump does and are registered with the
dencoder and the corpus harness, whose 18.2.0 and 19.2.0 archives carry
samples of each.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 7: Cluster tests

**Files:**
- Create: `rados/tests/osdclient_small_operations.rs`.
- Modify: `.github/workflows/test-with-ceph.yml:91` and `:101` (the two
  `for test in ...` lists).

**Interfaces:**
- Consumes: everything Tasks 1-6 produce; `common::{create_ioctx,
  init_tracing}`; `IoCtx::{set_xattr, get_xattr, write_full, read, stat,
  remove, execute_op}`; `OSDOp::set_xattr`; `OpBuilder::{op, write_full}`.
- Produces: nine `#[ignore]` tests that CI runs in both the CRC-enabled
  and CRC-disabled legs.

- [ ] **Step 1: Write the tests**

Create `rados/tests/osdclient_small_operations.rs`:

```rust
//! Cluster tests for cmpxattr, assert_exists, zero, set_alloc_hint and
//! list_watchers. Run with:
//!   CEPH_CONF=... cargo test -p rados --test osdclient_small_operations -- --ignored --nocapture

mod common;

use bytes::Bytes;
use common::create_ioctx;
use rados::osdclient::types::OSDOp;
use rados::{AllocHintFlags, CmpOp, OSDClientError, OpBuilder};

const ENOENT: i32 = 2;
const EINVAL: i32 = 22;
const ECANCELED: i32 = 125;

fn val(s: &str) -> Bytes {
    Bytes::copy_from_slice(s.as_bytes())
}

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

#[tokio::test]
#[ignore]
async fn cmpxattr_guard_protects_a_write() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-cmpxattr-guard");
    ioctx
        .set_xattr(&oid, "user.tag", val("t1"))
        .await
        .expect("set tag");

    // RGW's shape: assert the id tag, then write under it.
    let guarded = OpBuilder::new()
        .cmpxattr("user.tag", CmpOp::Eq, val("t1"))
        .expect("cmp")
        .op(OSDOp::set_xattr("user.data", val("v1")).expect("set"))
        .build();
    ioctx
        .execute_op(&oid, guarded)
        .await
        .expect("the tag matches, the write applies");
    assert_eq!(
        ioctx.get_xattr(&oid, "user.data").await.expect("get"),
        val("v1")
    );

    let stale = OpBuilder::new()
        .cmpxattr("user.tag", CmpOp::Eq, val("t0"))
        .expect("cmp")
        .op(OSDOp::set_xattr("user.data", val("v2")).expect("set"))
        .build();
    let err = ioctx
        .execute_op(&oid, stale)
        .await
        .expect_err("a stale tag must fail");
    assert!(is_osd_error(&err, ECANCELED), "{err:?}");
    assert_eq!(
        ioctx.get_xattr(&oid, "user.data").await.expect("get"),
        val("v1"),
        "a failed guard must abort the whole request"
    );

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn cmpxattr_match_returns_one() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-cmpxattr-one");
    ioctx
        .set_xattr(&oid, "user.tag", val("t1"))
        .await
        .expect("set tag");

    let op = OpBuilder::new()
        .cmpxattr("user.tag", CmpOp::Eq, val("t1"))
        .expect("cmp")
        .build();
    let result = ioctx
        .execute_op(&oid, op)
        .await
        .expect("a holding comparison is a success, not an error");
    // PrimaryLogPG returns the comparison's truth value as the op's result.
    assert_eq!(result.ops[0].return_code, 1);
    assert!(result.result >= 0);

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn cmpxattr_missing_xattr_compares_as_empty() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-cmpxattr-missing");
    ioctx
        .write_full(&oid, val("data"))
        .await
        .expect("write_full");

    let absent = OpBuilder::new()
        .cmpxattr("user.none", CmpOp::Eq, Bytes::new())
        .expect("cmp")
        .build();
    ioctx
        .execute_op(&oid, absent)
        .await
        .expect("a missing attribute equals the empty string");

    let present = OpBuilder::new()
        .cmpxattr("user.none", CmpOp::Eq, val("x"))
        .expect("cmp")
        .build();
    let err = ioctx
        .execute_op(&oid, present)
        .await
        .expect_err("a missing attribute is not \"x\"");
    assert!(
        is_osd_error(&err, ECANCELED),
        "ECANCELED, not ENODATA: {err:?}"
    );

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn cmpxattr_u64_compares_decimal_text() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-cmpxattr-u64");
    ioctx
        .set_xattr(&oid, "user.ver", val("5"))
        .await
        .expect("set ver");

    // The OSD evaluates `value <op> stored`: the supplied operand is the
    // left-hand side (do_cmp_xattr in PrimaryLogPG.cc).
    for (op, value) in [
        (CmpOp::Eq, 5),
        (CmpOp::Gte, 5),
        (CmpOp::Gte, 6),
        (CmpOp::Lt, 4),
    ] {
        let built = OpBuilder::new()
            .cmpxattr_u64("user.ver", op, value)
            .expect("cmp")
            .build();
        ioctx
            .execute_op(&oid, built)
            .await
            .unwrap_or_else(|e| panic!("{value} {op:?} 5 must hold: {e:?}"));
    }

    let built = OpBuilder::new()
        .cmpxattr_u64("user.ver", CmpOp::Gte, 4)
        .expect("cmp")
        .build();
    let err = ioctx
        .execute_op(&oid, built)
        .await
        .expect_err("4 >= 5 must fail");
    assert!(is_osd_error(&err, ECANCELED), "{err:?}");

    // The OSD parses the stored value as decimal text; anything else is EINVAL.
    ioctx
        .set_xattr(&oid, "user.ver", val("five"))
        .await
        .expect("set text");
    let built = OpBuilder::new()
        .cmpxattr_u64("user.ver", CmpOp::Eq, 5)
        .expect("cmp")
        .build();
    let err = ioctx
        .execute_op(&oid, built)
        .await
        .expect_err("non-numeric text");
    assert!(is_osd_error(&err, EINVAL), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn assert_exists_guards_a_write() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-assert-exists");
    let write = || OpBuilder::new().assert_exists().write_full(val("x")).build();

    let err = ioctx
        .execute_op(&oid, write())
        .await
        .expect_err("missing object");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
    let err = ioctx
        .stat(&oid)
        .await
        .expect_err("the failed guard must not create the object");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");

    ioctx.write_full(&oid, val("seed")).await.expect("create");
    ioctx
        .execute_op(&oid, write())
        .await
        .expect("existing object");
    let read = ioctx.read(&oid, 0, 16).await.expect("read");
    assert_eq!(read.data, val("x"));

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn zero_clears_a_range_and_ignores_missing_objects() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-zero");
    ioctx
        .write_full(&oid, val("abcdefgh"))
        .await
        .expect("write_full");

    ioctx.zero(&oid, 2, 3).await.expect("zero");
    let read = ioctx.read(&oid, 0, 8).await.expect("read");
    assert_eq!(&read.data[..], b"ab\0\0\0fgh");

    let missing = unique("small-zero-missing");
    ioctx
        .zero(&missing, 0, 4)
        .await
        .expect("zeroing a missing object is a no-op");
    let err = ioctx
        .stat(&missing)
        .await
        .expect_err("and does not create it");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn set_alloc_hint_creates_the_object() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-alloc-hint");

    ioctx
        .set_alloc_hint(&oid, 4 << 20, 4 << 20, AllocHintFlags::INCOMPRESSIBLE)
        .await
        .expect("hint");
    let st = ioctx
        .stat(&oid)
        .await
        .expect("the hint created the object");
    assert_eq!(st.size, 0);

    // RGW's RadosWriter sends the hint and the data in one request.
    let oid2 = unique("small-alloc-hint-write");
    let op = OpBuilder::new()
        .set_alloc_hint(0, 0, AllocHintFlags::INCOMPRESSIBLE)
        .write_full(val("body"))
        .build();
    ioctx.execute_op(&oid2, op).await.expect("hint then write");
    let read = ioctx.read(&oid2, 0, 16).await.expect("read");
    assert_eq!(read.data, val("body"));

    ioctx.remove(&oid).await.expect("remove");
    ioctx.remove(&oid2).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn list_watchers_is_empty_without_watchers() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-watchers");
    ioctx
        .write_full(&oid, val("data"))
        .await
        .expect("write_full");

    let watchers = ioctx.list_watchers(&oid).await.expect("list_watchers");
    assert!(watchers.is_empty(), "{watchers:?}");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn list_watchers_on_missing_object_is_enoent() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("small-watchers-missing");

    let err = ioctx
        .list_watchers(&oid)
        .await
        .expect_err("missing object must fail");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
}
```

Add `osdclient_small_operations` at the end of both `for test in ...`
lists in `.github/workflows/test-with-ceph.yml` (the CRC-enabled leg at
line 91 and the CRC-disabled leg at line 101).

- [ ] **Step 2: Build the test binary**

Run: `cargo test -p rados --offline --test osdclient_small_operations --no-run`
Expected: compiles with no warnings.

- [ ] **Step 3: Run against the cluster (controller, unsandboxed)**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados --offline --test osdclient_small_operations -- --ignored --nocapture`
Expected: 9 passed. Then re-run the neighbours to prove Task 1's change
broke nothing:
`for t in osdclient_rados_operations osdclient_xattr_operations osdclient_omap_operations; do CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados --offline --test $t -- --ignored; done`
Expected: all pass (6, 4 and 8 tests).

If a test fails on an OSD behaviour the plan asserted, do not weaken the
assertion: report the observed code and the `PrimaryLogPG.cc` line that
explains it in the task report, and let the controller rule.

- [ ] **Step 4: Commit**

```bash
git add rados/tests/osdclient_small_operations.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
tests: small-op cluster tests

Pins against Ceph v19: a cmpxattr guard aborting the write it fronts, a
holding comparison returning 1, a missing attribute comparing as empty,
u64 mode parsing the stored value as decimal text, assert_exists failing
a write on a missing object, zero ignoring a missing object, the alloc
hint creating the object, and list_watchers on an unwatched and on a
missing object.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 8: Gate, push, draft PR, merge (controller)

**Files:**
- No tree changes on `cmpxattr-small-ops` beyond folded fmt hunks.

**Interfaces:**
- Consumes: the seven commits of Tasks 1-7 on `cmpxattr-small-ops`.
- Produces: `jhoblitt:cmpxattr-small-ops` with a green draft PR, merged
  into the fork's `main`.

- [ ] **Step 1: Per-commit build proof**

```bash
cd $R && git rebase -q bec893e --exec 'cargo check -p rados --offline --all-targets -q' && git log --oneline bec893e..HEAD
```
Expected: the rebase finishes with the same seven commits (no rewrite,
`git diff bec893e..HEAD --stat` unchanged).

- [ ] **Step 2: Container gate**

```bash
podman run --rm -v $R:/src -v $R/.cargo-home:/src/.cargo-home -w /src -e CARGO_HOME=/src/.cargo-home -e CARGO_TARGET_DIR=/src/target-tools localhost/rust-tools:1.98 sh -c 'cargo fmt --all --check; cargo clippy --workspace --all-targets --offline -- -D warnings -D clippy::uninlined_format_args'
```
Expected: fmt clean and zero clippy diagnostics. Any fmt hunk is folded
into the commit that introduced the lines (`git commit --fixup=<sha>` +
`GIT_SEQUENCE_EDITOR=true git rebase -i --autosquash bec893e`) with the
rewrite proof: `git diff <old-head> <new-head>` equals the saved fmt diff.

- [ ] **Step 3: Corpus check for the watcher types (unsandboxed)**

`ceph-dencoder` 19.2.2 ships in the cluster's image. Wrap it so the
harness's absolute paths resolve inside the container. The `--user` is
load-bearing: podman on this machine is a remote client to a rootful
service, and without it the export files the container writes to `/tmp`
are root-owned, so the harness cannot overwrite its shared export path
on the next sample and every sample after the first fails its
rust-to-ceph leg:

```bash
cat > /tmp/claude/ceph-dencoder <<'EOF'
#!/bin/sh
# ceph-dencoder from the v19.2.2 image; the corpus and /tmp are mounted at their host paths.
exec podman run --rm --user "$(id -u):$(id -g)" -v /home/jhoblitt/github/ceph/ceph-object-corpus:/home/jhoblitt/github/ceph/ceph-object-corpus:ro -v /tmp:/tmp quay.io/ceph/ceph:v19.2.2 ceph-dencoder "$@"
EOF
chmod +x /tmp/claude/ceph-dencoder
cd $R && cargo build -p rados --bin dencoder --offline
for t in watch_item_t obj_list_watch_response_t; do
  CORPUS_ROOT=/home/jhoblitt/github/ceph/ceph-object-corpus CEPH_DENCODER=/tmp/claude/ceph-dencoder CORPUS_TYPE=$t \
    cargo test -p rados --offline --test dencoder_corpus_comparison_test -- --ignored --nocapture
done
```
Expected: for both archives, `Result: 10/10 exact match` for
`watch_item_t` and `2/2` for `obj_list_watch_response_t`, every sample
`[OK] ... (decode + roundtrip + cross-decode)`, and the test exits 0. A
JSON mismatch names the field: fix the `Serialize` impl in Task 6's
commit with the fixup fold, never by marking the type an exception.

- [ ] **Step 4: Push and open the draft PR**

From `~/github/rados-rs` (unsandboxed): fetch the branch from `$R`, push
`cmpxattr-small-ops` to `origin`, then

```bash
gh pr create --repo jhoblitt/rados-rs --draft --assignee @me --base main --head cmpxattr-small-ops --title "osdclient: cmpxattr and small ops" --body "$(cat <<'EOF'
Stacked on the omap PR (#2): the first ten commits are `clippy-1.98`, `xattr-raw-name` and `omap-ops`; the seven after them are this PR.

**Motivation.** RGW guards every overwrite with `cmpxattr` on its id tag, guards bucket-index updates with `assert_exists`, and hints compressed objects with `set_alloc_hint`; none of the five ops here existed.

**What changed.** `cmpxattr` (string and u64 modes), `assert_exists`, `zero`, `set_alloc_hint` carrying `FAILOK`, and `list_watchers` with the `watch_item_t` decoder, each with `OpBuilder` and `IoCtx` methods and unit, encoding and cluster tests.

**Notable decisions.** `check_op_result` now fails only on negative codes: the OSD reports a holding comparison as 1.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
Start one background CI watcher for the PR in the same turn.

- [ ] **Step 5: Merge the PR when CI is green**

The fork's `main` is the integration point (owner's instruction of
2026-09-25). Once every check on the draft PR is green, mark it ready and
merge it with a merge commit, keeping the branch because the upstream PR
points at it:

```bash
gh pr ready N --repo jhoblitt/rados-rs && gh pr merge N --repo jhoblitt/rados-rs --merge
```

If GitHub reports the PR conflicting with something merged after this
branch was cut, resolve the conflicts in the merge commit on `main`
itself (a temporary worktree of `origin/main`, `git merge --no-ff`, the
unit suite, the container gate and the cluster suites, then
`git push origin HEAD:main`), never by rewriting the branch.

- [ ] **Step 6: Final whole-branch review and ledger**

Dispatch the whole-branch review of `bec893e..cmpxattr-small-ops` as
`subagent-driven-development` prescribes, apply its findings with the
fixup fold and proof, re-push with `--force-with-lease`, and record the
outcome, the PR number, the CI verdict and the merge commit
in the ledger. Then delete plan 1's SDD workspace.

---

## Roadmap for later plans

Unchanged from plan 1 after this package: `watch-notify`; `cls-crate`
(with `version` and `refcount`); `cls-user`; `cls-queue-gc`;
`cls-rgw-types`; `cls-rgw-bucket-index`; `cls-rgw-gc`; `cls-rgw-usage`;
`cls-rgw-lc`; `cls-rgw-olh`. Each package merges into the fork's `main`
once its CI is green (owner's instruction of 2026-09-25). Upstream PRs:
four are open (#107 to #110); this package's upstream PR waits for the
owner's word. Deferred minors carried forward:
hoist `CEPH_OSD_OP_FLAG_EXCL` out of `OSDOp::create` next to `FAILOK`;
`OpBuilder` has no xattr methods (tests use `.op(OSDOp::set_xattr(..)?)`);
a `list_watchers` cluster test with a live watcher belongs to
`watch-notify`.
