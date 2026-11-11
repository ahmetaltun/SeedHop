# Private sessions — implementation design, 2026-11-04

**v0.1.3 implementation.** Choose **File → New Private Session…**, then select a
download folder. A separate authenticated XPC helper uses transient access and
memory-only state. Add torrents or magnets, choose their initial files, and
pause, resume, or remove transfers within the private workspace. **End Private
Session** waits for the helper to exit. Payload files stay on disk in separate
session folders. Quit and update installation also end the private session.

The development smoke check added a paused fixture and ended the session with
the normal engine still connected and its library empty. The signed sandbox
build still needs the same focused check. The design and earlier checkpoints
below describe the implementation history.

## User-visible contract

A private session starts an empty, separate transfer workspace. Its torrent
metadata, resume state, transfer history and session settings remain in process
memory. Closing the workspace stops and drains its engine before discarding that
state. App termination, engine failure and system restart never restore it.
Downloaded and partially downloaded files remain in the chosen folder. Users
must add their torrents again to continue those files in a later normal session.

This protects against SeedHop retaining a private transfer library. It does not
hide traffic from peers, trackers, a VPN operator or an internet provider. It does
not remove payloads, quarantine attributes, filesystem metadata, OS memory/swap,
crash records, backups or source torrent files. No anonymity, zeroization or
secure-erasure claim is permitted. Private sessions must retain all existing
network, input, file-opening and quarantine protections.

## Repository findings

These are verified source relationships, not runtime acceptance:

| Existing owner | Current behavior | Required private boundary |
| --- | --- | --- |
| `crates/seedhop-core/src/lib.rs`, `Engine::open_with_policy_override` | Acquires a library lease, loads policies, recovers layouts, reads resume metadata, then creates JSON session persistence | Separate constructor that never enters ordinary library startup/recovery |
| `vendor/rqbit/crates/librqbit/src/session.rs`, `persistence_factory` | `SessionOptions.persistence: None` selects no session store and `NonPersistentBitVFactory` | Reuse this existing memory-only branch; also explicitly disable DHT persistence |
| `crates/seedhop-core/src/diagnostics.rs`, `Accounting::checkpoint` | Statistics retention off still loads connection settings and can finish a pending disk purge | Explicit nonpersistent accounting backend; do not reuse the statistics-off branch |
| `crates/seedhop-core/src/lib.rs`, mutation/checkpoint/shutdown methods | Statistics checkpoints and limit saves use `state_directory` | Route every state write through a backend that cannot expose a library path in private mode |
| `crates/seedhop-core/src/management.rs` and `scheduling.rs` | Queue, layout and schedule operations persist state | Memory-only supported mutations; reject journal-dependent operations before touching files |
| `crates/seedhop-core/src/layout_journal.rs` | Durable records protect relocation/rename and recovery | Keep this recovery design for normal libraries; disable these operations in the initial private session |
| `macOS/Host/EngineHost.swift`, `start`, `persistIntent`, `login`, shutdown | Foreground startup also saves run intent; background startup may auto-restore | Private host must never read/write run intent or register/login-restore a session |
| `macOS/App/EngineClient.swift`, `connect` | Chooses ordinary foreground XPC service or background Mach service | Private engine has an explicit, separately authenticated service identity with no background fallback |
| `macOS/App/SeedHopApp.swift` | Library windows share one `LibraryStore` | Private workspace needs its own store, engine and import routing |
| `macOS/App/ImportSession.swift` | Computes a destination using the torrent name and permits existing-file reuse | First private implementation must own a fresh exclusive payload subtree |

Ordinary startup cannot safely become private by pointing `state_directory` at a
temporary folder. That still writes torrent metadata and can leave it after a
crash. Deleting a temporary folder on normal exit does not meet this contract.
Likewise, disabling only `statistics.json` leaves JSON session metadata and other
write paths active.

## Chosen implementation boundaries

### 1. Explicit engine state ownership

Introduce an explicit library/private state backend. Preserve the existing
ordinary `Engine::open` behavior and on-disk formats. A private constructor takes
a validated policy snapshot and an approved payload destination, not an ordinary
library root or an arbitrary caller-selected state directory.

