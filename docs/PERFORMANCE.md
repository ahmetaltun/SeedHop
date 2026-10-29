# Controlled large-library performance

## Table text correction — 2026-10-29

The fixed-height mitigation disabled SwiftUI's automatic cell measurement and
left text blank when the first transfer arrived in an empty library. Removing
that override restores text while retaining 28/40-point density controls.
The workspace regression now checks rendered name pixels after first arrival
and a live update; native interface and workspace checks pass.

**UI-PERF-001 is reopened:** the controlled large-library rerun emitted 11 native
table reentrancy warnings. Selection and scroll anchors remained stable. The
zero-warning results below describe the earlier binary, not this correction.

## Workspace follow-up — 2026-10-28

Current evidence is retained in
[power-user-workspace-2026-10-28.json](results/power-user-workspace-2026-10-28.json).
The earlier step-7 measurements below remain historical evidence.

The workspace retains all original latency budgets and adds 1,000-peer and
100-tracker fixtures. Final runs 31–33 use one debug binary. They report zero
native table reentrancy warnings, zero publications for unchanged library
snapshots, stable native selection, and stable scroll anchors. The compact
workspace shows 16 full 28-point rows at 1280×800; comfortable rows have a
40-point pitch. Native pane and column persistence checks pass.

- **UI-PERF-001:** resolved in these controlled stress runs. The transfer table
  uses fixed native heights aligned with SwiftUI's minimum row height, avoiding
  the estimated-height cache reentrancy. Physical input remains a separate gate.
- **UI-PERF-002:** the earlier changing-layout misses do not recur in these runs.
  The 100-row changing-layout p95 values are 53.57, 50.75, and 51.85 ms, against
  the unchanged 100 ms budget.
- **UI-PERF-003:** remains open. Focused 100-row filtering has p95 values of
  99.80, 103.31, and 101.88 ms against 100 ms. Run 31 passes all 43 applicable
  checks; runs 32 and 33 each miss this one check. Do not claim overall
  performance acceptance from the passing run alone.

The filter measurement begins with the native search field focused and the
restored table/selection settled. Toolbar expansion and restoration of the
previous query are setup, outside the filter timer. The timer still includes
model mutation, native row changes, and layout flushes. Every earlier failed or
incomplete run is retained with its methodology notes; no budget was relaxed.
Native-highlight assertions were added after they exposed a binding issue across
hosting boundaries. The final table restores highlights by torrent identity
without letting restoration change the model's captured command targets.

Split panes accept their proposed workspace size instead of recursively measuring
hosted table constraints. Transfer samples no longer force split relayout.
Filter matching is reused until inputs change, and filtering plus hidden-selection
cleanup publishes one mutation. Native widths remain fixed, with horizontal
scrolling when columns exceed the viewport. These changes retain the native
command, focus, file-control, and recovery regressions.

Physical scrolling, frame presentation, large diagnostic XPC transport latency,
VoiceOver, actual accessibility settings, and representative-user tasks remain
separate acceptance gates. See [WORKSPACE-USABILITY.md](WORKSPACE-USABILITY.md).

Roadmap step 7 measures debug-build engine snapshots and the real native library
scene with clearly labeled sample data. It does not certify public swarms,
physical scrolling, compositor frame rate, or a user's real library.

## Fixtures and measurement boundaries

- Real paused engine libraries contain 100 or 1,000 single-file torrents; a
  separate real torrent contains 10,000 one-byte files. Discovery is disabled,
  state/downloads are temporary, and setup time is recorded separately.
- Swift fixtures contain 100/1,000 transfer rows or one 10,000-file transfer.
  Controlled transport supplies fresh decoded snapshots to the real library
  scene without opening a live engine or modifying a real library.
- Samples separate engine snapshot/encoding, Swift JSON encoding/decoding,
  model reads, unchanged/changing snapshot layout, programmatic native scrolling,
  and filtering until native rows appear. Two warmups precede retained samples.
