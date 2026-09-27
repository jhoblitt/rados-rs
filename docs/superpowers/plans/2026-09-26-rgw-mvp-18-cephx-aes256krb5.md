# rados-rs RGW MVP, plan 18 of N: `cephx-aes256krb5`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Two parts.** Part A is implemented now. Part B is deferred until
> rooket ([jhoblitt/rooket#58](https://github.com/jhoblitt/rooket/issues/58))
> can provide the clusters it needs. Do not implement Part B in this plan.

**Goal:** Let the `rados` crate authenticate with the cephx key type
`CEPH_CRYPTO_AES256KRB5` (0x2, `aes256k`) that v19.2.6 and v20.2.4 add
and make the default for every new key. Today a fresh v19.2.6 cluster's
`AgA…` keyring gets an opaque `EACCES` from the mon. After Part A the
client implements the protocol for it, the legacy `AQ…` path is
unchanged, and a long-lived client drops and refreshes its service
tickets when the monmap's `auth_epoch` rises.

**Coverage gap, stated plainly.** Part A is verified by unit tests,
known-answer tests pinned to Ceph's own `src/test` vectors and to
independently computed literals, mock-server tests, and the existing
suites against the existing v19.2.2 cluster, which holds AES keys only.
Part A does **not** show end-to-end authentication against any cluster
issuing `aes256k` keys, mixed ticket types from a real mon, or a real
`auth_epoch` rise. Those need a v19.2.6 or v20.2.4 cluster, which waits
for rooket#58 (Part B). The PR description says so.

This matters for Rook: on v19.2.6+ and v20.2.4+, Rook makes `aes256k`
the preferred key type while still accepting AES, so a newly created RGW
key is `aes256k` and older AES keys (CSI's, and an RGW key created
before the upgrade) keep working beside it.
- `setDefaultCephxKeyType` runs `ceph mon set auth_allowed_ciphers
  aes,aes256k` and `ceph mon set auth_preferred_cipher aes256k` by
  default (rook `dc7829268`, `pkg/operator/ceph/cluster/cephx.go:99-131`,
  `pkg/apis/ceph.rook.io/v1/security.go:91-99`).
- It creates the RGW's cephx key with `CephxKeyTypeUndefined`, which
  takes that default (`pkg/operator/ceph/object/config.go:173-174`).
- `setRotatingServiceKeyType` then sets `auth_service_cipher = aes256k`
  once mons, mgrs and OSDs are updated (`cephx.go:140-163`, called at
  `cluster.go:183`). See Global Constraints, "Rook's default policy".

**Architecture.**

Part A (implement now), eight commits:
1. The RFC 8009 primitive (AES256-CTS-HMAC-SHA384-192) as a standalone
   module.
2. `CryptoKey` carries its type from the key header, validates it as C++
   `_set_secret` does, and dispatches on it. Every encrypt and decrypt
   names its cephx key usage; the AES path ignores the usage and stays
   byte-identical.
3. The new-type authenticate challenge (HMAC-SHA256, XOR-folded), with
   per-ticket session-key types.
4. and 5. The two latent bugs the research found, one commit each: the
   encrypted ticket blob decrypted with the wrong key, and the mock
   server's challenge check.
7A. A key-type cluster test run against the existing v19.2.2 cluster
   (AES), and added to the existing CI lists.
8. MonMap v10 decode, `auth_epoch` tracking, AUTH masked out of renewal,
   and the `MAuthReply` handling that ticket refresh needs.
9. The gate: unit tests, clippy, fmt, and the existing suites against
   the existing v19.2.2 cluster.

Part B (deferred until rooket#58; not implemented here): the v19.2.6
bring-up (B0), mon/OSD-captured known-answer fixtures (B6), the aes256k
cluster scenarios including Rook's policy and mixed ticket types (B7),
the `auth_epoch` wipe cluster test (B8), and the v19.2.6 CI matrix entry
with its cluster evidence (B9).

The mechanism is identical at v19.2.6 and v20.2.4 (the research report's
preamble), so the plan cites v19.2.6 only.

**Tech Stack:** as plan 3. No new dependency. The primitive is written
on the `rados` crate's existing `aes = "0.8"` (`Aes256`), `hmac` and
`sha2` (`Sha384`, `Sha256`; workspace `hmac = "0.12"`, `sha2 = "0.10"`),
`subtle = "2.6"` (already used by `msgr2/phase/sign.rs:18`) and `rand`
(`rand = "0.8"`, `rados/Cargo.toml:77-95`, `workspace = true`). `cbc`
stays for the AES-128 path. RustCrypto's `cts` crate would be a new
dependency and is not used.

**Spec:** `docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`,
"`cephx-aes256krb5` (after `object-locator-routing`)". The research
report is `.superpowers/research/cephx-aes256krb5.md`, cited below as
"R§n". Every Ceph fact here comes from it with its citation, or is
re-read at the tag and cited `path:line@v19.2.6`.

## Global Constraints

Plans 3 to 17's Global Constraints apply where they fit: commit style,
gates, push after every gate, merge when green, upstream PR, offline
builds, no new dependencies, and no `unwrap` on production paths.
Plus:

- **Branch and base.** Branch `cephx-aes256krb5` is based on fork `main`
  at `c482382`. Plan 17 (`object-locator-routing`) merges first; if the
  fork's `main` moves past `c482382` before this branch merges, rebase
  onto `main` after plan 17 merges. Everything lands in the `rados`
  crate (`auth`, `monclient`, `denc/monmap.rs`), its tests, and the
  existing CI workflow's test lists. `rados-cls` and `msgr2` are
  untouched: the msgr2 auth signature already HMACs whatever raw
  session-key bytes it is handed (R§4; `msgr2/protocol.rs:897-905`,
  `msgr2/state_machine.rs:399-413`).
- **No new cluster plumbing (owner ruling).** rados-rs gains no
  make/bash/podman/compose variant, no second compose project, no sed
  copy of the compose file and no `CEPH_IMAGE` parameter. Any cluster
  other than the existing v19.2.2 docker-compose cluster waits for
  rooket#58. That covers v19.2.6, v20.2.4, Rook cipher settings and
  fresh aes256k-only clusters.
- The wire and crypto facts:
  - **Constants.** `CEPH_CRYPTO_NONE` 0x0, `AES` 0x1 and `AES256KRB5` 0x2
    (`ceph_fs.h:95-97`). A key encodes as `u16 type | u32 sec | u32 nsec
    | u16 len | secret`.
  - **`CryptoKey::decode`** reads that header and then calls
    `_set_secret` (`Crypto.cc:1064-1128@v19.2.6`):
    - an empty secret is accepted with any type;
    - an unknown type is an error (`-EOPNOTSUPP`, thrown as
      `malformed_input`);
    - AES needs at least 16 bytes and uses the first 16 (R§1);
    - AES256KRB5 needs at least 32 bytes and uses all of them as the KDF
      key, with no truncation (R§2);
    - NONE accepts anything;
    - bytes after the secret are ignored (`decode_base64`,
      `Crypto.h:226-233@v19.2.6`).
  - **AES256KRB5**, with `Ke`, `Ki` and the KDF block defined under
    "Key derivation":
    - `encrypt(usage, P)` = `C || H`, where
      `C = AES-256-CBC-CS3(Ke, IV = 0^16, N || P)` with `N` 16 random
      bytes, and `H = HMAC-SHA384(Ki, 0^16 || C)[..24]`. The output
      length is `16 + |P| + 24` (R§2).
    - `decrypt` rejects input shorter than 40 bytes and any MAC
      mismatch, CTS-decrypts, and drops the first 16 bytes (R§2).
  - **Key derivation.**
    - `Ke = HMAC-SHA384(secret, 00000001 | BE32(usage) | AA | 00 |
      00000100)[..32]`.
    - `Ki` is the same with type `55` and `000000c0`, truncated to
      `[..24]`.
    - The block is 14 bytes (R§2). `Kc` (`99`) is never derived.
  - **CS3** (always swap the last two blocks). For `n = 16 + |P| >= 16`,
    `m = ceil(n/16)` and `r = n - 16(m-1)` (1..=16):
    1. zero-pad the input to `16m`;
    2. CBC-encrypt it;
    3. emit `c_1..c_{m-2} | c_m | c_{m-1}[..r]`.

    `m == 1` is plain CBC.
  - **Verified in session:** a Python model of exactly this definition
    reproduces all four vectors of `src/test/crypto.cc:390-447@v19.2.6`.
- **Cephx key usages** (`CephxProtocol.h:34-54@v19.2.6`, R§3):

  | Usage | Name |
  |---|---|
  | 0x03 | `AUTH_CONNECTION_SECRET` |
  | 0x04 | `TICKET_SESSION_KEY` |
  | 0x05 | `TICKET_BLOB` |
  | 0x10 | `AUTHORIZE` |
  | 0x11 | `AUTHORIZE_CHALLENGE` |
  | 0x12 | `AUTHORIZE_REPLY` |
  | 0x20 | `ROTATING_SECRET` |
  | 0x30 | `TICKET_INFO` |

  Plain `encrypt`/`decrypt` in C++ is usage 0 (`Crypto.h:73,320-323`).
  Legacy AES ignores the usage entirely (R§2), so passing usages is
  wire-neutral for AES.
- **Challenge** (`CephxProtocol.cc:34-63@v19.2.6`). The input is
  `CephXChallengeBlob = le64 server_challenge | le64 client_challenge`.
  - For AES it is `encode_encrypt`: `u32 len | E(u8 1 | le64 magic |
    blob)`.
  - For every other type it is `encode_hash`:
    `HMAC-SHA256(raw secret, blob)`, 32 bytes, with no envelope and no
    length prefix (`CephxProtocol.h:755-765`).
  - Both results are XOR-folded over complete little-endian u64 words.
    The mon recomputes the value and returns `-EACCES` on a mismatch.
- **Ticket types** (R§5, "Who enforces what").
  - The AUTH session key has the client key's type.
  - Each service session key has type `min(requested, rotating service
    key type)` (`CephxKeyServer.cc:601-604@v19.2.6`).
  - So an `aes256k` client gets type 2 for AUTH and `min(2, service
    cipher)` for OSD/MGR. Mixed per-ticket types are normal, and every
    decrypt uses the key it is handed, with that key's type.
- **`TICKET_BLOB` (0x05)** is decrypted with the same service handler's
  **previous** session key, before `session_key = msg_a.session_key`
  overwrites it (`CephxProtocol.cc:174-216@v19.2.6`).
  - The mon encrypts only the primary AUTH ticket's blob, and only when
    the client presents its old AUTH ticket, under that old ticket's
    session key (`CephxServiceHandler.cc:291@v19.2.6`). Extra tickets and
    principal replies always pass `false` (`:352`, `:425-426`).
- **Mon policy** (R§5).
  - A fresh v19.2.6 cluster allows only `aes256k` (`auth_allowed_ciphers`).
    An entity whose stored key type is not allowed is rejected at
    `CEPHX_GET_AUTH_SESSION_KEY` with `-EACCES`.
  - An `aes` key can only be created after `ceph mon set
    auth_allowed_ciphers aes,aes256k`, and even then can fail with
    `EPERM` until the mon's next health tick
    (`AuthMonitor.cc:482-509,1532-1537@v19.2.6`).
  - `mon set` takes `name` ∈ {`auth_service_cipher`,
    `auth_allowed_ciphers`, `auth_preferred_cipher`} and a value
    (`MonCommands.h:568-572@v19.2.6`).
  - Rotating service keys are generated with `auth_service_cipher`
    (R§5, `CephxKeyServer.cc:293-314`). `mon set auth_service_cipher`
    never regenerates existing rotating keys: `_rotate_secret` only adds
    keys while `need_new_secrets(now)` (`CephxKeyServer.cc:285-330`).
- **Rook's default policy** (verified in rook `dc7829268`). Rook sets
  `allowed = {aes, aes256k}` and `preferred = aes256k`
  (`cephx.go:99-131`), and sets `auth_service_cipher = aes256k` once
  mons, mgrs and OSDs are updated (`cephx.go:140-163`, `cluster.go:183`).
  It never wipes rotating keys, deliberately, because "older clients …
  hang" (`cephx.go:159-161`), so on an upgraded cluster the service keys
  turn over to aes256k naturally within about 2-3 h. During that window
  an aes256k client holds mixed ticket types. Existing keys (CSI's, and
  an RGW key created before the upgrade) stay AES, because
  `get-or-create` returns the existing key.
- **MonMap v10** (`MonMap.cc:254-286,350-364@v19.2.6`).
  - The encoding is `ENCODE_START(10, 6)`. After the v9 stretch fields
    it appends:
    - `u32 auth_epoch`
    - `i32 auth_service_cipher`
    - `vector<i32> auth_allowed_ciphers` (sorted)
    - `i32 auth_preferred_cipher`
  - Decoding a pre-v10 map yields epoch 0, `service = aes`,
    `allowed = {aes, aes256k}` and `preferred = aes`.
- **`auth_epoch` handling in C++.**
  - `MonClient::handle_monmap` compares the old `auth_epoch` with the new
    one. On a rise it calls `_wipe_secrets_and_tickets()`:
    `invalidate_all_tickets()`, which clears only `have_key_flag` and
    keeps each blob, then `_check_auth_tickets()` at once
    (`MonClient.cc:428,466-469,649-656@v19.2.6`;
    `CephxProtocol.cc:293-306`).
  - The pre-map stub's `auth_epoch` is `UINT_MAX`, so the first real
    map never triggers a wipe (R§5).
  - `auth wipe-rotating-service-keys` bumps `auth_epoch` (R§5). It
    exists at v19.2.6 (`MonCommands.h:179`) and deliberately keeps the
    AUTH rotating keys (`prepare_rotating_update(wipe=true)`,
    `CephxKeyServer.cc:509-521@v19.2.6`), so the AUTH ticket stays valid
    for a `CEPHX_GET_PRINCIPAL_SESSION_KEY` request.
  - A wipe rebuilds every non-AUTH `RotatingSecrets` from scratch, and
    `add` uses `++max_ver` from 0 (`Auth.h:200-206`), so rotating
    `secret_id`s restart at 1 and can repeat across a wipe.
  - The C++ client never puts AUTH in a principal request. A needed
    AUTH key goes through `GET_AUTH_SESSION_KEY` instead
    (`CephxClientHandler.cc:60-126@v19.2.6`), and the server skips AUTH
    in a principal request (`CephxServiceHandler.cc:396-399@v19.2.6`).
  - `MAuthReply` decodes as `u32 protocol, i32 result, u64 global_id,
    bufferlist result_bl, string result_msg`
    (`MAuthReply.h:44-51@v19.2.6`).
  - **The AUTH ticket is never renewed by rados-rs**, before or after
    this plan. `auth_mon_ticket_ttl` is 72 h. Service-ticket renewal
    works until the AUTH ticket expires; after that, principal requests
    fail until the mon session re-authenticates. In-session AUTH renewal
    is the next priority for a long-lived RGW (Roadmap).
- **rados-rs today.** Verified at `f039bde` (`watch-notify`).
  `rados/src/auth/`, `msgr2/` and `denc/` are byte-identical to the
  research tree `dbbd993`. The pre-flight re-verifies the line numbers
  below on `c482382`.
  - **Key parsing.**
    - `CryptoKey::from_base64` (`auth/types.rs:42-47`) stores the whole
      decoded blob, header included, as `secret` and forces
      `crypto_type = AES` (`new`, `:34-40`).
    - `aes_key_bytes` (`:72-86`) then slices bytes `[12..28]`. An
      `AgA…` key therefore becomes the first 16 bytes of its 32-byte
      secret under AES, and the mon answers `EACCES` (R§6).
    - `require_aes` (`:57-65`) rejects the type-2 session keys that
      `Denc::decode` (`:149-166`) reads with their true type.
  - **Test keys.** `AQAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAEAAAABAgMEBQYHCA==`
    is used in `client.rs:753,764,838,851,892`, `protocol.rs:516` and
    `server.rs:304`. Its header says `len = 0`: header-correct parsing
    turns it into an empty key followed by ignored trailing bytes.
    - `keyring.rs:119`'s `client.test` key has header length `0xa759`
      and is rejected outright.
    - `keyring.rs:133` asserts a 28-byte "secret".
    - All of these change in Task 2.
  - **Usage sites** (each passes its usage from Task 2 on).
    - `client.rs:246`: 0x04. `decrypt_service_ticket` is called with
      the client key at `:474` and with the AUTH session key at `:313`.
    - `client.rs:318`: 0x05, which uses the wrong key (Task 4).
    - `client.rs:350`: 0x03.
    - `client.rs:567`: 0x11.
    - `client.rs:662`: 0x10 (`build_authorizer`, also reached by the
      renewal authorizer at `:704`).
    - `provider.rs:240`: 0x12.
    - `server.rs:179`: the challenge-check `decrypt` that Task 5
      deletes; it needs a usage argument in the meantime.
    - `server.rs:210`: 0x04. `server.rs:262`: 0x30. `server.rs:287`:
      0x03. The server test decrypt at `:325` is 0x30.
    - The AES challenge at `client.rs:190` is usage 0.
  - **Errors swallowed on the decrypt paths.** These make some tests
    vacuous unless they decrypt directly (Tasks 3 and B6):
    - `decode_connection_secret` turns every decrypt or decode error into
      `Ok(None)` (`client.rs:383-386`). In CRC mode `None` is also the
      success result.
    - Extra-ticket decode errors are dropped silently
      (`client.rs:487-493`, and the `break` at `:283`).
    - `ServiceAuthProvider::try_extract_connection_secret` uses `.ok()?`
      on the 0x12 decrypt (`provider.rs:228-248`).
  - **Unhandled encrypted ticket blob.** `ServiceTicketInfo::decode`
    (`protocol.rs:133-147`) reads `ticket_enc` and then decodes the blob
    as plaintext whatever its value. The primary ticket list has the
    same `ticket_enc = 1` bug as `client.rs:318`, and it is equally
    dormant: rados-rs always sends an empty `old_ticket`
    (`client.rs:151-156`).
  - **Mock server (`auth/server.rs`).**
    - Its challenge check (`:174-198`) AES-decrypts the 8-byte
      `authenticate.key`, which fails on every input.
    - Its AUTH_DONE layout (`:209-212,274-294`) is not the one
      `CephXClientHandler::handle_auth_done` decodes, nor Ceph's.
    - No test drives it with an auth handler.
      `msgr2_server_accept_test.rs:94-150` passes `None`.
    - The `test_secret` helper (`:312`) calls `CryptoKey::new(Bytes)`.
  - **Ticket renewal replies are dropped.**
    - `MonClient::check_auth_tickets` (`monclient/client.rs:1069-1127`)
      sends an `MAuth` renewal request. It includes AUTH in the mask
      when that ticket is due.
    - `dispatch_message` (`:1129-1200`) has no `CEPH_MSG_AUTH_REPLY`
      arm, so the reply falls into the "unknown message type" error arm
      and renewed tickets never land.
  - **The OSD is not authenticated to the client.** rados-rs never
    checks the OSD's `nonce_plus_one` reply (C++ `verify_reply`,
    `CephxProtocol.cc:594`). Pre-existing; Roadmap.
  - **Shared handler.** Every mon connection and every
    `ServiceAuthProvider` shares one `Arc<Mutex<CephXClientHandler>>`
    (`auth_config.rs:128-130`, `provider.rs:72-74,185-187`). Tickets
    stored in that handler are therefore seen by every new or
    reconnecting OSD session.
  - **MonMap decode.** `denc/monmap.rs` decodes up to v9
    (`MONMAP_ENCODING_VERSION = 9`, `:16`) and skips trailing bytes
    (`denc/codec.rs:538-544`), so a v10 map already decodes with its
    auth fields dropped.
  - **Corpus.** The corpus holds `MonMap` samples only up to
    `19.2.0-404` (no v10). `rados-dencoder` registers no cephx type.
  - **CI.** `ci.yml` runs `cargo clippy --workspace --all-targets --
    -D warnings` and `cargo test --workspace --all-targets` (`:87`), so
    a non-ignored integration test runs in CI.
    `.github/workflows/test-with-ceph.yml` is one job (`test-ceph`,
    "Ceph Integration Tests"). It brings up
    `docker/docker-compose.ceph.yml`, whose three services hard-code
    `image: quay.io/ceph/ceph:v19.2.2` and `container_name:
    ceph-{mon,mgr,osd}`, and runs `cargo test -p rados --test <t> --
    --ignored` over `for test in` lists. Part A changes neither the
    compose file nor the job's shape.
- **Existing clusters on the host** (R§7).
  - The rados-rs cluster: v19.2.2, containers `ceph-{mon,mgr,osd}`, mon
    `v2:127.0.0.1:6789`, daemons 6000-7000, conf `/tmp/ceph/ceph.conf`.
    It holds AES keys and stays that way: it is Part A's regression
    check that AES still works.
    - Part A runs the existing suites (and Task 7A's new test) against
      it read/write, as every prior plan does.
    - Part A **never changes its auth config**: no `ceph mon set`, no
      `auth wipe-rotating-service-keys`, no new entities or key types,
      no container restart or recreate.
    - **Never run `docker/docker-compose.ceph.yml` locally with a
      different image (an edited `image:`, or `CEPH_IMAGE` set if the
      file is ever parameterized) or without `-p`.** Its default project
      and `ceph-*` container names are the existing cluster's, so compose
      would recreate those containers on the new image: an in-place
      upgrade.
  - The rgw-go cluster: v19.2.6, `rgw-go-*`, mon 3300, daemons
    7100-7199. Never touched; nothing in this plan names its containers,
    project or volumes.
- **Code shape.**
  - Key usages are `pub const CEPHX_KEY_USAGE_*: u32` in
    `auth/protocol.rs`, with the C++ names.
  - `CryptoKey::encrypt(&self, usage: u32, plaintext: &[u8])` and
    `decrypt(&self, usage: u32, ciphertext: &[u8])`.
  - A MAC mismatch compares with `subtle::ConstantTimeEq`. C++'s
    `memcmp` is the one deviation, and it does not change the wire.
- **Known-answer literals.** Every KAT in Part A compares against a
  literal from Ceph's `src/test/crypto.cc@v19.2.6` or against a literal
  computed independently of the Rust crates (Python, recorded in the
  task). No Part A test checks the crypto crates only against
  themselves.
- **Sandbox.** Build, unit tests, clippy and fmt run sandboxed. Any
  command that talks to the existing cluster (`podman`, or a test with
  `CEPH_CONF=/tmp/ceph/ceph.conf`) runs unsandboxed and is labelled so.
- **Commits.**
  - The trailer is `Co-Authored-By: Claude Opus 5.5
    <noreply@anthropic.com>` and nothing after it.
  - Commits are made with `git -c user.name='Joshua Hoblitt' -c
    user.email='josh@hoblitt.com' commit`.
  - The PR body ends with the Claude Code line and carries no session
    URL (owner's rule).

## Review Focus

1. **The primitive is byte-exact** (Task 1, pinned by vectors):
   - the 14-byte KDF block;
   - the CS3 swap, including the exact-multiple case (vector 3,
     `n = 32`) and `m = 5`;
   - the MAC over `0^16 || C` truncated to 24 bytes and compared in
     constant time;
   - the 40-byte minimum;
   - the secret used whole (a 33-byte secret ending in a non-zero
     byte derives different keys from its 32-byte prefix; HMAC
     zero-pads its key, so a trailing `00` cannot differ).
2. **Key parsing and dispatch** (Tasks 2 and 3):
   - Parsing follows the header, with `_set_secret`'s rules including an
     accepted empty secret.
   - An unknown type is a clear error naming the type, never `EACCES`.
   - The AES path is byte-identical for every usage, pinned by
     `crypto.cc`'s AES vector and by the v19.2.2 suites.
   - Every call site carries the usage in the table.
   - The HMAC challenge has no length prefix and folds complete words
     only, pinned by an independent literal.
   - Each ticket decrypts with its own key's type.
   - Connection-secret and ticket tests decrypt directly or assert the
     exact stored set, so the swallowed errors cannot hide a failure.
3. **The two bug fixes stay in their own commits** (Tasks 4 and 5):
   - The ticket blob decrypts with the handler's previous key under
     0x05, in both the extra-ticket path and the primary list.
   - The mock server recomputes the fold with the shared function and
     compares it with `authenticate.key`.
4. **Ticket refresh** (Task 8): an `auth_epoch` rise invalidates the
   non-AUTH tickets, sends one principal request at once without AUTH in
   the mask, and the `MAuthReply` lands in the shared handler.
5. **The existing cluster is used, never reconfigured** (Tasks 7A and
   9): no `mon set`, no wipe, no new key type, no compose run.

---

# Part A: implement now

### Task 0: Branch, workspace, pre-flight (controller)

As plan 3's Task 0, with branch `cephx-aes256krb5` off fork `main` at
`c482382`. Then:

- [ ] **Step 1: Pre-flight** (sandboxed). Re-verify every "rados-rs
  today" line reference on `c482382`, and record the drift in the
  ledger.
- [ ] **Step 2: Existing cluster present, recorded only** (unsandboxed,
  read-only):

```bash
podman ps --format '{{.Names}} {{.Image}} {{.Status}}' | grep -E '^(ceph-|rgw-go-)'   # record, do not touch
grep -E 'mon host' /tmp/ceph/ceph.conf
```

  Expected: `ceph-{mon,mgr,osd}` on `quay.io/ceph/ceph:v19.2.2`. If the
  existing cluster is missing or on another image, stop and bring it to
  the owner; do not bring it up or recreate it from this plan.

---

### Task 1: The RFC 8009 primitive

**Files:**
- Create: `rados/src/auth/aes256krb5.rs`.
- Modify: `rados/src/auth/mod.rs` (`pub(crate) mod aes256krb5;`).

**Interfaces** (all `pub(crate)`):
- Constants: `KEY_LEN = 32`, `BLOCK_LEN = 16`, `CONFOUNDER_LEN = 16`,
  `MAC_LEN = 24` and `MIN_CIPHERTEXT_LEN = 40`.
- `fn validate_secret(secret: &[u8]) -> Result<()>`: at least 32 bytes.
- `fn derive(secret: &[u8], usage: u32, kind: u8, len: usize) ->
  Vec<u8>`: the KDF. `kind` is `0xAA`, `0x55` or `0x99`, and `0x99`
  exists only so the test can pin `Kc`.
- `fn encrypt(secret: &[u8], usage: u32, plaintext: &[u8]) ->
  Result<Vec<u8>>`: draws the confounder from `rand::thread_rng()`.
- `fn encrypt_with_confounder(secret, usage, confounder: &[u8; 16],
  plaintext) -> Result<Vec<u8>>`: C++ allows an injected confounder only
  in unit tests (`Crypto.h:98-104`, R§7). Here it is `#[cfg(test)]` or
  `pub(crate)` and documented as such.
- `fn decrypt(secret, usage, ciphertext) -> Result<Vec<u8>>`.
- **Lint gate.** The module's functions have no caller until Task 2, and
  CI runs clippy with `-D warnings`, so `dead_code` would fail this
  commit. Add `#![cfg_attr(not(test), allow(dead_code))]` at the top of
  `aes256krb5.rs`; Task 2 removes it.

CBC is done block by block on `aes::Aes256` (`BlockEncrypt` /
`BlockDecrypt`) with a zero IV. CS3 encrypt follows the definition in
Global Constraints. Decrypt:
1. Check the length.
2. Split `C || H`.
3. Recompute `H` and compare it with `ConstantTimeEq`.
4. For `m >= 2`, with `r` as in Global Constraints:
   1. `D = AES-Dec(Ke, C[16(m-2)..16(m-1)])` (this is `c_m`).
   2. `tail = C[16(m-1)..]` (this is `c_{m-1}[..r]`).
   3. `P_m = D[..r] XOR tail`.
   4. `c_{m-1} = tail || D[r..]`.
   5. CBC-decrypt `C[..16(m-2)] || c_{m-1}` with IV `0^16` to get
      `P_1..P_{m-1}`.

   For `m == 1`, plain CBC-decrypt the single block.
5. Drop the first 16 bytes.

Errors are `CephXError::CryptographicError` with a message naming the
cause: "ciphertext shorter than 40 bytes", "integrity check failed", or
"secret shorter than 32 bytes". Derived keys are computed per call
rather than cached, because cephx encrypts a handful of messages per
connection.

Unit tests:
- **RFC 8009 vectors.** The four vectors of
  `src/test/crypto.cc:390-447@v19.2.6` (key `6D404D37…82460C52`, usage
  2, confounders `F764E9FA…`, `B80D3251…`, `53BF8A0D…`, `763E6536…`,
  plaintext lengths 0, 6, 16, 21), copied byte for byte from the file.
  Each is encrypted with its fixed confounder and must equal the
  ciphertext. Each is decrypted and must equal the plaintext.
- **Longer CS3 vectors** (`m = 5`, computed in Python by the plan
  review, whose model reproduces Ceph's vectors 1 and 4 byte for byte;
  independent of the Rust crates). Usage 4, the RFC key, confounder
  `a0a1a2a3a4a5a6a7a8a9aaabacadaeaf`, plaintext the counting bytes
  `00 01 02 ...` as in Ceph's RFC vectors (corrected at implementation:
  the literals decrypt to counting bytes, not zeros):
  - plaintext length 53 (`n = 69`, `r = 5`):
    `ca0522838c12dfd084d33a407bf109a3e80556051f828e1fe307f9392d7cba733b6510a90ea5abbdea280d562a78bac74eb2626fe95091e7be6f0f759a8bf17b0bdff321a656ca4738330db2aa0d3a2efd5902db3852fecbc2be298fd6`
  - plaintext length 64 (`n = 80`, exact multiple):
    `ca0522838c12dfd084d33a407bf109a3e80556051f828e1fe307f9392d7cba733b6510a90ea5abbdea280d562a78bac7890dadd0da9838a25c00514c275511000bdff321a626f7044cae44ffc3ad8de1646067624725c1e101f6743674defcbf7c96332c08179340`

  Encrypt with the fixed confounder must equal each; decrypt must return
  the zeros.
- **Derived keys** for usage 2, verified in session against the vector
  key:
  - `Kc = EF5718BE86CC84963D8BBB5031E9F5C4BA41F28FAF69E73D`
  - `Ke = 56AB22BEE63D82D7BC5227F6773F8EA7A5EB1C825160C38312980C442E5C7E49`
  - `Ki = 69B16514E3CD8E56B82010D5C73012B622C4D00FFC23ED1F`

  The `Ke` block pinned as `00000001 00000002 aa 00 00000100`.
- **Negative cases.**
  - Flip the last MAC byte, and separately a ciphertext byte, and expect
    "integrity check failed".
  - 39 bytes gives the length error.
  - Decrypting vector 2 with usage 3 fails the integrity check.
  - Encrypting vector 2's plaintext with usage 3 and its confounder
    differs from the vector (C++ `EncryptUsage`,
    `crypto.cc:496-539@v19.2.6`).
- **Secret length.** Lengths 0 to 31 are rejected and 32 to 49 accepted
  (`crypto.cc:360-377@v19.2.6`). A 33-byte secret whose last byte is
  non-zero produces a different ciphertext from its 32-byte prefix under
  the same confounder (a trailing `00` cannot: HMAC zero-pads its key).
- **Round trip** of random plaintexts of length 0 to 64 with random
  usages, crossing every CTS boundary. This is a supplement; the
  literals above are the pins.

Commit:

```
auth: add the aes256k cipher (RFC 8009 AES256-CTS-HMAC-SHA384-192)

Ceph v19.2.6 and v20.2.4 add CEPH_CRYPTO_AES256KRB5 and generate every
new key with it. Per usage, Ke and Ki come from one HMAC-SHA384 over
1 | usage | 0xAA/0x55 | 0 | bit length; a message is a 16-byte random
confounder and the plaintext under AES-256-CBC with ciphertext stealing
(CS3, zero IV), followed by HMAC-SHA384(Ki, zero IV | ciphertext)
truncated to 24 bytes. Pinned by the four RFC 8009 vectors Ceph ships in
src/test/crypto.cc, two longer independently computed vectors, and the
derived keys for usage 2.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 2: `CryptoKey` carries its type; every site names its usage

**Files:**
- Modify: `rados/src/auth/types.rs`, `rados/src/auth/protocol.rs`,
  `rados/src/auth/error.rs`, `rados/src/auth/client.rs`,
  `rados/src/auth/provider.rs`, `rados/src/auth/server.rs`,
  `rados/src/auth/keyring.rs`, `rados/src/auth/mod.rs` and
  `rados/src/auth/aes256krb5.rs` (remove Task 1's `dead_code` allow).
- Modify the `lib.rs` re-exports if `KeyType` is public.

**Interfaces:**
- **Key types.**
  - `pub const CEPH_CRYPTO_NONE: u16 = 0x0;` and
    `CEPH_CRYPTO_AES256KRB5: u16 = 0x2;` beside `CEPH_CRYPTO_AES`.
  - `pub enum KeyType { None, Aes, Aes256Krb5 }` with
    `TryFrom<u16>` and `as_u16`.
  - A new `CephXError::UnsupportedKeyType(u16)`. Its message is
    "unsupported cephx key type {n} (supported: none=0, aes=1,
    aes256k=2)".
- **`CryptoKey`** keeps its public fields (`crypto_type: u16`,
  `created`, `secret`), so its `Denc` and `Serialize` encodings do not
  change. `secret` now always holds the raw secret, never the header.
  - `CryptoKey::new(key_type: KeyType, secret: Bytes) -> Result<Self>`
    validates.
  - `CryptoKey::empty()` replaces the placeholder
    `CryptoKey::new(Bytes::new())` at `types.rs:340`.
  - `from_base64` decodes through `Denc::decode`, which now applies
    `_set_secret`'s rules:
    - an empty secret is accepted with any type;
    - otherwise an unknown type is `UnsupportedKeyType`;
    - AES needs at least 16 bytes, AES256KRB5 at least 32 bytes, and
      NONE accepts anything;
    - trailing bytes are ignored.
  - `key_type(&self) -> Result<KeyType>`.
- **Dispatch.** `encrypt(&self, usage: u32, plaintext)` and
  `decrypt(&self, usage: u32, ciphertext)` dispatch on the type:
  - AES is the existing AES-128-CBC with the first 16 bytes and ignores
    `usage`;
  - AES256KRB5 goes to Task 1;
  - NONE, an empty key, or an unknown type is an error naming the type.

  `require_aes` and the header-slicing branch of `aes_key_bytes` go. The
  AES path keeps `aes_key_bytes`' "first 16 bytes" rule, but as a
  private helper.
- `pub fn hmac_sha256(&self, data: &[u8]) -> Result<[u8; 32]>` keyed
  with the whole raw secret, whatever the key's type
  (`Crypto.cc:234-259@v19.2.6`, R§2). An empty key is an error.
- **`protocol.rs`.** The eight `CEPHX_KEY_USAGE_*` constants with their
  C++ names and values. `AES_KEY_LEN`, `AES_BLOCK_LEN`, `CEPH_AES_IV`
  and `CRYPTO_KEY_HEADER_SIZE` stay.
- **Every call site** passes the usage from the table under "rados-rs
  today".
  - The AES challenge at `client.rs:190` passes 0.
  - `server.rs:179`'s challenge decrypt passes 0 until Task 5 deletes
    it.
  - `calculate_session_key` returns `UnsupportedKeyType` for a non-AES
    key until Task 3 replaces that branch.
  - The `server.rs:312` `test_secret` helper moves to the new
    `CryptoKey::new(KeyType, Bytes)` signature.
  - The comment at `provider.rs:284-290` states AES-only 36-byte sizes.
    Add the aes256k sizes (`16 + |P| + 24`) or drop the numbers; the
    `con_mode` rule it explains is unchanged.
  - The mock server's `random_aes_key` becomes `random_key(key_type)`.
    - The AUTH session key takes the client key's type.
    - Each service key takes `min(client key type, service secret type)`
      (`CephxKeyServer.cc:601-604@v19.2.6`).
    - The connection secret is raw random bytes (16), not a typed key:
      a 16-byte aes256k `CryptoKey` would fail validation, and its
      length is independent of the key type (R§4).
- **Tests use real keys:**

  | Key | Base64 |
  |---|---|
  | AES, secret `00 01 … 0f` | `AQAAAAAAAAAAABAAAAECAwQFBgcICQoLDA0ODw==` |
  | aes256k, the RFC 8009 key | `AgAAAAAAAAAAACAAbUBNN/r3n53w0zVo0yBmmADrSDZHLqigJtFrcYJGDFI=` |
  | type 3 | `AwAAAAAAAAAAACAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=` |
  | aes256k, 31 bytes | `AgAAAAAAAAAAAB8AEREREREREREREREREREREREREREREREREREREREREQ==` |

  - These replace every use of the length-0 key (`client.rs`,
    `protocol.rs:516`, `server.rs:304`) and `keyring.rs:119`'s
    `client.test` key.
  - `keyring.rs:133` asserts a 16-byte secret and type AES for
    `client.admin`.

Unit tests:
- **Parsing.**
  - The AES key parses to type 1 with 16 bytes.
  - The aes256k key parses to type 2 with 32 bytes equal to the RFC key.
  - Type 3 is `UnsupportedKeyType(3)`, and its message contains "3".
  - The 31-byte aes256k key is rejected.
  - A 33-byte aes256k key is accepted.
  - An empty-secret header parses as an empty key.
  - A keyring with an `AgA…` entry parses to type 2.
- **AES byte-identity.** With secret `00..0f` and plaintext
  `00112233…ff`, `encrypt` gives
  `b38f5bc9354cf8c61315666f37d7793a11907be9d83c3570587b979b03d2a501`
  (`src/test/crypto.cc:42-80@v19.2.6`) for usages 0, 0x04, 0x10 and
  0x30, and decrypts back.
- **aes256k dispatch.** A `CryptoKey` of type 2 decrypts Task 1's
  vector 2 under usage 2 through `CryptoKey::decrypt`, round-trips under
  usage 0x04, and fails to decrypt under 0x03.
- **`hmac_sha256`, pinned to literals.**
  - Ceph's own vectors (`src/test/crypto.cc:668-718@v19.2.6`), with
    secret `00112233445566778899aabbccddeeff`. The C++ test hands this
    16-byte secret to the AES256KRB5 handler without validation; here
    it goes in an AES-typed `CryptoKey`, which is valid because
    `hmac_sha256` is type-independent:
    - `"blablabla"` →
      `42c7027e8be06dca2c0b444373fefdbeac5b4034eca44a69de3a291634ed8df9`
    - `"testing1234blablabla"` →
      `4bd3ac394acc9706dd09e65c68add4cf092ccda1e799e35c527385bd7973c698`
  - The RFC key (aes256k) over 16 zero bytes →
    `480e73b4fe5f14c60d407394f7ca4fa4107aa6ca79de56579e888e622ba72ac2`
    (computed in Python).
- **Existing tests.** All existing `auth` tests pass with the new keys.

Commit:

```
auth: carry the cephx key type and name the key usage at every site

A keyring key is a header (type, created, length) and the secret;
CryptoKey kept the header in its secret and always claimed AES, so an
aes256k keyring was used as the first 16 bytes of its secret under AES
and the mon refused it. Parse the header as CryptoKey::decode does,
validate the secret for its type (AES at least 16 bytes, aes256k at
least 32), report an unknown type by number, and dispatch encrypt and
decrypt on the type. Every cephx encrypt and decrypt now names its key
usage (0x03 connection secret, 0x04 ticket session key, 0x05 ticket
blob, 0x10-0x12 authorizer, 0x30 ticket info); AES ignores it, so the
AES wire is unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 3: Authenticate with an aes256k key

**Files:**
- Modify: `rados/src/auth/client.rs`, and `rados/src/auth/protocol.rs`
  if the fold moves there.

**Interfaces:**
- `pub(crate) fn cephx_calc_client_server_challenge(key: &CryptoKey,
  server_challenge: u64, client_challenge: u64) -> Result<u64>` mirrors
  C++. `calculate_session_key` (`client.rs:168-207`) calls it, and so
  does Task 5's server.
  - AES: the existing envelope encrypt with a `u32` length prefix.
  - Every other valid type: `hmac_sha256(le64 server | le64 client)`,
    with no prefix.
  - Both fold complete u64 words.
- Ticket handling is unchanged: each `TicketHandler.session_key` is the
  `CryptoKey` decoded from its own `CephXServiceTicket`, type included,
  so dispatch is per ticket. `handle_auth_done` (`:494`) and
  `ServiceAuthProvider` (`provider.rs:332`) keep returning the raw
  `secret`, which is 32 bytes for a type-2 key and is the msgr2 HMAC key
  (R§4).

Unit tests:
- **Challenge fold.**
  - AES: the fold of a fixed challenge pair under the AES test key
    equals the value the base commit's `calculate_session_key` produced.
    Compute it on `c482382` first and pin it as a literal (a regression
    pin; the AES wire itself is pinned by `crypto.cc`'s AES vector and
    the v19.2.2 suites).
  - aes256k, pinned to an independently computed literal (Python): for
    `s = 0x0123456789abcdef`, `c = 0xfedcba9876543210` under the RFC key,
    `HMAC-SHA256(RFC key, le64(s) | le64(c))` =
    `87956304acfb25b7e02f0ef45a6b68737b1c2a819da54d8e2f8b36f77ab8770c`,
    and the fold is `0x46778d1186712d33`.
  - The aes256k fold is deterministic, while an aes256k `encrypt` of
    the same blob is not. This pins the reason the code branches.
- **Hand-built AUTH_DONE** builders in the test module: `CephXResponseHeader`,
  `ServiceTicketReply`, the connection-secret envelope and extra
  tickets, encrypted with `CryptoKey::encrypt` under the right usages.
  - (a) All AES.
  - (b) Client key aes256k, AUTH session key aes256k, extra OSD and MGR
    tickets aes256k.
  - (c) Mixed: client key aes256k, AUTH session key aes256k, OSD
    session key AES, MGR aes256k.

  Each asserts:
  - the set of stored services is exactly the set built (so a silently
    dropped extra ticket fails the test);
  - every stored handler's `session_key.crypto_type` and `secret`
    equal what was built;
  - the returned session-key bytes are the raw AUTH secret;
  - the connection-secret blob, taken out of the built AUTH_DONE, passes
    `CryptoKey::decrypt(0x03, …)` directly under the AUTH session key
    with `Ok` and the correct magic, and fails under 0x04. Asserting
    only `handle_auth_done`'s returned `Option` is not enough:
    `decode_connection_secret` maps every error to `None`;
  - in SECURE mode, `handle_auth_done` returns `Some` connection secret
    of the built length;
  - `build_authorizer(OSD)` output decrypts under the OSD key with 0x10;
  - `decrypt_authorize_challenge` accepts a challenge encrypted under
    0x11 and rejects one encrypted under 0x12 when the key is aes256k.
- **Reply decrypt.** `ServiceAuthProvider` decrypts an authorize reply
  under 0x12 for both key types. Because `try_extract_connection_secret`
  uses `.ok()?`, the test also decrypts the reply blob directly under
  0x12 (`Ok`) and under 0x10 (error) for the aes256k key.

Commit:

```
auth: authenticate with an aes256k key

Ceph computes the cephx challenge response by encrypting the challenge
blob for AES keys, and by HMAC-SHA256 over the blob keyed with the raw
secret for every other key type, since aes256k's confounder makes
encryption non-deterministic; both are folded over complete
little-endian u64 words. Session keys keep the type the mon gives each
ticket (the client key's type for AUTH, the lower of it and the
service's for the rest), so one session can mix AES and aes256k
tickets.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 4: Fix: an encrypted ticket blob uses the previous session key

**Files:**
- Modify: `rados/src/auth/client.rs` (`try_decode_single_ticket`,
  `:305-331`; `handle_auth_done`, `:469-487`) and
  `rados/src/auth/protocol.rs` (`ServiceTicketInfo`).

**Rules:**
- **The key.** When `ticket_enc != 0`, the blob bytes are
  `E(TICKET_BLOB = 0x05)` under the session key that the same service's
  `TicketHandler` held before this reply
  (`CephxProtocol.cc:196-216@v19.2.6`). They are not decrypted with the
  key just decrypted from `msg_a`, which is what `:318` does today.
- **No previous key.** No handler, or an empty key, is an error ("ticket
  blob encrypted but no previous session key for {service}").
- **`ServiceTicketInfo::decode`** keeps the raw blob bytes when
  `ticket_enc != 0` and decodes `CephXTicketBlob` only after the caller
  decrypts them. Its `ticket_enc == 0` behaviour and its encoding are
  unchanged. The primary ticket list in `handle_auth_done` takes the
  same rule.
- **Dormant today, and where it would go live.** The mon encrypts only
  the primary AUTH ticket, and only when the client presents its old
  AUTH ticket (`CephxServiceHandler.cc:291@v19.2.6`); extra tickets and
  principal replies always pass `false` (`:352`, `:425-426`). So the
  primary-list rule goes live with the roadmap's in-session AUTH
  renewal, and the extra-ticket and principal-reply paths (the latter
  reached through Task 8) handle it only as C++'s client does, for
  parity.

Unit tests:
- With a session whose OSD handler holds key `K_old`, of each type in
  turn, an extra ticket with `ticket_enc = 1` and its blob encrypted
  under `K_old` with 0x05 decodes. The handler afterwards holds the new
  key and the new blob.
- The same bytes encrypted under the new key fail, asserted on the
  function's `Result` rather than through the extra-ticket loop that
  drops errors.
- `ticket_enc = 1` with no previous handler errors.
- The primary list takes the same three cases, with the AUTH handler.

Commit:

```
auth: decrypt an encrypted ticket blob with the previous session key

When a client presents its old AUTH ticket, the mon may encrypt the new
AUTH ticket's blob (ticket_enc) under that old AUTH session key, with
key usage TICKET_BLOB; the client decrypted an encrypted blob with the
key it had just received, and the primary ticket list did not decrypt
it at all. Use the previous session key in both paths. Both are dormant
while the client sends no old ticket.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 5: Fix: the mock server's challenge check

**Files:**
- Modify: `rados/src/auth/server.rs` (`handle_authenticate`,
  `:174-198`).

**Rules:**
- **The check.** Recompute `expected =
  cephx_calc_client_server_challenge(client_secret, server_challenge,
  authenticate.client_challenge)` (Task 3) and compare it with
  `authenticate.key`. A mismatch is `AuthenticationFailed("challenge
  verification failed")`, as C++ does (R§6).
- **What goes.** Delete the decrypt of the 8-byte `key` (`:179`) and the
  `server_challenge + 1` comparison.
- **What stays.** The handler's AUTH_DONE layout, which is neither
  Ceph's nor the client's, is out of scope. The type docs say so in one
  sentence.

Unit tests (the mock-server tests):
- `handle_initial_request` followed by `handle_authenticate` with a
  payload from a `CephXClientHandler` fed the server's challenge
  succeeds for the AES keyring and for an aes256k keyring.
- The same payload against a keyring holding a different key of the
  same type fails.
- A payload whose `key` is off by one fails.
- For the aes256k keyring with the fixed challenges of Task 3, the
  server's recomputed value equals the literal `0x46778d1186712d33`.

Commit:

```
auth: verify the cephx challenge in the server handler as Ceph does

The server tried to AES-decrypt the client's 8-byte folded challenge
response, which fails for every input. Ceph recomputes the response
from the stored key and both challenges and compares the folded
values; do the same, for AES and aes256k keys alike.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 7A: The key-type cluster test, against the existing cluster

The existing v19.2.2 cluster holds AES keys, so this is the AES
regression and the ticket-type introspection check. It does not test
aes256k end to end (Part B does).

**Files:**
- Create: `rados/tests/cephx_aes256k.rs`, with every test `#[ignore]`.
- Modify: `.github/workflows/test-with-ceph.yml`: `cephx_aes256k` joins
  both existing `rados` `for test in` lists (CRC on and off). No other
  workflow change; the job, its name and its cluster are unchanged.

**Test `keyring_key_type_end_to_end`:**
- Read the keyring's key for the entity and log its type. When
  `CEPH_TEST_EXPECT_KEY_TYPE` is set, assert it equals that value.
- Connect, write `o`, read it back.
- `exec` `hello.say_hello` (the class call of
  `osdclient_exec_operations.rs`).
- Watch `o` and notify from a second client; the watcher acks and the
  notify returns one ack (plan 12's API).
- Through `mon_client().get_service_auth_provider()`, assert that the
  AUTH ticket's `session_key.crypto_type` equals the client key type
  (the C++ rule).
- Assert that the OSD ticket's type equals
  `CEPH_TEST_EXPECT_OSD_TICKET_TYPE` when that is set, and otherwise
  `CEPH_TEST_EXPECT_KEY_TYPE`. Where both are unset, assert only that it
  is not above the client key's type (`min` rule).
- The test issues no mon command that changes auth state.

**Controller run** (unsandboxed; the existing cluster, read/write I/O
only, no auth config change):

```bash
CEPH_CONF=/tmp/ceph/ceph.conf CEPH_TEST_EXPECT_KEY_TYPE=1 \
  cargo test -p rados --offline --test cephx_aes256k -- --ignored --nocapture
```

Commit:

```
rados: cephx key-type cluster test

The test connects with the keyring's key, writes, reads, calls a class
method and round-trips a watch and notify, then checks that the AUTH
ticket has the client key's type and the OSD ticket no higher. It runs
in the existing CI job against v19.2.2, whose keys are AES.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 8: MonMap v10, `auth_epoch`, ticket refresh

This task cannot be deferred. The research does not show it safe to
defer (R§9: without it, tickets fail at OSDs after a rotation).
Worse, rados-rs drops every `MAuthReply` today, so a long-lived
client's tickets are never renewed at all. It stays the last code task
because nothing before it depends on it.

**Files:**
- Modify: `rados/src/denc/monmap.rs`, `rados/src/monclient/monmap.rs`,
  `rados/src/monclient/messages.rs`, `rados/src/monclient/client.rs` and
  `rados/src/auth/client.rs`.

**Interfaces and rules:**
- **MonMap v10 decode.**
  - `MonMap` gains `pub auth: Option<MonMapAuth>`, where `MonMapAuth {
    epoch: u32, service_cipher: i32, allowed_ciphers: Vec<i32>,
    preferred_cipher: i32 }`. It is decoded when `version >= 10` and is
    `None` below 10.
  - `None` is not filled with C++'s pre-v10 defaults, because rados-rs
    has no consumer of those defaults.
  - Serialization uses `#[serde(skip_serializing_if =
    "Option::is_none")]`, so the corpus JSON of v6-v9 samples is
    unchanged.
  - The encoder stays v9: rados-rs never sends a monmap, and emitting
    v10 would change the corpus round trip.
  - `MonMapState` gains `pub auth_epoch: Option<u32>`.
- **The rise rule.** In `MonClient::handle_monmap`, read the held map's
  `auth_epoch` before replacing it. It is a rise only when both are
  `Some` and the new one is larger. The first map records and never
  wipes, mirroring C++'s `UINT_MAX` stub.
- **On a rise:**
  1. `CephXClientHandler::invalidate_service_tickets()` marks every
     handler except AUTH as due (`renew_after = Some(UNIX_EPOCH)`) and
     keeps its blob, as C++'s `invalidate_ticket` clears only
     `have_key_flag`.
  2. Call `check_auth_tickets` at once.

  AUTH is left valid because the wipe keeps the AUTH rotating keys
  (`CephxKeyServer.cc:509-521@v19.2.6`), and rados-rs has no in-session
  `GET_AUTH_SESSION_KEY` path.
- **`check_auth_tickets`** masks AUTH out of `needed_keys`: the C++
  client never sends AUTH in a principal request, and the server skips
  it there anyway (`CephxServiceHandler.cc:396-399@v19.2.6`).
- **`MAuthReply`.** A decoder in `monclient/messages.rs`, with the field
  order of `MAuthReply.h:44-51@v19.2.6`, following an existing plain
  message such as `MMonSubscribeAck`. It gets a `CEPH_MSG_AUTH_REPLY`
  arm in `dispatch_message`:
  - If `result != 0`, warn with the errno and `result_msg` and keep the
    old tickets.
  - Otherwise, lock the shared handler and call
    `handle_principal_reply(result_bl)`.
- **`CephXClientHandler::handle_principal_reply(Bytes) -> Result<()>`.**
  - Decode a `CephXResponseHeader`. A request type other than
    `CEPHX_GET_PRINCIPAL_SESSION_KEY` is ignored at debug level; a
    non-zero status is an error.
  - Decode the ticket list with `decode_extra_tickets` under the AUTH
    handler's session key (0x04, with Task 4's 0x05 rule) and store it.
  - The handler Arc is shared (`auth_config.rs:128-130`), so every OSD
    session that connects or reconnects afterwards presents the new
    tickets.
- **Limit (the AUTH ticket).** With this plan, service-ticket renewal
  works until the AUTH ticket expires (`auth_mon_ticket_ttl`, 72 h).
  After that, principal requests fail until the mon session
  re-authenticates. In-session AUTH renewal is the next priority for a
  long-lived RGW (Roadmap).

Unit tests:
- **v10 decode, hand-built.** Encode a v9 `MonMap` with the existing
  encoder, append the four v10 fields in `MonMap.cc:254-286@v19.2.6`
  order (`auth_epoch = 3`, `service_cipher = 2`, `allowed_ciphers =
  [1, 2]`, `preferred_cipher = 2`), and patch the header to struct
  version 10 (compat 6) with the new length. It decodes to `auth =
  Some { epoch: 3, service_cipher: 2, allowed_ciphers: [1, 2],
  preferred_cipher: 2 }`. The mon-produced fixture is Part B (B6).
- **Old maps.** Re-encoding a v9 `MonMap` still produces v9, and the
  corpus v6-v9 samples still decode with `auth: None`.
- **The rise rule**, as a pure function over `(Option<u32>,
  Option<u32>)`:

  | Held | New | Rise |
  |---|---|---|
  | None | 5 | no |
  | 5 | 5 | no |
  | 5 | 6 | yes |
  | 6 | 5 | no |
  | 5 | None | no |

- **Invalidation.** `invalidate_service_tickets` leaves AUTH's
  `need_key()` false and every other handler's true, with the blobs
  kept.
- **Mask.** With every handler due, the renewal request's `needed_keys`
  excludes AUTH.
- **Principal reply.** `handle_principal_reply` of a hand-built reply,
  AES and aes256k, replaces the OSD handler's key and blob, and the
  stored service set is exactly the one built. A wrong-type header is
  ignored, and a non-zero status errors.
- **Dispatch.** `MAuthReply` decodes a hand-built payload in the
  `MAuthReply.h` field order.

Commit:

```
monclient: refresh service tickets when the monmap's auth epoch rises

Ceph v19.2.6 monmaps (v10) carry an auth epoch that `auth
wipe-rotating-service-keys` raises; Ceph's MonClient then invalidates
its tickets and asks for new ones at once. Decode the v10 fields, do
the same for every service ticket (the wipe keeps the AUTH keys, so the
AUTH ticket stays valid for the request, and AUTH is never in the
principal request), and handle the monitor's MAuthReply, which was
dropped as an unknown message, so renewed tickets now land in the
handler every OSD session shares.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 9: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6. The per-commit build proof runs across the seven
commits (Tasks 1, 2, 3, 4, 5, 7A, 8).

**Gate** (sandboxed unless labelled):
- `cargo test --workspace --all-targets --offline`, the container
  fmt/clippy gate (`-D warnings`), and the corpus check for `MonMap` on
  both archives that carry it (unchanged JSON).
- Unsandboxed, against the existing v19.2.2 cluster exactly as in every
  prior plan: each suite in both CI lists (rados and rados-cls, CRC on
  and off), including `cephx_aes256k` with
  `CEPH_TEST_EXPECT_KEY_TYPE=1`, with `CEPH_CONF=/tmp/ceph/ceph.conf`.
  No `mon set`, no wipe, no new key type, no compose command.

**Evidence, in the ledger:** the unit-test count, the clippy/fmt result,
each v19.2.2 suite result, and the statement that no aes256k cluster was
exercised (Part B).

**Implementers:**
- Tasks 1 to 3 and Task 8 go to the opus tier: they are crypto and
  protocol state, not transcription.
- Tasks 4 and 5 go to the opus tier too: they are small but
  correctness-critical.
- Task 7A is small and runs against the existing cluster (controller).
- The whole-branch review is the session model, with one reviewer
  focused on Review Focus 1 and 2 against the Global Constraints'
  crypto facts.

The PR body:

```
**Motivation.** Ceph v19.2.6 and v20.2.4 add the cephx key type `aes256k` (AES256-CTS-HMAC-SHA384-192, RFC 8009) and use it for every new key; Rook creates new RGW keys with it. rados-rs cannot authenticate with such a key.

**What changed.** `CryptoKey` parses its type from the key header; the RFC 8009 cipher, per-usage keys and the HMAC challenge are added; two latent ticket and challenge bugs are fixed; MonMap v10's auth epoch triggers ticket refresh, and renewal replies are no longer dropped. Tests: Ceph's RFC and HMAC vectors, independent known answers, mock-server tests, and the v19.2.2 suites. Not yet run against an aes256k cluster.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

---

# Part B: deferred until rooket#58 (do not implement in this plan)

> **Deferred.** Everything in Part B waits for rooket
> ([jhoblitt/rooket#58](https://github.com/jhoblitt/rooket/issues/58)) to
> provide v19.2.6 (and later v20.2.4) clusters with configurable cephx
> cipher settings. Part B adds no compose file change, no sed copy of
> the compose file, no second compose or podman project, no
> `CEPH_IMAGE` parameter and no make or bash wrapper to rados-rs. The
> cluster, its ports and its conf path come from rooket; the commands
> below name them as `$ROOKET_CONF` and `rooket`'s own tooling, to be
> settled when #58 lands. Cluster settings need no rooket feature: the
> cipher policy goes in the Rook chart's `cephClusterSpec.security.cephx`
> values, `mon_allow_pool_delete` in its `configOverride`, and extra
> client keys of a chosen type are made by the test with `ceph auth
> get-or-create ... --key-type` under the exported admin keyring.
> None of these steps runs in this plan, so none
> carries a sandbox label yet; when Part B is scheduled, every step that
> drives the cluster runs unsandboxed. A Part B plan re-reads this
> section against the merged Part A before starting.

### Task B0: v19.2.6 bring-up (via rooket)

- A rooket-provided v19.2.6 cluster: fresh, so `aes256k` is the service,
  allowed and preferred cipher (R§5).
- Record `ceph mon dump -f json | jq '{auth_epoch, auth_service_cipher,
  auth_allowed_ciphers, auth_preferred_cipher}'`, confirm `ceph versions`
  shows 19.2.6, create `test-pool` with the `rados` application, and
  confirm the admin key begins `AgA`. If any of this differs, stop and
  bring it to the owner.
- A fresh v19.2.6 mon accepts every key the existing compose
  entrypoint's recipe would create (`ceph-authtool --gen-key` and
  `get-or-create` both default to aes256k, and `monmaptool` allows only
  aes256k), which matters only if rooket reuses that recipe.
