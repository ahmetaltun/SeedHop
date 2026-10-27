# Files inspector redesign plan

Date: 2026-10-27. Status: implemented with automated checks; manual accessibility
and physical-input acceptance remain open. The design specification below records
the intended behavior; the implementation record distinguishes verified scope.

## Implementation and verification record

### UI correction after live review

The first implementation left checkboxes centered across two text lines and
placed priority only in selection-dependent controls. Live review also exposed
a conditional last-file notice that shifted the file viewport. The correction
uses 44-point rows, aligns checkbox/icon/disclosure with the filename line, and
puts a small priority pop-up beside each row's secondary metadata. Priority no
longer requires row selection; batch priority remains in Actions. Status and
search counts occupy a fixed-height footer. The last-file restriction remains
in checkbox help and file details without adding a header row. These decisions
supersede the row size and footer-priority proposal below.

The correction passed 38 controlled UX regressions and 16 native checks. New
native assertions compare checkbox/icon/filename centers, activate a row's
priority pop-up with no selected row, retain priority menu items across progress
updates, and compare the viewport frame across selection, search, the final-file
restriction, and operation status. The 1,000/10,000-file fixtures took 1,270/1,232
ms including the same 1,200 ms of layout waits, with nine materialized cells each.

Implemented `macOS/App/TorrentFilesView.swift` with an `NSOutlineView` adapter,
cached path hierarchy, native mixed-state checkboxes, separate multiselection,
path search, shared/context actions, batch priority changes, and file details.
The Files tab now has a compact transfer header and its own scroll region.
No Rust or XPC protocol changes were required. Existing rename transactions,
progress, cancellation, and storage-recovery guards remain in use.

Initial implementation verification on 2026-10-27 (superseded by the correction
above where counts and layout differ):

- Ad-hoc development build and deep/strict code-signature verification passed.
- All 38 controlled Swift UX regressions passed, including tree filtering,
  rename identity, progress-only tree reuse, batch command targets, preservation
  of untargeted priorities/skipped files, checking guards, failure, and retry.
- All 14 native fixture checks passed. New checks exercise expansion, selection,
  controlled Space events, checkbox callbacks, context-menu Rename and focus
  restoration, search ancestors, and readable last-included-file text.
- Native 1,000/10,000-file fixtures took 1,266/1,270 ms respectively, including
  three 400 ms layout waits each. Both materialized eight cells. The scoped
  test enforces a five-second budget and confirms byte updates reuse the tree.
  These timings do not close the separate whole-library performance findings.
- The final render pass includes 72 fixtures. Files cases cover 250/280/360-point
  widths, light/dark appearances, search, no results, offline data, and increased
  contrast. Representative fixtures were visually inspected.

During development, the scale fixture exposed repeated child-list filtering and
target-set construction inside per-file filtering. Both were removed before the
final run; the original 59-second result is not accepted performance evidence.

Commands and local logs: `make build` (`.build/files-build.log`), packaged
`--verify-ux` (`.build/files-ux.log`), packaged `--verify-interface`
(`.build/files-interface.log`), and packaged `--render-designs --output .build/design`
(`.build/files-render.log`). Durable evidence: `results/files-inspector-2026-10-27.json`.

Manual keyboard/pointer use, VoiceOver speech, deep-hierarchy scrolling, real OS
accessibility preference changes, and live-transfer rename/cancellation through
the new outline have not been certified. The native fixture invokes app-owned
controls and controlled events; it is not physical input. Engine and physical
release gates were not rerun for this Swift presentation/store change.

## Goal and scope

Make torrent contents easy to scan, choose, and manage within the existing macOS
inspector. Keep SeedHop's quiet visual language and native interaction patterns.
Preserve current download selection, priorities, rename transactions, torrent
identity, queue membership, and pause intent.

This work covers the torrent-details Files tab. Import-time file selection can
reuse components later, but changing the import flow is outside this plan.

## Current problems and evidence

`macOS/App/LibraryView.swift`, `files(_:)`, currently renders each file as a
checkbox with two lines of information, followed by a priority menu and Rename
button. The surrounding inspector is 250–360 points wide with 20-point insets.