Only the library backend owns `LibraryLease`, durable settings, accounting paths
and layout recovery. Private code must not manufacture a fake root, load saved
torrents, create `engine.lock` or access ordinary recovery records. Centralize
checkpoint/settings dispatch so maintenance, mutation and shutdown share the same
nonpersistent decision. Audit all direct state writes before enabling the mode.

Private rqbit configuration explicitly uses no session persistence, nonpersistent
bitfields, no DHT persistence, and an ephemeral listening port. Core accounting
keeps session counters in memory without reading or serializing saved totals or
dates. The existing 15-minute activity window remains applicable. Queue/file
selection/piece preferences may change in memory; no private transfer can be
promoted into the ordinary library implicitly.

Initial file selection is supported. The current core increment rejects later
file membership/priority changes until promotion of boundary-piece storage has
its own ownership acceptance. Piece preferences and basic queue/limit controls
remain in memory. Normal-library file controls retain their existing behavior.

Initial private sessions reject relocation, rename, storage-recovery commands,
library migration, durable statistics controls, scheduling and login/background
configuration. Enforce this in the core and host, not only disabled UI controls.
Allow stopping/removing a transfer without deleting its files. Defer payload
deletion from private workspaces until ownership and confirmation have separate
acceptance. Preserve all existing normal-library operations.

### 2. Protection snapshot before any network work

Obtain the current validated network/encryption settings through the owning
ordinary engine, or under the library lease when no ordinary engine owns it.
Do not steal its lease or copy its session. Resolve pending/corrupt/required
network policy through existing error paths; never substitute direct routing.
This read-only path must not start an ordinary engine, complete a statistics
purge or repair/write ordinary settings. Do not log or persist the snapshot.

The private constructor validates the supplied policy before metadata lookup,
listeners, DHT or peers start. Bound DNS, destination filtering and encryption
remain identical to normal transfers. Interface/address changes retain the
existing blocked latch. The workspace displays the active protection state.
The initial private implementation freezes this snapshot: changing ordinary
settings cannot silently weaken a running private engine. Ending a private
session remains possible while networking is blocked. Starting another requires
a freshly validated snapshot.

### 3. Separate foreground lifecycle

Use a separate embedded XPC service identity built from the existing host code.
Do not add another runtime or an independently maintained engine implementation.
Packaging and signing must authenticate the exact private helper at both ends.
The private role is fixed by service identity, never selected by an untrusted
request to the ordinary/background host. It cannot publish a background service,
write run intent or survive loss of its owning app connection intentionally.

The initial UI supports one private workspace alongside the normal library.
Its store, details, selection, import sheets and notification bookkeeping are
separate. Normal transfers keep their original foreground/background lifecycle.
A private engine crash does not reconnect automatically or replay requests into
a normal engine. Old hosts must reject private startup; require a protocol bump
and a matching session-kind acknowledgement before enabling the workspace.
No compatibility fallback may start an ordinary session instead.

End-session first rejects new requests and cancels metadata work, then drains
engine-owned network/file work using the existing shutdown barrier. Close the
workspace only after confirmed shutdown or confirmed helper exit. Report failure
without claiming the session ended. App termination must end private engines
even when normal transfers continue in the background. Updates must use the
same drain/exit barrier; private requests are never queued for post-update replay.

### 4. Payload ownership and folder authority

Before the first write, create a fresh private-session child directory within a
user-approved destination using exclusive, owner-only creation. Reject a child
that already exists, is replaced, or resolves through an unapproved/symlink path.
Private torrents use fresh exclusive children within that session directory.
Do not adopt existing files or share normal-library payload paths in the first
implementation. Preserve quarantine and existing safe-file opening checks.

On end/crash, keep complete and partial payloads. Never recursively delete a
selected parent directory or treat an ordinary download folder as disposable.
Do not write torrent metadata, journal files or session bookkeeping into payload
directories. Unsupported file operations fail before writing anything.

Sandbox private helpers receive only user-approved transient bookmark authority.
They must not use `FolderAccessStore`'s persistent approval/archive path for
private destinations. Keep scopes alive until engine file work drains. The
normal app's existing grants are neither removed nor broadened. Native folder
dialogs may have OS-managed history; the UI must not promise otherwise.