- Native layout measurements include a 1 ms cooperative yield and explicit
  AppKit layout flushes. They are wall time through layout, not display-present
  timing. Fixture mutations yield after native row readiness before another
  selection change. AppKit still logs the delegate warning documented below.
- Rate updates retain selected torrent identity and scroll offsets. Explicit
  filters must clear hidden selection. Model row toggles and real transfer
  safety remain covered by the existing UX/engine regressions.

## Budgets chosen before production edits

The initial engine baseline measured approximately 368 ms p95 for a 10,000-file
snapshot and 10.3 ms for 1,000 transfers. Initial native p95 changing layouts were
approximately 60/175/193 ms for 100/1,000/10,000-file cases. Unchanged snapshots
also caused two unnecessary publication events each.

| Controlled debug measurement | p95 target |
|---|---:|
| Engine snapshot, 100 transfers | 5 ms |
| Engine snapshot, 1,000 transfers | 20 ms |
| Engine snapshot, 10,000 files | 25 ms |
| Engine JSON encoding, any fixture | 100 ms |
| Swift JSON encode or decode, any fixture | 50 ms |
| Combined model reads, any fixture | 25 ms |
| Unchanged snapshot through native layout | 20 ms |
| Changing snapshot, 100 transfers | 100 ms |
| Changing snapshot, 1,000 transfers or 10,000 files | 125 ms |
| Programmatic scroll through native layout | 50 ms |
| Filter until native rows, 100 transfers | 100 ms |
| Filter until native rows, 1,000 transfers | 125 ms |

Every unchanged snapshot should produce zero observable-store publications.
Selection, scroll anchors during updates, and hidden-selection clearing must
remain correct. These targets bound the controlled workload; they do not promise
60 fps or an equivalent result on other hardware or optimized release builds.

## Results — 2026-10-27

Measurement and targeted optimization are implemented. **Budget acceptance is
open:** the final binary passed all 37 applicable checks in one run, while two
other retained runs missed latency limits. No target was relaxed and no failed
sample was discarded. This is not a fully accepted performance milestone.

The host was a MacBookPro18,3 with 32 GiB memory and macOS 15.8.1 (24H32).
Measurements use debug builds, two warmups, and 15 retained samples for engine,
JSON, model-read, and unchanged-layout measurements. Changing-layout, scroll,
and filter measurements retain nine samples. Nearest-rank p95 is therefore the
maximum retained sample in these small runs; the raw samples remain in evidence.

| Engine p95 | Before | After | Budget |
|---|---:|---:|---:|
| Snapshot, 100 transfers | 0.75 ms | 0.69 ms | 5 ms |
| Snapshot, 1,000 transfers | 10.34 ms | 12.84 ms | 20 ms |
| Snapshot, 10,000 files | 367.98 ms | 9.18 ms | 25 ms |

The 10,000-file snapshot improved approximately 40 times. The 1,000-transfer
snapshot increased by 2.50 ms, remaining within budget; not every measurement
improved. Engine JSON payload sizes match before and after for every fixture:
46,304, 461,205, and 919,572 bytes. Encoding p95 remains below 100 ms.

The native comparison uses the retained paced baseline, rather than the earlier
exploratory unpaced run quoted above. The after column spans all three runs of
the final binary, including failures.

| Native p95 | Before | After range | Budget |
|---|---:|---:|---:|
| Changing layout, 100 transfers | 60.94 ms | 57.72–178.88 ms | 100 ms |
| Changing layout, 1,000 transfers | 147.29 ms | 84.60–94.03 ms | 125 ms |
| Changing layout, 10,000 files | 174.84 ms | 79.68–98.50 ms | 125 ms |
| Unchanged layout, 100 transfers | 58.39 ms | 2.25–11.62 ms | 20 ms |
| Unchanged layout, 1,000 transfers | 136.30 ms | 3.33–16.52 ms | 20 ms |
| Unchanged layout, 10,000 files | 99.07 ms | 2.98–3.00 ms | 20 ms |
| Model reads, 1,000 transfers | 18.27 ms | 2.06–2.31 ms | 25 ms |
| Filter, 1,000 transfers | 133.04 ms | 71.69–97.85 ms | 125 ms |
| Programmatic scroll, 100 transfers | 12.16 ms | 17.15–50.82 ms | 50 ms |

