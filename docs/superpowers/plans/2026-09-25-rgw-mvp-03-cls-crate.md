# rados-rs RGW MVP, plan 3 of N: `cls-crate`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `rados-cls` workspace crate with its feature layout and
its first two object-class clients, `version` and `refcount`, each a
one-to-one mirror of the C++ `cls_*_client.h`, with wire bytes pinned in
unit tests, the corpus harness comparing every type the corpus carries
against `ceph-dencoder`, and cluster tests against Ceph v19.2.2.

**Architecture:** Two new crates. `rados-dencoder` takes over the
`dencoder` binary and the corpus harness from `rados`, because a binary
inside `rados` cannot depend on `rados-cls` (a cycle) and the corpus
harness must see class types. `rados-cls` depends on `rados` and holds one
module per class behind a Cargo feature of the same name; every request
and reply struct is a `VersionedDenc` derive (or a hand-written
`VersionedEncode` where a field is version-gated), every client function is
either an `OSDOp` constructor for compound use or an async free function
over `IoCtx`, and a private `call` module holds the one shape every class
call takes. `Serialize` impls match the C++ `dump()` so the harness's JSON
comparison is strict.

**Tech Stack:** Rust 2024 (upstream `rust-version` 1.88; the machine has
1.98), tokio, `bytes`, `serde`, the `rados` crate's `Denc`,
`VersionedEncode` and derives; podman for the cluster and the tooling
container; `gh` for the fork.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`rados-cls` (branches `cls-crate` onward)".

## Global Constraints

- Ceph floor is Squid (v19): hand-written `decode_content` bodies call
  `rados::check_min_version!` at the version Squid emits; no branches for
  older formats; cluster tests run only against v19.2.2. Derived structs
  (`VersionedDenc`) are all version 1, so the floor is met by construction.
- Every derive in `rados-cls` carries `#[denc(crate = "rados")]`: the
  derive macros default to a crate path (`::rados_denc`) that does not
  exist, so a derive without the attribute does not compile.
- Encoding goes through `Denc`; raw `put_*`/`get_*` only inside `Denc`
  impls or `encode_content`/`decode_content`.
- No `unwrap` or `expect` on production paths; tests may use them.
- `rados-cls` has every class feature on by default so `cargo test
  --workspace --all-targets` and CI's clippy line cover the modules; an
  RGW build turns defaults off and names the classes it calls.
- Gates on every task: `cargo test --workspace --lib --offline` green with
  no warnings; before the push, in the tooling container: `cargo fmt --all
  --check` clean and `cargo clippy --workspace --all-targets --offline --
  -D warnings -D clippy::uninlined_format_args` with zero diagnostics;
  cluster and corpus tests by the controller.
- rustfmt and clippy are not installed on the host: implementers keep
  lines under 100 columns and match the surrounding style; the controller
  runs the container gate and folds any reflow into the owning commit.
- Branch `cls-crate` is based on the fork's `main` (d5a456c). A draft PR
  against the fork's `main` for CI and review, merged there once green,
  then an upstream PR from the same branch.
- Commit subjects follow upstream's `<module>: <imperative summary>` style
  (`cls: add the version class`), one logical change per commit, a body
  stating the wire or server fact the change rests on, ending with the
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer after a
  blank line. Commits are made with
  `git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit`.
- The branch is pushed to the fork after every gate; a scratchpad clone is
  never the only copy of committed work.
- Builds use the scratchpad clone's project-local `CARGO_HOME` with
  `--offline`; every implementer starts with the two lines
  `export R=<scratchpad>/rados-rs` and `export CARGO_HOME=$R/.cargo-home`
  (two statements, not one), then `cd $R`. New crates add no dependency
  that is not already in the workspace's lock file.

## Review Focus

1. A failed version condition, in `inc_conds` and in `check_conds`, fails
   the request with `ECANCELED` (not the `EAGAIN` the C++ header comment
   claims), and none of the request's writes apply. Pinned in Task 5
   (`version_inc_conds_and_check`, `version_check_guards_a_write`).
2. An object that was never versioned reads as `{ver: 0, tag: ""}`, a
   success, not an error; and the first `inc` initialises the version to 1
   with a 24-character random tag and then bumps it, so it reads back as
   2, and later `inc`s keep the tag. Pinned in Task 5
   (`version_inc_creates_then_bumps`).
3. `refcount::put` of the last reference removes the object; `put` of an
   unknown or already-retired tag is a silent success; `put` on an object
   with no references at all is `EINVAL`. Pinned in Task 5
   (`refcount_get_put_removes_at_zero`, `refcount_put_without_refs_is_einval`).
4. The implicit reference is the empty-string tag: an object with no
   refcount xattr reads as `[]` normally and as `[""]` with `implicit_ref`,
   and a `put` with `implicit_ref` on such an object removes it. Pinned in
   Task 5 (`refcount_implicit_ref`).
5. JSON shapes the corpus harness compares strictly: `implicit_ref` dumps
   as an integer (`dump_int((int)b)`), a `VersionCond` as an unsigned, and
   `obj_refcount`'s map as an array of `{"oid", "active"}` objects. Pinned
   in Tasks 3 and 4 (unit) and by the corpus run in Task 6.

---

### Task 0: Branch and workspace (controller)

**Files:**
- No tree changes.

**Interfaces:**
- Produces: branch `cls-crate` at `d5a456c` checked out in the scratchpad
  clone `$R`; the SDD ledger; the three Ceph containers up (`podman ps`),
  `/tmp/ceph/ceph.conf` present, `/tmp/claude/ceph-dencoder` present.

- [ ] **Step 1: Confirm the branch and the cluster**

```bash
cd $R && git status --short && git log --oneline -1
podman ps --format '{{.Names}} {{.Status}}' | grep ceph-
ls /tmp/ceph/ceph.conf /tmp/claude/ceph-dencoder
```
Expected: clean tree at `d5a456c Merge pull request #5 ...`; `ceph-mon`,
`ceph-mgr`, `ceph-osd` all `Up`; both files present.

- [ ] **Step 2: Workspace and ledger**

Run the `sdd-workspace` script from `$R` against the plan copy in
`$R/.superpowers/plan-copies/`, write the ledger's identity line, and
record the rulings carried from plans 1 and 2 (container gate, unsandboxed
cluster and corpus runs, push after every gate, merge when green, an
upstream PR after each merge).

---

### Task 1: `rados-dencoder` takes over the dencoder binary and the corpus harness

**Files:**
- Create: `rados-dencoder/Cargo.toml`.
- Move: `rados/src/bin/dencoder.rs` → `rados-dencoder/src/main.rs`;
  `rados/tests/dencoder_corpus_comparison_test.rs` →
  `rados-dencoder/tests/dencoder_corpus_comparison_test.rs` (both with
  `git mv`, content unchanged except the one comment noted below).
- Modify: `Cargo.toml` (workspace members), `rados/Cargo.toml` (drop the
  `[[bin]]` block at lines 21-23), `.github/workflows/ci.yml:120` and
  `:126`.

**Interfaces:**
- Consumes: the binary's imports (`rados::osdclient::osdmap::{OsdInfo,
  OsdXInfo, PgId}`, `rados::osdclient::{OSDMap, ...}`, `rados::{Denc,
  ...}`), which stay valid from another crate; the harness's
  `get_rust_dencoder`, which resolves `target/debug/dencoder` from the
  parent of `CARGO_MANIFEST_DIR` (still the workspace root) and falls back
  to `cargo build --bin dencoder` at the workspace root.
- Produces: package `rados-dencoder` with `[[bin]] name = "dencoder"`;
  the corpus harness as `rados-dencoder`'s integration test. Tasks 2-4
  add `rados-cls` to its dependencies and register class types.

- [ ] **Step 1: Create the crate and move the files**

`rados-dencoder/Cargo.toml`:

```toml
[package]
name = "rados-dencoder"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "ceph-dencoder work-alike for the Rust encodings, used by the corpus comparison test"
license.workspace = true
repository.workspace = true
homepage.workspace = true

