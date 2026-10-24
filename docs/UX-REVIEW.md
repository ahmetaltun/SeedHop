# Milestone 0 UX review follow-up

This pass reviews source, executable presentation behavior, real foreground-XPC
import behavior, and rendered app-owned views. It does not automate system input
or certify a VoiceOver walkthrough. The controlled transport is used only by
the development self-test, never by a normal app session.

## Findings and fixes

| Trigger | Previous behavior | Resulting behavior |
|---|---|---|
| Add a second torrent with the main Add button | The previous source could reopen | Every new import starts with clean input; an already-open draft is preserved |
| Invoke Pause, then change selection | An asynchronous task could pick up later targets | Action targets are captured immediately |
| Confirm removal after selection changes | Removal could affect the new selection | Confirmation retains the original targets and explicit keep/delete choice |
| Select several transfers | Inspector showed one item but operated on all | Inspector shows aggregate totals and explicit group actions |
| Change a filter or search | Invisible rows could remain selected | Selection is restricted to the new visible results; live snapshot updates still retain identity |
| Add while filtering a different state/name | Successful import could remain invisible | The library shows all transfers and selects the added item |
| Rapidly change file checkboxes | Concurrent writes could overwrite a newer choice | Saving feedback disables overlapping changes until the snapshot confirms the result |
| Cancel metadata lookup, then retry | Late results or errors could update the sheet | Every attempt has its own cancellation identity; obsolete replies are discarded |
| Dismiss while an add is committing | The sheet could disappear before confirmation | Cancel, input editing, and interactive dismissal are disabled until the add completes |
| Retry after replacing an oversized torrent file | Cached URL size could reject the replacement | An off-main reader checks the open file and enforces a bounded read |
| An operation fails while the engine is connected | Banner always suggested reconnecting | Operation errors are dismissible; connection recovery offers Retry only when disconnected |
| Navigate to a filter with no matching state | Search-empty copy did not fit | The filter explains its state and offers Show all transfers |

Keyboard search is implemented through the focused library scene: Command-F
focuses the search field; Escape clears it and then releases focus. URL drops
accept magnet URLs as well as torrent file URLs, and highlight the drop target.

## Automated checks

Executed on the development build: all 13 Swift regression scenarios passed;
the real foreground-XPC import/restart smoke check passed; the app and both
hosts built with verified ad-hoc signatures; 18 fixture screens rendered.
The revised multi-selection, filter-empty, import, and recovery layouts were
visually reviewed in light and dark appearances.

`make ux-test` runs 13 named Swift regression scenarios with controlled, delayed
replies. They cover selection changes during commands, removal confirmation,
overlapping file writes, cancelled and stale metadata, duplicate adds, retained
choices after failure, invalid links, and bounded file-read retries. Temporary
files are isolated and removed; no real transfers or background registrations
are created by these tests.

`make smoke` uses the same `ImportSession` as the UI with real torrent bytes and
the signed foreground XPC helper, then checks paused addition, persisted limits,
restart/restoration, removal retaining files, and shutdown.

`make render` includes light/dark multi-selection, empty-filter, and disconnected
recovery fixtures alongside the original library, compact, empty, import,
file-selection, and settings screens. These are design fixtures, not claims
about live transfer activity.

## Live acceptance walkthrough still required

1. Launch normally, add a magnet and a torrent file, and verify that cancelling
   metadata leaves no unwanted transfer. Try a broken file, then replace it and retry.
2. Complete file selection, destination selection, paused addition, start,
   pause/resume, inspection, and removal using pointer and keyboard separately.
3. Select mixed paused/running transfers. Confirm group action labels, status
   feedback, filter changes, and correct removal targets.
4. Run Command-F, type a search, clear with Escape, navigate rows, open the
   inspector, and reach file checkboxes without a pointer.
5. Use VoiceOver to check table row descriptions, search, progress, selection,
   notices, modal focus, cancellation, and error recovery.
6. Review light/dark appearances, Increase Contrast, Reduce Motion, compact and
   expanded layouts, and large-library scrolling under actual live updates.
7. Close and reopen the last window while a transfer runs; ensure the engine
   continues and the library reconnects without losing selection or scroll.

The environment exposes no native UI automation surface. App-owned rendering
and model assertions cannot substitute for these input and accessibility checks.
