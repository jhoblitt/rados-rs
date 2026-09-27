# Upstream Ceph bug registry

Defects in ceph/ceph that rados-rs meets as a client that must coexist with
radosgw. The rules are binding and live in the spec
(`docs/superpowers/specs/2026-09-24-rados-rs-rgw-mvp-design.md`, section
"Upstream Ceph bug registry"). IDs are stable, and entries are updated in
place.

Each entry records:

- **Component**, and **Status**. `confirmed` means the evidence below was
  checked. `suspected` means the entry says what is still to be checked.
- **Symptom**, as a client sees it.
- **Affected** and **Fixed in**, the latter with release and commit.
- **Evidence**: `tag:path:line`, and how it was established (source reading,
  `ceph-dencoder`, or a named cluster test).
- **rados-rs**: the workaround, doc note or pinning test, naming its commit
  or fork PR (jhoblitt/rados-rs).
- **Found**: who found it, and when.
- **Upstream**: report status. It is `not filed` unless a tracker or PR exists.
- **See also**: the matching entry in rgw-go's registry
  (`docs/ceph-upstream-bugs.md` in rgw-go), where one exists.

Paths are in ceph/ceph. `main` means ceph/ceph main at 986f3c892e7
(2026-09-27). The cited files are unchanged there since 7ed73efc1be. A
cluster test named here is one of rados-rs's `#[ignore]` suites, run
against v19.2.2.

## CEPH-BUG-001: cls_2pc_queue leaks reserved capacity on commit, abort and expire

- **Component:** OSD class `cls_2pc_queue`, which backs the persistent
  bucket-notification queues.
- **Status:** confirmed.
- **Symptom:**
  - `reserve` adds `size + 10·entries` to `reserved_size`, but `commit`,
    `abort` and `expire_reservations` subtract only `size`.
  - `reserved_size` therefore grows by ten bytes per reserved entry, until
    `2pc_queue_reserve` answers `ENOSPC` on a queue that has room.
  - radosgw turns that `ENOSPC` into a rate-limit error.
- **Affected:** v16.1.0 (the first form, 4fba777a1d1) through v19.2.3,
  v20.2.0 through v20.2.2, and the v20.0.0, v20.1.x and v20.3.0
  pre-release tags.
- **Fixed in:** each series carries its own pair. The first commit of each
  pair subtracts the overhead, the second recomputes `reserved_size` once.
  - Squid only, v19.2.4 onward: b97fe168f62 and 8f86e0926f4.
  - Tentacle only, v20.2.3 onward: 98ed23288db and 7a8f84046b8.
  - v21.0.0 onward and main: 00ad83d3ab2 and 7f4eaee30cb. The squid and
    tentacle pairs are cherry-picks of these.
  - The recompute is incomplete; see CEPH-BUG-002.
- **Evidence:**
  - Source reading:
    - `v19.2.3:src/cls/2pc_queue/cls_2pc_queue.cc:137` adds the overhead.
    - Only `size` is subtracted by `commit` (`:300`), `abort` (`:386` and
      `:396`, then `:401`) and `expire` (`:493` and `:522`, then `:543`).
    - The same code is at `v20.2.2:…:139,302,388,398,403,495,524,545`.
  - Pinned on v19.2.2 by the cluster test `squid_leaks_the_entry_overhead`.