[[bin]]
name = "dencoder"
path = "src/main.rs"

[dependencies]
rados = { path = "../rados", version = "0.1.4" }
bytes = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
```

Then:

```bash
mkdir -p rados-dencoder/src rados-dencoder/tests
git mv rados/src/bin/dencoder.rs rados-dencoder/src/main.rs
git mv rados/tests/dencoder_corpus_comparison_test.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
```

In the root `Cargo.toml` `members` list add `"rados-dencoder",` after
`"rados-denc-macros",`. In `rados/Cargo.toml` delete the three-line
`[[bin]]` block (`name = "dencoder"`, `path = "src/bin/dencoder.rs"`) and
the blank line after it. In the moved harness, change the comment
`// manifest_dir is the package root (rados/); one level up is the workspace root.`
to `// manifest_dir is the package root (rados-dencoder/); one level up is the workspace root.`

In `.github/workflows/ci.yml` change line 120 to
`        run: cargo build -p rados-dencoder --bin dencoder` and line 126
to
`        run: cargo test -p rados-dencoder --test dencoder_corpus_comparison_test -- --ignored --nocapture`.

- [ ] **Step 2: Verify**

Run: `cargo build -p rados-dencoder --offline && cargo test -p rados-dencoder --offline --test dencoder_corpus_comparison_test --no-run && cargo test --workspace --lib --offline`
Expected: the binary builds at `target/debug/dencoder`, the harness
compiles, the lib suites pass, no warnings; `target/debug/dencoder list_types`
prints the catalogue.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml rados/Cargo.toml rados-dencoder .github/workflows/ci.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
dencoder: move the binary and the corpus harness into their own crate

A binary inside the rados package cannot depend on the object-class
crate that follows without a dependency cycle, and the corpus harness
has to see class types. Nothing in the code changes; the harness finds
target/debug/dencoder from the workspace root as before.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 2: The `rados-cls` crate and its call helper

**Files:**
- Create: `rados-cls/Cargo.toml`, `rados-cls/src/lib.rs`,
  `rados-cls/src/call.rs`.
- Modify: `Cargo.toml` (members), `rados-dencoder/Cargo.toml`
  (dependency).

**Interfaces:**
- Consumes: `rados::osdclient::OSDOp::call(class, method, indata: Bytes)
  -> Result<OSDOp, OSDClientError>`; `rados::IoCtx::exec(oid, class:
  &str, method: &str, indata: Bytes) -> Result<Bytes>`;
  `rados::encode_with_capacity(&T, features) -> Result<Bytes, RadosError>`;
  `rados::OpReply { return_code, outdata }`; the `From<RadosError> for
  OSDClientError` conversion.
- Produces: crate `rados-cls` with features `version` and `refcount` (both
  default); `pub(crate)` helpers `call::op(class, method, &req) ->
  Result<OSDOp>`, `call::bare_op(class, method) -> Result<OSDOp>`,
  `call::exec(&IoCtx, oid, class, method, &req) -> Result<Bytes>`,
  `call::decode::<T>(&OpReply) -> Result<T>`, `call::decode_bytes::<T>(Bytes)
  -> Result<T>`. Tasks 3 and 4 build on these.

- [ ] **Step 1: Write the crate**

`rados-cls/Cargo.toml`:

```toml
[package]
name = "rados-cls"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Client-side encodings for Ceph object classes, on top of the rados crate"
license.workspace = true
repository.workspace = true
homepage.workspace = true

[features]
# Every class is on by default so the workspace's tests and lints cover
# them; a client that calls only some classes turns defaults off and
# names the ones it uses.
default = ["version", "refcount"]
version = []
refcount = []

[dependencies]
rados = { path = "../rados", version = "0.1.4" }
bytes = { workspace = true }
serde = { workspace = true }

[dev-dependencies]
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true, features = ["env-filter"] }
```

`rados-cls/src/lib.rs`:

```rust
//! Client-side encodings for Ceph object classes.
//!
//! One module per class, each behind a Cargo feature of the same name. A
//! module mirrors the class's `cls_*_client.h`: request and reply structs
//! encoded as Ceph does, `OSDOp` constructors for use inside a compound
//! operation, and async functions over [`rados::IoCtx`] for the single-op
//! case. Errors are the OSD's errno for the call, as
//! [`rados::OSDClientError::OSDError`].

mod call;

#[cfg(feature = "refcount")]
pub mod refcount;
#[cfg(feature = "version")]
pub mod version;
```

`rados-cls/src/call.rs`:

```rust
//! The one shape every class call takes: encode the request struct, send
//! a `CALL` op naming the class and method, decode the reply struct.

use bytes::Bytes;
use rados::osdclient::error::Result;
use rados::osdclient::{IoCtx, OSDOp, OpReply};
use rados::{Denc, encode_with_capacity};

/// A `CALL` op for `class::method` carrying `req`, for a compound operation.
pub(crate) fn op<R: Denc>(class: &str, method: &str, req: &R) -> Result<OSDOp> {
    OSDOp::call(class, method, encode_with_capacity(req, 0)?)
}

/// A `CALL` op for `class::method` with no request payload.
pub(crate) fn bare_op(class: &str, method: &str) -> Result<OSDOp> {
    OSDOp::call(class, method, Bytes::new())
}

/// Send one class call on `oid` and return the method's raw reply.
pub(crate) async fn exec<R: Denc>(
    ioctx: &IoCtx,
    oid: &str,
    class: &str,
    method: &str,
    req: &R,
) -> Result<Bytes> {
    ioctx
        .exec(oid, class, method, encode_with_capacity(req, 0)?)
        .await
}

/// Decode a reply struct from an op's outdata.
pub(crate) fn decode<T: Denc>(reply: &OpReply) -> Result<T> {
    decode_bytes(reply.outdata.clone())
}

/// Decode a reply struct from raw outdata.
pub(crate) fn decode_bytes<T: Denc>(mut out: Bytes) -> Result<T> {
    Ok(T::decode(&mut out, 0)?)
}
```

Add `"rados-cls",` to the root `Cargo.toml` `members` after
`"rados-denc-macros",` (keep the list alphabetical: `rados`,
`rados-cls`, `rados-denc-macros`, `rados-dencoder`, `examples`). In
`rados-dencoder/Cargo.toml` `[dependencies]` add
`rados-cls = { path = "../rados-cls", version = "0.1.4" }` after the
`rados` line.

- [ ] **Step 2: Verify**

Run: `cargo build -p rados-cls --offline && cargo test --workspace --lib --offline`
Expected: builds and passes with no warnings (the `call` helpers are
unused until Task 3; `mod call;` compiles because every item is
`pub(crate)` and the crate has `#[cfg]`-gated users; if the build warns
about dead code with both features on, the warning is a defect to fix by
using the helpers in Task 3, not by allowing it).

