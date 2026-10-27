# Everyday transfer management roadmap

## Goal and scope

Make SeedHop dependable and understandable during ordinary downloading, file
management, and recovery. Work in ordered, independently verified increments.
Keep the existing native design and engine architecture. Each feature includes
implementation, error behavior, focused regression coverage, and interface review.

RSS, remote control, signing, notarization, and release work are deferred.
Search integrations, media playback, and advanced scheduling are outside this
roadmap. Existing library search remains supported. Protocol expansion is not
required: current v2/hybrid, encryption, and uTP evidence is recorded in
`V2-COMPLETION.md`. Physical lifecycle acceptance remains open independently.

## Non-negotiable behavior

- Preserve torrent identity, selected files, priorities, queue order, and user
  pause intent through file operations, errors, cancellation, and restart.
- Keep originals authoritative until the new layout is durably committed.
- Never overwrite unrelated files or follow symlinks during copying or cleanup.
- Distinguish a committed move with incomplete cleanup from a failed move.
- Keep UI interaction and snapshot polling available during long operations.
- Report implementation, automated evidence, and manual acceptance separately.

## Ordered implementation

### 1. Reject moves that cannot fit

Trace the existing relocation/rename transaction and add destination-capacity
preflight before suspending the transfer or creating target files. Include
present unselected files. Account for full logical file size because the current
copy path does not preserve sparse allocation; conservatively reserve the full
expected length of present partial files. Missing files need no copy allocation.
Apply the same check to local renames, which currently use a copy transaction.

Acceptance:

- Insufficient capacity returns required/available space and a recovery action.
- An early rejection creates no target files and changes no transfer intent.
- Missing sources, unsafe paths, non-regular sources, and arithmetic overflow
  have explicit behavior.
- Focused capacity/path tests and existing management tests pass.

Capacity is advisory, not a reservation. A later disk-full error must still roll
back safely. Do not claim this check guarantees successful copying.

### 2. Show operation progress and allow safe cancellation

Add application-owned operation state with a unique operation identity, target
torrent, phase, copied bytes, and total bytes. Expose it through UniFFI, XPC,
and snapshots. Use bounded chunk copying so cancellation can be observed between
writes. Scope cancellation to the exact operation so a delayed request cannot
cancel a later move. The cancellation command must bypass the mutation lock.

Show preparing, copying, saving, and verifying states in the library/inspector.
Allow cancellation only before the durable layout commit; once saving begins,
explain that the operation is finishing. Keep verification progress distinct
from bytes copied. Disable conflicting actions without blocking navigation.

Acceptance:

- Large operations show live progress while snapshots remain responsive.
- Cancellation removes only this operation's uncommitted targets and restores
  original paths and transfer intent.
- Late or repeated cancellation cannot affect another operation.
- Selection changes and closing the inspector do not retarget the operation.
- Rust state/copy tests, Swift delayed-reply tests, and packaged XPC checks pass.

### 3. Recover interrupted transactions

Persist a minimal transaction journal before copying. Record original/new layout
and exact files owned by the transaction; treat the session's committed layout
as authoritative. On restart, reconcile the journal with that layout before
resuming writes. Retain ambiguous files and offer a clear recovery action.
Never delete an existing path merely because its name matches a journal entry;
cleanup must establish transaction ownership and unchanged file identity.

Acceptance:

- Process interruption before commit preserves the old layout and bytes.
- Interruption after commit preserves the new layout and bytes.
- Retrying a destination explains retained copies rather than silently failing.
- Cleanup errors identify which layout is active and what remains.
- Deterministic interruption tests pass at copy, save, and cleanup boundaries.
- Cross-volume checks compare exact bytes and preserved intent. Actual removable
  drive loss remains a separately recorded physical acceptance gate.

### 4. Make transfer states and recovery actions useful

Audit actual engine states and their native presentation. Distinguish queued,
finding peers, downloading, verifying, paused, and storage failure. Show queue
position when relevant. Add retry/reveal/change-location actions only where the
engine can perform them safely. Preserve choices and captured action targets.
Avoid inferring tracker failure solely from zero download speed.

Acceptance: each state has accurate copy, applicable actions, stable selection,
and deterministic delayed/failing command coverage. Review light/dark, compact,
empty, busy, and recovery screens.

### 5. Bring settings and connection details up to date

Remove stale claims that encrypted peers are unimplemented. Expose supported
encryption policies with plain-language explanations and durable preferences.
First determine whether changes can apply live or require controlled engine
restart; show that behavior explicitly and preserve run intent. Reflect actual
TCP/uTP capability without claiming reachability from an assigned listen port.
Do not add advanced connection switches without a concrete supported use case.