- Repeated priority and rename controls compete with names and consume height.
- Paths appear as flat filenames, obscuring folders such as `Subs`.
- Two-line filename wrapping produces irregular rows and weak scan alignment.
- The last included file's entire Toggle becomes disabled. This dims its name
  and size, although the restriction only concerns excluding that file.
- A checkbox communicates download inclusion, but there is no separate row
  selection for file actions.
- Per-file downloaded bytes exist in `FileItem`, but this view omits progress.
- The complete inspector scrolls as one page. Long torrent descriptions push
  file controls away, and large file lists share that same scroll region.

## Recommended layout

Use a compact, Finder-like outline with two-line file rows. Use disclosure
triangles only for actual folders. Do not squeeze a desktop table's columns
into the current narrow inspector.

```text
Files                                  Actions ▾
1 of 4 included · 1.95 GB
[ Search files                                ]
───────────────────────────────────────────────
☑  [video] Runner.2026.1080p…[YTS.BZ].mp4
           1.95 GB · 42%
☐  [image] YTS.GG - Official site.jpg
           39 KB · Skipped
☐  [text]  YTSifyUP… (TOR).txt
           848 bytes · Skipped
▸ ◩ [folder] Subs
             1 file · Mixed
───────────────────────────────────────────────
1 file selected             Priority: Normal ▾
```

The sketch demonstrates structure, not measured values. Folder checkboxes show
all, none, or mixed inclusion; the example folder state is illustrative.

Keep the existing inspector header, tab selector, and bottom transfer actions.
For Files, give the content outline its own remaining-height scroll region.
Keep Files summary, search, and action bar stationary. Condense the transfer
identity above the tabs in Files mode: small icon, bounded title, status, and
existing transfer progress. Keep full title accessible through selectable detail
text or an accessible popover. Avoid nested vertical scroll views.

### Row specification

- Target 48–52-point file rows, starting at 50 points; verify against real text.
- Use 12–13-point primary names and 11-point secondary metadata.
- Place download checkbox, file-type icon, and name on a stable leading grid.
- Use semantic primary and secondary text colors. Reserve accent for controls
  and active states; do not tint every filename or repeat green Normal labels.
- Keep names on one line with middle truncation and a visible extension.
  Expose the full current relative path through help, accessibility, and the
  selected-file details popover. Tooltips must not be the only access path.
- Show localized size and progress on the second line. Show nondefault priority
  as text there when present, e.g. `39 KB · Skipped · High priority`.
- Use native selection highlighting and subtle separators. Avoid per-file cards.
- Keep text readable for skipped files and while commands are unavailable.
  Disable relevant controls separately from the filename and metadata.

Use numeric progress initially rather than adding another bar to every row.
Audit the engine's per-file byte semantics before labeling 100% as complete;
checking and disconnected states must not imply verified or current data.

### Inspector width

Design for the existing 250, 280, and 360-point inspector widths. At 250 points,
retain the two-line row and put the priority control in Actions if necessary.
Do not solve clipping by reducing text below the proposed type scale.

A wider, resizable file manager with separate Size, Progress, and Priority
columns is a possible future feature. It is not required to fix this inspector.

## Interaction contract

### Inclusion versus row selection

The checkbox means “Download this file.” Clicking the name selects a row for
actions without changing inclusion. Command-click and Shift-click support
native multiselection. Selection remains visually distinct from inclusion.

The selected-file count and included-file count use different wording. Search
and selection do not change the torrent's download set.

Keep the existing requirement that at least one file remains included. Explain
the disabled checkbox through accessible help and nearby selected-file details:
“Keep at least one file included.” Never dim the entire last-included row.

### Folders

Build the hierarchy from each file's current relative path, including local
renames. Use `(torrent ID, file index)` for leaf identity; never use names or
visible row positions as command targets. Folder identity is a torrent-scoped
relative path. Expansion survives polling; renamed paths may invalidate only
the affected folder state.

