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

### Files inspector — 2026-10-27

The details Files tab now uses a compact native folder outline, distinct row selection
and download inclusion, direct per-row priority pop-ups, shared Actions menus,
context actions, and scoped path search. Row priority does not require selection;
the Actions menu supports batch priority changes. Low/Normal/High priorities support selected files and folder
descendants in one command using the current confirmed snapshot. Changing
priority does not include skipped files. Folder inclusion is one selection
command and cannot exclude the final included file. While searching, folder
changes are unavailable; individual matching files remain actionable.

Rename remains a single-file operation using the existing sheet, transaction,
progress, cancellation, and recovery behavior. Folder rename, batch rename,
per-file opening, and Quick Look are outside this change. Checking suppresses
normal progress percentages; errors show progress unavailable, and disconnected
data is explicitly last known. Numeric progress derives from the engine's
per-file have-byte snapshot, not a new filesystem verification pass.

The design plan and verification record are in `FILES-INSPECTOR-REDESIGN.md`.
Checkboxes, icons, and disclosure indicators align with each filename line.
Operation messages and search counts use a fixed-height footer; selecting the
last included file never inserts a notice above the list or moves its viewport.
Controlled native callbacks and rendered fixtures do not certify physical input
or VoiceOver speech; those manual acceptance gates remain separate.

## Storage transactions

Moves and renames quiesce peer writes, copy into exclusively created target
files, flush bytes and directories, then persist the new layout. Only after
commit do they remove originals. New paths are verified with torrent hashes;
running transfers continue automatically. Protocol offsets remain independent
from local names.

Before suspending transfers or creating target files, moves and renames check
available destination capacity. The estimate includes present unselected files
and logical lengths of sparse files; present partial files reserve at least
their expected final length. Missing source files require no copy space. An
insufficient-space error reports required and available bytes and asks the user
to free space or choose another folder. This is an advisory check, not a space
reservation: allocation can still fail later, using the existing rollback path.

Conflicting targets, invalid indices, traversal, and symlink paths are rejected.
Descriptor-relative opens and cleanup do not follow symlinks. A failed copy or
layout save retains original bytes; failed saves restore the old layout. Copies
require additional free space, even for a local rename. An interruption before
commit can leave unreferenced target copies; one after commit can leave extra
originals. These copies are deliberately retained to avoid data loss. Existing
targets are never overwritten, so leftover copies need inspection before retrying
that destination. Empty source directories can remain.

## Progress and cancellation

Moves and copy-based renames publish a unique engine-generated operation ID,
target torrent, phase, and copied/total bytes through UniFFI and XPC snapshots.
The library shows progress even when the inspector is closed or another transfer
is selected. Preparing, copying, saving, verification startup, cleanup, and
restoration have distinct messages. The copy total uses actual source lengths
after peer writes are quiesced; it is separate from capacity reservations and
from torrent hash-verification progress.

Cancel is available while preparing or copying. Copying checks cancellation
between chunks of at most 1 MiB, including before the first read. An operating
system read, write, or flush already in progress must return before cancellation
can finish. Cancellation removes this operation's uncommitted targets, retains
original files/layout, and restores the existing paused/running/queued intent.
Cleanup or resume failures are reported rather than hidden as successful
cancellation.

Entering the saving phase atomically closes cancellation before the new layout
is saved. Late cancellation returns `false`; an old ID cannot cancel a later
attempt. The interface removes Cancel during saving and explains that the change
is finishing. After commit, hash verification is shown in the transfer list
independently of copied bytes. Conflicting transfer commands, imports, and host
switches are blocked while the file operation is active; navigation remains
available. Confirmed cancellation is a typed result, not an operation error.

Operation progress is transient, while moves/renames now have a durable journal.
Restart reconciles it against the saved session before resuming writes and
removes only copies with verified ownership and bytes. Changed or uncertain
files remain, with Retry Cleanup, Keep Extra Copies, and folder inspection actions.
See [interrupted-operation recovery](STORAGE-RECOVERY.md). Controlled process-crash
checks do not certify physical drive loss or power loss.

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

The October 26 progress/cancellation follow-up additionally checks bounded copy
cancellation, the cancellation/save race, paused/running/queued cancellation and
retry, and v2/hybrid completed/partial copy cleanup with present unselected files.
The packaged `--verify-engine` test observes copying and cancels both a move and
a rename over real XPC before exercising successful changes and restart.
`--verify-ux` covers selection/inspector changes, stale requests, saving-phase
guards, cancellation failures/retries, and conflicting commands. Portable results
are in [`results/file-progress-2026-10-26.json`](results/file-progress-2026-10-26.json).

Physical sleep/wake, unplugging, interface loss, and actual logout/login were
deferred at the user's request. These gates are not closed. Native pointer,
keyboard, VoiceOver, and very large-library reviews remain manual. Encryption
and uTP remain separate protocol projects.

Results summary: [`results/file-controls-2026-10-25.json`](results/file-controls-2026-10-25.json).
That historical summary predates the capacity preflight. The ordered follow-up
plan and current implementation record are in [`PRODUCT-ROADMAP.md`](PRODUCT-ROADMAP.md).

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