Acceptance: settings round-trip through restart, rejected saves remain retryable,
and the selected policy demonstrably reaches the engine. Existing transport and
encryption regressions remain applicable; rerun affected cases only.

### 6. Complete native interaction and accessibility

Audit import, file selection, progress, cancellation, rename, settings, and
recovery. Add explicit error announcements where missing. Verify focus entry and
return for sheets, visible focus, keyboard access, and accessible names/values.
Use existing UX/interface harnesses for regressions and rendered fixtures for
visual checks. Record a separate manual pointer, physical keyboard, and VoiceOver
walkthrough; source labels and menu callbacks do not certify spoken output.

Acceptance: automated presentation/navigation checks pass; manual observations
record exact failures or passing behavior without claiming unperformed checks.

### 7. Measure and improve large-library performance

Create clearly labeled fixtures for 100 and 1,000 transfers and a torrent with
10,000 files. Measure snapshot cost, UI update time, scrolling, filtering, and
selection stability. Profile before changing data flow. Optimize the measured
bottleneck, preserving stable identities and avoiding unnecessary row updates.
Choose an explicit response-time budget from baseline measurements before edits.

Acceptance: retain before/after measurements, demonstrate responsive interactions
during updates, and verify no selection or scroll regression. Fixture performance
does not replace real-library acceptance.

## Working method and checkpoints

Finish each numbered step before starting dependent work. Record changed behavior,
commands/results, and remaining limits below. Run focused Rust tests for storage
changes; build and run UX/interface/XPC checks when their layers change. Run
formatting and Clippy for Rust edits. Do not repeat unrelated protocol matrices
without an affected path or a new failure.

After step 7, review normal daily use against these criteria before expanding
features. Deferred release and physical lifecycle gates remain separately tracked
in `RELEASE-VALIDATION.md`.

## Execution record

- Plan created on 2026-10-26.
- **Step 1 complete (2026-10-26):** relocation and copy-based renaming now check
  destination capacity before quiescing peers or creating targets. Checks count
  present unselected files, sparse logical lengths, and expected lengths of
  present partial files. Unsafe/non-regular sources and size overflow fail early.
  Low-space errors include required/available bytes and a recovery action.
- **Evidence:** `cargo test -p seedhop-core --lib storage::tests --locked`
  passed all five tests. The engine-level sparse-source rejection test covers
  both move and rename, unchanged original bytes and transfer record, and
  restored identity/path/selection/priority/pause intent/queue position.
  `cargo test -p seedhop-core --test management --locked` passed both tests.
  Both targeted v2 tests passed:
  `v2_and_hybrid_layout_changes_keep_identity_bytes_and_running_intent` and
  `partial_v2_and_hybrid_downloads_continue_after_relocation_and_rename`.
  `cargo fmt --all -- --check`, workspace/all-target Clippy with warnings denied,
  and `git diff --check` passed. Engine tests required local listener access
  outside the filesystem/network sandbox; the initial sandbox listener failures
  were environmental, and the reruns passed.
- **Limits:** capacity is not reserved; mid-copy allocation failure still uses
  existing rollback. Progress, cancellation, transaction journaling, manual native
  interaction, and physical drive acceptance are not completed by this increment.
- **Step 2 complete (2026-10-26):** moves and copy-based renames publish operation
  IDs, phases, and actual copied/total bytes through UniFFI and XPC snapshots.
  The library shows progress independently of selection and inspector visibility.
  Copying uses chunks of at most 1 MiB. Cancellation bypasses the mutation lock,
  cleans uncommitted copies, and restores original transfer intent. Entering
  saving atomically rejects further cancellation; stale IDs cannot affect retries.
  Conflicting transfer commands/imports/host switches are guarded. Cancellation
  is a typed result, while cleanup/resume failures remain visible errors.
- **Step 2 evidence:** seven core unit tests and three management tests pass.
  The new v2/hybrid cancellation test proves cleanup of both completed unselected
  copies and the partial selected-file copy, unchanged source bytes, and restart
  preservation. Existing v2/hybrid layout and partial-download completion tests
  also pass. The final development app passes 19 controlled UX regressions,
  three native menu/focus checks, and the packaged XPC import/management/restart
  suite, including observed copy progress and cancellation for both moves and
  renames. All 32 design fixtures render; progress and saving states were reviewed
  in light/dark and compact/expanded layouts. Formatting, workspace/all-target
  Clippy with warnings denied, and diff checks pass. The build uses local ad-hoc
  signing only. Retained results and source/binary digests are in
  `results/file-progress-2026-10-26.json`.
