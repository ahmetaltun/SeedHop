# Development completion — 2026-10-29

The requested development scope is implemented: weekly scheduling is included in
this release, and RSS and remote control are cancelled. The existing engine,
queue, settings, and authenticated native XPC architecture remain in use.
Distribution signing and physical lifecycle tests are excluded by the user's
scope decision. This record does not certify a public release.

## Delivered behavior

- Up to 32 weekly windows apply download/upload caps or suspend downloads and
  seeding. Windows support selected weekdays, overnight/week-wrap boundaries,
  and 24-hour durations. Overlapping windows are rejected.
- The engine persists scheduling atomically, applies current local time through
  the existing timer, restores normal caps outside windows, and preserves manual
  pause intent, selections, priorities, and queue order. It retries failed queue
  application and exposes a warning. It does not wake or launch the Mac.
- Settings keeps edits local until Save. Failed saves retain the draft. The UI
  distinguishes normal/effective caps and scheduled suspension. Protocol version
  4 carries these settings through typed UniFFI and authenticated XPC.
- The native transfer grid fixes blank names, row-height reentrancy, and repeated
  filtering budget misses. Fixed native cells retain compact/comfortable density,
  stable selection and captured command targets. Existing widths, visibility,
  sort preferences, and recognized legacy order remain usable. Cached offscreen
  cells receive current values.

See [SCHEDULING.md](SCHEDULING.md),
[POWER-USER-WORKSPACE.md](POWER-USER-WORKSPACE.md), and
[PERFORMANCE.md](PERFORMANCE.md) for behavior and measurement details.

## Verification

- Complete serial Rust workspace suite: 134 passed, zero failed, three ignored.
  The explicitly ignored real-engine performance fixture was run separately.
- All-target/all-feature workspace Clippy with warnings denied and formatting
  checks pass. The final development app builds without compiler warnings.
- All 43 controlled UX regressions, 25 native PASS observations, packaged
  foreground XPC checks, and 153 design renders pass. XPC includes scheduling
  save/failure/retry/restart and restoration of normal limits.
- Three fresh final-binary UI performance processes meet every original UI
  latency/stability budget, with zero table reentrancy warnings in every run.
  The fresh real-engine benchmark also passes; each combined evaluation passes
  all 43 applicable checks. Snapshot p95 is 1.11/15.00/11.93 ms for the
  100-transfer/1,000-transfer/10,000-file fixtures. No budget or sample count
  was relaxed.

One earlier default parallel Rust run failed a direct librqbit metadata active
peer-cap assertion (five observed against four). Its isolated rerun and the final
complete serial suite passed. The failing log remains retained; this record does
not claim the parallel invocation passed.

Portable results retain raw final performance samples, budget decisions,
source/binary digests, command/log records, and historical exploratory samples in
[development-2026-10-29.json](results/development-2026-10-29.json). Build products
and full local logs remain under `.build`. Changes are local and uncommitted.

## Remaining acceptance boundaries

Physical keyboard/pointer delivery, VoiceOver speech/navigation, actual OS
accessibility settings, representative-user tasks, and real-library physical
scrolling/frame presentation have not been certified by these automated checks.
The existing [manual workspace protocol](WORKSPACE-USABILITY.md) and
[accessibility matrix](ACCESSIBILITY-ACCEPTANCE.md) remain available for that
separate acceptance work. No signing, notarization, or physical lifecycle gate
was closed by this development pass.
