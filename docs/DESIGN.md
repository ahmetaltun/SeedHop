# SeedHop design foundation

## Direction

Quiet, warm, precise. The interface should feel at home on a Mac while making
large transfer libraries easy to scan. Visual and interaction review is a
completion requirement for every new screen.

## Tokens and components

`macOS/App/DesignSystem.swift` owns semantic surfaces, an adaptive forest/mint
accent, hairline separators, 12-point panel radii, status colors, type hierarchy,
and reusable components. SF system type is used throughout, with rounded type
for headlines and large metrics, and monospaced digits for changing values.

- Headline: 27 pt semibold; modal headline: 22 pt.
- Primary controls and transfer names: 12–13 pt.
- Supporting labels: 10–12 pt; section eyebrows: 10 pt with expanded tracking.
- Primary content insets: 26–28 pt; panel insets: 16–20 pt.
- Status uses both words and color; selection follows native table behavior.
- `BrandMark`, `StateBadge`, `TransferProgress`, `Metric`, and `InlineNotice`
  provide repeated presentation conventions.

## Main window

A quiet navigation rail leads into a configurable, sortable native table.
Rates and download totals sit above the library. The inspector contains
overview, file-selection, and tracker information. Tracker credentials are not
displayed. Columns can be resized, reordered, hidden, and restored through the
native header customization menu. Selection uses stable info hashes.

Empty, no-results, connecting, switching, and error states have explicit copy
and actions. File lists use lazy stacks. Polling pauses when the library window
is hidden; byte values are formatted for the user's locale.

## Import

The first step accepts a magnet or torrent file. The second shows the actual
metadata, size, destination, selected files, paused option, and explicit reuse
of existing files. Cancel sends an explicit cancellation request to the engine.
Existing files are never silently reused or overwritten.

## Accessibility and interaction

Native buttons, fields, table selection, checkboxes, segmented controls, and
switches provide keyboard and accessibility behavior. Status badges include
text; custom progress includes an accessibility value. Inspector transitions
respect Reduce Motion. Every icon-only control has a label or help text.

Shortcuts: Command-N add, Command-O open, Shift-Command-V paste magnet,
Command-R resume, Command-period pause, Command-delete remove,
Option-Command-I inspector, Option-Command-Q quit and stop transfers.

## Visual QA

`make render` produces 12 light/dark fixture screens. Sample transfers are
available only through explicit design-preview mode. These renders verify
layout and typography; they do not prove VoiceOver navigation, live keyboard
interaction, or full system contrast behavior. Those remain manual acceptance
checks recorded in `VALIDATION.md`.