If `cargo build` reports dead code for `call.rs` items at this step, add
`#![allow(dead_code)]` is NOT the fix: continue to Task 3, which uses every
helper, and confirm the warning is gone there before committing Task 2
and Task 3 together as the two commits below (Task 2's commit first).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml rados-cls rados-dencoder/Cargo.toml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the rados-cls crate

One module per object class behind a feature of the same name, each
mirroring the class's cls_*_client.h; a private call module holds the
encode-CALL-decode shape they share. The dencoder crate depends on it so
class types join the corpus comparison.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 3: The `version` class

**Files:**
- Create: `rados-cls/src/version.rs`.
- Modify: `rados-dencoder/src/main.rs` (imports, `get_type_info`,
  `list_types`), `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`
  (`CORPUS_TYPES`).

**Interfaces:**
- Consumes: Task 2's `call` helpers; `rados::{Denc, RadosError,
  VersionedDenc}`; `rados::osdclient::{IoCtx, OSDOp, OpReply}`.
- Produces: `version::CLASS = "version"`; `ObjVersion { ver: u64, tag:
  String }` (`Denc`, `Serialize`, `Default`, `PartialEq`) with `inc()` and
  `is_empty()`; `VersionCond` (`None = 0 .. TagNe = 7`, `Denc` as u32,
  `Serialize` as unsigned); `ObjVersionCond { ver, cond }`; request and
  reply structs `SetOp`, `IncOp`, `CheckOp`, `ReadRet`; op constructors
  `set_op(&ObjVersion)`, `inc_op()`, `inc_conds_op(&ObjVersion,
  VersionCond)`, `check_op(&ObjVersion, VersionCond)`, `read_op()`, all
  `-> Result<OSDOp>`; `decode_read(&OpReply) -> Result<ObjVersion>`; async
  `set(&IoCtx, &str, &ObjVersion) -> Result<()>`, `inc(&IoCtx, &str)`,
  `inc_conds(&IoCtx, &str, &ObjVersion, VersionCond)`, `check(&IoCtx,
  &str, &ObjVersion, VersionCond)`, `read(&IoCtx, &str) ->
  Result<ObjVersion>`. Task 5 uses them all.

- [ ] **Step 1: Write the failing unit tests**

Create `rados-cls/src/version.rs` with the module doc and the test module
only:

```rust
//! The `version` class: a `(ver, tag)` pair an object carries in its
//! `ceph.objclass.version` xattr, used by RGW as an optimistic-concurrency
//! guard on its metadata objects. Mirrors `cls_version_client.h`.
//!
//! Server facts the API rests on (`src/cls/version/cls_version.cc`): a
//! failed condition fails the request with `ECANCELED`; an object that was
//! never versioned reads as `{ver: 0, tag: ""}`; `inc` on such an object
//! initialises the version to 1 with a random 24-character tag and then
//! increments it, so the first `inc` reads back as 2, and later `inc`s keep
//! the tag; `set` stores its argument without any check.

#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use rados::osdclient::types::OpData;

    fn v(ver: u64, tag: &str) -> ObjVersion {
        ObjVersion {
            ver,
            tag: tag.to_owned(),
        }
    }

    /// `obj_version::encode`: ENCODE_START(1, 1), u64 ver, string tag.
    fn objv_wire(ver: u64, tag: &str) -> Vec<u8> {
        let mut content = ver.to_le_bytes().to_vec();
        content.extend_from_slice(&(tag.len() as u32).to_le_bytes());
        content.extend_from_slice(tag.as_bytes());
        let mut wire = vec![1, 1];
        wire.extend_from_slice(&(content.len() as u32).to_le_bytes());
        wire.extend_from_slice(&content);
        wire
    }

    #[test]
    fn obj_version_encodes_as_ceph() {
        let bytes = encode_with_capacity(&v(123, "foo"), 0).expect("encode");
        assert_eq!(
            bytes.as_ref(),
            &[1, 1, 15, 0, 0, 0, 123, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, b'f', b'o', b'o'][..]
        );
        assert_eq!(bytes.as_ref(), &objv_wire(123, "foo")[..]);
        let back = ObjVersion::decode(&mut bytes.clone(), 0).expect("decode");
        assert_eq!(back, v(123, "foo"));
    }

    #[test]
    fn cond_encodes_as_u32_after_the_version() {
        let cond = ObjVersionCond {
            ver: v(1, "foo"),
            cond: VersionCond::Eq,
        };
        let bytes = encode_with_capacity(&cond, 0).expect("encode");
        let mut want = vec![1, 1, 25, 0, 0, 0];
        want.extend_from_slice(&objv_wire(1, "foo"));
        want.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(bytes.as_ref(), &want[..]);
        let back = ObjVersionCond::decode(&mut bytes.clone(), 0).expect("decode");
        assert_eq!(back, cond);
    }

    #[test]
    fn inc_op_with_one_condition_matches_the_corpus_instance() {
        // cls_version_inc_op::generate_test_instances(): objv {123, "foo"},
        // one cond {ver {123, "foo"}, VER_COND_GE}.
        let op = IncOp {
            objv: v(123, "foo"),
            conds: vec![ObjVersionCond {
                ver: v(123, "foo"),
                cond: VersionCond::Ge,
            }],
        };
        let bytes = encode_with_capacity(&op, 0).expect("encode");
        let mut cond = vec![1, 1, 25, 0, 0, 0];
        cond.extend_from_slice(&objv_wire(123, "foo"));
        cond.extend_from_slice(&[3, 0, 0, 0]);
        let mut content = objv_wire(123, "foo");
        content.extend_from_slice(&[1, 0, 0, 0]);
        content.extend_from_slice(&cond);
        let mut want = vec![1, 1, content.len() as u8, 0, 0, 0];
        want.extend_from_slice(&content);
        assert_eq!(content.len(), 56);
        assert_eq!(bytes.as_ref(), &want[..]);
        assert_eq!(IncOp::decode(&mut bytes.clone(), 0).expect("decode"), op);
    }

    #[test]
    fn ops_name_the_class_and_method() {
        let op = inc_conds_op(&v(5, "t"), VersionCond::Eq).expect("op");
        assert!(op.indata.starts_with(b"versioninc_conds"));
        assert!(matches!(
            op.op_data,
            OpData::Call {
                class_len: 7,
                method_len: 9,
                ..
            }
        ));
        let op = read_op().expect("op");
        assert_eq!(&op.indata[..], b"versionread");
        assert!(matches!(op.op_data, OpData::Call { indata_len: 0, .. }));
    }

    #[test]
    fn decode_read_unwraps_the_reply() {
        let ret = ReadRet { objv: v(7, "x") };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&ret, 0).expect("encode"),
        };
        assert_eq!(decode_read(&reply).expect("decode"), v(7, "x"));
    }

    #[test]
    fn json_matches_ceph_dencoder_dump() {
        // obj_version::dump: dump_int("ver"), dump_string("tag");
        // obj_version_cond::dump: dump_object("ver"), dump_unsigned("cond");
        // cls_version_inc_op::dump: dump_object("objv"), encode_json("conds").
        let op = IncOp {
            objv: v(123, "foo"),
            conds: vec![ObjVersionCond {
                ver: v(123, "foo"),
                cond: VersionCond::Ge,
            }],
        };
        assert_eq!(
            serde_json::to_value(&op).expect("json"),
            serde_json::json!({
                "objv": {"ver": 123, "tag": "foo"},
                "conds": [{"ver": {"ver": 123, "tag": "foo"}, "cond": 3}]
            })
        );
        assert_eq!(
            serde_json::to_value(ReadRet { objv: v(1, "a") }).expect("json"),
            serde_json::json!({"objv": {"ver": 1, "tag": "a"}})
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline version`
Expected: FAIL to compile: `ObjVersion`, `VersionCond`, `IncOp`, ... not
found.

- [ ] **Step 3: Implement the module**

Insert between the module doc and the test module:

```rust
use bytes::{Buf, BufMut, Bytes};
use rados::osdclient::error::Result;
use rados::osdclient::{IoCtx, OSDOp, OpReply};
use rados::{Denc, RadosError, VersionedDenc};
use serde::Serialize;

use crate::call;

/// The class name.
pub const CLASS: &str = "version";

/// `obj_version`: a counter and a tag that changes when the counter is
/// reset. `ENCODE_START(1, 1)`: `ver`, then `tag`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ObjVersion {
    pub ver: u64,
    pub tag: String,
}

impl ObjVersion {
    /// What the class does on `inc`: the counter goes up, the tag stays.
    pub fn inc(&mut self) {
        self.ver += 1;
    }

    /// `obj_version::empty`: no tag means the object was never versioned.
    pub fn is_empty(&self) -> bool {
        self.tag.is_empty()
    }
}

/// `VersionCond`: how the class compares the object's version with the
/// one supplied. The ordered forms compare `ver` only; the tag forms
/// compare `tag` only; `Eq` compares both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum VersionCond {
    #[default]
    None = 0,
    Eq = 1,
    Gt = 2,
    Ge = 3,
    Lt = 4,
    Le = 5,
    TagEq = 6,
    TagNe = 7,
}

