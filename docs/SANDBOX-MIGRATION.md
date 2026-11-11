# App Sandbox and library migration

**v0.1.3.** The signed release workflow enables App Sandbox. On first launch,
choose **Import Existing Library…** before approving download folders. The
previous library is normally in `~/Library/Application Support/SeedHop`. Pick
the existing SeedHop folder, then approve the listed download folders. For a
fresh library, approve a download folder directly.

Use **Settings → General → Folder access → Restore Previous Library…** for
rollback. SeedHop stops the current engine and archives the sandbox copy before
re-enabling the old library. Downloaded files stay in place. To bring across
appearance, language, or layout preferences, use **Import Settings…** and
select an exported settings JSON file or the old
`~/Library/Preferences/app.seedhop.mac.plist`. The preference change applies
after restart and can be undone. Background and notification permissions use
the new app's settings and permissions.

The development notes below describe the original isolated implementation.
The signed release still needs a focused migration and folder-access check.

## Ownership and access

| Component | Library ownership | External folder access |
| --- | --- | --- |
| Existing production app and helpers | Existing Application Support library | Existing behavior; no activation change |
| Sandbox development app | Team-prefixed app group, separate bundle identity | Folder picker and its own persistent bookmarks |
| Sandboxed foreground XPC engine | Same app group library; existing exclusive engine lease | Receives temporary grants, stores its own persistent bookmarks |
| Sandboxed background agent | Same app group library and lease | Its own persistent bookmarks, restored before engine startup |

The application sends implicit-scope bookmarks over the authenticated XPC
connection. Each receiver creates bookmarks owned by its own executable rather
than relying on another executable's app-scoped bookmarks. Active scopes remain
held for engine lifetime and are balanced on replacement or process exit. Missing
or revoked grants cannot authorize a path. Startup preflights both the default
download root and every saved torrent destination; recovery and file operations
remain subject to the OS sandbox as well.

