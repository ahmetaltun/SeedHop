# Localization

SeedHop supports English (`en`), German (`de`), Spanish (`es`), French (`fr`),
Italian (`it`), Brazilian Portuguese (`pt-BR`), Turkish (`tr`), Russian (`ru`),
Ukrainian (`uk`), Polish (`pl`), Japanese (`ja`), Korean (`ko`), and Simplified
Chinese (`zh-Hans`). English is the source and fallback language.

SeedHop follows macOS preferred languages by default. General settings includes a
language selector with all supported languages, shown in their own names. Selecting
“Follow macOS” removes the app-specific override; a specific language uses the native
per-app language preference, also recognized by System Settings. The app interface
updates immediately without restarting transfers or rebuilding open views. macOS-owned
permission prompts and other system UI retain the language managed by macOS.
Numbers, dates, units, and time controls follow regional preferences independently
of interface language. Scheduling still uses the Mac's local wall time.

RTL translations and mirrored layouts are outside this release. Unicode content,
including filenames in RTL scripts, remains verbatim. Developer tools, raw tracker
and OS details, diagnostic logs, and project documentation remain English.

## Sources and build

`macOS/Resources/Localization/Localizable.xcstrings` owns application translations.
`InfoPlist.xcstrings` owns the local-network permission explanation and document
type descriptions. `NativeMenus.xcstrings` owns standard macOS menu labels, including
About, Settings, Hide, Quit, and the menu-bar headings. Native menu refreshes change
titles only; their actions, shortcuts, validation, and dynamic window/service entries
remain intact. `scripts/localization.py` owns the enabled language list and
plural requirements. Runtime resources are native `.strings` and `.stringsdict`
files produced by Xcode's `xcstringstool`; no translation service runs at build time.

Normal builds extract source keys into `.build/localization-extraction`, check
coverage without rewriting catalogs, compile resources, and verify the bundle
before signing. They remove stale `.lproj` directories, retain English as the
development language, and advertise only bundled languages. The XPC service also
receives localized permission metadata. Release builds reject catalog-update and
pseudolocalization flags.

Use localizable SwiftUI literals for text rendered in the scene locale. For computed
strings, AppKit, notifications, and alerts, use `String(localized:bundle:locale:)`
with `AppLocalization.shared.bundle` and `AppLocalization.shared.locale`. These
observable resources update existing views when the language changes; do not cache
translated labels in static constants or rebuild view identity to change language. Existing shared view helpers accept already
localized `String` values. Use complete messages instead of translated fragments.
Mark user names, paths, URLs, hashes, endpoints, and protocol values as verbatim;
never use translated text as a control value or persistence key. AppKit columns,
menu actions, and restored widths use stable identifiers.

For counts with grammatical agreement, add native plural variants. The first
numeric argument drives the current whole-message plural entries. Preserve all
argument types; use positional placeholders to change argument order. Add new
inflected count messages to `COUNT_KEYS` so a plain string cannot silently replace
their plural forms. Russian, Ukrainian, and Polish require `one/few/many/other`;
French, Spanish, Italian, and Brazilian Portuguese also include `many`; CJK uses
`other` only. Do not construct English-style suffixes in code.

Native time pickers use a fixed Gregorian UTC date as a time-only carrier. This
prevents DST gaps from normalizing saved values. The engine still receives the
existing local-wall-time minute values and Monday-based weekday mask. Weekday
labels follow app language; their displayed order follows regional preferences.

## Update and translate

1. Add the complete English message with context. Explain destructive actions,
   ambiguous terms, units, and each placeholder at its source or in the catalog.
2. Run `make localization-update`. This explicitly extracts and syncs strings,
   removes obsolete keys, and preserves existing translations. A development
   update build can contain incomplete translations; it is not release approval.
3. Translate every required entry in every enabled language. For English edits
   within an existing catalog entry, run the update command before translating;
   changed source entries are marked for revision. Refresh every affected form.
4. Check placeholders, plural forms, protected names, keyboard symbols, and
   newlines. Set refreshed catalog units to `translated`. This means a translation
   is present; it does not mean a human reviewed it.
5. For AI-drafted updates, run
   `python3 scripts/localization.py record-ai-provenance` after translating.
   This records the source digest and explicitly records `humanReviewed: false`.
   Preserve separate, truthful provenance if human review is added later.
6. Run the checks below and inspect the affected layouts before committing.

`translation-status.json` records AI provenance separately from runtime catalogs.
Source-digest changes block release checks until translations and provenance are
updated. Missing translations use English defensively at runtime but fail the
release guard. AI translations can still contain wording errors; fluent review
is recommended and has not been claimed for this initial translation set.