impl TryFrom<u32> for VersionCond {
    type Error = RadosError;

    fn try_from(value: u32) -> std::result::Result<Self, RadosError> {
        Ok(match value {
            0 => Self::None,
            1 => Self::Eq,
            2 => Self::Gt,
            3 => Self::Ge,
            4 => Self::Lt,
            5 => Self::Le,
            6 => Self::TagEq,
            7 => Self::TagNe,
            other => {
                return Err(RadosError::Protocol(format!(
                    "invalid version condition {other}"
                )));
            }
        })
    }
}

/// `obj_version_cond` encodes the condition as a `uint32_t`.
impl Denc for VersionCond {
    fn encode<B: BufMut>(&self, buf: &mut B, features: u64) -> std::result::Result<(), RadosError> {
        (*self as u32).encode(buf, features)
    }

    fn decode<B: Buf>(buf: &mut B, features: u64) -> std::result::Result<Self, RadosError> {
        Self::try_from(u32::decode(buf, features)?)
    }

    fn encoded_size(&self, _features: u64) -> Option<usize> {
        Some(4)
    }
}

/// `obj_version_cond::dump` uses `dump_unsigned`.
impl Serialize for VersionCond {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u32(*self as u32)
    }
}

/// `obj_version_cond`: a version to compare against and how.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ObjVersionCond {
    pub ver: ObjVersion,
    pub cond: VersionCond,
}

/// `cls_version_set_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct SetOp {
    pub objv: ObjVersion,
}

/// `cls_version_inc_op`: `objv` is ignored by the server; only `conds`
/// matter, and they may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct IncOp {
    pub objv: ObjVersion,
    pub conds: Vec<ObjVersionCond>,
}

/// `cls_version_check_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct CheckOp {
    pub objv: ObjVersion,
    pub conds: Vec<ObjVersionCond>,
}

/// `cls_version_read_ret`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReadRet {
    pub objv: ObjVersion,
}

fn one_cond(objv: &ObjVersion, cond: VersionCond) -> Vec<ObjVersionCond> {
    vec![ObjVersionCond {
        ver: objv.clone(),
        cond,
    }]
}

/// `cls_version_set`: store `objv` unconditionally.
pub fn set_op(objv: &ObjVersion) -> Result<OSDOp> {
    call::op(CLASS, "set", &SetOp { objv: objv.clone() })
}

/// `cls_version_inc(op)`: bump `ver` by one; a missing version is
/// initialised to 1 first, so the first `inc` reads back as 2.
pub fn inc_op() -> Result<OSDOp> {
    call::op(CLASS, "inc", &IncOp::default())
}

/// `cls_version_inc(op, objv, cond)`: bump `ver` only if `cond` holds for
/// `objv` against the stored version; otherwise the request fails with
/// `ECANCELED` and nothing in it applies.
pub fn inc_conds_op(objv: &ObjVersion, cond: VersionCond) -> Result<OSDOp> {
    call::op(
        CLASS,
        "inc_conds",
        &IncOp {
            objv: objv.clone(),
            conds: one_cond(objv, cond),
        },
    )
}

/// `cls_version_check`: a read op that fails the request with `ECANCELED`
/// unless `cond` holds; RGW puts one ahead of the writes it guards.
pub fn check_op(objv: &ObjVersion, cond: VersionCond) -> Result<OSDOp> {
    call::op(
        CLASS,
        "check_conds",
        &CheckOp {
            objv: objv.clone(),
            conds: one_cond(objv, cond),
        },
    )
}

/// `cls_version_read`: the op; decode its reply with [`decode_read`].
pub fn read_op() -> Result<OSDOp> {
    call::bare_op(CLASS, "read")
}

/// Decode the reply to [`read_op`]. An object that was never versioned
/// reads as `ObjVersion::default()`.
pub fn decode_read(reply: &OpReply) -> Result<ObjVersion> {
    Ok(call::decode::<ReadRet>(reply)?.objv)
}

/// Store `objv` on `oid`.
pub async fn set(ioctx: &IoCtx, oid: &str, objv: &ObjVersion) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "set", &SetOp { objv: objv.clone() })
        .await
        .map(drop)
}

/// Bump `oid`'s version; a missing one is initialised first, so the first
/// call reads back as 2.
pub async fn inc(ioctx: &IoCtx, oid: &str) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "inc", &IncOp::default())
        .await
        .map(drop)
}

/// Bump `oid`'s version if `cond` holds for `objv`; `ECANCELED` otherwise.
pub async fn inc_conds(
    ioctx: &IoCtx,
    oid: &str,
    objv: &ObjVersion,
    cond: VersionCond,
) -> Result<()> {
    let op = IncOp {
        objv: objv.clone(),
        conds: one_cond(objv, cond),
    };
    call::exec(ioctx, oid, CLASS, "inc_conds", &op)
        .await
        .map(drop)
}

/// Succeed if `cond` holds for `objv` against `oid`'s version; `ECANCELED`
/// otherwise.
pub async fn check(
    ioctx: &IoCtx,
    oid: &str,
    objv: &ObjVersion,
    cond: VersionCond,
) -> Result<()> {
    let op = CheckOp {
        objv: objv.clone(),
        conds: one_cond(objv, cond),
    };
    call::exec(ioctx, oid, CLASS, "check_conds", &op)
        .await
        .map(drop)
}

/// Read `oid`'s version; `ObjVersion::default()` if it was never versioned.
pub async fn read(ioctx: &IoCtx, oid: &str) -> Result<ObjVersion> {
    let out = ioctx.exec(oid, CLASS, "read", Bytes::new()).await?;
    Ok(call::decode_bytes::<ReadRet>(out)?.objv)
}
```

Register the corpus-covered types with the dencoder. In
`rados-dencoder/src/main.rs` add the import
`use rados_cls::version::{CheckOp as VersionCheckOp, IncOp as VersionIncOp, ObjVersion, ReadRet as VersionReadRet, SetOp as VersionSetOp};`
(wrapped under 100 columns) and, in `get_type_info` after the
`"obj_list_watch_response_t"` arm, a new section:

```rust
        // Object classes (rados-cls)
        "obj_version" => Some(type_info_denc::<ObjVersion>()),
        "cls_version_set_op" => Some(type_info_denc::<VersionSetOp>()),
        "cls_version_inc_op" => Some(type_info_denc::<VersionIncOp>()),
        "cls_version_check_op" => Some(type_info_denc::<VersionCheckOp>()),
        "cls_version_read_ret" => Some(type_info_denc::<VersionReadRet>()),
```

and in `list_types`, after the `obj_list_watch_response_t` line:

```rust
    println!();
    println!("OBJECT CLASSES (rados-cls)");
    println!("  obj_version       - cls_version version [versioned]");
    println!("  cls_version_set_op / cls_version_inc_op / cls_version_check_op / cls_version_read_ret [versioned]");
```

In `rados-dencoder/tests/dencoder_corpus_comparison_test.rs` add to
`CORPUS_TYPES` after the `obj_list_watch_response_t` entry:

```rust
    // Object classes (rados-cls)
    TypeSpec::new("obj_version", None, false),
    TypeSpec::new("cls_version_set_op", None, false),
    TypeSpec::new("cls_version_inc_op", None, false),
    TypeSpec::new("cls_version_check_op", None, false),
    TypeSpec::new("cls_version_read_ret", None, false),
