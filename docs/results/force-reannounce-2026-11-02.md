# Force Reannounce validation — 2026-11-02

Implemented protocol version 9 across the engine, UniFFI bridge, host, and native
app. Torrent context/Transfers menus and Trackers-tab selected/all actions use
live engine controls. Scheduling acknowledgments remain distinct from tracker
results. Mixed selections skip unavailable transfers and continue after errors.

HTTP refresh respects tracker minimums, a 60-second local cooldown, and retry
backoff. Regular announces retain their advertised interval. UDP refresh waits
for the advertised interval. Existing tracker streams deliver peers; refresh does
not restart transfers or broaden private discovery. Pending requests are temporary
and target a tracker/hash monitor generation inside the current engine instance.

## Completed checks

| Check | Result |
| --- | --- |
| Production tracker unit/loopback suite | 16 passed; 1 pre-existing external-network test ignored |
| Core unit suite | 28 passed |
| Local core integration suite | 2 passed, covering v1, v2, hybrid, private, and trackerless cases |
| UniFFI bridge suite | 5 passed |
| Native UX presentation suite | 67 passed, including 4 new reannounce regressions |
| Native transfer controls | Passed, including eligible/busy mixed reannounce menu dispatch |
| Bundled localization | 16 language/region launches; 1,472 assertions passed |
| Catalog validation | 725 required entries across 13 languages passed |
| Rust workspace check, format, and Clippy | Passed; Clippy treats warnings as errors |
| Isolated production tracker Clippy | Passed; warnings treated as errors |
| Ordered vendor patch | Reverse applicability and whitespace checks passed |
| Apple Silicon development app | Built; arm64 verification and deep strict codesign verification passed |

Reproduction commands:

```sh
python3 scripts/test_tracker_comms.py
cargo test --offline --locked -p seedhop-core --test reannounce -p seedhop-ffi --lib
cargo check --offline --locked --workspace
cargo fmt --all -- --check
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
python3 scripts/build.py
.build/SeedHop.app/Contents/MacOS/SeedHop --verify-ux
.build/SeedHop.app/Contents/MacOS/SeedHop --verify-interface --transfer-controls-only
python3 scripts/verify_localizations.py --bundle .build/SeedHop.app
```

The tracker runner creates an isolated manifest using the root lockfile's resolved
versions and compiles the actual production sources. Network tests use local
fixtures, not public trackers. Native UI checks use controlled sample data.

## Remaining acceptance and existing protocol boundaries

Manual pointer/keyboard and VoiceOver walkthroughs were not performed. Automated
AppKit menu invocation and presentation checks do not certify those interactions.
Public tracker interoperability and release signing/notarization were not tested;
the built app is ad-hoc signed for local development.

Public tier scheduling still uses the existing flattened tracker set. UDP still
uses the existing single 10-second request timeout, rather than BEP 15's complete
retransmission schedule. Both are separate engine improvements.
