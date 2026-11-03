# Local privacy controls — development checkpoint, 2026-11-03

Development builds now expose Settings → General → Local activity → **Clear Local
Activity…**. The action requires confirmation. Release builds exclude both the
control and its native host command. Runtime tests remain deferred at the user's
request; compilation is not functional acceptance.

## Stored data and clearing scope

| Data | Existing location/lifetime | This action |
| --- | --- | --- |
| Activity timeline | App memory; at most 200 rows per transfer | Clear this app process's rows |
| Traffic graphs | App memory; at most 900 samples/15 minutes per transfer | Clear samples |
| Tracker activity events | Running engine memory | Clear queues in the connected engine |
| Selected details and peer rate baselines | App memory | Clear cached details; fresh observations may return |
| SeedHop recent-document list | Native app-specific recent items | Request clearing through NSDocumentController |
| SeedHop notifications | App-specific Notification Center records | Request removal of delivered and pending notifications |
| Saved library, torrent metadata and bitfields | Library/session files | Preserve |
| Lifetime statistics and timestamps | Library statistics.json | Preserve |
| Network policy, encryption, queue and file intent | Existing library settings | Preserve |
| Folder approvals, migration backups and preference undo | Existing private stores | Preserve |

No payload file is opened, relocated, deleted or rechecked by the core clearing
method. No transfer is paused or removed. Current tracker observations, peer
connections and counters continue to describe running transfers. This does not
erase exported files, system-wide Finder history, backups, crash data or storage
remnants; there is no secure-erasure claim.

## Connected and offline behavior

The native host handles the command before its usual lazy engine startup. When
an engine exists, its tracker-event queues are cleared without resetting event
sequence numbers. Local clearing is permitted while network policy blocks
transfers; it does not change the required connection or bypass folder approval.

A disconnected app clears its local presentation records and requests the native
recent-item/notification cleanup, but opens no engine connection. This avoids
activating a background agent's saved login restoration just to clear history.
The UI reports that engine history was not confirmed cleared and asks the user to
reconnect and retry. An engine failure or missing acknowledgement gets the same
partial-result message, rather than a complete-success claim.

The action is unavailable during connection changes, file operations, busy
transfer mutations or another clear. Fixture, render and self-test launches
disable the control and do not invoke native cleanup APIs.

## Replies arriving after a clear

Clearing advances an activity generation. Detail replies and errors from older
requests cannot refill cleared caches. Sampling and detail application are
suppressed during the clear; the current watcher can resume with a fresh request
afterward. Old tracker events are also filtered through the clearing second for
the remainder of the app session, including after a partial engine failure.
Events created in that same second may be suppressed because tracker timestamps
have one-second precision.

Running transfers can generate new events, graph samples and notifications. If
an engine operation times out, it may finish later; the UI does not represent a
timeout as successful engine clearing.

## Verification and next privacy work

The Rust core/FFI build completed, Swift bindings were regenerated, and Clippy
passed for both libraries with warnings denied. Swift compiler checks cover the
development app, sandbox app, native host and release app. No app or engine was
launched, no library or preferences were migrated, and no actual history,
notifications or recent items were cleared during implementation.

Deferred acceptance must exercise connected and network-blocked clearing,
disconnected and interrupted connections, no-start behavior, multiple transfers,
old detail replies/errors, new post-clear events, pending/delivered notifications,
app-specific recent items, fixtures and native accessibility. Confirm that payload
bytes, statistics, counters, pause state, queue order and policy remain intact.

Automatic expiry and private sessions remain future work. Optional statistics
retention is implemented in the following development increment. Existing explicit
library removal remains separate from these controls.

## Optional saved-statistics retention

Settings → General → Saved statistics now offers **Remember transfer statistics
between launches** in development builds. Existing libraries keep their previous
default of remembering statistics. Merely viewing the setting does not upgrade
the library or start an engine session.

Turning it off requires confirmation and clears this active library's saved
upload/download totals, added/completed dates and accumulated seeding time.
`statistics.json` becomes an empty object. Current-session upload/download counters
remain in memory; retained-history fields become unavailable rather than claiming
that unknown historical totals are zero. Payload files, verification bitfields,
selected files, trackers, queue/pause intent and session resume metadata remain.

While retention is off, accounting checkpoints do not serialize session counters.
Fresh engine startup does not deserialize old statistics and empties that file
before creating the torrent session. Re-enabling persistence first finishes any
pending purge, then starts fresh retained totals. It does not persist counters
accumulated while retention was off or restore erased historical dates. Only
currently registered transfer accounting entries can be written at that boundary.

### Ordering, compatibility and failures

An explicit change uses connection-settings format 4. It requires both a Boolean
`remember_statistics` and a Boolean `network_policy_required`; missing flags are
an error, not defaults. Formats 1 and 2 retain legacy behavior. Format 3 remains
reserved for inactive sandbox migration copies. Readers limited to formats 1/2
refuse format 4 before session startup. Re-enabling statistics does not downgrade
the format. Network-policy and encryption saves preserve the retention fields and
the existing required-network barrier.

The disable transaction is:

1. Persist format 4, retention off, and `statistics_clear_pending: true`.
2. Atomically replace the active statistics file with an empty object and sync it.
3. Persist completion by clearing the pending marker.

A failed purge remains visible as pending. Maintenance in a running engine retries
it, and startup retries it before network/session restoration, even when the
required network is unavailable. Persistence cannot be re-enabled while that
purge fails. An initial settings-write failure may leave the previous choice in
place; a rename followed by a flush failure may leave the new readable choice.
Errors remain reported in both cases. There is no successful-durability claim on
an unsuccessful save.

A drained engine with retention off can exit without serializing session counters
or being held open solely by an unfinished statistics purge. Its pending marker
remains for the next startup. Payload/session-stop failures retain their existing
handling. A pending purge is therefore not represented as completed just because
the application exits.

When connected, the setting updates the owning engine. When disconnected, it uses
a foreground control connection and the existing exclusive library lease; it does
not activate background login restoration. A library already owned by another
engine cannot be edited through that offline path. Preferences-only foreground
helpers do not create or overwrite background run intent on disconnection. The
host rechecks connection ownership before stopping an engine after its last client
leaves. No user service was registered or restarted while implementing this.

Sandbox migration accepts format-4 libraries but refuses an unfinished statistics
purge. Complete that operation in the source first. Import an existing library
before saving policies into a new sandbox development library.

### Scope and remaining acceptance

This setting affects the active library's accounting file. It does not remove
older library copies, migration originals, backups, exports, notification
deduplication identifiers in app preferences, or necessary torrent metadata.
Restoring another library snapshot can restore that snapshot's older policy and
statistics. This is neither secure erasure nor a private session.

Rust build and workspace/all-target Clippy checks passed, Swift bindings were
regenerated, and app/host/release compiler checks passed. Existing tests were
compiled, not executed. No actual statistics, preferences or library were changed
by these checks; the app bundle was not rebuilt or launched.

Deferred runtime acceptance must cover disable/re-enable boundaries, byte counters
and unknown historical dates, process restart, shutdown, concurrent sampling,
failed guard writes, purge failure and retry, directory-flush failures, missing or
malformed format-4 flags, older readers, preservation of VPN/encryption settings,
sandbox migration/rollback, and foreground/background ownership. Confirm payload
and resume continuity and that off-period counters never reappear in saved totals.
