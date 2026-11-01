# System menu fixes — 2026-11-01

## Changes

- Preserve internal `NSMenu.title` values. Localize visible menu-item titles instead. Controlled AppKit tracking showed that changing View's internal title disrupted native full-screen placement after a language change; preserving it retained the command.
- Refresh native leaf labels before display when native delegates recreate them during tracking. Preserve menu structure, hidden shortcut items, and SwiftUI-owned command labels.
- Exclude window-list entries by their reveal action rather than excluding every NSWindow target. Standard Minimize and Zoom commands can themselves target NSWindow and still require localization.
- Match native window commands by action and tiling groups by their child actions before falling back to titles. Actual system wording, such as Russian Minimize's “Убрать в Dock”, can differ from the app catalog. No system actions or menu structure are replaced.
- Put Sidebar and Details commands in View, with availability tied to the focused library scene. View retains these entries when native window commands are unavailable.
- Use Control-Command-S for the custom sidebar. Option-Command-S collided with AppKit's hidden `toggleSidebar:` command and disappeared from the visible custom command after language changes, including with native localization disabled. Control-Command-S retained its visible shortcut and dispatched the custom action.
- Give the library WindowGroup a default value and use that value when invoking Show Library. This reveals an existing matching window while retaining WindowGroup URL handling and native tab support.
- Add eight missing native Window-menu labels in all 13 languages. Correct French AutoFill to avoid colliding with Fill, and align Russian Minimize with macOS's native wording. Translation provenance remains marked as AI-drafted, without human review.
- Resolve interface-test menu indexes again after asynchronous updates and ignore hidden items.

## Verification

Environment: macOS 15.8.2, Apple Silicon, Swift 6.2.4 compiler in Swift 5 mode.

- Standard `python3 scripts/build.py --skip-rust` build, catalog compilation, and deep/strict ad-hoc signature verification passed. Rust artifacts were reused; no Rust code changed for this task.
- Catalog validation passed for 678 required entries in 13 languages.
- `--verify-menus` passed on the final packaged value-based WindowGroup implementation from a Turkish launch. It uses real AppKit popup tracking, captures labels while menus remain open, and selects window commands during tracking. It checks internal titles, native View/Window action retention, translated labels and tiling-group captions, visible Sidebar/Details shortcuts across every bundled language, focus changes through Bandwidth Limits and Settings, repeated Show Library actions, and reopening after closing the library. A constructed Control-Command-S event toggles and restores the custom sidebar through AppKit's key-equivalent dispatch.
- The existing `--verify-interface` suite passed in a fresh fixture app built from the standard package, including search/list focus, library reopening, import/file controls, bandwidth settings, scheduling, transfer controls, and native table layout.
- `--verify-live-language` passed from a Turkish launch through Turkish, English, French, and Russian, including standard application-menu labels, shortcuts, table identity, widths, selection, transfer data, plural rules, and region.
- `git diff --check` passed. The code knowledge graph was updated.

## Evidence boundaries

The initial investigation manually called SwiftUI menu-update delegates. Those calls can themselves strip AppKit-inserted entries and are not reliable evidence of physical menu-bar tracking. The new menu acceptance check uses AppKit's actual tracking loop and cancellation timer.

An exploratory diagnostic app reused across several window experiments failed the interface suite on duplicate accessibility error announcements. A fresh fixture from the final standard package passed the full suite. The exploratory failure's exact cause was not isolated, and no accessibility-announcement fix is claimed here.

These checks use sample transfers and fixture preferences. Physical keyboard/pointer input, VoiceOver speech, full-screen transitions, other macOS versions, and human translation review remain outside this verification.
