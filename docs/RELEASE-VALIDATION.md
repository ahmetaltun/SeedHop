# Release validation — 2026-10-26

## Localization gate added 2026-10-30

Run `make localization-check`, `make localization-test`, and
`make localization-render` against the final build. Every enabled language must
have complete catalogs and metadata, valid plural/format arguments, passing
packaged-language launches, and reviewed light/dark layouts. Normal and release
builds enforce catalog and resource checks before signing. Record manual keyboard,
pointer, and VoiceOver acceptance separately; automated fixtures do not close
those gates. AI-drafted translations are permitted and are not human-reviewed.
See [localization](LOCALIZATION.md) for the supported languages and workflow.

## Previous validation checkpoint

Protocol implementation and retained evidence are committed at `56ffc38`.
All 84 source-file digests and five binary digests in
[`results/v2-final-2026-10-26.json`](results/v2-final-2026-10-26.json)
matched immediately before that commit. `git diff --check` passed.
The retained protocol results remain 94 Rust tests, three isolated LSD tests,
95 independent protocol cases and four packaged-native XPC cases; the protocol
matrix was not repeated during this release-validation pass.

The user resumed the previously deferred release gates. No release candidate
may be declared until the gates below pass. The development validation app is
not a release candidate, and its separate service identity does not certify the
distribution app's background-service identity.

## Accessibility evidence and pending acceptance

The existing packaged app passed these checks again:

- `--verify-interface`: Find focuses the native search field; Focus Transfer List
  focuses the native table; Show Library reopens the closed fixture window.
- `--verify-ux`: all 17 controlled presentation regressions pass, including
  retryable errors, cancellation, stable action targets and preserved choices.

These checks invoke app-owned menu actions and controlled model operations.
They do not deliver physical key events or listen to VoiceOver output.
Native computer automation is unavailable in this session.

Source inspection confirms labels for search/import, transfer progress/status,
Finder buttons, inspector close, file priorities and bandwidth direction/units.
This is source evidence, not a passing accessibility-tree or spoken-output test.
`InlineNotice` and the bandwidth validation error render text without explicit
accessibility announcement notifications or a live region. New or changed errors
therefore need a spoken-output test; automatic announcement is not established.

Run the following on the final signed build, using disposable transfers:

1. Enable macOS keyboard navigation. Without a pointer, use Command-N, Command-O,
   Command-Shift-V, Command-F, Return, Command-L, arrows, Tab and Shift-Tab.
   Verify visible focus, row selection, pause/resume and inspector access.
2. Reach file checkboxes, priority menus, Rename, destination selection and
   settings. Cancel import/removal with Escape. Confirm modal entry and return
   focus, and confirm that disabled controls cannot execute actions.
3. Enable VoiceOver. Read transfer names, status, progress, selected rows,
   search, file choices, priorities, icon buttons and bandwidth fields. Check
   usable names, values and selection states, without duplicate announcements.
4. Enter an invalid magnet and invalid bandwidth value. Open a malformed torrent.
   Trigger a recoverable engine/storage error. Keep focus on the initiating
   control and verify each new or changed error is announced. Retry and dismiss
   with the keyboard. Record the actual spoken text and focus destination.
5. Repeat import cancellation and error recovery with VoiceOver. Check Increase
   Contrast and Reduce Motion. Close and reopen the library during a real
   transfer, then check scrolling and selection under live updates.

Record build hash, macOS version, keyboard settings, VoiceOver observations,
expected/actual behavior and pass/fail for each step. Any missing error
announcement or inaccessible control keeps this gate open.

## Signing and notarization

Only an Apple Development signing identity is currently available. A fresh
certificate-backed development fixture was built at
`.build/release-validation-2026-10-26/SeedHop.app`, with separate app/service
namespace `app.seedhop.validation.2fbd941e2d2a`. Deep/strict signature validation
passes; Hardened Runtime is enabled and the app has no debug library-validation
exception. Build output is `.build/release-signing-2026-10-26.log`.

The fixture passed three certificate-authenticated foreground/background handoff
cycles, preserving running/paused intent, identity and selection. Three simulated
login-policy cases also pass: resume off across a changed session remains idle;
resume on restores; same-session restart restores with login resume off. Exact
fixture bytes pass, and the isolated agent is unregistered by the test. Output
is `.build/release-lifecycle-2026-10-26.log`. These simulated cases do not certify
actual logout/login, permission revocation or helper updates.

Portable results, log digests and signed fixture binary digests are retained in
[`results/release-validation-2026-10-26.json`](results/release-validation-2026-10-26.json).

Distribution signing/notarization is blocked on an available Developer ID
Application certificate and local notarization authentication. Do not submit
the Apple Development fixture as the distribution artifact. Apple requires a
[Developer ID signature for notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
Install credentials locally; record only the notarytool keychain profile name,
never passwords or private keys, in validation notes.

After credentials are available, validate the intended distribution namespace,
nested code, designated requirements, helper authentication, secure timestamps,
Hardened Runtime and entitlements. Submit the signed package, retain the
submission ID/result, staple its ticket and verify Gatekeeper acceptance.
Test background permission denial, re-enabling and helper replacement on that
signed build. An isolated development handoff does not close these checks.

## Actual lifecycle and recovery acceptance

No external physical drive was attached during preflight. Actual login,
sleep/wake, drive unplug/reconnect and interface switching need target-system
participation. They have not passed in this session. Run them on the intended
signed build after signing/notarization succeeds.

Use a known fixture with recorded source SHA-256, at least one partially
downloaded transfer and one paused transfer with a non-default file selection.
Record torrent identities, selected files, pause intent, queue order and paths
before each fault. Keep evidence outside any volume being unplugged.

| Gate | Action | Required observation |
|---|---|---|
| Actual login, resume off | Enable background transfers, disable login resume, quit and actually log out/in | No automatic transfer start; opening the app preserves identities, choices and pause intent. |
| Actual login, resume on | Enable login resume, quit and actually log out/in | Previously running transfers restore automatically; paused transfers remain paused; file choices and identities survive. |
| Sleep/wake | Sleep during an incomplete transfer; wake and restore connectivity | No duplicate owner; transfer resumes or gives an actionable recoverable error; exact final bytes and saved intent survive. |
| Physical drive loss | Download disposable data to a removable drive; unplug, reconnect, retry if requested | No replacement destination or lost membership; original paths, identity and choices survive; exact final bytes pass. |
| Network interface loss | Disable or disconnect the active interface during transfer; restore it, then change interface where available | No duplicate transfer or state loss; discovery/connections recover and exact final bytes pass. |

Use a peer reachable through the interface under test; loopback transfers do not
exercise physical interface recovery. Keep fault timing and OS/network logs.
Do not replace these checks with edited login tokens, directory renames,
loopback relay outages or a disk-image unmount.

## Release candidate stop condition

Build the optimized candidate only after accessibility, distribution signing,
notarization, background identity and all actual recovery gates pass. Preserve
the exact commit, toolchain, bundle version and signing identity in the record.
Validate candidate nested signatures, notarization/stapling, Gatekeeper,
authenticated XPC and transfer integrity. Any changed binary must receive its
own applicable package and release validation. Retain artifact SHA-256 and test
results before marking the candidate accepted.
