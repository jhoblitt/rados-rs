# rados-rs RGW MVP, plan 18B: `cephx-aes256krb5` on a rooket cluster

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Plan 18's Part B, made executable.** Plan 18 Part A merged as fork
> PR #20 (`51bb3b5`) with its review fixes (`e601f44`, `c8c12cf`,
> `6e61fc7`, `e28525a`). Part B waited for rooket
> ([jhoblitt/rooket#58](https://github.com/jhoblitt/rooket/issues/58)),
> which has shipped. This plan replaces plan 18's Tasks B0, B6, B7, B8
> and B9. Every Part B scenario is kept. B9's CI matrix entry is dropped
> by owner decision (Task 8); its cluster evidence goes in the PR body.

**Goal:** Show Part A working end to end against Ceph releases that
issue `aes256k` keys, on clusters provisioned by rooket, and pin what the
C++ side produced:
- known-answer fixtures captured from a real v19.2.6 mon and OSD, checked
  in CI without a cluster (B6);
- an AES key and an `aes256k` key side by side, as Rook's CSI and RGW keys
  are (B7);
- a long-lived `aes256k` client holding mixed ticket types across a
  service-cipher change (B7);
- a key type the mon does not allow, refused (B7);
- service tickets refreshed when the monmap's `auth_epoch` rises (B8);
- passing against rooket clusters: every existing suite on v19.2.6, and
  the cephx suites on v20.2.4, Rook v1.20.7's default image (B7 test 1,
  and the second release run).

**Architecture.** Five test commits and one docs commit (Tasks 1-6). No
production code:
1. `tests/cephx/mod.rs` (entity helpers) and `cephx_capture.rs`: an
   ignored capture tool that records a cephx transcript on the wire and
   the mon's own log of the session keys.
2. Fixtures and `cephx_transcript_kat.rs`: known answers from the
   capture, not ignored, so `ci.yml` runs them.
3. More helpers and `two_key_types_side_by_side` in `cephx_aes256k.rs`:
   non-disruptive, safe in the existing CI lists.
4. `cephx_policy.rs`: the scenarios that change the cluster's auth
   policy, opt-in, serial, and self-restoring. Never in CI.
5. The `auth_epoch` wipe test in `cephx_policy.rs`.
6. `tests/common/mod.rs` docs: how to point every suite at a rooket
   cluster.

**Decisions** (the reasons are in Global Constraints):
- **No rooket config dir is committed.** The recipe (two files, three
  lines) is documented in `rados/tests/common/mod.rs`. This is the least
  intrusive choice for upstream `tchaikov/rados-rs`, whose CI does not use
  rooket; the dir becomes worth committing only if rooket is ever put in
  CI (Roadmap).
- **Suites run against the rooket cluster with no rados-rs harness code.**
  `CEPH_CONF` names rooket's conf, which fork `main` reads since
  `cephconfig-entity-section` merged (PR #24, `bd55b0a`); `CEPH_EXEC`
  names the toolbox; the pool settings the suites need are Rook defaults
  or rooket's base. Controller runs confirmed it.
- **Policy changes are made by the tests, through the admin client's mon
  command, and restored by the tests.** The toolbox is for recording,
  capture and recovery only.
- **v20.2.4 is a second, fresh rooket cluster** (`ROOKET_NAME=rados-rs-v20`),
  not a tag change on the running one: a tag change is an in-place Ceph
  upgrade by Rook, which gives an upgraded cluster rather than the fresh
  default-image cluster Rook v1.20.7 users get, and whose one-OSD upgrade
  path this plan does not verify.
- **CI is unchanged** (owner decision, Task 8). The cluster tests run
  locally, and their results go in the PR body.

**Tech Stack:** as plan 3. No new dependency: the tests use the `rados`
crate's existing dev-dependencies `tempfile`, `serde_json`, `hex` and
`rand` (`rados/Cargo.toml`, `[dev-dependencies]`).

**Spec:** plan 18
(`.superpowers/plan-copies/2026-09-26-rgw-mvp-18-cephx-aes256krb5.md`),
its Global Constraints and "Review edits applied". The research report
is `.superpowers/research/cephx-aes256krb5.md`, cited as "R§n".

## Global Constraints

Plans 3 to 18's Global Constraints apply where they fit: commit style,
gates, push after every gate, merge when green, an upstream PR after the
merge, offline builds, no new dependencies, and no `unwrap` on production
paths (this plan has none). Plan 18's wire and crypto facts, key usages,
ticket-type rules, mon policy and `auth_epoch` facts stand as written
there. Plus:

- **Branch and base.** Branch `cephx-aes256krb5-cluster` off fork `main`
  at or after `bd55b0a`, where PR #24 (`cephconfig-entity-section`) has
  merged: the conf lookup is entity-aware, and the msgr2 tests default
  to cephx. The branch lives in its own worktree, `$W`; the shared clone
  `$R` is never switched (Task 0). Task 0 stops if `origin/main` does not
  contain `bd55b0a`.
  Everything lands in `rados/tests/`: new files, `cephx_aes256k.rs`,
  `common/mod.rs`, and one doc line each in `object_locator_routing.rs`
  and `osdclient_split_merge.rs`. No production code, no workflow, no
  compose file.
- **Owner rulings.**
  - No new make/bash/podman/compose cluster plumbing in rados-rs. Clusters
    come from rooket.
  - Cluster settings come from Rook chart values or runtime `ceph config`
    / `ceph mon set`, never a new rooket feature.
  - Never add a `configOverride` layer: rooket's one-worker base sets
    `configOverride` to `[global] mon_data_avail_crit = 0` and
    `osd_pool_default_size = 1` (rooket `internal/values/base.go:111,126-137`),
    and a higher layer's string replaces it rather than adding to it
    (rooket README, "Chart values and profiles").
  - Commits carry only `Co-Authored-By: Claude Opus 5.5
    <noreply@anthropic.com>`, never a session link, and are made with
    `git -c user.name='Joshua Hoblitt' -c user.email='josh@hoblitt.com'
    commit`. PR bodies are at most 100 words and end with the Claude Code
    line, with no session URL.
  - Any repository-settings change (a required status check, a ruleset)
    stops and asks the owner.
- **The clusters are the controller's.** Implementers never run rooket,
  kubectl or podman, and never bring a cluster up or down. The ambient
  kubectl context is never used, and nothing in this plan runs a bare
  `kubectl`. Names used below:

  ```bash
  S=/tmp/claude-1000/-home-jhoblitt-github-ceph/a4b023ae-515b-4381-b56a-e1fedacbb0f6/scratchpad
  R=$S/rados-rs
  W=$S/wt-cephx-cluster                # the branch's worktree of $R (Task 0 Step 1)
  export CARGO_TARGET_DIR=$R/target    # shared with the clone, so /tmp does not fill
  ROOKET=$S/bin/rooket                 # built from rooket main; never ~/bin/rooket (old)
  export ROOKET_NAME=rados-rs          # Task 7 uses rados-rs-v20
  TB="$ROOKET k -n rook-ceph exec deploy/rook-ceph-tools --"      # toolbox, no stdin
  export CEPH_CONF=$S/rooket-ceph/ceph.conf
  export CEPH_EXEC="$ROOKET k -n rook-ceph exec -i deploy/rook-ceph-tools --"
  ```

  - The v19.2.6 cluster: `$ROOKET up --rook-version v1.20.7 --workers 1
    --config-dir $S/rooket-harness --wait`, where `config.yaml` is
    `profiles: [host-network]` and `values/rook-ceph-cluster.yaml` is
    `cephImage: {tag: v19.2.6}`; then `$ROOKET ceph-config --out
    $S/rooket-ceph`. Ready in 3m16s (controller, 2026-09-27).
  - `ceph-config` writes `ceph.conf` (`[global] mon_host = v2:IP:3300…`
    and `[client.admin] keyring = <out>/ceph.client.admin.keyring`) and the
    keyring (rooket `cmd/cephconfig.go:37-41,291-298`). It must be re-run
    whenever the cluster is recreated (new admin key).
  - `rooket k` forwards its arguments to kubectl with the cluster's own
    kubeconfig and passes stdin, stdout and stderr straight through
    (rooket `cmd/kubectl.go:25-35`). It refuses a cluster with no
    kubeconfig (`cmd/state.go:172-181`), so it cannot fall through to the
    ambient context.
- **Live facts (controller, 2026-09-27, `rados-rs` cluster).** These are
  settled; tasks record them again only as evidence.
  1. `ceph versions`: mon, mgr and osd all 19.2.6. client.admin is
     AES256KRB5 (type 2); `ceph-config` warns about it.
  2. `ceph mon dump -f json`: `auth_epoch: 0`, `auth_service_cipher`
     aes256k (2), `auth_allowed_ciphers` [aes, aes256k],
     `auth_preferred_cipher` aes256k (2), `min_mon_release_name` squid.
     These are monmap fields, not config-db keys: `ceph config get mon
     auth_service_cipher` is ENOENT.
  3. The toolbox has `/usr/bin/rados`, `/usr/bin/radosgw-admin` and
     `/usr/bin/ceph-dencoder`. `CEPH_EXEC` as above works for
     `object_locator_routing`'s CLI calls (7/7 passed).
  4. `ceph config set mon mon_allow_pool_delete true` through the toolbox
     works; the pool-delete tests passed.
  5. Every suite in both CI lists passes against the cluster (13 rados
     files, `monclient_command_operations`, all 12 rados-cls files), with
     two harness workarounds that `cephconfig-entity-section` has since
     removed (PR #24, `bd55b0a`):
     `CephConfig` reads `keyring` only from `[client]`/`[global]`
     (`rados/src/cephconfig/config.rs:127-131@1f9bf31`) while rooket puts
     it under `[client.admin]`; and `msgr2_connection_tests` falls back to
     `AuthMethod::None` when the conf has no `auth_client_required`
     (`rados/tests/msgr2_connection_tests.rs:81-86,166@1f9bf31`,
     `rados/src/msgr2/mod.rs:320-322`).
  6. `cephx_aes256k` with `CEPH_TEST_EXPECT_KEY_TYPE=2` passes: the AUTH
     and OSD tickets are type 2. The SECURE-mode msgr2 test passes with
     the aes256k session key and a 64-byte connection secret.
  7. Pools the tests create get size 1 (rooket's
     `osd_pool_default_size = 1`) and go active.
- **Rook v1.20.7 facts** (all `path:line@v1.20.7`, `~/github/rook`).
  - **Cipher policy.** `setDefaultCephxKeyType` runs `ceph mon set
    auth_allowed_ciphers aes,aes256k` and `ceph mon set
    auth_preferred_cipher aes256k`
    (`pkg/operator/ceph/cluster/cephx.go:98-133`,
    `pkg/apis/ceph.rook.io/v1/security.go:91-111`), and
    `setRotatingServiceKeyType` runs `ceph mon set auth_service_cipher
    aes256k` (`cephx.go:139-163`), both gated on Ceph v19.2.6+/v20.2.4+
    (`pkg/daemon/ceph/client/auth.go:126-135`). The values come from
    `spec.security.cephx` when it is set; these are the defaults.
  - **Re-applied on every reconcile**, unconditionally, with no read
    first. `setDefaultCephxKeyType` runs after `mons.Start`
    (`cluster.go:513`; `cephx.go:104-125`). `setRotatingServiceKeyType`
    runs only after mgrs, OSDs and `setIsSafeToRotateCephxKeys` succeed
    (`cluster.go:150-183`; `cephx.go:145-151`). Values come from
    `spec.security.cephx` when set, else `aes,aes256k` / `aes256k`
    (`security.go:91-111`).

    Reconciles come from:
    - operator start, or a manager reload (every CephCluster spec edit
      reloads);
    - deleting an owned object: a mon, OSD or mgr Deployment, a Service,
      a Secret or a ConfigMap;
    - a data change in an owned Secret or Service;
    - **retries of a failed reconcile**, with exponential backoff capped
      at 1000 s. Each retry that gets past `mons.Start` re-issues the
      first two `mon set`s.

    (`cluster/predicate.go:234-317`, `controller/predicate.go:176-274`.)
    A pod restart, Deployment status changes and status-only writes do
    not reconcile (`controller/predicate.go:252-255,290-301`). There is
    no periodic resync (`cr_manager.go:149-159`); controller-runtime's
    10 h cache resync is filtered out as unchanged.

    So Task 0 confirms that the last reconcile succeeded, and while
    `cephx_policy` runs nobody restarts the operator, edits the
    CephCluster or deletes a Rook-owned object. A racing reconcile can
    only write Rook's values back, and those are the tests' restore
    target. It can therefore fail a test but cannot leave the cluster off
    Rook's policy, and any AES rotating keys it leaves behind are wiped
    by the test's restore. The only opt-out is the
    `ceph.rook.io/do-not-reconcile` label
    (`pkg/apis/ceph.rook.io/v1/labels.go:28`,
    `cluster/controller.go:365-374`); this plan does not use it.
  - **No wipe, no rotation.** Rook never runs `auth
    wipe-rotating-service-keys` (`cephx.go:158-160` says why), and key
    rotation is off by default (`config/keyring/cephx.go:129-131`).
  - **Keys.** client.admin and `mon.` are generated with `ceph-authtool`
    and no `--key-type` on v19.2.6+, so they take the **operator
    image's** `ceph-authtool` default
    (`controller/cluster_info.go:125-129,284-288`), generated once at
    cluster creation (`:110-117`): aes256k (R§1; live fact 1). client.admin has
    `allow *` on mon, osd, mgr and mds
    (`pkg/daemon/ceph/client/keyring.go:32-39`). **CSI keys are AES**:
    the chart sets `cephClusterSpec.security.cephx.csi.keyType: aes`
    (`deploy/charts/rook-ceph-cluster/values.yaml:153-156`,
    `pkg/operator/ceph/csi/secrets.go:117-118`), which is exactly plan
    18's "CSI's AES key beside a new aes256k RGW key".
  - **Pool defaults.** Rook sets `mon allow pool delete = true` and `mon
    allow pool size one = true` (`pkg/operator/ceph/config/defaults.go:55-63`,
    `config/config.go:113`), written with `config assimilate-conf` into
    the mon config DB's `global` section on every reconcile's `startMons`
    (`config/monstore.go:277-300`). Live fact 4's `config set` was
    therefore redundant; Task 0 checks `mon_allow_pool_delete` and sets
    nothing.
  - **Toolbox.** The chart's toolbox runs `cephImage` unless a toolbox
    image is given (`templates/deployment.yaml:39-45`), and writes
    `/etc/ceph/ceph.conf` and a client.admin keyring from the
    `rook-ceph-mon` secret (`:58-59,66-110,186-214`). `kubectl exec -i`
    pipes stdin to the command whatever the pod spec says; `:182-183` is
    `tty: true` and concerns only the main process. The chart leaves the
    toolbox off (`values.yaml:26`); rooket enables it
    (`internal/values/base.go:109`).
  - **Mon logs.** Every daemon logs to stderr with the prefix `debug `
    (`config/defaults.go:40-51`); the `mon` container is the pod's first
    (`cluster/mon/spec.go:211-213`), so `rooket k -n rook-ceph logs
    deploy/rook-ceph-mon-a -c mon` shows the mon's log. Rook overrides no
    `debug_auth`. A rooket cluster with fewer than 3 hosts has one mon
    (rooket `internal/values/base.go:77-78,113-114`).
  - **Versions.** Rook v1.20.7 supports Ceph 19 and 20 without
    `allowUnsupported` (`pkg/operator/ceph/version/version.go:43,53`);
    the chart's default image is `quay.io/ceph/ceph:v20.2.4`
    (`values.yaml:104-108`).
- **Ceph facts for the scenarios** (`path:line@v19.2.6`, same at
  v20.2.4 unless stated; `~/github/ceph`).
  - **`key_type`.** `auth add`, `auth get-or-create-key` and `auth
    get-or-create` take `name=key_type,type=CephString,req=false` after
    `--`, which makes it non-positional (`src/mon/MonCommands.h:160-197`,
    +4 lines at v20.2.4; `src/common/cmdparse.cc:146-147,180-181`). In a
    mon command it is the JSON field `"key_type"`; on the CLI it is
    `--key-type aes` (dashes become underscores,
    `src/pybind/ceph_argparse.py:1190-1215`). The mon accepts `aes`,
    `aes256k`, `none`, or the default `preferred`, which means
    `auth_preferred_cipher` (`src/mon/AuthMonitor.cc:1510-1527`,
    `src/auth/Crypto.cc:1225-1238`).
  - **An existing entity keeps its key.** `auth get-or-create` on an
    existing entity returns its key whatever `key_type` asks for, and
    compares only caps (`AuthMonitor.cc:1831-1851`); only a new entity's
    key is created with the type (`:1878`). The key type is still
    validated first (`:1824-1828`).
  - **Refused key types are EINVAL on the wire.**
    `AuthMonitor::get_cipher_type` refuses an insecure type (every type
    but aes256k, `Crypto.cc:1257`) unless `mon_auth_allow_insecure_key`
    is true, and a type not in `auth_allowed_ciphers`, both with EPERM
    (`AuthMonitor.cc:1533-1545`); but `auth get-or-create` replaces any
    negative result with EINVAL (`:1824-1828`). So a mon command gets
    `retval == -22` and `outs` `creating key with insecure key type
    ("aes") not allowed` or `refusing to create key with type (aes) that
    cannot be used for auth (auth_allowed_ciphers)`.
  - **When AES becomes creatable.** `mon_auth_allow_insecure_key`
    defaults to false (`src/common/options/mon.yaml.in:769-784`). The
    leader's `check_health` sets its default to true while an insecure
    type is allowed, and back to false otherwise
    (`AuthMonitor.cc:489-509`); it runs from `AuthMonitor::tick` every
    `mon_tick_interval` (5 s, `mon.yaml.in:1355`) and on every auth
    proposal (`:131-188,479`). After `mon set auth_allowed_ciphers
    aes,aes256k` that takes the monmap commit, a re-election and up to
    one tick: about 10 s. The tests allow 30 s.
  - **`mon set`** takes `name` ∈ {`auth_service_cipher`,
    `auth_allowed_ciphers`, `auth_preferred_cipher`} and a `value`
    (`MonCommands.h:568-572`; `:573-577@v20.2.4`). The handler
    (`src/mon/MonmapMonitor.cc:1253-1325`, +1 at v20.2.4):
    - checks only that each name is a known type and, for aes256k, that
      the quorum has the feature. So `auth_service_cipher aes` is
      accepted, and so is `auth_allowed_ciphers aes256k` while AES
      entities exist (the list is split on `", "`);
    - answers an unchanged value with `already set` and 0, proposing
      nothing (`:1264-1268`), so repeating a `mon set` is safe;
    - otherwise proposes a new monmap whose `epoch` rises by 1. It leaves
      `auth_epoch` alone: `bump_auth_epoch`'s only caller is the wipe
      (`AuthMonitor.cc:2133-2135`).
    - A committed monmap makes each mon re-bootstrap and re-elect
      (`MonmapMonitor.cc:93-96`). That drops no client session
      (`Monitor::_reset`, `src/mon/Monitor.cc:1367-1399`) but can delay
      the next command, so the test helpers retry and then check the
      state they asked for.
  - **An entity whose key type is not allowed** fails
    `CEPHX_GET_AUTH_SESSION_KEY` with `-EACCES`
    (`src/auth/cephx/CephxServiceHandler.cc:195-209`).
  - **Wipe.** `auth wipe-rotating-service-keys` (`MonCommands.h:179`,
    `:183@v20.2.4`; handler `AuthMonitor.cc:2107-2139`):
    - keeps the AUTH rotating keys and regenerates the mon/osd/mds/mgr
      ones with `auth_service_cipher` (`src/auth/cephx/CephxKeyServer.cc:501-538,293,309-312`),
      restarting their `secret_id`s at 1 (`src/auth/Auth.h:205`);
    - sets the monmap's `auth_epoch` to the AuthMonitor's next version,
      not the old value plus one (`AuthMonitor.cc:2133-2135`,
      `MonmapMonitor.cc:1600-1610`), so tests assert only that it rose;
    - the new monmap is pushed to every subscribed client once it
      commits (`MonmapMonitor.cc:1547-1561`, `Monitor.cc:5514-5519`).
    - A daemon's C++ MonClient wipes its rotating secrets and refetches
      them at once on the rise (`src/mon/MonClient.cc:466-469,649-656`;
      +7 at v20.2.4, `:473`, `:658`).
      Until it has, it refuses the new tickets
      (`src/auth/RotatingKeyRing.cc:87`, `CephxProtocol.cc:506`); hence
      the tests' I/O retries.
    - `mon set auth_service_cipher` regenerates nothing: rotating keys
      are only added by the tick's rotation or the wipe
      (`AuthMonitor.cc:87,2127`). Without a wipe they turn over in about
      3 × `auth_service_ticket_ttl` (1 h) (`CephxKeyServer.cc:285-331`,
      `Auth.h:215-216`).
  - **Health** (`AuthMonitor::check_health`, same lines at v20.2.4):
    - `AUTH_INSECURE_SERVICE_TICKETS` (ERR) while `auth_service_cipher`
      is not aes256k (`:521-523`);
    - `AUTH_INSECURE_ROTATING_SERVICE_KEY_TYPE` (WARN) while any
      mon/osd/mds/mgr rotating key is still AES, cleared by a wipe
      (`:613-649`);
    - `AUTH_INSECURE_KEYS_ALLOWED` and `AUTH_INSECURE_KEYS_CREATABLE`
      (WARN) while aes is allowed (`:501,513`);
    - `AUTH_INSECURE_CLIENT_KEY_TYPE` (WARN) for client entities with an
      AES key (`:598`). Rook's CSI keys raise it on every Rook v1.20.7
      cluster, so Task 0's baseline already has WARNs. A test must leave
      no ERR, and no health code that was not in the baseline.
  - **Session-key log.** `build_service_ticket` logs at `debug_auth` 10,
    with the prefix `cephx: ` (`src/auth/cephx/CephxProtocol.cc:22-24,89`):
    `build_service_ticket service session_auth_info(<svc> id=<secret_id>
    session_key=<key> service_secret=<key> ticket.name=<entity>
    ticket.global_id=<gid>)` (`:65-75`). A key prints as the base64 of
    its whole encoding, header included (`CryptoKey::print`,
    `src/auth/Crypto.cc:1151-1154`): an aes256k key starts `Ag`, and the
    third character varies with `created`. The line is logged for every
    ticket the mon issues to a client (`CephxServiceHandler.cc:289,351,425`)
    and for the mon's own authorizers (`Monitor.cc:6433`), so the
    capture filters on both `ticket.name` and `ticket.global_id`. The same
    line prints the rotating `service_secret`. At the same level,
    `build_service_ticket_reply encoding <n> tickets with secret <key>`
    prints the authenticating entity's long-term key
    (`CephxProtocol.cc:125-126`; `CephxServiceHandler.cc:218,289-291`).
    So while `debug_auth` is 10, the mon logs the permanent key of every
    entity that authenticates to it, client.admin included, and the node
    keeps that log until the cluster is deleted. The level is raised only
    around the capture and restored by a trap. The capture tool never
    prints, stores or returns a log line; its failure messages give
    counts and global_ids only. No log text goes into a fixture or the
    ledger. `debug_auth`'s default is `1/5` (`src/common/subsys.h:56`).
  - **`mon getmap`** encodes with the CLI connection's features
    (`MonmapMonitor.cc:350-376`), which gives a current CLI the v10
    struct (`src/mon/MonMap.cc:254` `ENCODE_START(10, 6)`, `:279-286`),
    and the CLI writes the raw bytes to `-o`; `-o -` writes to stdout
    (`src/ceph.in:1173-1174`). `ceph-dencoder … import -` reads stdin
    (`src/tools/ceph-dencoder/ceph_dencoder.cc:209-212`).
  - **Pools.** Pool creation takes its size from `osd_pool_default_size`
    and checks no `mon_allow_pool_size_one`; only `osd pool set size 1`
    does (`src/mon/OSDMonitor.cc:7768-7770,8441-8445`). Pool deletion
    needs `mon_allow_pool_delete` (`:14769-14771`), default false
    (`src/common/options/global.yaml.in:1619`).
  - **AUTH_DONE layout.** The AUTH session key takes the client key's
    type (`src/auth/cephx/CephxServiceHandler.cc:278`). In both modes the
    mon encrypts a connection secret under the AUTH session key with
    usage 0x03 (`:309-328`); its length is 0 in CRC mode and 64 in SECURE
    mode (`src/auth/Auth.h:147-155`), so a CRC-mode AUTH_DONE still
    carries a decryptable envelope of an empty secret. The extra tickets
    follow, built per requested service (`:329-354`).
  - **`mon dump -f json`** has `auth_epoch`, `auth_service_cipher`
    (`{name, value}`), `auth_allowed_ciphers` (a list of `{name, value}`)
    and `auth_preferred_cipher` at v19.2.6 and v20.2.4
    (`src/mon/MonMap.cc:520-545`), and none of them at v19.2.2
    (`MonMap::dump` at `src/mon/MonMap.cc:430-445@v19.2.2`). This is how a
    test tells an aes256k-capable cluster apart (rados-rs facts,
    "`auth_epoch` cannot tell pre-v10 from epoch 0").
  - **Object classes.** `hello` is in `osd_class_load_list` and
    `osd_class_default_list` at both tags
    (`src/common/options/osd.yaml.in:626-638@v19.2.6`,
    `:671-683@v20.2.4`).
- **rados-rs facts** (`@1f9bf31`, fork `main` before PR #24, whose tree
  equals the clone's `261ca38`). Re-find by symbol at `bd55b0a`: PR #24
  (`cephconfig-entity-section`, merged) rewrote `cephconfig/config.rs`
  and changed `client.rs`, `monclient/auth_config.rs`, `msgr2/mod.rs`,
  `cephx_aes256k.rs`, `osdclient_integration_test.rs` and
  `msgr2_connection_tests.rs`: the conf lookup is entity-aware, and the
  msgr2 tests default to cephx. Since then `ClientBuilder` reads the conf
  as its own entity, so a test entity other than client.admin finds no
  keyring in rooket's conf and passes one explicitly with
  `.keyring(path)`, as Task 3's `client_for` does; an explicit
  `.keyring()` wins.
  - **`auth_epoch` cannot tell pre-v10 from epoch 0.** `MonMapState`
    keeps `pub auth_epoch: u32`, 0 for a map without the v10 fields and
    `u32::MAX` before the first map (`rados/src/monclient/monmap.rs:34-39,106,421`);
    a fresh v19.2.6 cluster's epoch is also 0 (live fact 2). So a test
    decides "aes256k-capable" from the mon's `mon dump` JSON, never from
    `auth_epoch == 0`. The rise rule is `held < new`
    (`monmap.rs:446-448`), so 0 → 1 after the first wipe is a rise.
  - **Decoded v10 fields.** `rados::MonMap` has `pub auth:
    Option<rados::denc::MonMapAuth>`. `MonMapAuth` is in `rados::denc`,
    not at the crate root, with `epoch: u32` and `i32` ciphers
    `service_cipher`, `allowed_ciphers` and `preferred_cipher`
    (`rados/src/denc/monmap.rs:181-193`). There is no
    `rados::MonMap::decode(bytes)`: decode with
    `<rados::MonMap as rados::Denc>::decode(&mut buf, features)`
    (`denc/monmap.rs:379`) or `rados::monclient::MonMapState::decode(&[u8])`
    (`monclient/monmap.rs:288`).
  - **APIs the tests use**, all public:
    - `ClientBuilder::entity_name` and `::keyring`
      (`rados/src/client.rs:203,228`); `common::test_client_builder`
      (`rados/tests/common/mod.rs:35-51`).
    - `MonClient::invoke(Vec<String>, Bytes) -> CommandResult { retval,
      outs, outbl }` (`monclient/client.rs:1492`,
      `monclient/types.rs:7-14`), `get_monmap()` (`:1843`) and
      `get_service_auth_provider()` (`:1687`).
    - `ServiceAuthProvider::handler()` and `from_shared_handler`
      (`auth/provider.rs:203-222`); `MonitorAuthProvider::handler()`
      (`:127`); `CephXClientHandler::get_session()`
      (`auth/client.rs:730`), whose `ticket_handlers` hold
      `TicketHandler { session_key: CryptoKey, ticket_blob:
      Option<CephXTicketBlob { secret_id, blob }>, .. }`
      (`auth/types.rs:335-340,459-465`).
    - `CryptoKey::from_base64`, `decrypt(usage, ..)`, `key_type()`
      (`auth/types.rs:136,157,198`); the `CEPHX_KEY_USAGE_*` constants
      and `CEPH_CON_MODE_CRC = 1` / `SECURE = 2`
      (`auth/protocol.rs:19-30`).
    - `CephXClientHandler::new(entity, AuthMode::Mon)`,
      `set_secret_key_from_base64`, `handle_auth_done(payload,
      global_id, con_mode)`, `decrypt_authorize_challenge`
      (`auth/client.rs:109,141,486,587`).
    - The `AuthProvider` trait (`auth/provider.rs:21-50`); only
      `monclient/connection.rs:136` downcasts through `as_any`, so a
      wrapping provider works on a raw `msgr2::Connection`.
    - `ConnectionConfig`'s fields are all `pub` (`msgr2/mod.rs:249-315`),
      and `ConnectionConfig::with_auth_provider_and_service(provider, service)`
      builds one for a service; `Connection::connect`,
      `Connection::connect_with_target(addr, entity_addr, config)`,
      `establish_session` and `global_id`
      (`msgr2/protocol.rs:1040,1020,1324,1825`). The OSD session's
      pattern is `osdclient/session.rs:339-351`.
    - `IoCtx::close_primary_session_for_test` (`osdclient/ioctx.rs:813`).
    - `Keyring::from_string` (`auth/keyring.rs:33`), which trims each
      line, so the mon's tab-indented keyring text parses.
  - **Cleanup pattern.** Cluster tests wrap their body in
    `AssertUnwindSafe(..).catch_unwind()`, clean up, then
    `resume_unwind` (`rados/tests/object_locator_routing.rs:651,699-707`,
    `osdclient_split_merge.rs:239,313-321`). The new tests do the same
    for entities and policy, with the ordering and failure collection of
    the Test design rules.
  - **`CEPH_EXEC`** is split on whitespace into a program and leading
    arguments (`object_locator_routing.rs:165-171`,
    `osdclient_split_merge.rs:46-50`).
  - **CI lists.** `.github/workflows/test-with-ceph.yml:78,86,93,103,113,123`
    run the cluster suites against the compose v19.2.2 cluster;
    `ci.yml:87` runs `cargo test --workspace --all-targets`, so a
    non-ignored test in `rados/tests/` runs in CI without a cluster.
- **Test design rules.**
  - A test that changes cluster-wide auth state (`mon set`, `auth
    wipe-rotating-service-keys`) lives in `rados/tests/cephx_policy.rs`,
    never in `cephx_aes256k.rs`: cargo runs one binary's tests
    concurrently, and the CI lists run `cephx_aes256k`. `cephx_policy`
    runs only with `CEPH_TEST_ALLOW_AUTH_POLICY=1`, always with
    `--test-threads=1`, alone (no other suite against the same cluster
    at the same time), and is never added to a CI list.
  - Each policy test calls `assert_rook_policy` **before** it arms its
    cleanup guard. A cluster found off Rook's policy fails with the
    recovery commands and is not touched. The guard then runs every step
    whatever fails, collecting failures instead of panicking: restore the
    policy (with a wipe after any `auth_service_cipher` change, plan 18
    review finding 5), run the test's post-restore checks, remove objects
    and entities, and only then assert, panicking with every collected
    failure. If the body also panicked, the guard prints its own failures
    and `resume_unwind`s the body's panic.
  - `cephx_policy.rs` has
    `static POLICY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());`,
    and every test holds it for its whole run, so the tests serialize
    even without `--test-threads=1` (which is still passed, for readable
    output). Each body runs under
    `tokio::time::timeout(Duration::from_secs(300), ..)`, so a hang fails
    into the cleanup guard. Only a killed process (Ctrl-C, SIGKILL) skips
    cleanup, and the next run's precondition reports it with the recovery
    commands. Retry budgets are wall time, not attempts: one `invoke` can
    take 60 s (`monclient/client.rs:936-938`, `defaults.rs:9`).
  - Entities are `client.rados-rs-<purpose>-<pid>-<nanos>` with caps `mon
    'allow r'` and `osd 'allow rwx pool=<test pool>'`, created with an
    explicit `key_type` (never the mon's preference), and removed in
    cleanup with `auth rm`. Every auth command (`auth get-or-create`,
    `auth rm`, `auth wipe-rotating-service-keys`) and `mon set` goes
    through the **admin** client: they need `auth rwx`
    (`MonCommands.h:179-181,190-197,229-232`) or mon write caps, which a
    test entity's `mon 'allow r'` lacks. `mon dump` needs only `mon r`
    (`:504-507`).
  - Creating an AES key retries a refusal (`retval == -22` with `outs`
    naming the insecure key type) every second for up to 30 s: the mon
    refuses insecure key types until its health tick sets
    `mon_auth_allow_insecure_key`'s default (Ceph facts, "When AES becomes
    creatable"). This is plan 18's "EPERM retry"; the wire value is
    EINVAL.
  - Rotating-key changes compare session-key and blob bytes, never
    `secret_id`, which restarts at 1 after a wipe (plan 18 review
    finding 1).
  - Expected ticket types are derived from the monmap the test reads
    (`min(key type, auth_service_cipher)` for services, the key's type
    for AUTH), not hard-coded, except where the test itself set the
    policy.
- **Sandbox.** Build, unit tests, clippy and fmt run sandboxed. Every
  command that reaches a rooket cluster (`$ROOKET …`, and any `cargo
  test` with `CEPH_CONF=$S/rooket-ceph…`) runs unsandboxed and is
  labelled so; the controller runs them.

## Review Focus

1. **The known answers are independent of rados-rs** (Task 2). Every
   expected key comes from the mon's own log. The AUTH_DONE and the
   OSD's challenge and reply are C++ output. Authorizers 1 and 2 are
   rados-rs output that the OSD accepted, and the capture writes no
   fixture unless every `establish_session()` returned Ok. The MonMap
   expectation comes from `ceph-dencoder`. The stored service set is
   compared with the logged set, so a silently dropped extra ticket
   fails. Connection secrets are decrypted directly, not through
   `handle_auth_done`'s `Option`.
2. **Disruptive tests cannot run by accident** (Tasks 4 and 5): their own
   binary, an env gate, the `POLICY` mutex (and `--test-threads=1`), a
   300 s body timeout, not in any CI list, a precondition checked before
   the cleanup guard is armed, and a restore that is asserted last.
3. **Restore is complete**: policy back to Rook's values with a wipe
   after each service-cipher change, entities removed even when an
   assertion fails, `debug_auth` restored by a trap, and a toolbox check
   after each run (Tasks 1, 4, 5).
4. **The mixed window is real** (Task 4): the same long-lived client
   holds an AUTH ticket of type 2 and an OSD ticket of type 1 after the
   epoch rise, and does I/O on a new OSD session with it.
5. **No harness plumbing in rados-rs**: no committed rooket config, no
   script, no compose or workflow change; documentation only.

---

### Task 0 (B0): Branch, workspace, cluster pre-flight (controller)

**Files:** none.

- [ ] **Step 1: Base and worktree** (unsandboxed git, per the sandbox's
  stale-git note):

```bash
cd $R && git fetch origin && git log --oneline -5 origin/main
git merge-base --is-ancestor bd55b0a origin/main && echo base-ok     # PR #24's merge
```

  Expected: `base-ok`. If not, stop: the suites cannot read rooket's
  conf. Then create `cephx-aes256krb5-cluster` from `origin/main` in its
  own worktree, never by switching the shared clone:

```bash
git -C $R worktree add --no-track -b cephx-aes256krb5-cluster $W origin/main
```

  `$R` is in use for other work (during review it was on another branch,
  and briefly mid-rebase), and the owner's rule builds every branch in a
  worktree. The implementer uses this `git worktree add` worktree of the
  scratch clone at `$S/wt-cephx-cluster` (`$W`), on branch
  `cephx-aes256krb5-cluster` from `origin/main`, with
  `CARGO_TARGET_DIR=$R/target` shared with the clone so `/tmp` does not
  fill. `EnterWorktree` applies only to the session's own repo, and this
  clone is not it. Every later `cd $W` means that worktree. Then the SDD
  ledger, as plan 3's Task 0 does, with the rulings above recorded in it.

- [ ] **Step 2: Record the cluster** (unsandboxed; read-only):

```bash
$ROOKET list
$TB ceph versions
$TB ceph mon dump -f json | jq '{epoch, auth_epoch, auth_service_cipher, auth_allowed_ciphers, auth_preferred_cipher, min_mon_release_name}'   # jq on the host
$TB ceph auth get client.admin | awk '$1 == "key" {print substr($3, 1, 2)}'   # prints Ag; never log the key
$TB ceph config get mon mon_allow_pool_delete      # true (Rook default)
$TB ceph osd pool ls detail | grep -E "^pool [0-9]+ 'test-pool'" || echo 'no test-pool'
$TB ceph health detail
$ROOKET k -n rook-ceph get cephcluster -o jsonpath='{range .items[*]}{.status.phase}{" "}{.status.message}{"\n"}{end}'   # Ready
$ROOKET k -n rook-ceph logs deploy/rook-ceph-operator --since=1h | grep -ci 'failed to reconcile' || true             # 0 (live-verify the wording)
$ROOKET k get pvc -A      # live-verify "no CSI volume is in use" (Task 4)
```

  Expected: live facts 1 and 2; the CephCluster `Ready` and no failed
  reconcile in the operator's last hour, so the last reconcile succeeded
  (Rook facts, "Re-applied on every reconcile"); no PVC, so no CSI volume
  is in use. Re-run the CephCluster and operator-log commands right
  before each `cephx_policy` run (Tasks 4, 5 and 7). If `test-pool` is
  missing, run `cargo test -p rados --offline --test
  osdclient_integration_test -- --ignored` in `$W` first (unsandboxed),
  which creates it as CI does (32 PGs, autoscale off; commit `a4f0c84`).
  Record the health codes present before any test, so later runs can be
  compared with them.

- [ ] **Step 3: Baseline, before Part A** (unsandboxed). Plan 18's B0
  evidence that Part A was needed. `6632c80` is fork `main` just before
  PR #20. Its `common` honours `CEPH_KEYRING`, and its `CephConfig` does
  not read `[client.admin]`, so the keyring is passed explicitly:

```bash
mkdir -p $S/rados-rs-6632c80 && git -C $R archive 6632c80 | tar -x -C $S/rados-rs-6632c80
cd $S/rados-rs-6632c80 && CEPH_KEYRING=$S/rooket-ceph/ceph.client.admin.keyring \
  cargo test -p rados --offline --test osdclient_object_io_test -- --ignored --exact test_rados_object_write_and_read_back
```

  Expected: failure at authentication (the pre-Part-A client used the
  first 16 bytes of the `AgA…` secret under AES; plan 18, "rados-rs
  today"). Record the error text. The post-Part-A success is live fact 6.
  `rm -rf $S/rados-rs-6632c80` afterwards.

---

### Task 1 (B6): The capture tool

**Files:**
- Create: `rados/tests/cephx/mod.rs`, test helpers shared by
  `cephx_capture.rs`, `cephx_aes256k.rs` and `cephx_policy.rs` (`mod
  cephx;` in each; cargo builds `tests/*.rs` and `tests/*/main.rs` as
  targets, not `tests/<dir>/mod.rs`, as with `common`). It starts with
  `#![allow(dead_code)]`, as `common/mod.rs:12` does, and holds:
  - `fn unique(purpose) -> String`: `rados-rs-<purpose>-<pid>-<nanos>`,
    as `object_locator_routing.rs:25-31` builds names;
  - `pub struct Entity { name: String, keyring: tempfile::NamedTempFile,
    key_type: u16 }` and `async fn create_entity(admin: &Client, purpose:
    &str, key_type: &str) -> Entity`: `{"prefix": "auth get-or-create",
    "entity": "client.<unique>", "caps": ["mon", "allow r", "osd", "allow
    rwx pool=<test pool>"], "key_type": key_type}` through `invoke`; the
    keyring text is `outbl`, written to the temp file. It retries the
    insecure-type refusal (`retval == -22`, `outs` containing `insecure
    key type`) every second for up to 30 s; any other non-zero `retval`
    fails at once with `outs`. It asserts the parsed key's type is the
    one asked for, since `get-or-create` on an existing entity returns
    its key whatever `key_type` says (Ceph facts, "An existing entity
    keeps its key");
  - `async fn remove_entity(admin: &Client, name: &str)`: `{"prefix":
    "auth rm", "entity": name}`, logging rather than panicking on
    failure, since it runs in cleanup.
- Create: `rados/tests/cephx_capture.rs`. Every test `#[ignore]`; not in
  any CI list.

**What it does.** One test, `capture_transcripts`, which returns early
with a printed skip unless `CEPHX_CAPTURE_OUT` (a directory) and
`CEPHX_MON_LOG_CMD` (a command prefix, split on whitespace as
`CEPH_EXEC` is) are both set. It:
1. Builds an admin client (`common::build_test_client`) and creates an
   `aes256k` entity with `cephx::create_entity(admin, "kat", "aes256k")`.
2. **Mon, twice.** For CRC and then SECURE: a `MonitorAuthProvider` for
   the entity with its key; its handler `Arc` is cloned first
   (`provider.handler()`), then the provider is moved into a `Recorder`.
   A raw `msgr2::Connection` to the first `v2:` mon address in
   `CEPH_CONF`, with `ConnectionConfig { preferred_modes: [Crc] or
   [Secure], entity_name, ..ConnectionConfig::with_auth_provider(Box::new(recorder)) }`,
   then `establish_session()`. The recorded payloads, the connection's
   `global_id()` and the mode go into the fixture.
3. **OSD.** Using the SECURE session's handler, a
   `ServiceAuthProvider::from_shared_handler(handler)` in a second
   `Recorder`, and a raw connection to `osd.0`'s `v2:` address (read from
   the admin client's OSDMap), built as `osdclient/session.rs:339-351`
   builds one:
   `ConnectionConfig::with_auth_provider_and_service(Box::new(recorder), EntityType::OSD.bits())`,
   with the mon-assigned `global_id` and the entity name set as fields,
   then `Connection::connect_with_target(addr, entity_addr, config)`
   (`msgr2/protocol.rs:1020`) and `establish_session()`. This records
   authorizer 1, the `AUTH_REPLY_MORE` challenge, authorizer 2 and the
   final reply.
4. **Mon log.** Runs `$CEPHX_MON_LOG_CMD` (for example `rooket k -n
   rook-ceph logs deploy/rook-ceph-mon-a -c mon --since=10m`), retrying
   every 2 s for up to 20 s until, for each captured `global_id`, a
   `build_service_ticket service session_auth_info(` line with
   `ticket.name=<entity>` and that `ticket.global_id=` names every
   service stored in that session. It parses `<svc>` and `session_key=`
   from those lines only (the mon's own authorizers log the same line),
   and keeps nothing else from the log. It never prints, stores or
   returns a log line, and its failure messages give counts and
   global_ids only: at `debug_auth` 10 the mon also logs rotating and
   long-term keys (Ceph facts, "Session-key log").
5. Only after both mon sessions and the OSD session returned Ok, writes
   `v<release>-mon-crc.json`, `v<release>-mon-secure.json` and
   `v<release>-osd.json` into `CEPHX_CAPTURE_OUT`, where `<release>`
   comes from `CEPHX_CAPTURE_RELEASE` (required, e.g. `19.2.6`).
6. Removes the entity (`auth rm`) in a cleanup guard.

**`Recorder<P: AuthProvider>`** delegates every trait method to `P` and
appends `{direction: "out"|"in", con_mode, global_id, payload_hex}` for
each `build_auth_payload` result and each `handle_auth_response` input to
an `Arc<Mutex<Vec<_>>>` it shares with its clones (`clone_box` clones the
inner provider and shares the log). `as_any` returns the recorder
itself.

**Fixture fields** (JSON; keys committed here come from a disposable
cluster's disposable entity: test material, not secrets):
- mon: `release`, `entity`, `client_key` (base64), `con_mode`
  (1 or 2), `global_id`, `auth_done_hex` (the final `in` payload),
  `connection_secret_len` (0 or 64, from `Auth.h:147-155`), and
  `mon_logged_session_keys`: `{ "auth": "Ag…", "mon": …, "osd": …,
  "mgr": … }` for exactly the services the log shows for that
  `global_id`.
- osd: `release`, `entity`, `con_mode`, `global_id`,
  `osd_session_key` (the mon-logged OSD key of the SECURE session),
  `authorizer1_hex`, `challenge_hex`, `authorizer2_hex`, `reply_hex`.

**Commit:**

```
tests: capture a cephx transcript from a live cluster

An ignored tool for the known-answer tests: it authenticates a fresh
aes256k entity to the mon in CRC and SECURE mode and to an OSD through
raw msgr2 connections, records each auth payload on the wire by wrapping
the AuthProvider, and takes the session keys from the mon's own
debug_auth log (build_service_ticket prints every ticket's session key),
so the expected keys do not come from rados-rs.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

**Controller capture run** (unsandboxed; raises `debug_auth` on the mon,
and a trap restores it). Run the block as one shell invocation, so the
trap covers the capture. The first `config get` must print `1/5`; if it
prints anything else, an earlier restore failed: stop.

```bash
mkdir -p $W/rados/tests/fixtures/cephx
$TB ceph config get mon debug_auth            # 1/5; anything else: stop
(
  trap '$TB ceph config rm mon debug_auth' EXIT
  $TB ceph config set mon debug_auth 10
  cd $W && CEPHX_CAPTURE_OUT=$W/rados/tests/fixtures/cephx CEPHX_CAPTURE_RELEASE=19.2.6 \
    CEPHX_MON_LOG_CMD="$ROOKET k -n rook-ceph logs deploy/rook-ceph-mon-a -c mon --since=10m" \
    cargo test -p rados --offline --test cephx_capture -- --ignored --nocapture
)
$TB ceph config get mon debug_auth            # 1/5 again
$TB ceph mon getmap -o - 2>/dev/null > $W/rados/tests/fixtures/cephx/monmap-v19.2.6.bin
$CEPH_EXEC ceph-dencoder type MonMap import - decode dump_json \
  < $W/rados/tests/fixtures/cephx/monmap-v19.2.6.bin \
  | jq '{epoch, auth_epoch, auth_service_cipher, auth_allowed_ciphers, auth_preferred_cipher}'
```

  - Controller verifies live, on the first run: the mon log is reachable
    as `deploy/rook-ceph-mon-a` with container `mon` (a rooket cluster
    with fewer than 3 hosts has one mon: Rook facts, "Mon logs"), and the
    `debug `-prefixed `build_service_ticket` lines appear there at
    `debug_auth 10`. If they do not, stop: a KAT whose expected keys come
    from rados-rs itself is not a known answer.
  - If the shell dies before the trap runs, run
    `$TB ceph config rm mon debug_auth` by hand, and check that it reads
    `1/5`, before anything else.
  - `ceph mon getmap -o -` writes the v10 encoding (Ceph facts, "`mon
    getmap`") to stdout, and `ceph-dencoder … import -` reads stdin, so
    the dencoder checks the committed bytes and nothing is left in the
    toolbox's `/tmp`.
  - The `ceph-dencoder` JSON goes into Task 2's MonMap assertion
    verbatim, and into the ledger.

---

### Task 2 (B6): Known answers from a v19.2.6 mon and OSD

**Files:**
- Create: `rados/tests/fixtures/cephx/v19.2.6-mon-crc.json`,
  `v19.2.6-mon-secure.json`, `v19.2.6-osd.json`,
  `monmap-v19.2.6.bin` (from Task 1's run).
- Create: `rados/tests/cephx_transcript_kat.rs`. **Not** ignored: it
  needs no cluster and runs in `ci.yml:87`. It reads the fixtures from
  `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cephx/")` for
  each release in `const RELEASES: &[&str] = &["19.2.6"]`.

**Tests** (public API only):
- **Mon, CRC and SECURE.** `CephXClientHandler::new(entity,
  AuthMode::Mon)`, `set_secret_key_from_base64(client_key)`, then
  `handle_auth_done(auth_done, global_id, con_mode)`. Then:
  - the stored service set (`get_session().ticket_handlers` keys) equals
    the set in `mon_logged_session_keys`, exactly;
  - each stored ticket's `session_key.crypto_type` and `secret` equal
    `CryptoKey::from_base64(logged)`'s, and the AUTH key is type 2 (the
    client key's type, `CephxServiceHandler.cc:278`);
  - each service key's type equals `min(2, service_cipher)` of
    `monmap-v<release>.bin` (2 on this cluster);
  - taking the connection-secret envelope out of `auth_done_hex` by hand
    (after `CephXResponseHeader` and `ServiceTicketReply`, both public in
    `rados::auth::protocol`), it decrypts
    under the logged AUTH key with 0x03 to magic-correct plaintext
    holding a secret of `connection_secret_len` bytes, and fails under
    0x04. This is the only assertion that pins the CRC-mode envelope,
    because `decode_connection_secret` still maps a CRC-mode error to
    `Ok(None)` (`auth/client.rs:419-452`, the CRC-only `Ok(None)` at
    `:447-450`);
  - in SECURE mode, `handle_auth_done` returns `Some` secret of
    `connection_secret_len` (64) bytes, and a copy of `auth_done_hex`
    with one ciphertext byte of the envelope flipped makes it return an
    error (review fix `e601f44`).
- **OSD.** With `osd_session_key`:
  - `challenge_hex`'s encrypted blob decrypts under 0x11 (the OSD
    produced it), read with the envelope layout
    `decrypt_authorize_challenge` uses (`auth/client.rs:587-635`), and
    fails under 0x12;
  - `authorizer2_hex`'s `CephXAuthorizeB` decrypts under 0x10 with
    `have_challenge` set and `server_challenge_plus_one` equal to the
    decrypted challenge plus one (`auth/protocol.rs:459-470`);
  - `reply_hex` decrypts under 0x12 to a `CephXAuthorizeReply` whose
    `nonce_plus_one == nonce + 1`, with `nonce` from authorizer 2, and
    whose `connection_secret` is 64 bytes in SECURE mode and empty or
    absent in CRC mode (`Auth.h:147-155`), and fails under 0x11.
- **MonMap.** `monmap-v19.2.6.bin` decoded with
  `<rados::MonMap as rados::Denc>::decode(&mut buf, features)`
  (`denc/monmap.rs:379`; or
  `rados::monclient::MonMapState::decode(&[u8])`, `monclient/monmap.rs:288`)
  has `auth == Some(rados::denc::MonMapAuth { .. })` (`epoch: u32`, `i32`
  ciphers; `denc/monmap.rs:181-193`) equal to the `ceph-dencoder` JSON
  recorded in Task 1 (`auth_epoch`, service 2, allowed `[1, 2]`,
  preferred 2 on this cluster), and its `epoch` equals the dump's.

- [ ] **Run** (sandboxed): `cargo test -p rados --offline --test
  cephx_transcript_kat`. Then, to prove each assertion bites, flip one
  byte of each fixture's `auth_done_hex`/`reply_hex` locally and see the
  test fail (not committed).

**Commit:**

```
tests: cephx known answers from a v19.2.6 mon and OSD

Fixtures captured from a Ceph v19.2.6 cluster: the mon's AUTH_DONE for
an aes256k client in CRC and SECURE mode, an OSD's challenge and
authorizer reply, and the monmap. The expected session keys are the
ones the mon logged, and the expected monmap fields are ceph-dencoder's,
so the client's decryption of every ticket, the connection-secret
envelope (usage 0x03), the OSD challenge (0x11) and reply (0x12), and
the MonMap v10 decode are checked against C++ output. They run without a
cluster.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 3 (B7): An AES and an aes256k client side by side

**Files:**
- Modify: `rados/tests/cephx/mod.rs` (the helpers below).
- Modify: `rados/tests/cephx_aes256k.rs` (`mod cephx;` and a new test).

**Helpers** added to `cephx/mod.rs`:
- `pub struct MonAuth { epoch: u32, service_cipher: u16, allowed:
  Vec<u16>, preferred: u16 }` and `async fn mon_auth(&Client) ->
  Option<MonAuth>`: `{"prefix": "mon dump", "format": "json"}` through
  `invoke`, parsed with `serde_json`; `None` when the dump has no
  `auth_service_cipher` (a pre-v19.2.6 cluster; see Global Constraints).
- `async fn client_for(entity: &Entity) -> Result<Client, _>`:
  `common::test_client_builder()?.entity_name(&entity.name).keyring(entity.keyring.path()).build()`.
- `async fn ticket_types(&Client) -> (u16, u16)`: the AUTH and OSD
  tickets' `session_key.crypto_type`, as `keyring_key_type_end_to_end`
  reads them (`cephx_aes256k.rs:93-105`).
- `async fn ticket_bytes(&Client, EntityType) -> (Bytes, Bytes)`: that
  ticket's `session_key.secret` and `ticket_blob.blob`.

**Test `two_key_types_side_by_side`** (`#[ignore]`, in the existing CI
lists with the rest of `cephx_aes256k`):
- Skip with a printed reason unless `mon_auth` is `Some` and `allowed`
  contains both 1 and 2. On the CI's v19.2.2 cluster it skips; on a
  Rook v1.20.7 cluster (live fact 2) it runs. It changes no policy.
- Create `aes` (key type 1, standing in for CSI's key) and `aes256k` (2,
  standing in for a new RGW key) entities; build both clients and keep
  both up.
- `ticket_types` is `(1, min(1, service))` for the AES client and `(2,
  min(2, service))` for the aes256k client, where `service` is
  `mon_auth().service_cipher`; on Rook's policy that is `(1, 1)` and
  `(2, 2)`.
- Each writes its own object and reads the other's.
- The aes256k client watches one object and the AES client notifies it
  and gets one ack; then the other way round (plan 12's API, as
  `keyring_key_type_end_to_end` uses it at `cephx_aes256k.rs:69-91`).
- Cleanup guard: remove both objects and both entities.

- [ ] **Controller run** (unsandboxed; creates and removes two entities,
  no policy change):

```bash
cd $W && CEPH_TEST_EXPECT_KEY_TYPE=2 cargo test -p rados --offline --test cephx_aes256k -- --ignored --nocapture
$TB ceph auth ls | grep -c 'client.rados-rs-' || true      # 0: no entity left behind
```

  The skip path (no auth fields in `mon dump`) is the one CI's v19.2.2
  cluster takes; the PR's CI run exercises it.

**Commit:**

```
tests: an aes and an aes256k client side by side

Rook v1.20.7 allows both key types and keeps CSI's key AES while new
keys are aes256k. The test creates one entity of each type, keeps both
clients up at once, checks that each gets tickets of its own type (the
AUTH ticket takes the key's type, the rest the lower of it and the
service cipher), and has them read each other's objects and notify each
other. It changes no policy, and skips on clusters that predate aes256k.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 4 (B7): Policy scenarios: mixed ticket types and a refused key type

**Files:**
- Create: `rados/tests/cephx_policy.rs` (`mod common; mod cephx;`). Every
  test `#[ignore]`, returns early with a printed skip unless
  `CEPH_TEST_ALLOW_AUTH_POLICY=1` and `mon_auth` is `Some`. Its module doc
  states the rules: opt-in, the `POLICY` mutex and `--test-threads=1`,
  alone, never in CI, and the toolbox commands that restore the cluster
  by hand.

**Shared policy helpers** (in `cephx_policy.rs`):
- `mon_set(admin, name, value)`: `{"prefix": "mon set", "name": name,
  "value": value}`, retried for up to 30 s on an error or a non-zero
  `retval` (an unchanged value answers `already set` with 0, so a retry
  is safe), then polls `mon_auth` until it shows the value.
- `wipe(admin)`: reads `mon_auth().epoch` as `before`, sends
  `{"prefix": "auth wipe-rotating-service-keys"}`, and polls until
  `mon_auth().epoch > before`. A wipe is not idempotent, so after an
  `invoke` error it re-sends only if the epoch has not risen within
  10 s.
- `assert_rook_policy(admin)`: service 2, allowed `[1, 2]`, preferred 2;
  on failure, panic with the recovery commands:
  `$TB ceph mon set auth_allowed_ciphers aes,aes256k`,
  `$TB ceph mon set auth_preferred_cipher aes256k`,
  `$TB ceph mon set auth_service_cipher aes256k`,
  `$TB ceph auth wipe-rotating-service-keys`.
- `restore(admin)`: the same four changes through `mon_set`/`wipe`,
  each retried for up to 30 s, then a check of Rook's policy. It returns
  its failures instead of panicking; the cleanup guard asserts last
  (Test design rules). It wipes unconditionally, also in
  `key_type_not_allowed_is_refused`, which never changes the service
  cipher. That is harmless, and it is why the controller run's
  "`auth_epoch` higher than before" holds after both tests.
- `eventually(what, bound, poll)`: retry an async check every `poll` until
  `bound`, then panic naming `what`.

**Test `mixed_ticket_types_in_a_long_lived_client`:**
1. `assert_rook_policy`, before the cleanup guard is armed. Create an
   aes256k entity; build client `C` and keep it for the whole test.
   `ticket_types(C) == (2, 2)`. Write and read object `o`.
2. `mon_set(auth_service_cipher, aes)`, then `wipe` (the `mon set` alone
   regenerates no rotating key: Ceph facts, "Wipe"). The mon accepts
   `aes` here (Ceph facts, "`mon set`").
3. Within 60 s: `C.mon_client().get_monmap().auth_epoch` rose, and
   `ticket_types(C) == (2, 1)`. The AUTH ticket keeps its type (the wipe
   keeps the AUTH rotating keys) and the refreshed OSD ticket is
   `min(2, 1)`. This is Rook's upgrade window in one client.
4. `close_primary_session_for_test("o")`, then write and read `o`,
   retrying every 2 s for up to 60 s: the OSD must fetch the new AES
   rotating keys before it accepts the new ticket.
5. A fresh aes256k client `D` gets `(2, 1)` too and can read `o`.
6. Cleanup guard, per the Test design rules: `restore` (sets `aes256k`
   and wipes again; without that second wipe the rotating keys stay AES,
   `AUTH_INSECURE_ROTATING_SERVICE_KEY_TYPE` stays raised, and later
   tests see AES OSD tickets: Ceph facts, "Health"). Then record, rather
   than assert, `ticket_types(C) == (2, 2)` within 60 s and `C` reading
   `o` on a new session. Then remove `o` and the entity. Then assert.
- While `auth_service_cipher` is aes, the cluster is at HEALTH_ERR
  (`AUTH_INSECURE_SERVICE_TICKETS`). Rook lists that code among the
  HEALTH_ERR codes that do not stop its child-resource controllers
  (`pkg/operator/ceph/controller/controller_utils.go:192-198@v1.20.7`,
  used only by `IsReadyToReconcile`). The CephCluster reconcile checks
  health only before a Ceph image change, which a HEALTH_ERR would block
  (`cluster/version.go:195-205`). No image change is in flight.

**Test `key_type_not_allowed_is_refused`:**
1. `assert_rook_policy`, before the cleanup guard is armed. Create an
   AES entity (with the insecure-type retry). A client for it connects
   (sanity), and is dropped.
2. `mon_set(auth_allowed_ciphers, aes256k)`. The mon accepts this while
   AES entities exist (Ceph facts, "`mon set`"). From here CSI's AES keys
   are refused too (Rook facts, "Keys"); the window lasts seconds and no
   CSI volume is in use on this cluster (Task 0 Step 2's `get pvc`).
3. `tokio::time::timeout(Duration::from_secs(30), client_for(&aes))`
   returns `Ok(Err(e))`: the build itself fails. `Err(Elapsed)` is not a
   refusal. If the live run shows it, stop and bring it to the owner.
   The mon rejects the entity at `CEPHX_GET_AUTH_SESSION_KEY` with
   `-EACCES` (`CephxServiceHandler.cc:195-209`). The error the builder
   returns is pinned after the first live run: the controller records
   it, and the implementer replaces the provisional `Ok(Err(_))` match
   with a match on the observed variant and, where the text carries it,
   `os error 13` (`msgr2/phase/auth.rs:26-31,165-233` renders the mon's
   errno), in the same commit, before it is made.
4. Cleanup guard, per the Test design rules: `restore`; then record,
   rather than assert, whether a fresh `client_for(aes)` connects again
   within 30 s; then remove the entity; then assert. A later AES
   `create_entity` may meet the insecure-type refusal for up to a health
   tick after the restore; its retry covers that.

- [ ] **Controller run** (unsandboxed; changes and restores the cluster's
  auth policy; nothing else may use the cluster meanwhile; first re-run
  Task 0 Step 2's CephCluster and operator-log checks):

```bash
cd $W && CEPH_TEST_ALLOW_AUTH_POLICY=1 cargo test -p rados --offline --test cephx_policy -- --ignored --nocapture --test-threads=1 \
  mixed_ticket_types_in_a_long_lived_client key_type_not_allowed_is_refused
$TB ceph mon dump -f json | jq '{auth_epoch, auth_service_cipher, auth_allowed_ciphers, auth_preferred_cipher}'
$TB ceph health detail
$TB ceph auth ls | grep -c 'client.rados-rs-' || true
```

  Expected afterwards: Rook's policy, `auth_epoch` higher than before (a
  wipe sets it to the AuthMonitor's next version, not old + 1, and
  `restore` wipes in both tests), the health codes recorded in Task 0
  and no others (a raised
  `AUTH_INSECURE_ROTATING_SERVICE_KEY_TYPE` means a wipe was missed: run
  `$TB ceph auth wipe-rotating-service-keys`), and no `client.rados-rs-*`
  entity.

**Commit:**

```
tests: cephx policy scenarios on a live cluster

Opt-in tests that change the cluster's auth policy through mon commands
and restore Rook's on the way out. With the service cipher switched to
aes and the rotating keys wiped, a long-lived aes256k client keeps its
aes256k AUTH ticket, refreshes to an AES OSD ticket when the auth epoch
rises, and does I/O on a new OSD session with it, as during Rook's
upgrade window. With aes dropped from the allowed ciphers, an AES key is
refused. They run alone and serially, never in CI.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 5 (B8): Service tickets refresh when the auth epoch rises

**Files:**
- Modify: `rados/tests/cephx_policy.rs`.

**Test `tickets_refresh_when_auth_epoch_rises`** (same gate and rules):
1. `assert_rook_policy`, before the cleanup guard is armed. An aes256k
   entity and client `C`; write `o`. Record `ticket_bytes(C, OSD)` and
   `get_monmap().auth_epoch`.
2. `wipe`, through the admin client (the command needs `auth rwx`).
3. Within 30 s: `C`'s `auth_epoch` is larger, and the OSD ticket's
   session-key bytes and blob bytes have both changed. `secret_id` is not
   compared: it can repeat, because a wipe restarts rotating numbering at
   1 (`CephxKeyServer.cc:509-521`, `Auth.h:200-206`).
4. `close_primary_session_for_test("o")`, then write and read `o`,
   retrying every 2 s for up to 60 s.
5. Cleanup guard: remove `o` and the entity, then `assert_rook_policy`
   (a wipe changes no policy).
- **If the OSD never accepts** the new ticket within 60 s
  (`AuthMonitor.cc:2108-2109` wonders whether a restart is needed),
  stop: the controller records the OSD's log for the window and brings it
  to the owner, who decides whether step 4 becomes a recorded
  limitation. Do not weaken the test on your own.

- [ ] **Controller run** (unsandboxed; one wipe; first re-run Task 0
  Step 2's CephCluster and operator-log checks):

```bash
cd $W && CEPH_TEST_ALLOW_AUTH_POLICY=1 cargo test -p rados --offline --test cephx_policy -- --ignored --nocapture --test-threads=1 \
  tickets_refresh_when_auth_epoch_rises
$TB ceph mon dump -f json | jq '{auth_epoch, auth_service_cipher, auth_allowed_ciphers, auth_preferred_cipher}'
```

**Commit:**

```
tests: service tickets refresh when the auth epoch rises

`auth wipe-rotating-service-keys` raises the monmap's auth epoch. The
test checks that a connected client notices, fetches new service
tickets (new session-key and blob bytes; the rotating secret_id can
repeat after a wipe, so it is not compared), and does I/O on a new OSD
session with them.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 6: Document running the suites on a rooket cluster

**Files:**
- Modify: `rados/tests/common/mod.rs`, module docs only. Add a section
  "Running against a rooket cluster" (normative; `cephx_aes256k.rs`,
  `cephx_policy.rs` and `cephx_capture.rs` point here rather than repeat
  it):
  - the two config files (`config.yaml`: `profiles: [host-network]`;
    `values/rook-ceph-cluster.yaml`: `cephImage: {tag: v19.2.6}`) in any
    directory, and why host networking (the host must reach the mons and
    OSDs; `rooket ceph-config` refuses otherwise);
  - `rooket up --rook-version v1.20.7 --workers 1 --config-dir <dir>
    --wait`, `rooket ceph-config --out <dir>`, `CEPH_CONF`, and
    `CEPH_EXEC="rooket k -n rook-ceph exec -i deploy/rook-ceph-tools --"`;
  - that no pool setting is needed (rooket's one-worker base gives
    `osd_pool_default_size = 1`; Rook allows pool delete and size-one
    pools by default);
  - that the suites run as `.github/workflows/test-with-ceph.yml` lists
    them, and `cephx_policy` only as its own docs say;
  - `osdclient_integration_test` creates `test-pool` (`:111-121`), and
    every later suite, the `cephx_*` ones included, opens it without
    creating it. So it runs first, as the CI lists order it.
  - `CEPH_EXEC`'s default in the two CLI suites is compose's
    `docker exec -i ceph-mon`. On a rooket cluster it must be set, and
    `-i` is required because `rados put` reads stdin.
  - Compose sets `mon_max_pg_per_osd = 1000`
    (`docker/docker-compose.ceph.yml:37`), while Rook's single OSD keeps
    Squid's 250. Live fact 5 shows this holds today; a PG-limit refusal
    would be a harness limit, not a regression. Nothing else in
    `tests/common` is compose-specific.
- Modify: the `CEPH_EXEC` module docs of `object_locator_routing.rs:6-7`
  and `osdclient_split_merge.rs:7-8`: "runs the v19.2.2 `ceph` …" becomes
  "runs the cluster's `ceph` …", pointing to `common`. Nothing else in
  them changes.

**Commit:**

```
tests: document running the cluster suites on a rooket cluster

The suites need only CEPH_CONF and, for the two that call the C++ CLI,
CEPH_EXEC. Describe the rooket configuration (host networking and a
pinned Ceph image) and commands that provide both for a Rook-managed
cluster, which is how the aes256k cluster tests are run.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

---

### Task 7 (B7 test 1; B9's evidence, for the PR body): The suites on the rooket cluster, v19.2.6 then v20.2.4 (controller)

**v19.2.6** (unsandboxed; the `rados-rs` cluster, after Tasks 1-6).
Every suite, exactly as the CI lists run them
(`test-with-ceph.yml:78,86,93,103,113,123`), then the cephx tests:

```bash
cd $W
CRC_ON="osdclient_integration_test osdclient_object_io_test osdclient_pool_operations osdclient_rados_operations osdclient_xattr_operations osdclient_omap_operations osdclient_exec_operations osdclient_small_operations watch_notify object_locator_routing osdclient_split_merge cephx_aes256k osdclient_release"
CRC_OFF="osdclient_integration_test osdclient_object_io_test osdclient_rados_operations osdclient_xattr_operations osdclient_omap_operations osdclient_exec_operations osdclient_small_operations watch_notify object_locator_routing osdclient_split_merge cephx_aes256k osdclient_release"
CLS="cls_version_refcount cls_user cls_queue_gc cls_rgw_index cls_rgw_gc cls_rgw_usage cls_rgw_lc cls_rgw_olh cls_rgw_release_shapes cls_lock cls_2pc_queue cls_otp"
for test in msgr2_connection_tests msgr2_server_accept_test; do
  cargo test -p rados --offline --test "$test" -- --ignored --nocapture || echo "FAIL $test"
done
cargo test -p rados --offline --test monclient_command_operations -- --ignored --test-threads=1 --nocapture || echo "FAIL monclient_command_operations"
for crc in true false; do
  if [ "$crc" = true ]; then list=$CRC_ON; else list=$CRC_OFF; fi
  for test in $list; do
    RADOS_MS_CRC_DATA=$crc CEPH_TEST_EXPECT_KEY_TYPE=2 cargo test -p rados --offline --test "$test" -- --ignored --nocapture || echo "FAIL $test crc=$crc"
  done
  for test in $CLS; do
    RADOS_MS_CRC_DATA=$crc cargo test -p rados-cls --offline --test "$test" -- --ignored --nocapture || echo "FAIL $test crc=$crc"
  done
done
# re-run Task 0 Step 2's CephCluster and operator-log checks first
CEPH_TEST_ALLOW_AUTH_POLICY=1 cargo test -p rados --offline --test cephx_policy -- --ignored --nocapture --test-threads=1
```

This is a command sequence for the controller's shell, not a committed
script. Expected: no `FAIL` line. A failure not caused by auth (plan 18
notes `cls_2pc_queue`/`cls_rgw` changed between v19.2.2 and v19.2.6):
stop and bring it to the owner; do not adapt the test. Every result
goes into the ledger, which fills the PR's Results line (Task 9).

**v20.2.4** (unsandboxed; a second, fresh cluster, which is Rook
v1.20.7's default image, `values.yaml:104-108`). The controller runs it
beside `rados-rs`: both fit in the host's 61 GB RAM and ~200 GB free on
`/`. Per cluster that is about 10.7 GiB of memory requests and about
14 GiB of limits (chart defaults: mon 1/2Gi, mgr 0.5/1Gi, OSD 4/4Gi, MDS
4/4Gi, RGW 1/2Gi, toolbox 0.1/1Gi; rooket trims only CPU, rooket
`base.go:64-68`), and a sparse 10 GiB OSD image under
`~/.local/share/rooket/<name>/` (rooket `up.go:529`, `block.go:146-153`),
plus the image and the mon store on `/`:

```bash
mkdir -p $S/rooket-harness-v20/values
printf 'profiles: [host-network]\n' > $S/rooket-harness-v20/config.yaml
printf 'cephImage:\n  tag: v20.2.4\n' > $S/rooket-harness-v20/values/rook-ceph-cluster.yaml
ROOKET_NAME=rados-rs-v20 $ROOKET up --rook-version v1.20.7 --workers 1 --config-dir $S/rooket-harness-v20 --wait
ROOKET_NAME=rados-rs-v20 $ROOKET ceph-config --out $S/rooket-ceph-v20
```

Then run Task 0 Step 2, and, after
`export ROOKET_NAME=rados-rs-v20 CEPH_CONF=$S/rooket-ceph-v20/ceph.conf`,
run: `osdclient_integration_test` (it creates `test-pool`),
`osdclient_object_io_test`, `cephx_aes256k` with
`CEPH_TEST_EXPECT_KEY_TYPE=2`, and `cephx_policy` as in Task 4. (`TB`
and `CEPH_EXEC` call `$ROOKET k`, which reads `ROOKET_NAME` when it
runs.)

Expected: `ceph versions` all 20.2.4, live fact 2's monmap auth fields
(Rook applies the same policy, `auth.go:126-135`), `min_mon_release_name`
tentacle, and those suites passing.

The rest of the v19.2.6 sequence runs once, for information only. A
non-auth failure there goes into the ledger and the PR's Results line,
and is not a stop.

No v20.2.4 fixtures: the server auth code is byte-identical to v19.2.6
(`git diff --stat v19.2.6 v20.2.4` is empty for `AuthMonitor.cc`,
`CephxKeyServer.cc`, `CephxServiceHandler.cc`, `Auth.h` and `Crypto.cc`;
`CephxProtocol.cc` differs only by one `WITH_SEASTAR`→`WITH_CRIMSON`
rename). Bring the cluster up beside `rados-rs` and delete it when done.

---

### Task 8 (B9): CI for an aes256k cluster: decided, no change

**Owner decision (2026-09-27): CI is unchanged.** `test-with-ceph.yml`
keeps the compose v19.2.2 cluster. No workflow, job, matrix entry or
required check changes, so no repository-settings question arises. Part
B's cluster tests (Tasks 1, 3-5 and 7) run locally against rooket
clusters, and their results go in the PR body (Task 9). In CI, aes256k
is covered by the unit tests and Task 2's known answers, which need no
cluster, and `two_key_types_side_by_side` takes its skip path on
v19.2.2. Putting rooket in CI is on the Roadmap: it would make upstream
depend on jhoblitt/rooket, kind, helm and iSCSI, and at that point the
rooket config dir would be committed.

Nothing else depends on CI running these tests:
- `cephx_policy` and `cephx_capture` are never in a CI list.
- Task 2 needs no cluster.
- Task 3's only CI exposure is its skip path: v19.2.2's `MonMap::dump`
  has no auth fields (`src/mon/MonMap.cc:430-445@v19.2.2`).

---

### Task 9: Gate, push, draft PR, merge, upstream PR (controller)

As plan 3's Task 6. The per-commit build proof runs across the six
commits (Tasks 1-6).

**Gate** (sandboxed unless labelled):
- `cargo test --workspace --all-targets --offline` (includes
  `cephx_transcript_kat`), the container fmt/clippy gate (`-D warnings`).
- Unsandboxed: Task 7's sequence on v19.2.6 at the branch head, and on
  v20.2.4 if Task 7 ran it.
- A toolbox check that each cluster is left at Rook's policy with no
  `client.rados-rs-*` entity and `debug_auth` at its default.

**Evidence, in the ledger:**
- Task 0: the recorded cluster state and the pre-Part-A failure text.
- Task 1: the `ceph-dencoder` MonMap JSON and the fixture file names.
- Tasks 3-5: each run's output, the `mon dump` auth fields before and
  after, the health codes, and the observed refusal variant (Task 4).
- Task 7: every suite result on each release, with the release versions.
- Task 8: the owner's decision, as recorded there.
- The PR body's Results line, filled from Task 7.

**Implementers:**
- Tasks 1 and 2 go to the opus tier: capture plumbing over raw msgr2 and
  hand-decoding the AUTH_DONE layout are protocol work.
- Tasks 3-5 go to the opus tier: cluster-state tests whose cleanup must
  be right.
- Task 6 is docs (sonnet tier is enough).
- The controller runs every cluster step. The whole-branch review is the
  session model, focused on Review Focus 1-3.

The PR body:

```
**Motivation.** Part A (#20) added aes256k cephx keys but never met a cluster issuing them.

**What changed.** Tests only. Known answers captured from a Ceph v19.2.6 mon and OSD run in CI without a cluster. Opt-in cluster tests put aes and aes256k clients side by side, hold mixed ticket types across a service-cipher change, refuse a disallowed key type, and refresh tickets when the auth epoch rises.

**Results**, local on Rook v1.20.7 via rooket (CI stays on v19.2.2): Ceph v19.2.6, <N>/<N> suites and 3/3 policy tests pass; v20.2.4, <result>.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

Fill Results from Task 7's ledger. If v20.2.4 did not run, end the line after the v19.2.6 clause. Check `wc -w` ≤ 100.

---

## Roadmap for later plans

- **CI for an aes256k cluster.** Declined for now (Task 8). Revisit if
  upstream wants aes256k CI and accepts a rooket dependency; the rooket
  config dir is committed then.
- **In-session AUTH ticket renewal** (plan 18's Roadmap, unchanged): the
  next priority for a long-lived RGW, since `auth_mon_ticket_ttl` is 72 h.
  Task 5's refresh relies on the AUTH ticket still being valid.
- **Mutual authentication of the OSD** (`nonce_plus_one`, plan 18's
  Roadmap). Task 2 now pins the OSD's reply bytes, which a fix can reuse.
- **An upgraded-cluster scenario**: Rook upgrading v19.2.2 to v19.2.6 in
  place (R§7 scenario 4), and Rook's natural 2-3 h service-key turnover
  with no wipe. rooket makes this a tag change on a v19.2.2 rooket
  cluster, but the one-OSD upgrade path needs its own look first.
- **Plan 18's other Roadmap items** (the second mon handshake's stale
  challenge; mock server parity) stand.

## Review edits applied (2026-09-27)

1. Task 8 records the owner's decision (no CI change); the header, Decisions, Task 7 heading, Task 9 evidence and Roadmap follow it.
2. The PR body carries a Results line filled from Task 7's ledger, with the fill instruction and a `wc -w` ≤ 100 check.
3. "Session-key log" states that `debug_auth` 10 logs long-term keys; the capture block runs as one shell invocation with a trap, and `mon getmap -o -` feeds `ceph-dencoder import -`.
4. The precondition runs before the cleanup guard is armed, and the guard collects failures and asserts last (Test design rules, `restore`, Task 4 test 1 step 6 and test 2 step 4, Task 5 step 5).
5. The `POLICY` mutex, 300 s body timeout and wall-time retry budgets are in the Test design rules; Task 4's refusal must be `Ok(Err(e))`, not `Err(Elapsed)`.
6. The reconcile-race text is replaced; Task 0 Step 2 checks the CephCluster phase, operator reconcile failures and PVCs, re-run before each `cephx_policy` run; the HEALTH_ERR sentence is corrected.
7. v20.2.4 gates only the cephx subset and runs the rest for information; no v20.2.4 fixtures, Architecture item 7 and its commit dropped, Goal edited.
8. Task 0 Step 1 checks for `bd55b0a` and creates the branch in a `git worktree add` worktree `$W` with `CARGO_TARGET_DIR=$R/target`; PR #24 is recorded as merged throughout.
9. The MonMap decode path and `MonMapAuth` types, the OSD connection pattern, Review Focus 1 and Task 1 step 5 are corrected.
10. Rook facts corrected: toolbox stdin and chart default, pool defaults written in `startMons`, key generation, and the cipher-policy source.
11. Citations fixed: MonClient +7, `MonMap.cc:254`, rooket `base.go:111,126-137`, `msgr2/mod.rs:249-315`, the cleanup pattern, `decode_connection_secret`, and `controller/predicate.go:176-274`.
12. Task 6 documents `test-pool` ordering, `CEPH_EXEC`'s compose default and `-i`, and `mon_max_pg_per_osd`.
13. `restore`'s description says it wipes unconditionally, why that is harmless, and that it makes "`auth_epoch` higher" hold.