### 5. Native private workspace

Add an explicit development-only **New Private Session…** entry. Explain before
starting that session records will not be saved, files will remain, and transfers
stop when the private session ends. Require a destination choice. Show a persistent
Private Session label and the actual network protection state.

Disable completion notifications, recent-document registration, restoration of
private window/transfer state, persistent per-transfer UI state and automatic
diagnostic/crash attachments. Generic appearance/language preferences may still
be read. Do not persist magnet links, filenames, paths, tracker URLs or private
identifiers in app preferences. Clipboard/export actions remain explicit user
actions with their existing scope explanations.

External magnets/files target the normal library by default. Opening a normal
external link must never change an existing workspace's privacy mode. Private
imports occur through that workspace's explicit controls. Do not reuse the
normal external-open pending queue for private inputs.

## Implementation and regression order

1. Core backend and constructor: memory-only state and exclusive private payload
   admission implemented; runtime acceptance remains deferred.
2. FFI/protocol and private helper: explicit role, authenticated identity, no
   fallback, validated protection snapshot, deterministic drain/exit behavior.
3. Development workspace: separate store/imports, approved exclusive destination,
   transient grants, private state indicator and end-session behavior.
4. Static gates: native Rust all-target compilation/Clippy, regenerated bindings,
   ordinary/private/sandbox Swift checks, localization and package-manifest checks.
5. Runtime acceptance remains deferred at the user's request. Do not enable a
   production private-session entry or make successful privacy claims before it.

Required deferred runtime evidence:

| Boundary | Acceptance |
| --- | --- |
| No app-controlled history writes | Observe actual filesystem writes during inspect/add/download/pause/remove/end, including failure paths; compare ordinary library/preferences before and after |
| No restart restoration | Clean quit, app/helper crash, forced termination and relaunch start with no private transfers; payloads remain |
| Network policy | Required VPN unavailable, disconnect/reconnect, DNS failure and blocked shutdown preserve fail-closed behavior without direct fallback |
| Payload isolation | Normal/private simultaneous transfers, same torrent, existing paths, symlink/hard-link/replacement races and external drives cannot share or overwrite ownership |
| Native lifecycle | Last-window close, application Quit, Quit and Stop Transfers, background normal engine, update and signing rejection behave as specified |
| Request isolation | Old/mismatched host, private role on normal/background endpoint, stale replies and transport interruptions cannot enter ordinary state |
| Sandbox | Transient scopes work without writing private bookmark archives; revoked access fails safely and shutdown still drains |
| Metadata and identity | v1, v2, hybrid and magnet selection, priorities, piece preferences and transfer accounting remain correct |
| UI/privacy surfaces | Recent items, notifications, defaults, external-open queues, window restoration, diagnostics and accessibility reveal no automatic private history |
| Normal regression | Existing library recovery, migration, queueing, settings, run intent and background resumption remain compatible |

This checkpoint executes no runtime tests, launches no engine, changes no
preferences/library data and makes no signing, sandbox or private-session
certification claim.

## Core memory-backend checkpoint — initial inspection foundation

`Engine::open_private(PrivateEngineConfig)` now accepts an existing download
directory, an explicit network policy, an explicit encryption policy and discovery
intent. It accepts no state directory. It validates protection before constructing
a session. Normal `Engine::open` retains library loading, lease ownership,
recovery and JSON persistence.

`StateBackend` distinguishes a library root/lease from memory-only state. Shared
session construction preserves interface binding, bound DNS, local-destination
filtering, encryption and network-blocked maintenance. Private configuration uses
no rqbit session persistence, no DHT persistence, nonpersistent bitfields and an
ephemeral listening port. Accounting starts empty without reading statistics or
performing a pending purge. Maintenance and shutdown skip library checkpoints;
queue bookkeeping and basic bandwidth limits can remain in memory.

The library-requiring boundary rejects statistics settings, saved encryption or
network changes, schedules, alternative-limit persistence, relocation, rename,
recovery and payload deletion. Private imports/adds are rejected before metadata
parsing, destination creation or transfer admission. This deliberate restriction
prevents the existing normal import path from creating/adopting payload folders
before private ownership is available. Local metadata inspection and magnet
inspection use the existing bounded/parser/network paths. Failed private network
settings mutations cannot stop the engine or save a different policy.

