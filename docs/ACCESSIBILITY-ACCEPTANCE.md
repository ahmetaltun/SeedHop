# Native interaction and accessibility acceptance

This record separates app-owned automated checks from hands-on acceptance.
A native first-responder assertion, requested announcement, source label, or
rendered appearance does not prove physical key delivery or VoiceOver speech.

## Implemented behavior

- Errors in inline notices, transfer failure guidance, retained-file recovery, and
  bandwidth validation explicitly request AppKit accessibility announcements.
  Identical updates are suppressed within each mounted notice. Clearing and later
  repeating an error allows a new announcement. Requests target the active app's
  visible key window; offscreen design renders disable them.
- File-change phases use medium-priority announcements. Copied-byte updates do
  not trigger announcements; current bytes remain available as a progress value.
  Errors use high priority. Assistive technology decides whether/how to speak.
- Import initially focuses the magnet input. Resolved metadata focuses a
  native file table. Import closure returns focus to the transfer
  table. File/folder chooser callbacks request focus return to their invoking control.
- Import and inspector checkboxes name their file. Priority menus expose the
  current priority. Rename buttons name their file; native rename sheets focus
  the captured filename and restore the previous native responder when possible.
- Bandwidth Return moves from Download to Upload, then submits. Invalid values
  identify the failing field, return focus there, and request an announcement.
  Buttons and file choices remain disabled during conflicting/busy actions.
- Transfer-name accessibility uses one explicit name/state/progress summary,
  avoiding duplicate descendant progress values. Notice action buttons remain
  separate accessibility elements. Text/status labels supplement color.

## Automated evidence

See `results/accessibility-2026-10-27.json` for final commands, counts, source,
binary, render, and log digests. Native fixtures use controlled transport and
app-owned windows/menu/control callbacks. Announcement requests are observed in
the development fixture instead of interrupting assistive speech.

The final automated run passed 34 controlled presentation regressions, nine
native checks, packaged XPC, and the ad-hoc development build. All 58 fixtures
rendered; representative high-contrast import, settings, and recovery screens
were visually inspected in light/dark appearances. The native checks ran with
Full Keyboard Access, Reduce Motion, and Increase Contrast reported off, without
changing those preferences.

The metadata fixture uses the real library scene with a controlled transport and
waits for the native file table to appear before asserting focus. A detached
hosting-window check and a fixed metadata delay were insufficient proof. The
final assertions inspect actual native first responders; no proxy accessibility
tree is presented as VoiceOver evidence.

Rust/manifest source digests match the verified step 5 evidence. Engine tests
were not repeated for these Swift-only changes; packaged XPC still checks import,
file choices, cancellation, settings persistence, restart, and exact bytes.

## Performance follow-up findings — 2026-10-27

The subsequent performance pass has 36 controlled Swift regressions and nine
native checks passing. It does not perform any manual row below. Controlled
large-library native stress fixtures log AppKit's table-delegate reentrancy
warning in baseline and optimized runs (UI-PERF-001). Selection/scroll assertions
pass, but the callback causing the warning remains unresolved. Latency repeats
also leave budget acceptance open (UI-PERF-002). See `PERFORMANCE.md` and
`results/performance-2026-10-27.json` before accepting interaction stability.

## Manual acceptance matrix

The Files inspector redesign adds controlled native evidence for outline
selection, expansion, Space dispatch, checkbox callbacks, search ancestors,
context-menu Rename focus restoration, and last-included-file readability.
See `FILES-INSPECTOR-REDESIGN.md` and
`results/files-inspector-2026-10-27.json`. These checks do not close the manual
rows below; VoiceOver reading order and speech still require a live walkthrough.

Status on 2026-10-27: **not run** for every row below. No physical keyboard,
pointer, VoiceOver speech, or OS accessibility preference was changed or certified
by this implementation pass. Record observer, machine/OS, build digest, date,
actual behavior, and exact failures when executing these checks.

| Area | Walkthrough and expected behavior | Status / observation |
|---|---|---|
| Library keyboard | Command-F reaches Search; Return reaches the table; Command-L returns to the table; arrows and selection work during snapshot updates | Not run |
| Window lifecycle | Close the library while real transfers run; Command-0 reopens it without changing selection or transfer intent | Not run |
| Import entry/cancel | Command-N opens with Magnet link focused; Tab order reaches file chooser and Cancel; Escape closes and returns focus to the table | Not run |
| Import metadata | Open a safe torrent fixture; focus reaches Files to download; arrow-row navigation, Space toggling, Tab/Shift-Tab, file toggles, Select/Deselect all, destination, and Add paused work | Not run |
| Import errors | Invalid magnet and failed metadata read announce the error once; Retry remains reachable; a later recurrence is announced again | Not run |
| Busy import | During lookup, Cancel works; during add, closing/cancel/input are blocked until the add commits | Not run |
| File controls | Outline hierarchy, download inclusion versus row selection, mixed folders, scoped search, shared priority/actions, and Rename work with keyboard and VoiceOver; at least one file stays included | Not run |
| Rename | Name is focused/selected; Tab reaches Rename and Cancel; Escape restores prior focus; invalid name is announced; a selection change does not retarget the action | Not run |
| Move and cancellation | Copied bytes and phase are readable; phase changes announce without per-byte chatter; Cancel targets the current operation; saving is not cancellable | Not run |
| Recovery | Active folder and retained-copy message are understandable; Retry Cleanup, Keep Extra Copies, and folder actions are reachable; failure remains announced and retryable | Not run |
| Transfer failure | Typed error guidance, raw details disclosure, Retry, folder inspection, and applicable Move Files actions work from keyboard and pointer | Not run |
| Settings | Tabs, encryption picker, pending restart, queue, and bandwidth fields have clear names/order; Return and validation focus work; errors are announced | Not run |
| Settings restart | Restart preserves paused/running intent, selection, host mode, and login preference; failed reconnect keeps the library and pending policy visible | Not run |
| VoiceOver navigation | Rotor/groups and reading order expose notices and their action buttons separately; row summaries/progress are not duplicated; announcements neither interrupt excessively nor disappear | Not run |
| Increase Contrast | Enable the real OS setting; focus, selected rows, disabled controls, notices, and progress remain distinguishable in both appearances | Not run |
| Reduce Motion | Enable the real OS setting; navigation and live updates remain usable without unnecessary animated movement | Not run |
| Large content | Long filenames/paths, many files, scrolling, smaller windows, and text size/zoom retain access to every control | Not run |

Use an isolated library and controlled fixtures before destructive file actions.
The existing engine/FFI tests cover bytes and intent separately. Signing/release,
physical lifecycle, public swarms, RSS, and remote control remain outside this step.

## Completion boundary

The implementation and automated portion can close after the recorded checks pass.
Full step 6 manual acceptance remains open until the matrix contains real
observations. Performance work in step 7 can proceed independently; these manual
checks remain required before calling the application fully accepted for release.