Folder inclusion applies to all descendant leaves, including collapsed ones.
Show a mixed checkbox when only some descendants are included. Reject any
folder action that would exclude the torrent's final included file; explain why
and leave the complete selection unchanged.

Keep original file ordering within sibling groups initially, with folders first.
Do not add sorting controls for this narrow view.

### Shared actions

Provide one always-visible Actions menu. Mirror relevant actions in each row's
context menu, following Apple's guidance that context-menu commands also need
a discoverable interface entry point.

- Include in Download / Skip Download: applies to selected files or descendants.
- Priority → Low / Normal / High: applies to selected leaves or descendants.
- Rename…: available for exactly one file; retain the current sheet and storage
  transaction. Do not add folder rename or batch rename.
- Show File Details: presents the full path, size, inclusion, priority, and
  available progress without forcing filename wrapping in the list.

Show the shared Priority control for selected rows. Use Mixed when priorities
differ. Changing priority must never silently include a skipped file. Preserve
its stored priority so later inclusion retains the choice.

Do not add Open File, Quick Look, or per-file Reveal in Finder during this
redesign. Those need separate existence, completeness, and path-resolution rules.

### Search

Use one compact search field scoped to files. Match current relative paths
case-insensitively and retain ancestor folders for matching descendants. Explain
the result count; show “No matching files” with Clear Search.

Clear row selection when an explicit filter hides its targets. Preserve
inclusion and priorities. During search, disable folder-level mutations and
explain “Clear search to change the whole folder.” Individual matching files
remain actionable. This avoids hidden descendants changing unexpectedly.

Retain file-search text and expansion per torrent during the window session.
Do not change the app's existing Command-F transfer-search shortcut.

## Loading, errors, and command safety

- Distinguish metadata loading from a known empty file list. Display explicit
  loading, unavailable, and no-results messages rather than a blank pane.
- During checking, explain that file changes are temporarily unavailable.
- During disconnection, retain readable last-known rows and label their state.
- During an update, show one stable status line and prevent conflicting writes.
  Keep browsing, scrolling, and row selection available.
- Keep the confirmed state until the command succeeds. On failure, preserve
  selection and focus, report the error through the existing error mechanism,
  and allow retry after refresh.
- Capture torrent ID and file indices at invocation. A later selection change
  must not redirect a pending command or rename sheet.
- Use the existing engine selection and priority commands. Submit one complete
  selection or priority array for a folder or batch action; do not loop through
  single-file commands using stale snapshots.
- Apply existing busy, checking, connection, active file-operation, and storage
  recovery rules consistently. Keep rename progress and cancellation intact.

## Accessibility and keyboard behavior

Prefer native outline, checkbox, selection, and menu behavior. File actions must
work without hover or secondary click.

- Arrow keys navigate rows; Left/Right collapse or expand folders.
- Space toggles inclusion for focused selected rows, subject to the last-file
  rule; avoid triggering the checkbox twice through competing key handlers.
- Tab moves among search, outline, and shared actions with visible focus.
- Escape clears file search when that field owns focus, or cancels the active
  sheet according to native behavior. Preserve global shortcut behavior.
- Announce filename, file/folder role, inclusion or mixed state, size, priority,
  and available progress. Announce completion/errors once; avoid poll chatter.
- Use words as well as color for skipped, checking, mixed, and unavailable states.
- Restore focus after rename and errors. Validate light, dark, Increase Contrast,
  Reduce Motion, keyboard-only operation, and VoiceOver on the running app.

## Implementation sequence

### 1. Lock layout and data semantics

Create fixtures matching the supplied four-file example plus nested folders,
long Unicode names, mixed priorities, and a single included file. Audit per-file
downloaded-byte semantics. Render the proposed inspector at all three widths
before committing to the component structure.

Deliverable: reviewed compact layout, state matrix, and progress-label rules.

### 2. Replace the repeated row controls

Extract `TorrentFilesView.swift` from `LibraryView.files(_:)`. Add a pure
presentation tree with stable leaf identities, folder aggregates, and cached
path grouping. Reuse `DesignSystem.swift` tokens. Restructure Files scrolling
without changing Overview and Trackers unnecessarily.