This is not exposed through UniFFI, XPC, app settings or a private window. Native
protocol remains 10 and existing connection-settings formats remain unchanged.
The Rust lease accessor now returns `Option<&LibraryLease>` because memory-only
engines have no lease; workspace consumers compile with that API. Normal leases
remain owned for the engine lifetime. The production app still constructs only
ordinary library engines.

Workspace/all-target Clippy passed with warnings denied. A native arm64 FFI
library build passed and its architecture was checked. New backend and integration
regressions compile but have not run: memory state cannot yield a path/lease,
private startup ignores decoy disk settings, persistent mutations fail before
side effects, and malformed/unavailable protection cannot select a direct
fallback. Runtime filesystem observations, metadata lookup, shutdown and normal
library regression acceptance remain deferred. No app/helper was launched and no
live library or preferences were changed.

## Exclusive payload-ownership checkpoint

Private imports/adds now accept only the configured parent folder, reject
existing-file reuse, and create a fresh owner-only session child lazily on the
first successful preparation. Every transfer receives another fresh child.
Names contain random UUIDs rather than torrent hashes. Merely opening a private
engine for inspection pins the parent without creating payload folders.

The core retains directory descriptors and verifies named directory identity
before admission, resume, retry and recheck. An opt-in vendored filesystem factory
holds the transfer root and created nested directories open. Unknown files and
directories must be created exclusively; existing paths are not adopted. In-memory
file identities permit reopening the original regular, single-link, owner-only
files on reinitialization. Replaced files, symlinks, FIFOs, foreign hard links,
unknown existing directories and widened permissions are rejected on those opens.
Quarantine is required before payload writes.

The storage ledger retains an original file descriptor for each materialized
file, so unlinking it cannot let a recycled inode number authorize another file.
This costs an additional descriptor per file and retains created directory
descriptors. Large torrents and descriptor exhaustion require runtime acceptance;
resource failures report an error rather than adopting another path.

Already-open file descriptors continue to refer to their original owned inodes
even if a pathname changes. New-open checks do not establish an instantaneous
stop guarantee for ongoing I/O, nor immunity from arbitrary interference by the
same user or a compromised process. Directory creation/open and identity checks
still require adversarial runtime/race acceptance; compiler checks are not proof
of all filesystem race conditions.

Initial partial-file selection stores required boundary-piece bytes under
`.seedhop-parts/<file-index>` inside that transfer's exclusive root. Unlike normal
library boundary storage, these paths do not contain the torrent hash. Later
membership/priority changes are rejected in the private core; the storage factory
also refuses promotion into a different payload path. Relocation, rename,
payload deletion, durable settings and layout recovery remain unavailable.

End, removal without deletion, and failed admission preserve created folders,
complete/partial payload files and hidden boundary bytes. No recursive cleanup,
old-folder adoption or automatic conversion into a normal library occurs.
Empty folders can remain after errors. Retained folders have opaque names;
later normal-library reuse may require manual folder organization and explicit
verification using the original torrent. Native recovery/import UX remains to
be implemented. New private engines restore no earlier transfer records.

This remains a Rust API only. The caller must retain approved folder authority
for the engine lifetime; sandbox transient-grant and native-host integration are
not supplied by this increment. No app entry, protocol version, exported FFI API
or library format changes here.

Workspace/all-target Clippy and the native arm64 FFI build passed. Core integration
regressions compile for fresh ownership, retained payloads/no restoration, initial
partial selection, replacement-directory rejection, reopening owned file inodes,
and refusing replaced files/nested directories. No runtime test ran. A separate
attempt to compile the vendored dependency's unit-test target through the outer
workspace lacked its dev dependencies (`tempfile`, `tracing_subscriber`,
`pollster`); it is not a passed vendor-test gate. The new storage integration
regressions compile through the main workspace instead.

Next work is the read-only protection snapshot and authenticated private helper,
followed by transient grants and a separate native workspace. Runtime filesystem,
quarantine, failure, transfer-interoperability, ownership and normal-library
regressions remain mandatory before production activation.