- **Validation adjustment:** the first packaged cancellation fixture completed
  too quickly for a reliable XPC cancellation round trip. Its disposable sparse
  extension was increased from 128 MiB to 512 MiB; both cancellation cases then
  passed, including on the final rebuilt app. This changes the test window, not
  the production cancellation boundary. macOS icon tooling and local engine
  listeners required execution outside the sandbox.
- **Remaining limits:** an in-flight operating-system I/O/flush must return
  before cancellation can finish. Operation state is transient; process-crash
  journaling and recovery remain step 3. Physical drive/network/lifecycle checks
  and manual keyboard/VoiceOver acceptance remain open. No release or certificate
  acceptance is claimed by the development build.
- **Step 3 complete (2026-10-27):** moves/renames now write a versioned durable
  journal before target bytes. It records old/new layouts, directory/file
  identities, and completed SHA-256 copy receipts. Recovery runs under the lease
  before session restoration; the saved session chooses the active layout.
  Verified cleanup is restartable through recorded quarantine names. Uncertain,
  edited, replaced, or unavailable files remain intact. Retry Cleanup, Keep Extra
  Copies, and folder inspection cross UniFFI/XPC into the native library.
  Unknown authority/future records cannot be silently dismissed. Legacy filename
  maps are persisted before journaling; existing session schema remains unchanged.
- **Step 3 evidence:** the final 64-case SIGKILL matrix passes for v1/v2/hybrid
  moves and renames at journal, creation, copying, completion, saving, commit,
  quarantine, and cleanup boundaries. It also covers paused/running/queued intent,
  legacy maps, edited copies, repeated restoration, refusal to create files in
  a replacement active folder, and two exact-byte
  cross-volume cases on a disposable HFS+ image. The image was detached/deleted.
  Fourteen core unit tests, three management tests, targeted v2/hybrid
  cancellation/layout/partial-download tests, and legacy filename tests pass.
  Native evidence includes 21 presentation regressions, three menu/focus checks,
  real XPC recovery actions and restart, and 36 rendered fixtures. Recovery cards
  were visually reviewed in compact light and expanded dark layouts. Formatting,
  all-feature/all-target workspace Clippy with warnings denied, and diff checks
  pass. See `STORAGE-RECOVERY.md` and
  `results/storage-recovery-2026-10-27.json`.
- **Step 3 limits:** controlled SIGKILL and mounted-image checks do not certify
  physical drive loss or power failure. Recovery adds hashing reads and may retain
  extra copies after external edits. Missing active/default destinations still
  require reconnection under existing startup policy. Manual keyboard/VoiceOver,
  physical lifecycle, signing/notarization, and release remain open/deferred.
- **Step 4 complete (2026-10-27):** shared transfer presentation distinguishes
  finding peers, idle connected peers, queued positions, checked/copied/downloaded
  bytes, typed file errors, and stale disconnected snapshots. Retained-copy
  cleanup appears in attention filters, including an orphan notice without rows.
  Retry rechecks errors while preserving pause intent, identity, choices, and
  queue admission; stale Retry is harmless. Safe folder inspection and applicable
  move actions are exposed in error details. See `TRANSFER-STATUS.md`.
- **Step 4 evidence:** fifteen core unit tests, three management tests, one healthy
  stale-Retry test, three controlled recovery tests, and two real mounted-image
  ENOSPC tests pass. The new Retry case covers paused/running intent, high file
  priority, missing destinations, queue admission, restart, and exact payload bytes.
  Twenty-seven Swift regressions, packaged XPC, three native focus/menu checks,
  all-target/all-feature Clippy, formatting, and the ad-hoc build pass. Forty-six
  native fixtures render, with representative new screens visually reviewed.
  Portable evidence is in `results/transfer-status-2026-10-27.json`.
- **Step 4 limits:** real new Retry faults use a disposable HFS+ image; typed
  permission/device classification has unit coverage. Physical lifecycle,
  public-swarm acceptance, native key-event delivery, and VoiceOver remain open.
- **Step 5 complete (2026-10-27):** settings expose Disabled, Allow, Prefer, and
  Require peer-encryption policies with durable engine-owned preferences. Active
  and saved policies are distinct; an explicit engine restart applies a pending
  choice without changing transfer pause intent, queue/file choices, host mode,
  or the app's login-resume preference. Failed saves/restarts remain retryable.
  TCP/uTP, v1/v2/hybrid, disconnected ports, and reachability limits now have
  accurate descriptions. See `CONNECTION-SETTINGS.md`.