Prototype native SwiftUI hierarchy/selection behavior first. If it cannot meet
mixed-checkbox, keyboard, and large-list requirements, use a narrowly scoped
`NSOutlineView` adapter. Make that decision before duplicating accessibility
behavior with custom gesture-driven rows.

Deliverable: compact outline, readable names, hierarchy, progress, and stable
selection; current mutation capabilities remain available.

### 3. Wire actions and search

Add file-row selection distinct from `FileItem.selected`. Add shared and context
menus, path details, folder inclusion, batch priority, and scoped search.
Extend `LibraryStore` with one batch-priority path that builds a complete array
from the latest confirmed snapshot. Reuse `setFileSelection`, `updateTransfer`,
and `requestRename`; keep Rust/XPC contracts unless inspection proves a gap.

Deliverable: consistent command availability, stable targets, explicit mixed
states, and preserved transactional rename behavior.

### 4. Verify and document

Extend `PreviewFixtures.swift`, `UXSelfTest.swift`, and `InterfaceSelfTest.swift`
where appropriate. Update `docs/DESIGN.md` and `docs/FILE-MANAGEMENT.md` only
after the actual behavior is verified. Record automated and manual results
separately.

## Acceptance criteria and checks

| Area | Required evidence |
| --- | --- |
| Density | Four-file example occupies no more than 260 pt for four visible file rows and one folder row, excluding surrounding toolbar/search. No repeated Rename buttons or default-priority menus. |
| Width | No horizontal scrolling or overlapping controls at 250, 280, and 360 pt. Extensions remain identifiable. |
| Hierarchy | Deep folders, duplicate basenames, Unicode, renamed paths, collapsed descendants, and mixed inclusion work correctly. |
| Inclusion | Single, folder, and batch changes preserve at least one included file. Priority, pause intent, and torrent identity survive. |
| Selection | Polling preserves row selection, focus, scroll, and expansion. Filtering or switching torrents cannot redirect an action. |
| Priority | Low/Normal/High and Mixed display correctly. One batch command preserves untargeted priorities and skipped state. |
| Rename | Existing progress, cancellation, failure, recovery, and focus restoration remain usable. Stable file identity survives the new path. |
| Search | Relative-path matches retain ancestors; hidden descendants cannot be changed through filtered folder actions. |
| States | Loading, empty, no matches, checking, updating, disconnected, and command failure have explicit readable presentations. |
| Accessibility | Live keyboard and VoiceOver walkthrough passes; mixed checkbox values and unavailable reasons are understandable. |
| Scale | Profile 1,000 and 10,000-file fixtures. No tree rebuild for byte-only updates; no focus reset or sustained main-thread stalls. |

Run `make render` and visually inspect new light/dark and high-contrast Files
fixtures. Run `make ux-test` for command behavior and `make interface-test` for
native interaction coverage. Add focused tests for tree aggregation, filtering,
last-included guards, and batch array construction. Run engine regressions only
where command/store changes affect those paths; do not rerun unrelated protocol
or physical release gates for a presentation-only change.

Rendered fixtures prove layout only. Manual keyboard, VoiceOver, and large-list
profiling remain separate evidence. Stop when the criteria above pass; defer
new file operations, import redesign, configurable columns, and engine features.

## Reference guidance

- [Apple: Context menus](https://developer.apple.com/design/human-interface-guidelines/context-menus):
  keep actions relevant and also accessible through the main interface.
- Native component references for implementation:
  [SwiftUI OutlineGroup](https://developer.apple.com/documentation/swiftui/outlinegroup),
  [AppKit NSOutlineView](https://developer.apple.com/documentation/appkit/nsoutlineview).
- Repository constraints: `docs/DESIGN.md`, `docs/FILE-MANAGEMENT.md`,
  `docs/ACCESSIBILITY-ACCEPTANCE.md`.

Row sizes, layout, action grouping, and delivery phases above are SeedHop design
proposals, not dimensions or requirements prescribed by Apple.