```

(`obj_version_cond` has corpus samples but no `ceph-dencoder`
registration at this Ceph commit, so it is not compared; its bytes are
pinned by the unit test.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --lib --offline && cargo build -p rados-dencoder --offline && cargo test -p rados-dencoder --offline --test dencoder_corpus_comparison_test --no-run`
Expected: all pass, six new tests in `rados-cls`, no warnings;
`target/debug/dencoder type cls_version_inc_op list_types` is not needed,
but `target/debug/dencoder list_types` shows the new section.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/version.rs rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the version class

Mirrors cls_version_client.h: set, inc, inc_conds, check_conds and read
on the "version" class, with obj_version, obj_version_cond and the op
structs encoded as ENCODE_START(1, 1) types and the condition as a u32.
cls_version.cc fails a condition with ECANCELED, reads an unversioned
object as {0, ""}, and creates {1, random tag} on the first inc.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 4: The `refcount` class

**Files:**
- Create: `rados-cls/src/refcount.rs`.
- Modify: `rados-dencoder/src/main.rs`,
  `rados-dencoder/tests/dencoder_corpus_comparison_test.rs`.

**Interfaces:**
- Consumes: Task 2's `call` helpers; `rados::{Denc, RadosError,
  VersionedDenc, VersionedEncode}`, `rados::check_min_version!`,
  `rados::impl_denc_for_versioned!`.
- Produces: `refcount::CLASS = "refcount"`; `GetOp`, `PutOp` (`tag`,
  `implicit_ref`), `SetOp { refs: Vec<String> }`, `ReadOp { implicit_ref
  }`, `ReadRet { refs }`, `ObjRefcount { refs: BTreeMap<String, bool>,
  retired_refs: BTreeSet<String> }`; ops `get_op(&str, bool)`,
  `put_op(&str, bool)`, `set_op(&[String])`, `read_op(bool)` `->
  Result<OSDOp>`; `decode_read(&OpReply) -> Result<Vec<String>>`; async
  `get(&IoCtx, &str, &str, bool)`, `put(..)`, `set(&IoCtx, &str,
  &[String])`, `read(&IoCtx, &str, bool) -> Result<Vec<String>>`. Task 5
  uses them.

- [ ] **Step 1: Write the failing unit tests**

Create `rados-cls/src/refcount.rs` with the module doc and tests:

```rust
//! The `refcount` class: a set of reference tags an object carries in its
//! `refcount` xattr; RGW uses it for tail objects shared between copies.
//! Mirrors `cls_refcount_client.h`.
//!
//! Server facts the API rests on (`src/cls/refcount/cls_refcount.cc`):
//! `get` adds a tag (a repeat is a no-op); `put` retires a tag, is a silent
//! success for an unknown or already-retired tag, is `EINVAL` when the
//! object holds no references at all, and removes the object when it
//! drops the last one; `set` replaces the whole set and removes the object
//! when given none; the implicit reference an unrefcounted object is
//! assumed to hold is the empty-string tag, honoured only when
//! `implicit_ref` is set.

#[cfg(test)]
mod tests {
    use super::*;
    use rados::encode_with_capacity;
    use rados::osdclient::types::OpData;

    #[test]
    fn get_op_encodes_tag_then_bool() {
        let op = GetOp {
            tag: "foo".to_owned(),
            implicit_ref: true,
        };
        let bytes = encode_with_capacity(&op, 0).expect("encode");
        assert_eq!(
            bytes.as_ref(),
            &[1, 1, 8, 0, 0, 0, 3, 0, 0, 0, b'f', b'o', b'o', 1][..]
        );
        assert_eq!(GetOp::decode(&mut bytes.clone(), 0).expect("decode"), op);
    }

    #[test]
    fn set_op_encodes_a_list_of_strings() {
        let op = SetOp {
            refs: vec!["foo".to_owned(), "bar".to_owned()],
        };
        let bytes = encode_with_capacity(&op, 0).expect("encode");
        assert_eq!(
            bytes.as_ref(),
            &[
                1, 1, 18, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, b'f', b'o', b'o', 3, 0, 0, 0, b'b',
                b'a', b'r'
            ][..]
        );
    }

    #[test]
    fn read_op_is_one_bool() {
        let bytes = encode_with_capacity(&ReadOp { implicit_ref: false }, 0).expect("encode");
        assert_eq!(bytes.as_ref(), &[1, 1, 1, 0, 0, 0, 0][..]);
    }

    #[test]
    fn obj_refcount_is_v2_with_retired_refs() {
        // obj_refcount::generate_test_instances(): refs {"foo": true},
        // retired_refs {"bar"}; ENCODE_START(2, 1).
        let mut refs = BTreeMap::new();
        refs.insert("foo".to_owned(), true);
        let mut retired_refs = BTreeSet::new();
        retired_refs.insert("bar".to_owned());
        let obj = ObjRefcount { refs, retired_refs };
        let bytes = encode_with_capacity(&obj, 0).expect("encode");
        assert_eq!(
            bytes.as_ref(),
            &[
                2, 1, 23, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, b'f', b'o', b'o', 1, 1, 0, 0, 0, 3, 0,
                0, 0, b'b', b'a', b'r'
            ][..]
        );
        assert_eq!(ObjRefcount::decode(&mut bytes.clone(), 0).expect("decode"), obj);
    }

    #[test]
    fn obj_refcount_rejects_v1() {
        // Squid writes v2; a v1 blob (no retired_refs) is below the floor.
        let wire = [1u8, 1, 12, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, b'f', b'o', b'o', 1];
        assert!(ObjRefcount::decode(&mut &wire[..], 0).is_err());
    }

    #[test]
    fn ops_name_the_class_and_method() {
        let op = put_op("t", true).expect("op");
        assert!(op.indata.starts_with(b"refcountput"));
        assert!(matches!(
            op.op_data,
            OpData::Call {
                class_len: 8,
                method_len: 3,
                ..
            }
        ));
    }

    #[test]
    fn decode_read_unwraps_the_reply() {
        let ret = ReadRet {
            refs: vec!["a".to_owned()],
        };
        let reply = rados::OpReply {
            return_code: 0,
            outdata: encode_with_capacity(&ret, 0).expect("encode"),
        };
        assert_eq!(decode_read(&reply).expect("decode"), vec!["a".to_owned()]);
    }

    #[test]
    fn json_matches_ceph_dencoder_dump() {
        // cls_refcount_get_op::dump: dump_string("tag"), dump_int("implicit_ref", (int)b).
        assert_eq!(
            serde_json::to_value(GetOp {
                tag: "foo".to_owned(),
                implicit_ref: true
            })
            .expect("json"),
            serde_json::json!({"tag": "foo", "implicit_ref": 1})
        );
        // cls_refcount_read_ret::dump: an array section of strings.
        assert_eq!(
            serde_json::to_value(ReadRet {
                refs: vec!["foo".to_owned(), "bar".to_owned()]
            })
            .expect("json"),
            serde_json::json!({"refs": ["foo", "bar"]})
        );
        // obj_refcount::dump: refs as {"oid", "active"} objects, retired_refs as strings.
        let mut refs = BTreeMap::new();
        refs.insert("foo".to_owned(), true);
        let mut retired_refs = BTreeSet::new();
        retired_refs.insert("bar".to_owned());
        assert_eq!(
            serde_json::to_value(ObjRefcount { refs, retired_refs }).expect("json"),
            serde_json::json!({
                "refs": [{"oid": "foo", "active": true}],
                "retired_refs": ["bar"]
            })
        );
    }