- **Baseline.** Run `osdclient_object_io_test` against it with the fork
  `main` from before Part A, expecting the auth failure Part A fixes,
  then with Part A merged, expecting success. Record both.

### Task B6: Known-answer fixtures from a v19.2.6 mon and OSD

A mon at `debug_auth = 10` logs each ticket's `session_key=` as a base64
`CryptoKey` (`CephxProtocol.cc:65-75,89`; R§7, "Protocol transcript
KAT"). The client side is captured by wrapping the public `AuthProvider`
trait, which needs neither `tcpdump` nor a C++ build.

**Files:**
- Create: `rados/tests/cephx_capture.rs` (`#[ignore]`, run by the
  controller). It writes fixtures when `CEPHX_CAPTURE_OUT` is set.
- Create: `rados/tests/cephx_transcript_kat.rs` (not ignored, so it runs
  in `ci.yml`'s `cargo test --workspace --all-targets`).
- Create: `rados/tests/fixtures/cephx/v19.2.6-mon-crc.json`,
  `v19.2.6-mon-secure.json`, `v19.2.6-osd.json`, and
  `rados/tests/fixtures/monmap-v19.2.6.bin`.

**The capture tool:**
- `Recorder<P: AuthProvider>` delegates every call and appends
  `{direction, con_mode, global_id, payload_hex}` for each
  `build_auth_payload` output and each `handle_auth_response` input.
- **Mon, twice.** A raw msgr2 `Connection` to the rooket mon with
  `ConnectionConfig::with_auth_provider(Box::new(Recorder(mon_provider)))`
  and the admin keyring, once in CRC mode
  (`ConnectionConfig::prefer_crc_mode`) and once in SECURE mode.
- **OSD.** `ServiceAuthProvider::from_shared_handler(mon_provider.handler().clone())`
  in a `Recorder`, connected to an OSD with
  `with_auth_provider_and_service(.., OSD)` and the mon-assigned
  `global_id`. This records authorizer 1, the `AUTH_REPLY_MORE`
  challenge, authorizer 2 and the `AUTH_DONE` reply.
- **Capture steps:** `ceph config set mon debug_auth 10`; run the capture
  test with `CEPH_CONF=$ROOKET_CONF CEPHX_CAPTURE_OUT=…`; take the
  `session_key=` lines for each connection's `global_id` from the mon
  log; `ceph mon getmap` for `monmap-v19.2.6.bin`; `ceph config rm mon
  debug_auth`. Keys committed in fixtures come from the disposable
  cluster; they are test material, not secrets.

**Mon fixture fields:** `client_key` (the keyring's `AgA…`),
`con_mode`, `global_id`, `auth_done_hex`, `mon_logged_session_keys`
(`{service: base64}` for each ticket the log shows), and
`connection_secret_hex` (empty in CRC mode).

**OSD fixture fields:** `osd_session_key`, `authorizer1_hex`,
`challenge_hex`, `authorizer2_hex` and `reply_hex`.

**KAT tests** (public API only):
- **Mon (CRC).** `CephXClientHandler::set_secret_key_from_base64(client_key)`,
  then `handle_auth_done(auth_done, global_id, con_mode)`. Then:
  - the stored service set is exactly {AUTH, MON, OSD, MGR} (extra-ticket
    errors are dropped silently, so "every stored ticket matches" alone
    would pass over tickets never stored);
  - every stored ticket's type and secret equal the mon-logged key, and
    the OSD and MGR keys are type 2;
  - the connection-secret blob taken out of `auth_done_hex` passes
    `CryptoKey::decrypt(0x03, …)` directly under the logged AUTH key,
    with `Ok` and the correct magic, and fails under 0x04. (Asserting
    `None` from `handle_auth_done` in CRC mode proves nothing:
    `decode_connection_secret` returns `None` on every error.)
- **Mon (SECURE).** `handle_auth_done` returns `Some` connection secret
  of the negotiated length.
- **OSD.** With `osd_session_key`:
  - `challenge_hex` decrypts under 0x11;
  - `authorizer2_hex`'s `CephXAuthorizeB` decrypts under 0x10 and
    carries the challenge;
  - `reply_hex` decrypts under 0x12 to `nonce_plus_one == nonce + 1`,
    where `nonce` comes from authorizer 2;
  - both reply-side decrypts fail under a swapped usage.
- **MonMap.** `monmap-v19.2.6.bin` decodes to `auth = Some { epoch: <as
  dumped in B0>, service_cipher: 2, allowed_ciphers: [2],
  preferred_cipher: 2 }`.

### Task B7: aes256k cluster scenarios

**Files:**
- Modify: `rados/tests/cephx_aes256k.rs` (tests 2 and 3 below).
- Modify: `rados/tests/common/mod.rs`. `build_test_client` honours
  `CEPH_ENTITY` through `ClientBuilder::entity_name`, beside
  `CEPH_KEYRING`.

**Tests:**
1. `keyring_key_type_end_to_end` (from Task 7A) against the v19.2.6
   admin key with `CEPH_TEST_EXPECT_KEY_TYPE=2`, then every suite in both
   CI lists (rados and rados-cls, CRC on and off) against the rooket
   cluster.
2. **`two_key_types_side_by_side`.** The Rook mirror. Opt-in: it runs
   only when `CEPH_TEST_AES_ENTITY`/`CEPH_TEST_AES_KEYRING` and
   `CEPH_TEST_AES256K_ENTITY`/`CEPH_TEST_AES256K_KEYRING` are set.
   - Two `Client`s are up at once: an AES key standing in for CSI, and
     an `aes256k` key standing in for a newly created RGW key.
   - Assert their client key types are 1 and 2, their AUTH ticket types
     are 1 and 2, and their OSD ticket types are 1 and 2.
   - Each writes an object and reads the other's.
   - The `aes256k` client watches one object and the AES client
     notifies it and gets one ack; then the other way round.
3. **`key_type_not_allowed_is_eacces`.** Opt-in: it runs only when
   `CEPH_TEST_REJECTED_KEYRING` and `CEPH_TEST_REJECTED_ENTITY` are set,
   and otherwise logs a skip and returns. `Client::builder()` with that
   entity and keyring fails within 30 s, with an error whose text
   contains `os error 13`. `AUTH_BAD_METHOD` carries the mon's errno
   (`msgr2/phase/auth.rs:151-221`). Pin the exact variant observed.

**Scenario steps** (on the rooket cluster; `ceph` is the rooket
cluster's CLI):
- **Rook policy.** `ceph mon set auth_allowed_ciphers aes,aes256k`;
  `ceph mon set auth_preferred_cipher aes256k`; then
  `ceph auth get-or-create client.aes mon 'allow r' osd 'allow rwx
  pool=test-pool' --key_type aes`, **retried for up to 15 s**: it can
  fail with `EPERM` until `mon_auth_allow_insecure_key` is defaulted on
  the mon's next health tick (`AuthMonitor.cc:482-509,1532-1537`).
  `ceph auth get-or-create client.rgw mon 'allow rw' osd 'allow rwx'`
  (no key type: the preferred aes256k). Export both keyrings, confirm
  `AQ…` and `Ag…`, and run test 2.
  - The AES client gets all-AES session keys: AUTH follows the client
    key's type, and OSD is `min(1, 2)`. The `aes256k` client gets all
    type 2.
- **Mixed ticket types.** `ceph mon set auth_service_cipher aes`, then
  **always** `ceph auth wipe-rotating-service-keys` (the `mon set` never
  regenerates existing rotating keys), then run
  `keyring_key_type_end_to_end` as `client.rgw` with
  `CEPH_TEST_EXPECT_KEY_TYPE=2 CEPH_TEST_EXPECT_OSD_TICKET_TYPE=1`,
  retrying for up to 60 s. Then restore: `ceph mon set
  auth_service_cipher aes256k` **and** `ceph auth
  wipe-rotating-service-keys` again. Without the second wipe the
  rotating keys stay AES, the cluster raises HEALTH_ERR
  `AUTH_INSECURE_SERVICE_KEY_TYPE`, and later steps see the wrong ticket
  types. Each wipe raises `auth_epoch` and restarts rotating `secret_id`
  numbering (B8 relies on neither).
  - If the mon refuses `auth_service_cipher aes`, record the refusal and
    skip this step; Task 3 (c) pins the mixed path in unit tests.
- **AES refused.** `ceph mon set auth_allowed_ciphers aes256k`, then run
  test 3 with `client.aes`. If the mon refuses to drop `aes` while
  `client.aes` exists, record the refusal and bring it to the owner. Do
  not work around it.
- **Unrelated failures.** `git diff --stat v19.2.2 v19.2.6 -- src/cls`
  shows changes to `cls_2pc_queue` and `cls_rgw` (+120/-52). A failure
  on v19.2.6 not caused by auth: stop and bring it to the owner. Do not
  adapt the test.

### Task B8: `auth_epoch` wipe cluster test

**Cluster test** `tickets_refresh_when_auth_epoch_rises`, in
`rados/tests/cephx_aes256k.rs`.
- **Opt-in.** It runs only with `CEPH_TEST_ALLOW_WIPE=1`, and skips if
  the monmap has no `auth_epoch` (v19.2.2).
- **Steps.**
  1. Record the OSD ticket's `session_key.secret` and `ticket_blob.blob`
     bytes, and the monmap `auth_epoch`.
  2. Run `mon_client().invoke(vec![r#"{"prefix": "auth
     wipe-rotating-service-keys"}"#.into()], Bytes::new())`.
  3. Within 30 s, the monmap's `auth_epoch` is larger and the OSD
     ticket's session-key bytes and blob bytes have both changed.
     (`secret_id` may repeat, because a wipe restarts numbering at 1:
     `CephxKeyServer.cc:509-521`, `Auth.h:200-206`.)
  4. `close_primary_session_for_test("o")` (plan 12), then write and
     read `o`. Retry every 2 s up to 60 s: the OSD must itself fetch
     the new rotating keys before it accepts the new ticket.
- **If the OSD never accepts** the new ticket without a restart
  (`AuthMonitor.cc:2108-2109` wonders whether that is needed), record
  it. The test then pins only the client side: the rise, and the new
  session-key and blob bytes.
- Run it last on the rooket cluster, with `CEPH_TEST_ALLOW_WIPE=1`.

### Task B9: v19.2.6 CI matrix entry and cluster evidence

**Workflow.**
- `test-ceph` gains `strategy: { fail-fast: false, matrix: { include:
  [{ceph: v19.2.2, key_type: 1}, {ceph: v19.2.6, key_type: 2}] } }`, with
  `CEPH_TEST_EXPECT_KEY_TYPE: ${{ matrix.key_type }}` in the job `env`
  and the name `Ceph Integration Tests (${{ matrix.ceph }})`.
- How the v19.2.6 entry gets its cluster is rooket#58's outcome. This
  plan adds no compose variable, sed copy or project for it. If the only
  way to get a v19.2.6 CI cluster would be new compose plumbing in
  rados-rs, stop and bring it to the owner.
- The opt-in scenarios (B7 tests 2 and 3, B8's wipe) do not run in CI,
  because they need `ceph mon set`, extra entities or `auth
  wipe-rotating-service-keys` on the cluster.
- **Required check name.** If the fork's `main` requires the old check
  name, that is a repository-settings write: stop and ask the owner.
  Do not change it.
- A v19.2.6 failure not caused by auth: stop and bring it to the owner
  (see B7).

**Cluster evidence, in the ledger:**
- The B0 baseline failure and post-Part-A success.
- The B7 runs (every suite on v19.2.6), the Rook-mirror run, the
  mixed-ticket run including both wipes, and the AES-rejected run.
- The B8 wipe run.

---

## Roadmap for later plans

- **Part B of this plan**, once rooket#58 is ready.
- **In-session AUTH ticket renewal.** `CEPHX_GET_AUTH_SESSION_KEY` over
  `MAuth` with `old_ticket`, as C++ `CephxClientHandler::build_request`
  does. It is the next priority for a long-lived RGW: the AUTH ticket
  expires after `auth_mon_ticket_ttl` (72 h), and service-ticket renewal
  stops working then. It would make Task 4's 0x05 primary-list path
  live.
- **Mutual authentication of the OSD.** rados-rs never checks the OSD's
  `nonce_plus_one` reply (C++ `verify_reply`, `CephxProtocol.cc:594`),
  so the OSD is not authenticated to the client. Pre-existing.
- **A second mon handshake starts in the wrong phase.** On the shared
  handler it reuses the stale `server_challenge`: `reset()`
  (`client.rs:718-723`) has no production caller, and `build_auth_payload`
  (`provider.rs:135-143`) keys off `server_challenge`. Found by reading;
  not exercised by this plan.
- **More release coverage** (via rooket). The v20.2.4 image as a third
  matrix entry (same mechanism, per the research report's preamble). An
  upgraded-cluster scenario: v19.2.2 restarted on v19.2.6, expecting
  `allowed = {aes, aes256k}` and the old AES key still working (R§7
  scenario 4), and Rook's natural 2-3 h service-key turnover with mixed
  ticket types.
- **Mock server parity.** Ceph's AUTH_DONE layout, in
  `CephXServerHandler`.

## Review edits applied (2026-09-26)

- **Part A/Part B split.** Owner ruling: no new cluster plumbing
  (make/bash/podman/compose variants) in rados-rs; any cluster other
  than the existing v19.2.2 docker-compose cluster waits for rooket#58.
  So the primitive, key-type plumbing, usages, challenge fold,
  per-ticket types, MonMap v10, `MAuthReply`, renewal fixes, mock-server
  tests and literal-pinned KATs are Part A, gated on unit tests,
  clippy, fmt and the existing v19.2.2 suites; the v19.2.6 bring-up,
  captured KATs, aes256k cluster scenarios, wipe test and CI matrix
  entry are Part B, deferred. The sed-copied compose project and the
  `CEPH_IMAGE` compose parameter are removed. Part A states that it has
  no aes256k end-to-end coverage.
- **Finding 1** (secret_id repeats after a wipe): moved to Part B; B8
  compares session-key and blob bytes, and Global Constraints record the
  renumbering.
- **Finding 2** (vacuous pins): Part A: Task 3 decrypts the
  connection-secret blob directly under 0x03 vs 0x04, asserts the exact
  stored set, checks SECURE mode on hand-built data, and decrypts the
  authorize reply directly; the swallowed-error sites are listed in
  Global Constraints; the `nonce_plus_one` gap is in the Roadmap. The
  mon SECURE capture and the {AUTH, MON, OSD, MGR} assertion moved to
  Part B (B6).
- **Finding 3** (isolation contradiction): resolved without new
  plumbing. Part A runs the existing suites against the existing cluster
  read/write as every plan does but never changes its auth config; the
  review's second v19.2.2 project is dropped as new plumbing; the
  warning never to run the compose file with another image or without
  `-p` is in Global Constraints.
- **Finding 4** (false Rook claim): applied to Goal and Global
  Constraints (`setRotatingServiceKeyType`, no wipes, 2-3 h turnover,
  "a newly created RGW key").
- **Finding 5** (mixed-ticket contingency): moved to Part B; B7 always
  wipes after each `auth_service_cipher` change and retries the AES key
  creation for 15 s; the facts are in Global Constraints.
- **Finding 6** (dead_code under clippy): applied to Tasks 1 and 2.
- **Finding 7** (unrelated v19.2.6 failures): moved to Part B (B7, B9)
  as a stop point.
- **Finding 8** (mock-server key types): applied to Task 2.
- **Finding 9** (garbled decrypt step): applied to Task 1 verbatim.
- **Finding 10** (self-referential tests): applied; the review's
  literals are in Tasks 1, 2, 3 and 5 verbatim, plus Ceph's own
  `HMAC_SHA256` vectors from `src/test/crypto.cc`.
- **Finding 11** (ticket_enc overstatement, citations): applied to Task
  4's rules and commit message, Task 8, and Global Constraints
  (`CephxKeyServer.cc:509-521`, `CephxServiceHandler.cc:291,352,396-399,425-426`).
- **Finding 12** (call sites): applied to Task 2 and Global Constraints
  (`server.rs:179`, `server.rs:312`, `provider.rs:284-290`).
- **Finding 13** (sandbox labels): applied to what remains in Part A;
  Part B steps are not run in this plan and are marked to run
  unsandboxed when scheduled.
- **Finding 14** (branch protection): moved to Part B (B9): stop and ask
  the owner.
- **Finding 15** (AUTH never renewed): applied to Global Constraints,
  Task 8 and the Roadmap.
- **Attribution:** commits carry only the `Co-Authored-By` line; the PR
  body ends with the Claude Code line and no session URL, by the owner's
  rule (the review's "owner should confirm" is removed).
- **Base:** fork `main` at `c482382`, branch `cephx-aes256krb5`; rebase
  onto `main` after plan 17 merges if the base moves.
