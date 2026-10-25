# Transfer and file controls

Implemented on 2026-10-25 across the Rust engine, UniFFI, protocol-v2 XPC,
and native menus, file inspector, and transfer settings.

## Behavior

- **Force Recheck** discards the piece map and hashes files again. It retains
  full torrent identity, selection, priorities, queue membership, and user pause
  intent. Running transfers repair corruption automatically; paused transfers
  stay paused. Checking transfers reject another recheck. Unavailable destinations
  and failed persistence reach callers.
- **File priorities** in the Files inspector offer Low, Normal, and High.
  Selected files are requested in descending priority, with filename ordering
  as tie breaker. The selection checkbox controls skipping independently.
  Priority changes briefly suspend peers, persist choices, and restore intent.
- **Queueing** in Settings → Transfers caps concurrent incomplete downloads.
  Zero means unlimited. Completed transfers seed outside the cap; stalled
  downloads occupy a slot. User-paused transfers stay paused. Transfers → Move
  to Front of Queue changes durable FIFO order. Resume enters the queue rather
  than bypassing the cap. Queue suspension and user pause are separate persisted
  flags. Restoration parks downloads before admitting them in order.
- **Move Files…** chooses a destination folder and moves this torrent's files,
  including present unselected files. Unrelated files remain where they are.
  Selection, priorities, queue membership, and pause intent are retained.
- **Rename…** in the Files inspector changes a local filename without changing
  torrent metadata or protocol identity. New paths survive restart, recheck,
  relocation, seeding, and removal with file deletion.

## Storage transactions

Moves and renames quiesce peer writes, copy into exclusively created target
files, flush bytes and directories, then persist the new layout. Only after
commit do they remove originals. New paths are verified with torrent hashes;
running transfers continue automatically. Protocol offsets remain independent
from local names.

Conflicting targets, invalid indices, traversal, and symlink paths are rejected.
Descriptor-relative opens and cleanup do not follow symlinks. A failed copy or
layout save retains original bytes; failed saves restore the old layout. Copies
require additional free space, even for a local rename. An interruption before
commit can leave unreferenced target copies; one after commit can leave extra
originals. These copies are deliberately retained to avoid data loss. Existing
targets are never overwritten, so leftover copies need inspection before retrying
that destination. Empty source directories can remain. There is no cancellation
UI for these operations yet.

## Verification

- `tests/recovery.rs`: v1 corruption/recheck, restored verified bytes, pause intent,
  restart, and failed persistence.
- `tests/v2.rs`: v2/hybrid corruption repair; selection/priorities; paused and
  seeding moves/renames; partial active downloads completing exactly after move
  and rename; full identity, restart, and removal.
- `tests/management.rs`: FIFO cap/order, pausing queued transfers, Resume admission,
  completion outside the cap, restart, unsafe/conflicting targets, failed priority
  and layout saves, rollback, and deletion of current names.
- `tests/swarm.rs`: actual TCP piece Requests prove a High file is requested before
  the default alphabetical first file; all downloaded bytes compared exactly.
- Swift `--verify-engine`: every new command over XPC, exact moved/renamed bytes,
  durable settings and paused selection after helper restart.
- Swift `--verify-ux`: delayed commands, stable targets, duplicate suppression,
  failures/retries, and selection preservation.

Physical sleep/wake, unplugging, interface loss, and actual logout/login were
deferred at the user's request. These gates are not closed. Native pointer,
keyboard, VoiceOver, and very large-library reviews remain manual. Encryption
and uTP remain separate protocol projects.

Results summary: [`results/file-controls-2026-10-25.json`](results/file-controls-2026-10-25.json).

To rerun the isolated certificate lifecycle fixture when that gate resumes:

```sh
python3 scripts/build.py --identity 'Apple Development: Your Name (TEAMID)' --validation-root .build/lifecycle-validation
cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/lifecycle-fixture
.build/lifecycle-validation/SeedHop.app/Contents/MacOS/SeedHop --verify-lifecycle "$PWD/.build/lifecycle-fixture"
```

Use a fresh validation root for a fresh fixture library. The app registers only
its isolated validation agent, runs three handoff cycles and simulated login-scope
cases, then unregisters it. System permission may be required. This deliberately
does not log out, disable interfaces, unplug drives, or certify final signing.
