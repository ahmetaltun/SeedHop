# Recovery validation

Executed on 2026-10-25 on Apple Silicon, macOS 15.8.1, against the pinned
rqbit 9.0.1 engine. All four new tests passed. The full Rust workspace suite
also passed: 13 regular tests, with the disk-image test executed separately.
Formatting and workspace/all-target Clippy with warnings denied passed.

| Scenario | Fault and recovery | Result |
|---|---|---|
| Missing destination on add | Add fails without creating the folder or adding library membership; create folder and retry | PASS |
| Destination disappears before Resume/restore | Rename a paused transfer's folder away; reject Resume and restore, preserve session bytes, restore folder, resume original transfer | PASS; 262,144 bytes compared exactly |
| Interrupted connections | Close active TCP relay streams during a partial download, reject new peer connections and actual HTTP tracker requests, restore connectivity without issuing Resume | PASS; progress stalled at 524,288 bytes in retained run, then 4,194,304 bytes compared exactly |
| Full destination disk | Fill a dedicated HFS+ volume until the OS returns ENOSPC; engine payload write fails; free space and Resume original transfer | PASS; ENOSPC exposed in snapshot, then 2,097,152 bytes compared exactly |
| Full state disk | Fill the same isolated volume with the state directory on it; saving changed limits fails; free space and retry, then restart | PASS; caller receives ENOSPC, prior saved and active limits preserved, retried values survive restart |

The last two scenarios share one disk-image test. The retained runner report,
including the relevant test output, is
[`results/recovery-2026-10-25.json`](results/recovery-2026-10-25.json).

## Fixes established by the tests

`Engine::set_paused` now validates the saved destination and torrent paths before
Resume. Previously, an already-open paused transfer could accept Resume after its
folder disappeared. The test failed against that behavior and passes with the
new check. Failed Resume preserves paused intent and the saved session bytes.

The same operation now restarts an errored transfer even when its saved intent
already says running. Previously, the idempotent-command shortcut only persisted
the unchanged intent, leaving the transfer in error. The actual disk-full test
requires leaving the error state, rechecking disk data, and completing the same
transfer after space is reclaimed.

No vendored engine changes or dependency changes were needed.

## Reproduce

```sh
# Regular missing-destination and connection tests; needs local socket access.
cargo test -p seedhop-core --test faults --locked -- --nocapture

# Also run actual disk-full payload and state writes; macOS only.
make faults

cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`scripts/faults.py` creates a fresh 32 MiB HFS+ image under `.build/faults-*`,
mounts it without opening Finder, runs the opt-in disk-full test, and detaches
and deletes the image in cleanup. The Rust test refuses ordinary directories
and volumes larger than 64 MiB before writing filler data. It truncates and
flushes the filler before retrying so HFS+ has reclaimed the space. Logs and
`result.json` stay in the run directory; `.build/faults-result.json` points to
the latest run's evidence. The normal user library and download folders are
not used. The disk-full test is intentionally ignored by ordinary Cargo runs.

## Scope

These are real engine transfers through a loopback HTTP tracker and a switchable
TCP relay, with discovery disabled. Every recovered payload is compared with
the complete source byte array. The connection test waits for a tracker request
to fail during the outage and requires automatic recovery without pause/resume
or process restart.

Folder renaming exercises unavailable paths, not physical removable-drive
unplugging. The disk-full test exercises actual HFS+ allocation failures, not
injected errors, but does not certify APFS, quota exhaustion, or sudden power
loss. Network interface loss, routing/DNS changes, physical drive removal, and
sleep/wake remain target-system acceptance work. UI interaction and XPC recovery
are outside this engine test suite.


## Transfer-control follow-up

Force Recheck now has v1/v2/hybrid corruption and repair tests, plus selection
and pause-intent preservation. Active partial v2 and hybrid downloads also
complete with exact bytes after relocation and renaming. Queue and storage
transactions have durable restart and failure coverage. See `FILE-MANAGEMENT.md`.
Physical sleep/wake, unplugging, interface loss, actual logout/login, and further
signing validation were deferred by the user, not passed. See `VALIDATION.md`.
