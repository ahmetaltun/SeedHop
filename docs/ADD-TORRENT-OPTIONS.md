# Add-torrent options

## Acceptance and scope

Each new import starts with three unchecked options: Add to top of queue,
Download in sequential order, and Download first and last pieces first. Choices
belong to the import draft, survive an Add error, and do not become global defaults.
Existing Add paused and Verify and reuse existing files controls retain their behavior.

Normal downloads choose the rarest piece advertised by connected peers, with
randomized ties. Sequential downloads prefer earlier pieces in filename order.
Boundary priority prefers the first and last pieces of each selected file; when
combined with sequential downloading, boundaries precede sequential middle pieces.
Availability and parallel requests mean completion order is not guaranteed.
File priority takes precedence over these preferences. Explicit streaming requests
keep their existing priority. Excluded, complete, and in-flight pieces remain
subject to the existing selection, reservation, and verification rules.

Top-of-queue admission inserts before waiting downloads while preserving current
downloads' slots. With no active-download limit, queue order does not restrict
concurrency. Add paused and schedule suspension still apply. Explicit queue moves
retain their existing ability to change admission order.

## Implementation plan

1. Extend ImportSession and the import sheet with three Boolean choices. Carry
   them through optional XPC fields and one typed FFI/core options record. Keep
   existing Rust add entry points for current callers. Bump the protocol version
   so a stale host cannot silently ignore requested options.
2. Store sequential and boundary preferences in the existing session metadata.
   Missing fields deserialize to false. No separate settings file or new service.
3. Save the new admission order before adding membership. A queue write failure
   must not add a torrent while reporting failure. Normal reconciliation removes
   stale entries if a subsequent Add fails.
4. Maintain per-piece connected-peer availability at bitfield, HAVE, reject, and
   peer-lifetime transitions. Never enumerate other peer locks during acquisition.
   Rank candidates without sorting or allocating a full candidate list.
5. Test mode combinations, rarity, unavailable/excluded/in-flight pieces, file
   priorities, streaming precedence, availability updates, queue admission,
   persistence/default compatibility, failed Add, and native import requests.
6. Build the app, run workspace tests and native UX/interface checks, inspect the
   import-sheet render, and record actual results below.

## Non-goals

No global option persistence, new transfer inspector controls, streaming player,
strict completion-order guarantee, scheduler redesign, new dependency, or unrelated
engine tuning. Existing libraries load with both piece preferences disabled and
therefore use the new normal rarest-first policy.

## Validation

Completed on 2026-10-29. Evidence: [results/add-torrent-options-2026-10-29.json](results/add-torrent-options-2026-10-29.json).

- 139 workspace tests passed across the normal run, the complete 76-test serial
  protocol rerun, and the FFI check. Five existing opt-in environment/performance
  tests were not run. Documentation tests passed.
- Fourteen focused picker tests, one availability-lifecycle test, and one
  persistence/default-compatibility test passed in an isolated library fixture.
  The fixture omits uncached, unused optional/test dependencies; its unrelated
  read-buffer test uses the existing futures executor instead of pollster. The
  production picker, peer-lifecycle, and persistence sources are unchanged in the
  fixture. It is not a claim that the full upstream test suite was run.
- The native app built and passed strict signature verification. Forty-four UX
  checks, native interface checks, and the packaged protocol-v5 XPC smoke test
  passed. The XPC test checks preference persistence after restart and file
  management operations. Light and dark import previews were visually inspected.
- Formatting, Clippy with warnings denied, whitespace checks, and reverse patch
  applicability passed.

The first parallel protocol run exposed a fixture that assumed ascending piece
requests. That fixture now explicitly selects sequential downloading. A separate
metadata-budget timing assertion failed under contention and passed alone and in
all 76 serial protocol tests; its production code was not changed.

Native pointer/key delivery and VoiceOver speech remain manual acceptance checks.
Public-swarm throughput and availability are not guaranteed by these controlled
regressions.
