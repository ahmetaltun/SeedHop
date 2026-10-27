# Interrupted file-operation recovery

Moves and local renames use a versioned `layout-transaction.json` beside the
existing library state. The helper remains the sole writer under the library
lease. The saved session database determines the active layout; the journal
never substitutes a guessed destination or changes torrent identity.

## Transaction and restart order

1. Validate paths/capacity and quiesce peers without changing user pause intent.
   Persist the original layout, including any legacy filename mapping.
2. Durably record transaction/torrent IDs, old/new roots and filename maps,
   directory identities, source file identities/sizes/modification times, and
   changed paths before creating target files.
3. Create targets exclusively. Record device/inode identity and creation time
   when available before writing the first byte. Copy in bounded chunks, flush
   bytes/directories, and save SHA-256 receipts for completed source copies.
4. Atomically close cancellation, replace the in-memory layout, and save session
   metadata. A failed flush may have already renamed its database; recovery reads
   and synchronizes that saved database before choosing a side.
5. If the new layout is saved, verify replacement bytes before removing unchanged
   originals. Otherwise retain originals and clean only verified target prefixes.
   Persisted quarantine names make cleanup itself restartable. Start normal
   torrent hash verification after this cleanup attempt.

Recovery runs after acquiring the lease and before session restoration resumes
writes. Repeating recovery is safe after another interruption, including while a
file is quarantined. Completed cleanup removes and synchronizes the journal.
While a journal remains, a missing/replaced active folder prevents restoration
from creating replacement files. An unavailable extra-copy folder can wait while
the correctly identified active location restores.

Every deletion checks identity and bytes. A partial copy may be removed only
when the unchanged original still contains those bytes. An original may be
removed only when its identity/content are unchanged and its saved replacement
contains the same bytes. Descriptor-relative traversal rejects symlinks.
Quarantine renames never overwrite existing paths, and ownership is checked
again after rename and verification. Empty parent directories may remain.

## Retained copies and native actions

Unavailable/replaced volumes, changed/replaced files, missing byte receipts,
unrecognized layouts, and malformed/future records retain uncertain files.
A notice crosses UniFFI/XPC into the library, identifies the active original/new
location when known, and offers:

- **Retry Cleanup:** check again after access is restored.
- **Keep Extra Copies:** finish tracking a recognized transaction without deleting
  remaining files. This removes only the journal; retained files can still
  conflict with a later move to that destination.
- **Show Original Folder / Show Destination:** inspect retained files. A failed
  quarantine restoration reports its retained path for inspection as well.

Unknown authority or unsupported records cannot be dismissed with Keep Extra
Copies. Retain the record and use a compatible build or repair state before
retrying. New moves/renames are blocked while cleanup needs attention. Removing
the affected transfer also requires resolving its cleanup first. Retry/Keep
capture the journal ID, reject stale requests, and preserve current selection.

Operation progress is transient; the journal supplies durable recovery.
In-flight OS reads/writes/flushes must return before graceful cancellation
completes. Recovery hashing adds disk reads and favors retaining uncertain bytes.
If an active/default destination is unavailable, existing startup validation still
requires reconnecting it. Recovery never invents a replacement download folder.

## Compatibility

Libraries without a journal use their existing restoration path. The session
database shape and filename-mapping version are unchanged. Legacy v2/hybrid maps
are materialized through existing persistence before recording the original
layout. Snapshot notices are optional; JSON/XPC protocol version remains 2.

Older builds do not interpret journals. The saved session still names the active
layout, and extra copies remain. Use this recovery-capable build to finish
cleanup; binary downgrade behavior has not been separately certified. Unknown
journal versions are never silently migrated or discarded.

## Verification

`make layout-recovery` builds a separate lab binary with the explicit
`layout-fault-injection` feature, absent from normal app builds. It stops isolated
engines at journal/target creation, copying, copy completion, pre-commit saving,
commit, quarantine, and cleanup; sends actual SIGKILL; restarts; compares exact
bytes and saved intent; and repeats restoration for idempotency.

The 64-case matrix covers v1/v2/hybrid moves and renames, paused/running/queued
intent, non-default selection/priorities, present unselected files, edited copies
and Keep Copies, replaced-active-folder refusal, legacy maps, and two cross-volume cases on a disposable HFS+
disk image. The image is detached and deleted afterward. Unit tests additionally
cover replacement, symlinks, corrupt/future records, unavailable/replaced roots,
stale actions, journal-path conflicts, and quarantine recovery.

The development app's `--verify-storage-recovery FIXTURE_DIRECTORY` checks an
isolated retained-copy fixture through real XPC: notices, retry retaining an
edited copy, Keep Copies, exact originals, saved choices, and restart. Controlled
Swift tests cover presentation state; light/dark fixtures cover recovery layout.

These are process-crash and controlled filesystem checks. They do not certify
power loss, physical drive removal, sleep/wake, actual login, or manual keyboard/
VoiceOver acceptance. Signing and release remain deferred. Portable results:
`results/storage-recovery-2026-10-27.json`.