    #[test]
    fn json_truncates_tags_at_a_nul() {
        let mut refs = BTreeMap::new();
        refs.insert("a\u{0}b".to_owned(), true);
        let mut retired_refs = BTreeSet::new();
        retired_refs.insert("c\u{0}".to_owned());
        assert_eq!(
            serde_json::to_value(ObjRefcount { refs, retired_refs }).expect("json"),
            serde_json::json!({
                "refs": [{"oid": "a", "active": true}],
                "retired_refs": ["c"]
            })
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p rados-cls --lib --offline refcount`
Expected: FAIL to compile: `GetOp`, `ObjRefcount`, ... not found.

- [ ] **Step 3: Implement the module**

Insert between the module doc and the test module:

```rust
use std::collections::{BTreeMap, BTreeSet};

use bytes::{Buf, BufMut};
use rados::osdclient::error::Result;
use rados::osdclient::{IoCtx, OSDOp, OpReply};
use rados::{Denc, RadosError, VersionedDenc, VersionedEncode};
use serde::Serialize;
use serde::ser::{SerializeSeq, SerializeStruct};

use crate::call;

/// The class name.
pub const CLASS: &str = "refcount";

/// The tag every unrefcounted object is assumed to hold when
/// `implicit_ref` is set: `cls_refcount.cc`'s `wildcard_tag`.
pub const WILDCARD_TAG: &str = "";

/// `dump_int("implicit_ref", (int)implicit_ref)`.
fn bool_as_int<S: serde::Serializer>(b: &bool, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_u8(u8::from(*b))
}

/// `cls_refcount_get_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct GetOp {
    pub tag: String,
    #[serde(serialize_with = "bool_as_int")]
    pub implicit_ref: bool,
}

/// `cls_refcount_put_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct PutOp {
    pub tag: String,
    #[serde(serialize_with = "bool_as_int")]
    pub implicit_ref: bool,
}

/// `cls_refcount_set_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct SetOp {
    pub refs: Vec<String>,
}

/// `cls_refcount_read_op`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReadOp {
    #[serde(serialize_with = "bool_as_int")]
    pub implicit_ref: bool,
}

/// `cls_refcount_read_ret`: the active tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, VersionedDenc)]
#[denc(crate = "rados", version = 1, compat = 1)]
pub struct ReadRet {
    pub refs: Vec<String>,
}

/// `obj_refcount`: what the class stores in the `refcount` xattr.
/// `ENCODE_START(2, 1)`: `refs`, then from v2 `retired_refs`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjRefcount {
    /// Active tags; the class only ever stores `true`.
    pub refs: BTreeMap<String, bool>,
    /// Tags a `put` already retired, so a repeated `put` is a no-op.
    pub retired_refs: BTreeSet<String>,
}

impl VersionedEncode for ObjRefcount {
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
        self.refs.encode(buf, features)?;
        self.retired_refs.encode(buf, features)
    }

    fn decode_content<B: Buf>(
        buf: &mut B,
        features: u64,
        struct_v: u8,
        _compat_version: u8,
    ) -> std::result::Result<Self, RadosError> {
        rados::check_min_version!(struct_v, 2, "ObjRefcount", "Squid v19+");

        let refs = BTreeMap::<String, bool>::decode(buf, features)?;
        let retired_refs = BTreeSet::<String>::decode(buf, features)?;
        Ok(Self { refs, retired_refs })
    }

    fn encoded_size_content(&self, features: u64, _version: u8) -> Option<usize> {
        Some(self.refs.encoded_size(features)? + self.retired_refs.encoded_size(features)?)
    }
}

rados::impl_denc_for_versioned!(ObjRefcount);

/// `obj_refcount::dump` prints each tag with `c_str()`, so a tag holding
/// a NUL, as RGW once wrote them, dumps only up to it.
fn c_str(s: &str) -> &str {
    s.find('\0').map_or(s, |i| &s[..i])
}

/// `obj_refcount::dump`: `refs` as `{"oid", "active"}` objects,
/// `retired_refs` as strings, each tag through [`c_str`].
impl Serialize for ObjRefcount {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        struct Ref<'a>(&'a str, bool);
        impl Serialize for Ref<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut s = serializer.serialize_struct("ref", 2)?;
                s.serialize_field("oid", self.0)?;
                s.serialize_field("active", &self.1)?;
                s.end()
            }
        }
        struct Refs<'a>(&'a BTreeMap<String, bool>);
        impl Serialize for Refs<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (oid, active) in self.0 {
                    seq.serialize_element(&Ref(c_str(oid), *active))?;
                }
                seq.end()
            }
        }
        struct Retired<'a>(&'a BTreeSet<String>);
        impl Serialize for Retired<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for tag in self.0 {
                    seq.serialize_element(c_str(tag))?;
                }
                seq.end()
            }
        }
        let mut state = serializer.serialize_struct("ObjRefcount", 2)?;
        state.serialize_field("refs", &Refs(&self.refs))?;
        state.serialize_field("retired_refs", &Retired(&self.retired_refs))?;
        state.end()
    }
}

/// `cls_refcount_get`: add `tag` to the object's references.
pub fn get_op(tag: &str, implicit_ref: bool) -> Result<OSDOp> {
    call::op(
        CLASS,
        "get",
        &GetOp {
            tag: tag.to_owned(),
            implicit_ref,
        },
    )
}

/// `cls_refcount_put`: retire `tag`; removes the object when it was the
/// last reference.
pub fn put_op(tag: &str, implicit_ref: bool) -> Result<OSDOp> {
    call::op(
        CLASS,
        "put",
        &PutOp {
            tag: tag.to_owned(),
            implicit_ref,
        },
    )
}

/// `cls_refcount_set`: replace the references; removes the object when
/// `refs` is empty.
pub fn set_op(refs: &[String]) -> Result<OSDOp> {
    call::op(CLASS, "set", &SetOp { refs: refs.to_vec() })
}

/// `cls_refcount_read`: the op; decode its reply with [`decode_read`].
pub fn read_op(implicit_ref: bool) -> Result<OSDOp> {
    call::op(CLASS, "read", &ReadOp { implicit_ref })
}

/// Decode the reply to [`read_op`]: the active tags.
pub fn decode_read(reply: &OpReply) -> Result<Vec<String>> {
    Ok(call::decode::<ReadRet>(reply)?.refs)
}

/// Add `tag` to `oid`'s references.
pub async fn get(ioctx: &IoCtx, oid: &str, tag: &str, implicit_ref: bool) -> Result<()> {
    let op = GetOp {
        tag: tag.to_owned(),
        implicit_ref,
    };
    call::exec(ioctx, oid, CLASS, "get", &op).await.map(drop)
}

/// Retire `tag` on `oid`, removing the object if it was the last reference.
pub async fn put(ioctx: &IoCtx, oid: &str, tag: &str, implicit_ref: bool) -> Result<()> {
    let op = PutOp {
        tag: tag.to_owned(),
        implicit_ref,
    };
    call::exec(ioctx, oid, CLASS, "put", &op).await.map(drop)
}

/// Replace `oid`'s references with `refs`, removing the object if empty.
pub async fn set(ioctx: &IoCtx, oid: &str, refs: &[String]) -> Result<()> {
    call::exec(ioctx, oid, CLASS, "set", &SetOp { refs: refs.to_vec() })
        .await
        .map(drop)
}

/// Read `oid`'s active tags.
pub async fn read(ioctx: &IoCtx, oid: &str, implicit_ref: bool) -> Result<Vec<String>> {
    let out = call::exec(ioctx, oid, CLASS, "read", &ReadOp { implicit_ref }).await?;
    Ok(call::decode_bytes::<ReadRet>(out)?.refs)
}
```

Register with the dencoder: import
`use rados_cls::refcount::{GetOp as RefcountGetOp, ObjRefcount, PutOp as RefcountPutOp, ReadOp as RefcountReadOp, ReadRet as RefcountReadRet, SetOp as RefcountSetOp};`
(wrapped), add after the version arms:

```rust
        "cls_refcount_get_op" => Some(type_info_denc::<RefcountGetOp>()),
        "cls_refcount_put_op" => Some(type_info_denc::<RefcountPutOp>()),
        "cls_refcount_set_op" => Some(type_info_denc::<RefcountSetOp>()),
        "cls_refcount_read_op" => Some(type_info_denc::<RefcountReadOp>()),
        "cls_refcount_read_ret" => Some(type_info_denc::<RefcountReadRet>()),
        "obj_refcount" => Some(type_info_denc::<ObjRefcount>()),