- **rados-rs:**
  - The `rados-cls/src/two_pc_queue.rs` module doc records the leak and the
    per-release fix (2d8becf, PR #16).
  - The cluster test `squid_leaks_the_entry_overhead` (454c6ef, PR #16)
    expects the leak on a head that decodes below v3.
  - No request carries `reserved_size`, so a client cannot repair the count.
- **Found:** rados-rs cls_2pc_queue research and plan 14, 2026-09-25. rgw-go
  found it independently.
- **Upstream:** fixed by ceph/ceph PR #67169 (main), #67575 (squid) and
  #67576 (tentacle). No tracker is cited.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "The 2pc queue's
  reserved size drifts upward".

## CEPH-BUG-002: cls_2pc_queue's self-heal is skipped when another write comes first

- **Component:** `cls_2pc_queue`.
- **Status:** confirmed.
- **Symptom:**
  - A fixed OSD recomputes `reserved_size` only inside `reserve`, and only
    for a head it decodes below v3.
  - `commit`, `abort`, `expire_reservations` and `remove_entries` re-encode
    the head at v3 without recomputing. `expire_reservations` writes the
    head only when it removes a stale reservation.
  - A queue that drifted under CEPH-BUG-001 keeps that drift for good if its
    first write after the upgrade is one of those four, so the spurious
    `ENOSPC` persists.
- **Affected:** v19.2.4 and later, v20.2.3 and later, v21.0.0 and later, and
  main, on any queue that drifted under an earlier release.
- **Fixed in:** none, as of main.
- **Evidence:** source reading. `src/cls/2pc_queue/` is unchanged from
  v19.2.4 to v19.2.6.
  - `v19.2.6:src/cls/2pc_queue/cls_2pc_queue.cc:135` gates the recompute on
    `decoded_struct_v < 3`, inside `reserve`.
  - `v19.2.6:src/cls/2pc_queue/cls_2pc_queue_types.h:72` encodes with
    `ENCODE_START(3, 1, bl)` every time.
  - The head is re-encoded with no recompute at `cls_2pc_queue.cc:371`
    (commit), `:458` (abort), `:602` (expire) and `:698` (remove_entries).
    The expire write sits under `:597`, which requires a stale reservation.
  - The same gate is at `v20.2.3:…:137` and `main:…:137`.
- **rados-rs:** a doc note only. The `two_pc_queue.rs` module doc says that
  version 3 does not mean `reserved_size` is exact (2d8becf, PR #16). No
  test pins it.
- **Found:** rados-rs plan 14, PR #16, 2026-09-25. rgw-go found it
  independently.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "The 2pc queue's
  self-heal is skipped when another write comes first".

## CEPH-BUG-003: cls_2pc_queue hands out reservation id 0, which radosgw treats as "none"

- **Component:** `cls_2pc_queue`, together with radosgw's
  persistent-notification publisher.
- **Status:** confirmed.
- **Symptom:**
  - `last_id` is a `u32` that `reserve` pre-increments, and the increment
    does not skip 0.
  - After 2^32 reservations on one queue, a reserve therefore returns id 0,
    which is `NO_ID`.
  - radosgw neither commits nor aborts a reservation whose id is `NO_ID`.
    That event is lost, and its reservation's space stays reserved until
    the reservation expires.
  - The class itself treats id 0 like any other id: it commits or aborts
    reservation 0 normally.
  - It happens once per wrap.
- **Affected:** every tag checked: v19.2.6, v20.2.4, v21.1.0 and main.
- **Fixed in:** none.
- **Evidence:** source reading. It has not been reproduced, because that
  needs 2^32 reservations.
  - `v19.2.6:src/cls/2pc_queue/cls_2pc_queue_types.h:9-10` defines
    `id_t = uint32_t` and `NO_ID{0}`; `:62` declares `last_id`.
  - `v19.2.6:src/cls/2pc_queue/cls_2pc_queue.cc:186` does
    `++urgent_data.last_id`.
  - `v19.2.6:src/rgw/driver/rados/rgw_notify.cc:1149` takes the id as
    returned, `:1174` skips the commit when it is `NO_ID`, and `:1283-1284`
    skip the abort.
  - `v19.2.6:src/cls/2pc_queue/cls_2pc_queue.cc:292` (commit) and `:401`
    (abort) look the id up in the reservation map, with no `NO_ID` check.
- **rados-rs:** a doc note on `NO_ID` in `rados-cls/src/two_pc_queue.rs`
  (046b45f, PR #16).
- **Found:** rados-rs, PR #16, 2026-09-25.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "The 2pc queue hands
  out reservation id 0, which radosgw treats as none".

## CEPH-BUG-004: radosgw's notification-queue registry listing never advances past 1024 keys

- **Component:** radosgw's `read_queue_list` in `rgw_notify.cc`.
- **Status:** confirmed.
- **Symptom:**
  - The registry object is read in pages of 1024 keys, but `start_after` is
    never advanced.
  - Once `queues_list_object` holds more than 1024 queues, radosgw re-reads
    the first page for ever.
- **Affected:**
  - All of Squid: v19.2.2, v19.2.6 and the squid branch at a742f50616e were
    checked.
  - v20.2.0 through v20.2.2, and the v20.0.0, v20.1.x and v20.3.0
    pre-release tags.
- **Fixed in:**
  - v21.0.0 onward and main: b984980897d.
  - Tentacle, v20.2.3 onward: 15de1799510, a cherry-pick of b984980897d.
  - There is no squid backport as of a742f50616e.
- **Evidence:** source reading.
  - `v19.2.6:src/rgw/driver/rados/rgw_notify.cc:108` declares `start_after`,
    `:114` uses it, and nothing in the loop up to `:125` assigns it.
  - `v20.2.4:…:130` has the fix, `start_after = *queues.rbegin();`.
- **rados-rs:** a doc note in the `rados-cls/src/two_pc_queue.rs` module doc
  (2d8becf, PR #16).
- **Found:** rados-rs cls_2pc_queue research, 2026-09-25. rgw-go records it
  in its `docs/exclusions.md`.
- **Upstream:** filed and fixed. Both fix commits cite tracker #73812 in a
  `Fixes:` line. The fix is ceph/ceph PR #66246 (main) and its backport
  #66491 (tentacle). There is no squid backport.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "radosgw's notification
  queue listing never pages past 1024 queues".

## CEPH-BUG-005: cls_rgw usage trim never removes a payer-keyed record, and never reports done

- **Component:** cls_rgw `user_usage_log_trim`.
- **Status:** confirmed.
- **Symptom:**
  - `usage_log_add` keys a requester-pays record by the payer, but trim
    derives the keys it removes from the owner.
  - The record is found every round and never removed, so trim answers 0,
    never `ENODATA`.
  - radosgw's own trim loop runs until `ENODATA`, so `radosgw-admin usage
    trim` and the admin-ops usage trim spin for ever over such a record.
  - A trim filtered by the payer finds the payer's record and removes the
    owner's keys for its epoch and bucket. The owner's own record for the
    same hour and bucket is therefore deleted, although the filter excludes
    it.
- **Affected:** every release checked before v21: v19.2.2 through v19.2.6,
  v20.2.0 through v20.2.4, and the heads of the squid (a742f50616e) and
  tentacle (9208ed9a291) branches.
- **Fixed in:** v21.0.0 and later, through 674d42d9023, which derives the
  keys from the payer. It is not backported.
- **Evidence:**
  - Source reading:
    - `v19.2.2:src/cls/rgw/cls_rgw.cc:3592` keys records by payer; `:3770`
      builds the keys to remove from the owner; `:3808` answers `ENODATA`
      only when nothing was found and the scan was not truncated.
    - A trim filtered by user scans that user's keys (`:3654`), and
      `:3770-3778` then removes the owner's by-time and by-user keys.
      radosgw rounds a record's epoch to the hour
      (`v19.2.6:src/rgw/rgw_log.cc:136,143`).
    - The unbounded loops are `v19.2.2:src/cls/rgw/cls_rgw_client.cc:838-847`
      and `v19.2.2:src/rgw/driver/rados/rgw_rados.cc:10092-10106`.
    - The same code is at `v20.2.4:src/cls/rgw/cls_rgw.cc:4163,4201`.
    - The fix is at `v21.1.0:src/cls/rgw/cls_rgw.cc:4403`.
  - Pinned by the cluster test `usage_trim_gives_up_on_a_payer_keyed_entry`.
- **rados-rs:**
  - The trim loop is bounded at `MAX_TRIM_ROUNDS` = 1000 and then fails with
    `OSDClientError::Other` (`rados-cls/src/rgw/usage.rs`, ea2c204, PR #12).
  - `usage_trim_gives_up_on_a_payer_keyed_entry` pins it (e6daa93, PR #12).
    The test inverts on an image that carries 674d42d9023.
  - The `trim` doc names the fix release (fork PR #25, 7fb2a13).
- **Found:** rados-rs plan 09, 2026-09-25.
- **Upstream:** not filed as such. ceph/ceph PR #65329 fixed it in passing,
  on main only.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_rgw usage trim
  never removes a payer-keyed record".

## CEPH-BUG-006: cls_rgw usage trim with a bucket filter never reports done past 1000 skipped keys

- **Component:** cls_rgw `user_usage_log_trim`.
- **Status:** confirmed.
- **Symptom:**
  - The request carries no marker, so every round restarts at the start of
    the range. A round scans at most 1000 keys and skips records of other
    buckets.
  - If those 1000 keys hold no record for the bucket and more keys follow,
    the round answers 0 rather than `ENODATA`, and so does every later round.
  - `radosgw-admin usage trim --bucket` trims by the bucket's owner. It
    therefore hangs once that owner has more than 1000 records for other
    buckets in the range.
  - Records past those 1000 keys are never reached.
- **Affected:** every release checked: v19.2.2, v19.2.6, v20.2.4, v21.1.0
  and main.
- **Fixed in:** none.
- **Evidence:** source reading. It has not been reproduced on a cluster.
  - `v19.2.2:src/cls/rgw/cls_rgw.cc:3800` starts `iter` empty on each call;
    `:3696` skips other buckets; `:3808` answers `ENODATA` only when nothing
    was found and the scan was not truncated.
  - The same logic is at `v21.1.0:src/cls/rgw/cls_rgw.cc:4439,4225,4450`,
    and on main at `:4525`.
  - The caller path is `v19.2.6:src/rgw/rgw_usage.cc:170-171`, then
    `v19.2.6:src/rgw/driver/rados/rgw_sal_rados.cc:799-806`, then the
    unbounded loop in CEPH-BUG-005.
- **rados-rs:**
  - The same `MAX_TRIM_ROUNDS` bound applies (`usage.rs`, ea2c204, PR #12).
  - Its doc names the affected releases, through main (fork PR #25,
    7fb2a13; upstream tchaikov/rados-rs#131).
  - No test pins it.
- **Found:** the rados-rs plan 09 review, 2026-09-25. The radosgw-admin hang
  was traced on 2026-09-27.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_rgw usage trim
  with a bucket filter stalls behind 1000 other records".

## CEPH-BUG-007: cls_rgw link_olh refuses a delete marker on top of a delete marker

- **Component:** cls_rgw `bucket_link_olh`.
- **Status:** confirmed.
- **Symptom:**
  - A `link_olh` with `delete_marker` set, for a key whose OLH already
    points at a delete marker, answers `ENOENT` and links nothing.
  - A second delete marker therefore cannot be created.
  - Tracker #63799 (tracker not verified; tracker.ceph.com unreachable)
    describes the multisite effect: lifecycle leaves zones with delete
    markers that differ.
- **Affected:**
  - Pacific from v16.2.6, through the backport 1e575378b00 (ceph/ceph
    PR #42645), to v16.2.15.
  - v17.1.0 (69d7589fb13, from ceph/ceph PR #41897) through v19.2.2. That
    is all of Quincy and Reef, through v18.2.8.
  - v20.0.0.
  - The pacific, quincy and reef branch heads carry no revert.
- **Fixed in:**
  - Squid, v19.2.3 onward: 9cca4fd435a (ceph/ceph PR #62740).
  - v20.1.0 onward and main: 65e3e9b5888 (ceph/ceph PR #54957).
- **Evidence:** source reading.
  - `v19.2.2:src/cls/rgw/cls_rgw.cc:1676-1692` answers `-ENOENT` for a
    delete marker on a new instance when the OLH already refers to one.
  - The same check is at `v16.2.6:…:1572-1577`, `v17.2.9:…:1577-1582`,
    `v18.2.8:…:1703-1708` and `v20.0.0:…:1827-1832`. It is absent from
    v16.2.5, v19.2.3 and v20.1.0.
  - 9cca4fd435a removes the check; its message is "rgw: revert PR #41897 to
    allow multiple delete markers to be created".
- **rados-rs:** the per-release behaviour is documented on `link_olh` in
  `rados-cls/src/rgw/olh.rs` (e3f85c3, PR #21). No test pins it.
- **Found:** rados-rs release-shapes research (plan 16), 2026-09-25.
- **Upstream:** filed and fixed. The tracker is #63799 (tracker not
  verified; tracker.ceph.com unreachable); no fix commit cites it. The fix
  is ceph/ceph PR #54957 (main) and #62740 (squid).
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "Squid before 19.2.3
  refuses a delete marker on top of a delete marker".

## CEPH-BUG-008: cls_rgw complete_op writes a stale epoch onto the entry when it cancels

- **Component:** cls_rgw `bucket_complete_op`.
- **Status:** confirmed.
- **Symptom:**
  - A complete whose epoch is not newer than the entry's is turned into a
    cancel. Before that cancel, the class copies the op's version onto the
    entry and writes the entry back.
  - The entry's `ver.epoch` therefore falls back to the stale value.
  - A later complete whose epoch lies between the two then passes the
    staleness check and overwrites the newer metadata. For example, racing
    overwrites that complete in the order 10, 5, 7 leave 7's metadata
    indexed over object 10.
  - The fallback has been observed. The overwrite is derived from the code
    and has not been reproduced.
- **Affected:** v19.2.2 through main. Checked at v19.2.2, v19.2.6, v20.2.0,
  v20.2.4, v21.1.0 and main.
- **Fixed in:** none.
- **Evidence:**
  - Source reading: `v19.2.6:src/cls/rgw/cls_rgw.cc:1082-1086` turns a stale
    epoch into `CANCEL`; `:1094` does `entry.ver = op.ver` before the cancel
    branch; `:1115-1117` writes the entry back in the cancel branch.
  - Cluster test `index_stale_epoch_is_a_cancel`: after completes at epochs
    10 down to 1, the entry reads `ver.epoch == 1`.
- **rados-rs:** `index_stale_epoch_is_a_cancel` pins it (e687b90, PR #10).
  There is no workaround.
- **Found:** rados-rs plan 07, 2026-09-25, as a pinned fact. It was
  classified as a defect on 2026-09-27.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_rgw complete_op
  writes a stale epoch back when it cancels".

## CEPH-BUG-009: cls_rgw encode_packed_val writes 0x10000 as 0

- **Component:** cls_rgw types, in `cls_rgw_types.h`.
- **Status:** confirmed.
- **Symptom:**
  - The packed encoder uses the 2-byte form for values up to and including
    0x10000 and truncates to `u16`, so exactly 65536 is written as 0 and
    decodes as 0.
  - Fields that are affected:
    - `rgw_bucket_entry_ver` pool and epoch, on index entries and in
      prepare/complete requests.
    - `index_ver`, on index entries and bilog entries.
  - `rgw_bucket_dir_entry` also stores `ver.epoch` raw, but its decoder
    overwrites that value with the packed copy.
- **Affected:** v0.67 (b1578ba705a) onwards. Every tag checked is affected,
  through main.
- **Fixed in:** none.
- **Evidence:** source reading.
  - `v19.2.6:src/cls/rgw/cls_rgw_types.h:272-275` tests `<= 0x10000` and
    writes `(uint16_t)val`.
  - `:309-314` decodes a `u16`.
  - The users are `:343-344`, `:408` and `:623`. The raw epoch is decoded at
    `:418`, then overwritten at `:426`.
  - The encoder is unchanged on main (`src/cls/rgw/cls_rgw_types.h:285`).
- **rados-rs:** mirrors the behaviour for byte identity, with a unit test
  (`rados-cls/src/rgw/packed.rs`, 8b83b69, PR #9).
- **Found:** rados-rs plan 06, 2026-09-25.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_rgw encodes a
  packed value of exactly 65536 as 0".

## CEPH-BUG-010: Squid radosgw sends listing-time index suggestions unguarded during a reshard

- **Component:** radosgw, in `rgw_rados.cc` (`cls_bucket_list_ordered` and
  `cls_bucket_list_unordered`).
- **Status:** confirmed.
- **Symptom:**
  - A listing that finds stale index entries sends `dir_suggest_changes`
    with neither `assert_exists` nor `guard_bucket_resharding`.
  - A listing during a reshard can therefore change source shards while they
    are being copied.
  - The fix's commit message says "no changes to the bucket index should be
    allowed while resharding".
  - The resulting damage has not been reproduced here.
- **Affected:** all of Squid. v19.2.6 and the squid branch at a742f50616e
  were checked.
- **Fixed in:** v20.0.0 and later (461be1cd3d5). From v20.2.0 the class also
  guards the call itself (d011c522bb1). There is no squid backport.
- **Evidence:** source reading.
  - `v19.2.6:src/rgw/driver/rados/rgw_rados.cc:9902` and `:10138` send the
    suggestion bare.
  - `v20.2.4:…:10822-10823` and `:11060-11061` send `assert_exists` and the
    guard first.
- **rados-rs:** the per-release guard set is documented in the
  `rados-cls/src/rgw/index.rs` module doc (e3f85c3, PR #21).
- **Found:** the rgw-go phase 0 final review, 2026-09-26. rados-rs plan 16
  (PR #21) documented it the same day.
- **Upstream:** fixed by ceph/ceph PR #59609 (main). No tracker is cited,
  and the fix is not on squid.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "Squid does not guard
  listing-time index suggestions against resharding".

## CEPH-BUG-011: cls_lock get_info and assert_locked fail with EIO on an expired ephemeral lock

- **Component:** cls_lock.
- **Status:** confirmed.
- **Symptom:**
  - When an `EXCLUSIVE_EPHEMERAL` lock has no unexpired holder, `read_lock`
    deletes the object.
  - `get_info` and `assert_locked` are registered read-only, so the OSD
    fails the whole call with `EIO` instead of returning the lock state.
  - The object stays until a writing method succeeds on it: lock, unlock,
    break_lock or set_cookie.
  - radosgw's reshard lock is ephemeral.
- **Affected:** v14.1.0 (a289f2d8654) onwards. v19.2.6 through main were
  checked.
- **Fixed in:** none.
- **Evidence:**
  - Source reading:
    - `v19.2.6:src/cls/lock/cls_lock.cc:93-98` does the cleanup inside
      `read_lock`.
    - `:634-642` registers `get_info` as RD and `assert_locked` as
      RD|PROMOTE.
    - `v19.2.6:src/osd/PrimaryLogPG.cc:6181-6184` turns a write by a method
      not marked WR into `EIO`.
    - Both methods are still read-only on main
      (`src/cls/lock/cls_lock_ops.h:255,257`).
  - Pinned by the cluster test `expired_ephemeral_read_is_eio`.
- **rados-rs:** documented in `rados-cls/src/lock.rs` (6eaeab3, PR #15) and
  pinned by `expired_ephemeral_read_is_eio` (e137d2d, PR #15).
- **Found:** rados-rs cls_lock research, 2026-09-25.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_lock get_info and
  assert_locked fail with EIO on an expired ephemeral lock".

## CEPH-BUG-012: cls_otp divides by a zero step_size and crashes the OSD

- **Component:** cls_otp.
- **Status:** confirmed.
- **Symptom:**
  - `otp_set` stores any `step_size`, including 0.
  - liboath treats a step of 0 as its 30 s default and validates the code.
    `otp_instance::verify` then divides by `step_size`, which is an integer
    division by zero (SIGFPE).
  - The primary OSD of the object's PG crashes at the first `otp_check`
    whose code matches.
  - radosgw-admin never sends 0 (`v19.2.6:src/rgw/rgw_admin.cc:10824-10825`),
    but any other client of the class can.
- **Affected:** every tag checked: v19.2.2, v19.2.6, v20.2.4, v21.1.0 and
  main.
- **Fixed in:** none.
- **Evidence:** source reading. It was not run on a cluster, because it
  would crash an OSD.
  - `v19.2.6:src/cls/otp/cls_otp.cc:143` divides by `otp.step_size` (`:139`
    on main).
  - `otp_set_op` (`:301`) does not check `step_size`.
  - liboath's `oath_totp_validate4_callback` substitutes
    `OATH_TOTP_DEFAULT_TIME_STEP_SIZE` for 0 (oath-toolkit
    `liboath/totp.c`, main).
- **rados-rs:** refuses `step_size == 0` before sending
  (`rados-cls/src/otp.rs`, 5b8fe0c, PR #17), tested by
  `zero_step_size_is_refused_before_sending`.
- **Found:** rados-rs cls_otp research, with a liboath probe, 2026-09-25.
- **Upstream:** filed 2026-09-27 as tracker #80948 (rgw, Bug, New), by the
  rgw-go session at the owner's direction there; the rados-rs session had
  recorded "leave public, no report" earlier the same day. Already public in
  rados-rs's `rados-cls/src/otp.rs` docs (fork main, tchaikov/rados-rs#123)
  from 2026-09-26. Reachable only by a privileged actor: RADOS write caps on
  the OTP pool, or radosgw's admin API with the metadata=write cap; no S3
  end-user path (rgw-go's analysis).

## CEPH-BUG-013: cls_otp records the wrong replay index for a past-step match

- **Component:** cls_otp.
- **Status:** confirmed.
- **Symptom:**
  - liboath returns the absolute window distance of a match. Its sign goes
    to an out-parameter, and Ceph passes null for it.
  - `verify` adds that distance to the current step. A code from step t-1,
    accepted at step t, therefore records `last_success = t+1`.
  - The codes for steps t and t+1 are then refused as already used.
  - Conversely, a code from step t-1 is still accepted after the code for
    step t. An earlier code that was already used can be replayed once
    within the window.
- **Affected:** every tag checked: v19.2.2, v19.2.6, v20.2.4, v21.1.0 and
  main.
- **Fixed in:** none.
- **Evidence:**
  - Source reading: `v19.2.6:src/cls/otp/cls_otp.cc:133-150` calls
    `oath_totp_validate2(…, nullptr /* otp pos */, …)`, computes the index at
    `:143`, compares `index <= last_success` at `:145`, and stores
    `last_success = index` at `:150`.
  - liboath documents the return value as the "absolute value of position in
    OTP window" (`liboath/totp.c`).
  - Cluster test `otp_past_step_quirk`: a t-1 match records
    `last_success = 1 + t`.
- **rados-rs:** documented in `rados-cls/src/otp.rs` (5b8fe0c, PR #17) and
  pinned by `otp_past_step_quirk` (9ce2255, PR #17).
- **Found:** rados-rs cls_otp research, 2026-09-25.
- **Upstream:** filed 2026-09-27 as tracker #80949 (rgw, Bug, New), by the
  rgw-go session at the owner's direction there; the rados-rs session had
  recorded "leave public, no report" earlier the same day. Already public in
  rados-rs's `rados-cls/src/otp.rs` docs (fork main, tchaikov/rados-rs#123)
  from 2026-09-26.

## CEPH-BUG-014: cls_version's client header documents EAGAIN, but the class returns ECANCELED

- **Component:** cls_version.
- **Status:** confirmed.
- **Symptom:**
  - The header says a conditional `inc` returns `-EAGAIN` when its condition
    fails. Both `inc` and `check` return `-ECANCELED`.
  - A client written from the header misses the race signal.
  - The code's behaviour is intended: radosgw relies on `-ECANCELED`, and
    Ceph's own test asserts it. Only the header comment is wrong.
  - It stays in the registry because the spec counts code that contradicts
    its own documentation. rgw-go records it as a quirk.
- **Affected:** every tag checked: v19.2.6, v20.2.4, v21.1.0 and main.
- **Fixed in:** none.
- **Evidence:**
  - Source reading: compare `v19.2.6:src/cls/version/cls_version_client.h:19`
    with `v19.2.6:src/cls/version/cls_version.cc:166,196`.
  - radosgw detects a transition "via ECANCELED from cls_version_check()"
    (`v19.2.6:src/rgw/driver/rados/rgw_gc.cc:168,209`), and
    `v19.2.6:src/test/cls_version/test_cls_version.cc:190-206` expects
    `-ECANCELED` from a failed conditional `inc`.
  - The cluster tests in `rados-cls/tests/cls_version_refcount.rs` assert
    `ECANCELED`.
- **rados-rs:** documents `ECANCELED` in `rados-cls/src/version.rs` (60284ae,
  PR #6), and its tests assert it.
- **Found:** rados-rs plan 03, 2026-09-25.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "cls_version's header
  documents EAGAIN, but the class returns ECANCELED" (a quirk there).

## CEPH-BUG-015: the mon's default for mon_auth_allow_insecure_key lags auth_allowed_ciphers until its next tick

- **Component:** mon, AuthMonitor.
- **Status:** confirmed.
- **Symptom:**
  - The option is documented as following `auth_allowed_ciphers`, but only
    the leader recomputes its default, and only in `check_health`. That runs
    from the leader's tick (`mon_tick_interval`, 5 s) and from its own
    proposals.
  - `auth get-or-create --key-type aes` is refused in two windows:
    - right after `ceph mon set auth_allowed_ciphers aes,aes256k`, which is
      a MonmapMonitor change;
    - on a new leader that has not yet run `check_health`.
  - The client sees `EINVAL` with "creating key with insecure key type …
    not allowed", not `EPERM` (see CEPH-BUG-016).
- **Affected:** v19.2.6, v20.2.4, v21.1.1 and main. No other tag has the
  key-type switch: v19.2.5, v20.2.3, v21.1.0 and v21.3.0 (a tag on main)
  lack it.
- **Fixed in:** none.
- **Evidence:** source reading. It has not been observed on a cluster.
  - `v19.2.6:src/mon/AuthMonitor.cc:172-184`: the tick returns early on a
    peon, then calls `check_health`.
  - `:479`: `encode_pending` also calls it.
  - `:482-509`: `set_val_default` is derived from
    `monmap->auth_allowed_ciphers`.
  - `:1534-1537`: the refusal.
  - `v19.2.6:src/common/options/mon.yaml.in:769-781`: the long_desc, which
    says creation is allowed "so long as the Monitors also allow that
    cipher".
- **rados-rs:**
  - Nothing on main.
  - Plan 18 Part B (deferred) retries for up to 15 s, but it keys the retry
    on `EPERM`, which the client never receives.
- **Found:** the rados-rs plan 18 review, 2026-09-26.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "The monitor's default
  for insecure key creation lags auth_allowed_ciphers".

## CEPH-BUG-016: the mon reports a refused cephx key type as EINVAL

- **Component:** mon, AuthMonitor.
- **Status:** confirmed.
- **Symptom:**
  - `get_cipher_type` returns distinct codes:
    - `-EINVAL` for an unknown type;
    - `-ENOTSUP` for AES256KRB5 without the mon feature;
    - `-EPERM` for an insecure type that is not allowed, or for a type
      outside `auth_allowed_ciphers`.
  - All five of its callers replace that code with `-EINVAL`: `auth add`,
    the `auth *-pending` commands, `auth get-or-create[-key]`,
    `fs authorize` and `auth rotate`.
  - A client can tell a policy refusal from a bad argument only by the
    status text.
- **Affected:** v19.2.6, v20.2.4, v21.1.1 and main. Each series got the
  switch in its own commit: 2f6983f9931 (v19.2.6), faceef2a37b (v20.2.4),
  3ba936e837d (v21.1.1) and 8f379e24f42 (main). The first caller already
  dropped the code.
- **Fixed in:** none.
- **Evidence:** source reading.
  - `v19.2.6:src/mon/AuthMonitor.cc:1510-1548` returns the distinct codes.
  - The callers set `err = -EINVAL` at `:1644-1646`, `:1740-1742`,
    `:1824-1826`, `:1907-1909` and `:2070-2072`.
  - The reply is sent at `:2142-2145`.
- **rados-rs:** nothing on main. Plan 18 Part B must match `EINVAL` together
  with the status text.
- **Found:** plan 18 Part B drafting, 2026-09-27.
- **Upstream:** not filed.
- **See also:** rgw-go registry, jhoblitt/rgw-go#36, "The monitor reports a
  refused cephx key type as EINVAL".

## CEPH-BUG-017: radosgw caches any control-pool notifier's UPDATE_OBJ payload, and a read cap can send one

- **Component:** radosgw, the system-object cache (`RGWSI_SysObj_Cache`)
  and the control-pool watch that feeds it (`RGWSI_Notify`).
- **Status:** confirmed. The behaviour is radosgw's trust model; see the
  last Symptom item.
- **Symptom:**
  - Every radosgw in a zone watches the zone's control objects in
    `<zone>.rgw.control`, and passes each notify to its metadata cache.
  - An `UPDATE_OBJ` record replaces the cached copy of the object it names
    with the record's status, data, attributes and version.
  - Reads are then served from that copy, without RADOS, until one of
    these happens:
    - `rgw_cache_expiry_interval` passes: 900 s by default, never at 0;
    - the entry is evicted;
    - a later update replaces it.
  - The cache holds user records and the access-key index, among other
    system objects.
    - A gateway can therefore authenticate and authorise against a record
      that no RADOS object holds.
    - A record whose status is `ENOENT` makes an existing object read as
      missing.
  - Nothing identifies the sender:
    - the record carries no signature;
    - the callback does not look at the notifier's id;
    - the ack goes out whatever the callback returns, so a notifier cannot
      tell an applied record from a rejected one.
  - RADOS classes `notify` as a read, so read access to the control pool
    is enough to send one.
  - This is by design. The expiry option's own description says the
    notify relay keeps the gateways' caches consistent, and radosgw treats
    every notifier as a peer gateway.
    - It is recorded because that boundary is wider than the gateway
      principals it was drawn for. A principal with read-only access to
      the control pool, or to every pool, can change what each gateway in
      the zone believes.
    - A principal that can also read the meta pool can already read every
      user's secret key. What the notify adds is state that no RADOS object
      holds.
- **Affected:** every tag checked: v19.2.2, v19.2.6, v20.2.4, v21.1.1 and
  main.
- **Fixed in:** none.
- **Evidence:** source reading. It has not been exercised on a cluster.
  - `v19.2.6:src/rgw/services/svc_notify.cc:77` hands every notify to the
    cache callback and discards its result. `:79-80` ack it.
  - `v19.2.6:src/rgw/services/svc_sys_obj_cache.cc:465-502` is the
    callback:
    - it decodes the record at `:475`;
    - for `UPDATE_OBJ` it puts the record's object info into the cache
      (`:490-491`);
    - it never reads `notifier_id`.
  - `v19.2.6:src/rgw/rgw_cache.h:96-101`: the record holds an op, an
    object, the object info, an offset and a namespace.
  - `v19.2.6:src/rgw/rgw_cache.cc:142-215`: `put` stamps `time_added`
    (`:155`) and copies status, attributes, data and version from the
    record.
  - `get` (`:13-96`) drops an entry only after `rgw_cache_expiry_interval`
    (`:30-31`), and answers an `ENOENT` entry with `ENODATA` (`:72-75`).
  - `v19.2.6:src/rgw/services/svc_sys_obj_cache.cc:146-177`: a cache hit
    returns the cached bytes without reading RADOS, and `ENODATA` becomes
    `ENOENT`.
  - `v19.2.6:src/rgw/driver/rados/rgw_service.cc:122-124` routes
    system-object reads through the cache when `rgw_cache_enabled` is on.
    That is the default (`v19.2.6:src/common/options/rgw.yaml.in:288-297`).
  - Access-key lookup reads its index through that service
    (`v19.2.6:src/rgw/services/svc_user_rados.cc:651-656,765-787`).
  - `v19.2.6:src/common/options/rgw.yaml.in:3324-3336` defines
    `rgw_cache_expiry_interval`: its default of 900, and the long_desc
    about notify consistency.
  - The cap:
    - `v19.2.6:src/include/rados.h:258` makes `NOTIFY` a read-mode op;
    - `v19.2.6:src/osd/osd_op_util.cc:120-124` sets only the read flag for
      it;
    - `v19.2.6:src/osd/PG.cc:390-396` checks caps against those flags.
  - The user record stores each secret key
    (`v19.2.6:src/rgw/rgw_acl_types.h:46,58`).
  - The same apply path is at:
    - `v19.2.2`: `svc_notify.cc:76-79`, `svc_sys_obj_cache.cc:490-491`;
    - `v20.2.4` and `v21.1.1`: `svc_notify.cc:75-78`,
      `svc_sys_obj_cache.cc:490-491`;
    - main: `svc_notify.cc:75-78`, `svc_sys_obj_cache.cc:505-506`.
  - `NOTIFY` is still read-mode on main (`src/include/rados.h:260`).
- **rados-rs:** unaffected; this is radosgw's cache. For rgw-rs:
  - Its design keeps the gateway's cephx caps as narrow as radosgw needs,
    because every gateway in the zone trusts the notify channel
    (jhoblitt/rgw-rs design spec draft, section 17).
  - Its cache is to drop the entry an `UPDATE_OBJ` names and re-read it,
    rather than store the payload, so a notify can cost it a read but
    cannot plant a record (owner-agreed design, to be added to the rgw-rs
    spec in its review edits).
  - It still sends the full record on its own metadata writes, because
    radosgw applies it (section 8).
- **Found:** rgw-rs design planning, 2026-09-27, as an unverified
  candidate. It was confirmed from source the same day.
- **Upstream:** not filed. The owner chose on 2026-09-27 to record it
  publicly without a report.

## CEPH-BUG-018: radosgw aborts after 100 failed control-watch re-registrations, counted over its whole life

- **Component:** radosgw, `RGWWatcher` in
  `src/rgw/services/svc_notify.cc`.
- **Status:** confirmed. Whether an outage reaches the abort is derived
  from the code; it has not been reproduced.
- **Symptom:**
  - When a control-object watch breaks, radosgw drops the watcher and
    re-registers it on its finisher thread.
    - Each failed unwatch or watch adds one to a per-watcher counter.
    - The next attempt is queued at once, with no delay.
  - The first attempt after the counter passes 100 calls `abort()`, and
    radosgw exits on SIGABRT.
  - The counter is never reset, not even by a successful re-registration.
    - Failures from separate incidents therefore add up over the process's
      life.
    - Yet the log line says "Looping in attempt to reinit watch", and the
      commit that added it describes "a maximum retry timeout".
  - A failed unwatch both queues a retry and goes on to a watch attempt.
    One failure can therefore start two retry chains that share the
    counter.
  - A transient mon or OSD outage does not reach it with default settings:
    - `rados_osd_op_timeout` defaults to 0, and librados then arms no
      timeout for the watch and unwatch calls, so an outage makes them
      wait rather than fail;
    - the counter moves only when a call returns an error;
    - the OSD answers an unwatch of a watch it no longer holds with
      success.
  - It becomes reachable in two ways:
    - with `rados_osd_op_timeout` set, each attempt during an outage fails
      when that timeout expires and the next starts at once, so an outage
      long enough for more than 100 attempts to time out aborts the
      gateway;
    - failures that accumulate across separate incidents.
- **Affected:**
  - Squid from v19.2.3, through ff248d7ed94 (ceph/ceph PR #62402).
  - v20.1.0 onward, through 34366f0f0d8 (ceph/ceph PR #62253).
  - Checked at v19.2.6, v20.2.4, v21.1.1 and main.
  - v19.2.2 and v20.0.0 have no counter. Their `reinit` retries without
    limit, also with no delay (`v19.2.2:src/rgw/services/svc_notify.cc:88-101`).
- **Fixed in:** none.
- **Evidence:** source reading.
  - `v19.2.6:src/rgw/services/svc_notify.cc:36` declares `retries = 0`.
    `:103` and `:111` increment it, and nothing else assigns it.
  - `:90-93` abort once it exceeds 100.
  - `:82-87`: `handle_error` removes the watcher and queues `reinit`.
  - `:104` and `:112` queue each retry on the finisher, which is a plain
    `queue` (`v19.2.6:src/rgw/services/svc_finisher.cc:54-57`).
  - `:95-108`: after an unwatch that fails with anything but `ENOENT`,
    `reinit` queues a retry and still calls `register_watch` (`:108`).
  - librados waits for the call to finish:
    `v19.2.6:src/librados/IoCtxImpl.cc:1671` for the watch, `:1762` for
    the unwatch.
  - `v19.2.6:src/osdc/Objecter.cc:2317-2324` arms an op timeout only when
    `rados_osd_op_timeout` is above 0.
    - Its default is 0 (`v19.2.6:src/common/options/global.yaml.in:6379-6384`).
    - Nothing under `src/rgw` sets it.
  - `v19.2.6:src/osd/PrimaryLogPG.cc:6971,7027-7039` leaves the result at
    0 for an unwatch of an unknown watch.
  - The same logic is at
    `v20.2.4:src/rgw/services/svc_notify.cc:31,88-91,101,109`, and on the
    same lines at v21.1.1 and main. Only a `null_yield` argument was
    added.
  - 34366f0f0d8, "rgw: Try to handle unwatch errors sensibly", added the
    counter and the abort. Its message reads "add a maximum retry
    timeout".
- **rados-rs:** unaffected; the abort is in radosgw. For rgw-rs:
  - Its design delivers a broken watch as an event on the watcher.
  - rados-rs reconnects the watch on session resets, map changes, and
    every five seconds while it is in error.
  - The driver watches again after a not-connected error (jhoblitt/rgw-rs
    design spec draft, section 7).
  - The design sets no retry limit, and its lints make a panic in
    production code a build failure (section 11).
- **Found:** rgw-rs design planning, 2026-09-27, as an unverified
  candidate. It was confirmed from source the same day.
- **Upstream:** not filed. The abort came with the fix for tracker #70422
  (tracker not checked).

## CEPH-BUG-019: radosgw accepts 0 for its GC, lifecycle and usage shard counts, then faults on first use

- **Component:** radosgw's option table (`src/common/options/rgw.yaml.in`),
  and the GC, lifecycle and usage-log code that shards by those options.
- **Status:** confirmed.
- **Symptom:**
  - `rgw_gc_max_objs`, `rgw_lc_max_objs` and `rgw_usage_max_shards` are
    `int` options with no `min:`. The option system therefore accepts 0,
    and negative values too.
  - Their neighbour `rgw_usage_max_user_shards` has `min: 1`.
  - `rgw_gc_max_objs = 0`:
    - the GC shard-name array has no elements;
    - a chain's shard index comes back as -1;
    - so sending a chain to GC reads before the start of the array, which
      is undefined behaviour;
    - the first overwrite or delete of an object that has tail objects
      does this;
    - before v19.2.3 and v20.1.0 the shard helper had no guard for 0, and
      the same path was an integer division by zero instead.
  - `rgw_lc_max_objs = 0`: finding a bucket's lifecycle shard is an integer
    division by zero (SIGFPE). An S3 PutBucketLifecycleConfiguration or
    DeleteBucketLifecycle does this.
  - `rgw_usage_max_shards = 0`: naming a usage-log shard is an integer
    division by zero. Two things do this:
    - radosgw's usage-log flush, when `rgw_enable_usage_log` is on;
    - any usage read or trim.
  - Only an administrator can set these options. Once one of them is 0,
    ordinary S3 requests reach the fault.
- **Affected:** every tag checked: v19.2.2, v19.2.6, v20.2.4, v21.1.1 and
  main.
- **Fixed in:** none.
  - 456a5e661d1 (squid, v19.2.3, ceph/ceph PR #62884) and a2b76b0e09e
    (v20.1.0, ceph/ceph PR #62850) guard `rgw_shards_mod` against 0.
  - That guard was for a shard count a radosgw-admin user types. For GC it
    only turned the division by zero into the out-of-bounds read.
- **Evidence:** source reading. None of it was run, because each case
  crashes or corrupts a gateway.
  - Options:
    - `v19.2.6:src/common/options/rgw.yaml.in:427-436` (`rgw_lc_max_objs`),
      `:1515-1527` (`rgw_usage_max_shards`) and `:1692-1701`
      (`rgw_gc_max_objs`) set no `min:`;
    - `:1528-1541` (`rgw_usage_max_user_shards`) has `min: 1` at `:1540`;
    - the same holds on main, at `:489-499`, `:1846-1865` and `:2026-2048`.
  - GC:
    - `v19.2.6:src/rgw/driver/rados/rgw_gc.cc:35-37` sizes the name array
      from the option;
    - `:63-66` compute the index through
      `v19.2.6:src/rgw/driver/rados/rgw_tools.h:46-49`, which returns -1
      for a count of 0 or less;
    - `rgw_gc.cc:128` takes that index, and `:132` reads `obj_names[i]`.
  - What reaches GC: `v19.2.6:src/rgw/driver/rados/rgw_rados.cc:5400`, in
    `complete_atomic_modification` (`:5382`). That is called on write
    (`:3314`) and on delete (`:5963`).
  - `v19.2.2:src/rgw/driver/rados/rgw_tools.h:48-49` has no guard, and
    computes `hval % RGW_SHARDS_PRIME_0 % max_shards`.
  - Lifecycle:
    - `v19.2.6:src/rgw/rgw_lc.cc:1974` takes the hash modulo the option;
    - `get_lc_oid` (`:1978-1988`) calls it from `guard_lc_modify` (`:2593`)
      and from `fix_lc_shard_entry` (`:2715`);
    - `guard_lc_modify` serves `set_bucket_config` (`:2659`) and
      `remove_bucket_config` (`:2686`);
    - the S3 ops call those at `v19.2.6:src/rgw/rgw_op.cc:6020` (PUT) and
      `:6038` (DELETE).
  - Usage:
    - `v19.2.6:src/rgw/driver/rados/rgw_rados.cc:1625-1626` take the
      modulo;
    - `log_usage` (`:1654`), `read_usage` (`:1681`) and `trim_usage`
      (`:1723`) call it;
    - the flush is `v19.2.6:src/rgw/rgw_log.cc:174`, gated at `:558-559`.
  - The same code is on each later tag:
    - `v20.2.4`: `rgw_gc.cc:128,132`, `rgw_lc.cc:2031`, `rgw_rados.cc:1729`;
    - `v21.1.1`: `rgw_gc.cc:129,133`, `rgw_lc.cc:2424`, `rgw_rados.cc:1797`;
    - main: `rgw_gc.cc:129,133`, `rgw_lc.cc:2490`, `rgw_rados.cc:1817`.
- **rados-rs:** unaffected; these are radosgw's options. rgw-rs is to
  refuse a shard count below 1 when it loads its configuration, rather
  than fault on first use (to be added to the rgw-rs spec in its review
  edits).
- **Found:** rgw-rs design planning, 2026-09-27, as an unverified
  candidate. It was confirmed from source the same day.
- **Upstream:** not filed.

## Considered and excluded

- **Map decode with duplicate keys** (candidate 4).
  - Whether the first or the last value is kept depends on the decode path.
  - Ceph's encoders never repeat a key, so the difference shows only on
    malformed input. That is a parity note (`rados/src/denc/codec.rs`,
    PR #21), not a defect that a correct client meets.
- **Two writing class calls in one compound op both read the pre-op state**
  (candidate 7).
  - This is OSD/cls semantics, and Ceph documents it:
    `v19.2.6:src/cls/2pc_queue/cls_2pc_queue_client.h:52` says "multiple
    operations cannot be executed in a batch".
  - rados-rs has a parity note in `rados-cls/src/call.rs` (PR #21).
- **radosgw-admin creating a stray `default` zone under Rook** (candidate 8).
  - This is a designed fallback. With no `--rgw-zone` and no default zone in
    the realm, radosgw-admin loads or creates the global default zone
    (`v19.2.6:src/rgw/driver/rados/rgw_zone.cc:1205-1222`).
  - Rook's admin configuration names no zone, so this is deployment
    configuration.
- **Older clients hanging after `ceph auth wipe-rotating-service-keys`**
  (candidate 5; Rook `cephx.go:159-161`).
  - The command is new in v19.2.6 and v20.2.4. It ships together with the
    `auth_epoch` handling that lets upgraded clients refresh (6b7ea3304d2).
  - The docs say that only upgraded clients refresh
    (`doc/rados/configuration/auth-config-ref.rst:432@v19.2.6`).
  - An older client keeps a ticket that a service rejects until its
    scheduled renewal (`src/mon/MonClient.cc:1595-1601@v19.2.5`,
    `src/auth/cephx/CephxProtocol.cc:201-202@v19.2.5`).
  - No hang has been observed. Reopen this as `suspected` if a rooket repro
    shows a stall that outlasts ticket renewal.
- **v19.2.3 stopping the mapping of instance `"null"` to `""` in
  `unlink_instance`** (c860a396697, 012d8ebd71f).
  - This is an intentional rework of multisite null-version sync (the
    commits cite tracker #67152), not a fix of a defect.
  - rados-rs documents it in `olh.rs` (PR #21).
- **cls_lock `MUST_RENEW` by a non-holder answering `ENOENT`.** This is
  intended: the flag is documented as "lock must already be acquired"
  (`v19.2.6:src/cls/lock/cls_lock_types.h:14`).
- **An expired ephemeral lock's object being removed only by a succeeding
  write method.** This follows from a failed method's writes being
  discarded. The read-path `EIO` is CEPH-BUG-011.
- **The guard method's trip condition changing in v20.2.0.**
  - v19 trips on any status other than `NOT_RESHARDING`
    (`v19.2.2:src/cls/rgw/cls_rgw.cc:4606`).
  - v20 trips on `IN_PROGRESS` or on the reshard-log threshold
    (`v20.2.4:…:906-918`).
  - This is an intentional redesign that came with in-class guarding
    (d011c522bb1).
- **cls_2pc_queue `commit` comparing an xattr-map iterator with the head
  map's `end()`** (`v19.2.6:src/cls/2pc_queue/cls_2pc_queue.cc:319`, fixed
  only on main by 28aacb0792b).
  - This is formally undefined behaviour.
  - libstdc++ gives every `unordered_map` the same null `end()`, so the call
    still answers `ENOENT` (checked with GCC 15). No client-visible effect.
- **cls_rgw `read_olh_log` dereferencing `begin()` of an empty pending log**
  (`v19.2.6:src/cls/rgw/cls_rgw.cc:2057`, still on main).
  - This is formally undefined behaviour.
  - libstdc++ reads the header's node count, which is 0, so the reply is
    correct (checked with GCC 15). No client-visible effect.
- **librbd leaking `C_UpdateWatchCB` when `rbd_update_watch` fails**
  (`v19.2.6:src/librbd/librbd.cc:6887-6897`). It is unreachable, because
  `register_update_watcher` returns 0 on every path, and rados-rs has no
  librbd. See also rgw-go, "librbd leaks the update-watch context when
  registration fails".
- **radosgw adding `STANDARD` to an empty placement-target `storage_classes`
  on decode** (`v19.2.6:src/rgw/rgw_zone_types.h:624`, `rgw_zone.cc:757`).
  This is an intended normalisation inside radosgw. See also rgw-go,
  "radosgw adds STANDARD to an empty placement target on decode".
- **Two latent bugs in the crypto slice API** (`v19.2.6:src/auth/Crypto.h:276-280`
  and `Crypto.cc:1005-1008`).
  - `CryptoKey::decrypt(slice)` calls `encrypt`, and `enc_size` for
    AES256KRB5 is short.
  - Neither has a caller outside `src/test`, so both are unreachable.
- **HMAC compared with a `memcmp` that is not constant-time**
  (`v19.2.6:src/auth/Crypto.cc:976`). A hardening concern with no
  functional symptom, outside this registry's scope.
- **The cls_otp C++ client building `cls_otp_get_result_op` and never
  sending it** (`v19.2.6:src/cls/otp/cls_otp_client.cc:76-81`). The client
  and the class agree on the wire, since both use `cls_otp_check_otp_op`;
  the unused struct is dead code.
- **radosgw calling writing class methods through librados's read `exec`**
  (fixed in v20.0.0 by af176311c47, tracker #65889). No client-visible
  symptom has been established. It is a request-shape difference, which
  release-shapes records.
- **The note at `v19.2.6:src/mon/AuthMonitor.cc:2108-2109`, which asks
  whether a wipe needs daemons to restart.** It is an open question in
  Ceph's own comment, and nothing has been observed. Plan 18's wipe test
  (B8) will show it.
- **Quirks recorded in the plans' parity notes.** Examples: `cmpxattr`
  returning 1, `zero` on a missing object being a no-op, the OSD dropping a
  misdirected op without a reply, missing `ceph-dencoder` registrations, and
  dead request fields. These are surprising or intended C++/OSD behaviour,
  which the parity notes cover.