- **Step 5 evidence:** four settings tests cover all policies across restart and
  real listener enforcement, alongside five existing MSE tests, fifteen core unit
  tests, and three management tests. Thirty-two Swift presentation regressions,
  packaged XPC save failure/retry and policy activation, three native focus/menu
  checks, Clippy, formatting, and an ad-hoc build pass. Fifty-two fixtures render;
  representative pending/error/offline settings were visually reviewed. Evidence
  is in `results/connection-settings-2026-10-27.json`.
- **Step 5 validation adjustment:** the packaged cancellation fixture exhausted
  its fixed poll count during preparation. Its observer now uses an elapsed-time
  window with paced polling, and both move and rename cancellation pass. No
  production protocol or file-operation behavior changed for this adjustment.
- **Step 5 limits:** the new live policy check uses TCP loopback. Background
  restart uses the existing ownership handoff with controlled UI coverage;
  physical login/lifecycle, public swarms, native key-event delivery, and
  VoiceOver remain separate gates.
- **Step 6 implementation and automated portion complete (2026-10-27):** errors
  explicitly request accessibility announcements; file phases use medium priority
  without per-byte chatter, while errors and recovery use high priority. Import
  uses a native file table with captured-row Space toggling and explicit focus.
  Import closure and native rename cancellation restore transfer-table focus.
  File/priority/rename controls have contextual accessible names and values;
  bandwidth validation identifies and requests focus on the invalid field.
- **Step 6 evidence:** thirty-four controlled Swift regressions, nine native
  menu/focus/announcement-request checks, packaged XPC, and the ad-hoc development
  build pass. Fifty-eight fixtures render, including high-contrast import,
  settings, and recovery in both appearances. Representative new screens were
  visually inspected. Rust/manifest source digests match step 5, so its engine
  tests remain applicable without repeating unrelated protocol matrices.
  See `results/accessibility-2026-10-27.json`.
- **Step 6 validation boundary:** native sheet tests now use the actual library
  scene with controlled transport and wait for metadata-table readiness rather
  than assuming a fixed delay. Tests observe actual native responders and
  announcement requests, not VoiceOver speech. Full Keyboard Access, Reduce
  Motion, and Increase Contrast were read as off; no system preference changed.
- **Step 6 manual acceptance remains open:** physical pointer/key delivery,
  VoiceOver speech/navigation, real OS accessibility settings, and live scrolling
  have not been performed. `ACCESSIBILITY-ACCEPTANCE.md` records the exact
  walkthroughs with every unperformed check marked Not run. Do not call full
  step 6 acceptance complete until real observations are recorded.
- **Step 7 measurement and targeted optimization implemented (2026-10-27):**
  controlled fixtures cover 100/1,000 transfers and one 10,000-file torrent in the
  real engine and native library scene. Selected-file membership, inspector
  counts, unchanged policy publications, and repeated derived-store scans were
  optimized. The 10,000-file engine snapshot improved from 367.98 to 9.18 ms p95.
  Final native 1,000-row changing-layout p95 spans 84.60–94.03 ms, compared with
  147.29 ms before; unchanged snapshots emit zero publications instead of two.
- **Step 7 evidence:** 25 executed Rust regressions, 36 controlled Swift
  regressions, nine native checks, packaged XPC, 58 rendered fixtures, Clippy,
  formatting, and the ad-hoc build pass. Membership/selection, priority, pause
  intent, restart, derived-cache invalidation, row order, and scroll stability
  retain coverage. See `PERFORMANCE.md` and
  `results/performance-2026-10-27.json` for raw samples and artifact digests.
- **Step 7 budget acceptance remains open:** one final-binary run passes all
  37 checks; two other retained runs miss the 100-row changing-layout target,
  with one also missing its scroll target. Targets were not relaxed. AppKit's
  table-delegate reentrancy warning appears in baseline and final stress runs;
  its cause remains unresolved. Concurrent host load was observed but does not
  establish the cause of latency spikes. Native layout is not physical frame
  presentation or real-library performance certification.
- **Next acceptance work:** trace the native table warning and 100-row latency
  spikes under recorded stable load, then execute the step 6 manual matrix and
  real-library scrolling checks. Full step 6 and step 7 acceptance remain open.
  RSS, remote control, distribution signing, notarization, and release remain deferred.