```

a `list_types` line
`    println!("  cls_refcount_{{get,put,set,read}}_op / cls_refcount_read_ret / obj_refcount [versioned]");`
(doubled braces, because `println!` treats single braces as format
arguments)
and `CORPUS_TYPES` entries after the version ones:

```rust
    TypeSpec::new("cls_refcount_get_op", None, false),
    TypeSpec::new("cls_refcount_put_op", None, false),
    TypeSpec::new("cls_refcount_set_op", None, false),
    TypeSpec::new("cls_refcount_read_op", None, false),
    TypeSpec::new("cls_refcount_read_ret", None, false),
    TypeSpec::new("obj_refcount", None, false),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --workspace --lib --offline && cargo build -p rados-dencoder --offline && cargo test -p rados-dencoder --offline --test dencoder_corpus_comparison_test --no-run`
Expected: all pass, eight new tests, no warnings.

- [ ] **Step 5: Commit**

```bash
git add rados-cls/src/refcount.rs rados-dencoder/src/main.rs rados-dencoder/tests/dencoder_corpus_comparison_test.rs
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: add the refcount class

Mirrors cls_refcount_client.h: get, put, set and read on the "refcount"
class, plus obj_refcount, the ENCODE_START(2, 1) xattr value the class
keeps. cls_refcount.cc treats the empty-string tag as the implicit
reference, retires tags so a repeated put is a no-op, and removes the
object when the last reference goes.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 5: Cluster tests

**Files:**
- Create: `rados-cls/tests/cls_version_refcount.rs`.
- Modify: `.github/workflows/test-with-ceph.yml` (a step in each of the
  two osdclient legs).

**Interfaces:**
- Consumes: everything Tasks 3 and 4 produce; `rados/tests/common/mod.rs`
  through a `#[path]` include; `rados::{IoCtx, OSDClientError, OpBuilder}`.
- Produces: eight `#[ignore]` tests CI runs in both legs.

- [ ] **Step 1: Write the tests**

Create `rados-cls/tests/cls_version_refcount.rs`:

```rust
//! Cluster tests for the version and refcount classes. Run with:
//!   CEPH_CONF=... cargo test -p rados-cls --test cls_version_refcount -- --ignored --nocapture

// The rados crate's cluster helpers; one copy, shared across the workspace.
#[path = "../../rados/tests/common/mod.rs"]
mod common;

use bytes::Bytes;
use common::create_ioctx;
use rados::{OSDClientError, OpBuilder};
use rados_cls::refcount;
use rados_cls::version::{self, ObjVersion, VersionCond};

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

fn objv(ver: u64, tag: &str) -> ObjVersion {
    ObjVersion {
        ver,
        tag: tag.to_owned(),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
#[ignore]
async fn version_set_then_read() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-version-set");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    version::set(&ioctx, &oid, &objv(5, "tagA"))
        .await
        .expect("set");
    assert_eq!(version::read(&ioctx, &oid).await.expect("read"), objv(5, "tagA"));

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn version_inc_creates_then_bumps() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-version-inc");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    let none = version::read(&ioctx, &oid).await.expect("read");
    assert_eq!(none, ObjVersion::default(), "never versioned reads as {{0, \"\"}}");
    assert!(none.is_empty());

    version::inc(&ioctx, &oid).await.expect("first inc");
    let first = version::read(&ioctx, &oid).await.expect("read");
    assert_eq!(first.ver, 2, "init_version stores 1 and inc bumps it");
    assert_eq!(first.tag.len(), 24, "init_version makes a 24-char tag: {first:?}");

    version::inc(&ioctx, &oid).await.expect("second inc");
    let second = version::read(&ioctx, &oid).await.expect("read");
    assert_eq!(second, objv(3, &first.tag), "inc keeps the tag");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn version_inc_conds_and_check() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-version-conds");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");
    version::set(&ioctx, &oid, &objv(5, "t")).await.expect("set");

    version::inc_conds(&ioctx, &oid, &objv(5, "t"), VersionCond::Eq)
        .await
        .expect("matching version increments");
    assert_eq!(version::read(&ioctx, &oid).await.expect("read"), objv(6, "t"));

    let err = version::inc_conds(&ioctx, &oid, &objv(5, "t"), VersionCond::Eq)
        .await
        .expect_err("stale version");
    assert!(is_osd_error(&err, ECANCELED), "{err:?}");
    assert_eq!(version::read(&ioctx, &oid).await.expect("read"), objv(6, "t"));

    version::check(&ioctx, &oid, &objv(6, "t"), VersionCond::Eq)
        .await
        .expect("check eq");
    version::check(&ioctx, &oid, &objv(4, ""), VersionCond::Gt)
        .await
        .expect("6 > 4, tag ignored");
    let err = version::check(&ioctx, &oid, &objv(6, "other"), VersionCond::TagEq)
        .await
        .expect_err("tag differs");
    assert!(is_osd_error(&err, ECANCELED), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn version_check_guards_a_write() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-version-guard");
    ioctx.write_full(&oid, val("v0")).await.expect("write_full");
    version::set(&ioctx, &oid, &objv(1, "t")).await.expect("set");

    // RGW's shape: check the version, then write, in one request.
    let guarded = OpBuilder::new()
        .op(version::check_op(&objv(1, "t"), VersionCond::Eq).expect("op"))
        .write_full(val("v1"))
        .build();
    ioctx.execute_op(&oid, guarded).await.expect("guard holds");
    assert_eq!(ioctx.read(&oid, 0, 16).await.expect("read").data, val("v1"));

    let stale = OpBuilder::new()
        .op(version::check_op(&objv(2, "t"), VersionCond::Eq).expect("op"))
        .write_full(val("v2"))
        .build();
    let err = ioctx.execute_op(&oid, stale).await.expect_err("stale");
    assert!(is_osd_error(&err, ECANCELED), "{err:?}");
    assert_eq!(
        ioctx.read(&oid, 0, 16).await.expect("read").data,
        val("v1"),
        "a failed check must abort the whole request"
    );

    ioctx.remove(&oid).await.expect("remove");
}

#[tokio::test]
#[ignore]
async fn refcount_get_put_removes_at_zero() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-refcount-getput");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    refcount::get(&ioctx, &oid, "a", false).await.expect("get a");
    refcount::get(&ioctx, &oid, "b", false).await.expect("get b");
    refcount::get(&ioctx, &oid, "a", false).await.expect("get a again is a no-op");
    assert_eq!(
        refcount::read(&ioctx, &oid, false).await.expect("read"),
        strings(&["a", "b"])
    );

    refcount::put(&ioctx, &oid, "a", false).await.expect("put a");
    assert_eq!(refcount::read(&ioctx, &oid, false).await.expect("read"), strings(&["b"]));
    refcount::put(&ioctx, &oid, "a", false)
        .await
        .expect("a retired tag is a silent success");
    refcount::put(&ioctx, &oid, "nope", false)
        .await
        .expect("an unknown tag is a silent success");
    assert_eq!(refcount::read(&ioctx, &oid, false).await.expect("read"), strings(&["b"]));

    refcount::put(&ioctx, &oid, "b", false).await.expect("put b");
    let err = ioctx
        .stat(&oid)
        .await
        .expect_err("the last put removes the object");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
}

#[tokio::test]
#[ignore]
async fn refcount_set_overwrites_and_empty_removes() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-refcount-set");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    refcount::get(&ioctx, &oid, "old", false).await.expect("get");
    refcount::set(&ioctx, &oid, &strings(&["x", "y"]))
        .await
        .expect("set");
    assert_eq!(
        refcount::read(&ioctx, &oid, false).await.expect("read"),
        strings(&["x", "y"])
    );

    refcount::set(&ioctx, &oid, &[]).await.expect("set none");
    let err = ioctx.stat(&oid).await.expect_err("set none removes the object");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
}

#[tokio::test]
#[ignore]
async fn refcount_implicit_ref() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-refcount-implicit");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    assert!(refcount::read(&ioctx, &oid, false).await.expect("read").is_empty());
    assert_eq!(
        refcount::read(&ioctx, &oid, true).await.expect("read implicit"),
        vec![refcount::WILDCARD_TAG.to_owned()]
    );

    refcount::put(&ioctx, &oid, "z", true)
        .await
        .expect("put with implicit_ref retires the wildcard");
    let err = ioctx.stat(&oid).await.expect_err("and removes the object");
    assert!(is_osd_error(&err, ENOENT), "{err:?}");
}

#[tokio::test]
#[ignore]
async fn refcount_put_without_refs_is_einval() {
    common::init_tracing();
    let ioctx = create_ioctx().await.expect("create_ioctx");
    let oid = unique("cls-refcount-einval");
    ioctx.write_full(&oid, val("x")).await.expect("write_full");

    let err = refcount::put(&ioctx, &oid, "a", false)
        .await
        .expect_err("no references at all");
    assert!(is_osd_error(&err, EINVAL), "{err:?}");

    ioctx.remove(&oid).await.expect("remove");
}
```