### Glossary

| English term | Meaning and translation guidance |
| --- | --- |
| Download | Receive selected torrent content; distinguish from importing torrent metadata. |
| Upload | Send content to other peers. |
| Seed / seeding | Share complete selected content; use established BitTorrent terminology. |
| Peer | Another BitTorrent participant, not a tracker or a transfer. |
| Tracker | Service that helps discover peers; retain the standard local technical term. |
| Queue | Ordered downloads waiting for an active slot. |
| Pause | Preserve user intent and stop this transfer's activity. |
| Resume | Continue a paused transfer according to queue and scheduling rules. |
| Retry | Recover a failed transfer; preserve saved pause intent. |
| Recheck | Verify existing pieces against hashes; do not imply downloading them again. |
| Bandwidth limit | Rate cap across transfers. `KB/s` means decimal kilobytes per second. |
| Remove | Remove the library entry; downloaded files remain unless deletion is explicitly chosen. |

Keep SeedHop, BitTorrent, protocol names, units, hashes, and shortcut symbols intact.

## Validation

```sh
make localization-check
make localization-test
make localization-render
```

The catalog guard checks all enabled languages, translator context, format argument
positions and types, native plural categories, newlines, protected tokens, source
drift, extracted-key coverage, and compiled resources. Unit tests prove malformed
catalogs fail. Runtime fixtures launch the packaged development app in every
language and verify actual lookup, plurals, unsupported-language fallback,
preferred-language ordering, independent regional formatting, numeric validation,
structured messages, legacy reply decoding, and Unicode filename preservation.

Renders cover the library, minimum workspace, files/peers/trackers/activity
inspectors, both import stages, all settings sections, bandwidth, scheduling,
recovery, errors, and the minimum empty pane in light and dark appearances.
They use fixture data, not the user's library or screen. Output lives under
`.build/localization-renders`; runtime evidence is `.build/localization-results.json`.

Native rendering and local socket/XPC fixtures may need execution outside a
restrictive sandbox. Run tests against the final bundled executable. Native action
fixtures and screenshots are not evidence of physical key/pointer delivery or
VoiceOver speech. Record those manual checks separately using disposable transfers,
particularly in German, Russian, Turkish, Japanese, and Simplified Chinese.

### Pseudolocalization

```sh
python3 scripts/build.py --pseudolocalization
.build/SeedHop.app/Contents/MacOS/SeedHop --render-localizations -AppleLanguages '(en-XA)' --output .build/localization-pseudo
```

This development-only language accents and expands text while preserving format
arguments. A normal rebuild removes it. `--release --pseudolocalization` is rejected.

## Engine messages and compatibility

Protocol v6 carries stable message codes, typed text/unsigned-number arguments, and
optional diagnostic detail through UniFFI and XPC. The foreground app renders
localized guidance. Cancellation is a separate outcome. Unknown codes show generic
localized guidance with the original diagnostic available. Application-owned
validation has specific codes; raw third-party details stay verbatim.

The interface and both engine hosts must be rebuilt together. Version mismatches
ask users to restart. Existing saved libraries, queue state, pause intent, file
selections, and schedules retain their existing formats. Message metadata is
optional in Swift decoding; older details remain readable as diagnostics.

To add a language, add its BCP-47 identifier and plural requirements, translate both
catalogs completely, record provenance, and run the full launch/render matrix.
To withdraw a language, remove its translations and enabled identifier, then
rebuild. Native resource matching falls back through supported preferences to
English. Neither operation changes transfer data.

## Initial localization checkpoint — 2026-10-31

The initial ad-hoc native build passed catalog checks for 615 required entries in
13 languages, 16 packaged language/region launches with 1,408 assertions, all 46
existing UX regressions, and the packaged protocol v6 XPC smoke test. The Rust
suite passed 145 tests with five ignored; Clippy and release-mode Swift type
checking passed. Normal builds leave the catalogs unchanged.

The initial matrix generated 390 light/dark screenshots. Representative layouts
were inspected; a clipped German import option was corrected and checked in a
fresh 30-image German render. English and German native action fixtures passed.
The extended Russian fixture exposed English column-title selectors, which now
use stable IDs; that extended fixture was not repeated after the correction.
Pseudolocalization assets compile, but expanded-layout acceptance remains open.

Physical keyboard/pointer delivery, VoiceOver speech, and fluent human translation
review remain unverified. These automated results do not close those manual gates.