This follows Apple's guidance for [cross-process bookmark transfer and persistent
access](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox).
Main app and helpers use a [team-prefixed shared container](https://developer.apple.com/documentation/xcode/accessing-app-group-containers).
There are no broad home-directory or absolute-path filesystem exceptions.

The app has outgoing networking for Sparkle. Engines have incoming and outgoing
networking. Sparkle's installer service and its two documented lookup exceptions
are configured according to the [Sparkle sandbox integration guide](https://sparkle-project.org/documentation/sandboxing/).
The agent's Mach service uses the shared group's prefix. App-to-helper signing
requirements remain in force.

## Development build

The build entry point is:

```sh
python3 scripts/build.py --sandbox \
  --sandbox-team-id YOURTEAMID \
  --identity 'YOUR CERTIFICATE IDENTITY' \
  --validation-root .build/sandbox-profile
```

Replace both signing placeholders. The team ID must match the certificate. The
build rejects ad-hoc signing, missing isolation, and release activation. It checks
the signed app's team before reporting success. Sandbox and ordinary validation
builds use different bundle and service namespaces, even with the same output
root. This command has **not** been run as part of this increment.

Do not begin acceptance with a production library. Use an expendable library and
payload copies. macOS may require authorization for a development-signed app group;
the code does not fall back to another library when group access is unavailable.

## Migration and rollback

Settings → General contains development-only folder and library controls. Import
the previous library before granting download folders or starting a new library.
The destination must not already exist. The source must have a valid engine lock
and session, and its engine must be stopped. An unfinished layout transaction,
unknown state file, unsafe file type, unsupported settings version, or unfinished
statistics purge stops the
import. Existing sources are never silently replaced with an empty library.

Import performs these steps:

1. Acquire the source's existing engine lease without replacing its lock inode.
2. Copy allowlisted settings, session metadata and bitfields to a private staging
   directory. Payload files stay in their existing folders. Preserve queue,
   pause intent, file choices, network policy, encryption, schedule and limits.
3. Persist matching recovery receipts in staging and the source. The receipt
   preserves the source's exact connection-settings bytes, including absence.
4. Write a version-3 connection-settings barrier to the source. Current and older
   settings readers refuse that library before opening a session.
5. Publish the completed staging directory with an exclusive atomic rename. A
   concurrently created target causes failure rather than being overwritten.

The import reads only regular, singly linked files through no-follow descriptors.
Individual files are capped at 32 MiB, aggregate copied bytes at 256 MiB, and
session entries at 10,000. New directories use 0700 and files 0600. Atomic writes
and publication sync their data and containing directory. Failed staging copies
are retained for inspection, not loaded as an active library.

Run intent is not copied. Background registration and resume-at-login require a
new explicit choice. The isolated app has separate UI preferences. Appearance,
language and selected workspace preferences can now be transferred explicitly as
described below; notifications and access permissions remain separate choices.

**Restore Previous Library…** requires both engines to be stopped. It validates
the matching receipt and acquires both library leases when a migrated target
exists. It blocks the migrated reader with version 3 **before** restoring the
source's original settings. The migrated state and settings backup remain on disk.
Statistics-retention format 4 is preserved by migration and rollback; inactive
copies still use the version-3 barrier. Clearing statistics in the active copy
does not erase migration originals or other library snapshots.
Rollback restores the original state snapshot; later changes to the migrated copy
are not merged back. Do not move the original library while this receipt is active.

If an import is interrupted after blocking the source, use the restore control on
that same source folder. It also handles a staged copy that was never published.
Do not delete or downgrade the version-3 barrier manually. Version 3 is reserved
for an inactive migration copy, not a newly supported active settings format.
After rollback, forward migration needs a fresh isolated target; this slice never
overwrites the retained migrated library.

## Portable UI preferences

Development builds now expose Settings → General → Settings transfer. In the
ordinary development app, **Export Settings…** writes a bounded JSON file chosen
in a save panel. In the sandbox development app, **Import Settings…** validates a
selected file and asks for confirmation before staging it. No production
preference domain is read directly by the sandbox build.

The version-1 portable format contains only:

- Appearance and explicit language override, including “follow macOS.”
- Dock speed display, transfer-details visibility, sidebar visibility and row density.
- Native column order, recognized column widths and sort descriptors.

It contains no torrent history, filenames, folder paths, folder grants,
notification preferences, login/background choices, update preferences, or engine
security policies. It does not copy the complete UserDefaults domain, window
positions or opaque SwiftUI column-customization data. Core settings continue to
travel with the library-state migration instead.

Imports and undo apply on the next ordinary app launch before the library store
and views read their preferences. The live UI does not change while closing and
cannot overwrite the staged settings on exit. At application time, the code
durably saves the exact previous values of the portable keys, including absent
keys, before publishing the new preference domain. Unrelated preference keys
remain untouched. Language selection is refreshed explicitly to avoid using a
previously cached bundle-language choice.

**Undo Settings Import** cancels an import that has not yet applied, or schedules
restoration of its previous snapshot at the next launch. Only the latest import
has an undo checkpoint; staging another import replaces the prior checkpoint.
Interrupted application or undo retains its transaction for replay. A transaction
with an invalid version, missing backup or corrupt contents is retained and
reported as an error rather than guessed or deleted.

Each app identity has its own private transaction directory, owner-only files,
and an exclusive transaction lease. Reads reject symbolic links and non-regular
files. Portable input is capped at 64 KiB, the undo snapshot at 64 KiB and the
transaction file at 256 KiB. Identifiers, counts and finite numeric ranges are
validated before staging. Fixture, self-test and render launches neither apply
these transactions nor expose active transfer controls.

This explicit development path implements portable preference transfer; automatic
production preference migration remains gated with production activation. No
settings were exported, imported or restored while implementing it.

## Damaged folder-access records

The sandbox development build now supports recovery through the existing folder
picker. Choose **Grant Folder Access…**, or reselect a download/import/relocation
folder. The code activates and validates that real folder approval before reading
or replacing saved access records. A plain saved path never creates authority.

For a damaged version-1 archive, the repair retains structurally valid records,
adds the new approval, and backs up the exact original bytes before publishing
the replacement. Identical duplicate records collapse to one; conflicting records
for a path are excluded unless that path is explicitly reapproved. Unresolvable
bookmark blobs remain stored for reapproval, but grant no access while unresolved.
If the JSON structure is unreadable, existing approvals cannot be inferred; its
raw backup remains available and the required folders must be approved again.

The app and each helper repair only their own archives. A helper receiving valid
folder grants over the existing signed connection can repair its damaged archive
and create its own persistent bookmarks. Every incoming grant is validated before
the batch commits. Empty batches and ordinary startup do not rebuild damaged
archives. Re-selecting a folder also rereads the file when an in-memory cache
already exists, so damage is backed up before a later approval overwrites it.

Reads use bounded, no-follow, non-blocking descriptors and require regular,
singly linked, current-user-owned files. Writers hold a per-role process lease.
Backups use exclusive creation, mode 0600 and file/directory sync before atomic
replacement of the active archive. If backup creation fails, the active archive
is not replaced. Unsafe, inaccessible or oversized files fail closed. Recognizable
unsupported versions are never rewritten and show a compatibility message.

Backups remain inside the private shared container as
`<role>-folder-access-damaged-<UUID>.json`. They contain folder paths and bookmark
data, are not included in diagnostics, and are not automatically restored into
another executable. No automatic backup pruning is implemented in this slice.
An interrupted final publication may already have replaced the active archive;
the operation reports failure if durability cannot be confirmed. Repeating folder
approval is safe; no successful-durability claim is made on that failure path.

Scope counts are balanced on failed batches and replacement. Stale bookmark
renewals are written only when the corresponding disk record still matches the
record that produced the valid scope, preserving another writer's newer approval.
Runtime corruption, interruption and cross-process acceptance remain deferred.

## Verification status and release gates

Static checks completed for both sandbox and ordinary app/host variants. Python
build files compile. No app was launched, no user library was copied, no login
service was registered, and no runtime test was run in this increment.

Before production activation, deferred acceptance must cover:

- Signed, independently sandboxed app, XPC and background processes; shared Mach
  service registration, app-group access and absence of broad filesystem access.
- Folder approval across initial launch, relaunch, foreground/background handoff,
  login, revoked/stale grants, corrupt bookmark archives and external-drive changes.
  Reapproval-based archive repair is implemented; malformed, mixed-validity,
  conflicting-duplicate, unknown-version, empty-batch and failed-backup cases still
  require runtime acceptance. Verify exact backup bytes and balanced scope counts.
- Migration interruptions at every durable step, matching and mismatched receipts,
  active source/target engines, unsupported files, resource limits and rollback.
- Existing payloads after migration, preserved pause/queue/file intent, new imports,
  rename, relocation, removal, retained-copy recovery and quarantine behavior.
- Portable preference export/import, pending cancellation, exact undo, interruption
  replay, malformed/oversized archives, fixture isolation, and startup language and
  appearance. Production migration, localized native controls and accessibility
  remain gated. New translations are AI drafts requiring human review.
- Sparkle installation and background restoration. Development builds intentionally
  disable the updater, so entitlement configuration alone is not update acceptance.

Production state-location changes, final preference migration and production
activation remain gated on this work. There is no sandbox escape or release
readiness claim from compilation alone.