In `.github/workflows/test-with-ceph.yml`, after the
`Run osdclient Integration Tests (CRC enabled)` step and again after the
`(CRC disabled)` step, add a step with the same `env` block as the step
it follows:

```yaml
      - name: Run rados-cls Integration Tests (CRC enabled)
        run: cargo test -p rados-cls --test cls_version_refcount -- --ignored --nocapture
        env:
          RADOS_MS_CRC_DATA: "true"
          RUST_BACKTRACE: full
          RUST_LOG: info
```

(and `(CRC disabled)` / `"false"` for the second).

- [ ] **Step 2: Build the test binary**

Run: `cargo test -p rados-cls --offline --test cls_version_refcount --no-run`
Expected: compiles with no warnings.

- [ ] **Step 3: Run against the cluster (controller, unsandboxed)**

Run: `CEPH_CONF=/tmp/ceph/ceph.conf cargo test -p rados-cls --offline --test cls_version_refcount -- --ignored --nocapture`
Expected: 8 passed. If a test fails on a server behaviour the plan
asserted, do not weaken the assertion: report the observed code and the
`cls_version.cc` or `cls_refcount.cc` line that explains it.

- [ ] **Step 4: Commit**

```bash
git add rados-cls/tests/cls_version_refcount.rs .github/workflows/test-with-ceph.yml
git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com' commit -F- <<'EOF'
cls: version and refcount cluster tests

Pins against Ceph v19: a failed version condition is ECANCELED and
aborts the request it guards, an unversioned object reads as {0, ""},
the first inc creates {1, 24-char tag}, a refcount put of the last tag
or a set of none removes the object, retired and unknown tags are silent
successes, and the implicit reference is the empty-string tag.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
EOF
```

---

### Task 6: Gate, push, draft PR, merge, upstream PR (controller)

**Files:**
- No tree changes beyond folded fmt hunks.

**Interfaces:**
- Consumes: the five commits of Tasks 1-5 on `cls-crate`.
- Produces: `jhoblitt:cls-crate` with a green draft PR merged into the
  fork's `main`, and an upstream PR from the same branch.

- [ ] **Step 1: Per-commit build proof**

```bash
cd $R && git rebase -q d5a456c --exec 'cargo check --workspace --offline --all-targets -q' && git log --oneline d5a456c..HEAD
```

- [ ] **Step 2: Container gate**

```bash
podman run --rm --user "$(id -u):$(id -g)" -v $R:/src -v $R/.cargo-home:/src/.cargo-home -w /src -e CARGO_HOME=/src/.cargo-home localhost/rust-tools:1.98 cargo fmt --all --check
podman run --rm -v $R:/src -v $R/.cargo-home:/src/.cargo-home -w /src -e CARGO_HOME=/src/.cargo-home -e CARGO_TARGET_DIR=/src/target-tools localhost/rust-tools:1.98 cargo clippy --workspace --all-targets --offline -- -D warnings -D clippy::uninlined_format_args
```
Expected: fmt clean (fold any hunk into the commit that introduced the
lines with `git commit --fixup` + `rebase -i --autosquash d5a456c`, proving
`git diff <old> <new>` equals the saved fmt diff) and zero clippy
diagnostics.

- [ ] **Step 3: Corpus check (unsandboxed)**

```bash
cd $R && cargo build -p rados-dencoder --offline
for t in obj_version cls_version_set_op cls_version_inc_op cls_version_check_op cls_version_read_ret cls_refcount_get_op cls_refcount_put_op cls_refcount_set_op cls_refcount_read_op cls_refcount_read_ret obj_refcount; do
  CORPUS_ROOT=/home/jhoblitt/github/ceph/ceph-object-corpus CEPH_DENCODER=/tmp/claude/ceph-dencoder CORPUS_TYPE=$t \
    cargo test -p rados-dencoder --offline --test dencoder_corpus_comparison_test -- --ignored --nocapture 2>&1 | grep -E 'Testing Version|Result:|WARN|FAIL'
done
```
Expected: every type reports `N/N exact match` on both archives (sample
counts per archive: obj_version 10, set_op 10, inc_op 1 and 2,
check_op 10, read_ret 8 and 10, get_op 10, put_op 10, set_op 3, read_op 2,
read_ret 6, obj_refcount 10). A JSON mismatch names the field: fix the
`Serialize` impl in the owning commit with the fixup fold.

- [ ] **Step 4: Push, PR, CI, merge, upstream**

Push `cls-crate` to the fork; open the draft PR assigned to `@me` with
this body:

```
**Motivation.** Every RGW class client needs a home outside the RADOS client, and the first two classes, `version` and `refcount`, prove the pattern: RGW guards metadata objects with the former and shares tail objects with the latter.

**What changed.** A `rados-cls` crate, one feature-gated module per class, each mirroring its `cls_*_client.h` with op constructors for compound use and `IoCtx` functions; `rados-dencoder` takes over the dencoder binary and corpus harness so class types are compared against `ceph-dencoder`; unit, corpus and cluster tests.

**Notable decisions.** Every class is on by default; RGW builds turn defaults off.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Watch CI; when every check is green, `gh pr ready N && gh pr merge N
--merge` (branch kept); then
`gh pr create --repo tchaikov/rados-rs --base main --head jhoblitt:cls-crate`
with the same body prefixed by one line naming the fork-merged PRs it
stacks on (#107 to #111). Record everything in the ledger and delete the
workspace.

---

## Roadmap for later plans

`cls-user`; `cls-queue-gc`; `cls-rgw-types`; `cls-rgw-bucket-index`;
`cls-rgw-gc`; `cls-rgw-usage`; `cls-rgw-lc`; `cls-rgw-olh`; then
`watch-notify` last. Each adds a feature to `rados-cls`, registers its
corpus-covered types with `rados-dencoder`, and merges into the fork's
`main` with an upstream PR following. Deferred minors carried forward:
`obj_version_cond` has no `ceph-dencoder` registration upstream; the
`CmpOp` move from `omap.rs` to `types.rs` belongs to the omap PR; the
`MAX_DECODE_VERSION` trait semantics (rejects a newer `struct_v` where
C++ only rejects a newer compat) are crate-wide.