Identical snapshots now produce zero publications instead of two. Every retained
run preserved selected identity and scroll anchors during updates. Both applicable
filter cases cleared hidden selection; filtering is not exercised for the
single-torrent file fixture. Swift JSON and remaining scroll/filter measurements
meet their budgets in all three final-binary runs.

### Changes supported by measurements

- The engine builds a selected-file membership set once per snapshot instead of
  repeatedly scanning a vector for each file. File order, priority, choices,
  pause intent, identity, and restart behavior retain regression coverage.
- The file inspector computes selected-file count once per body evaluation,
  instead of once per visible row. Last-selected-file protection remains intact.
- The store publishes encryption-policy changes only when values differ.
- A sustained 1,000-row diagnostic stack sample identified repeated selected-row,
  filter-count, and activity-summary scans. Derived indices/counts now rebuild
  when rows, file operations, or recovery context change. Sparse selection reads
  current rows in snapshot order; large selections and duplicate IDs retain the
  original scan. Empty-query All avoids a redundant filter pass.

The diagnostic sample predates the derived-index optimization. Its inclusive
stacks overlap and are not independent CPU percentages. No formatter cache or
new persistence layer was added. Queue-position scanning remains a diagnostic
measurement because the real snapshot already meets its budget.

### Historical open findings and next acceptance work — 2026-10-27

- **UI-PERF-001:** baseline and every final native stress run emit 11 occurrences
  of `WARNING: Application performed a reentrant operation in its NSTableView delegate. This warning will become an assert in the future.`
  Selection and scroll assertions pass, but the exact callback/phase causing the
  warning is not established. Reproduce and trace this separately before accepting
  native interaction stability; its presence before optimization does not close it.
- **UI-PERF-002:** 100-row changing layout exceeded 100 ms in two of three final
  runs (140.18 and 178.88 ms); one also exceeded the 50 ms scroll budget at
  50.82 ms. A later host-load snapshot showed another app renderer at 132.8% CPU
  and WindowServer at 55.8%. This establishes concurrent load, not the cause of
  earlier spikes. Do not stop other user applications automatically. Repeat under
  recorded stable load and profile the slow phase if the miss persists. Keep all
  runs and the original targets when deciding acceptance.
- Physical pointer/keyboard/VoiceOver, actual OS accessibility settings, real
  library scrolling/reopening, XPC transport latency, frame presentation, and
  public-swarm performance remain unverified. The manual matrix remains open in
  `ACCESSIBILITY-ACCEPTANCE.md`.

## Regression proof and reproduction

Twenty-five executed Rust regressions pass: 15 core unit, three management,
four settings, one healthy stale-Retry, one selected-file membership/restart,
and one partial v2/hybrid relocation/rename test. The normal performance-test
invocation skips the explicitly ignored large benchmark. Disk-full tests were
not repeated for these changes.

All 36 controlled Swift regressions, nine native focus/menu/announcement checks,
packaged foreground XPC regression, workspace all-target/all-feature Clippy with
warnings denied, formatting, and the ad-hoc development build pass. The renderer
produces 58 fixtures; a representative high-contrast recovery screen was visually
checked after the change. These checks do not close manual acceptance.

Run `make performance` from the repository root. It builds with ad-hoc signing,
creates temporary real-engine fixtures, opens the labeled native sample library,
writes `.build/performance-*-current.json`, and applies the fixed budgets through
`scripts/check_performance.py`. Allow approximately two minutes for engine fixture
setup; setup is excluded from snapshot latency. Run native UI checks sequentially
with other app-launching tests. The checker exits nonzero for any missed target
and rejects diagnostic `--profile-performance` output.

Portable raw before/after/repeat measurements, all budget results, and
source/binary/log digests are retained in
`results/performance-2026-10-27.json`. Diagnostic samples and local logs are hashed
there; their full files remain under `.build`. RSS, remote control, distribution
signing, notarization, and release remain deferred.
