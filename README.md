# SeedHop

<img src="macOS/Resources/AppIcon.png" alt="SeedHop app icon" width="128" align="right">

A native macOS torrent client built with **SwiftUI, AppKit, and Rust**.
The first milestone combines a carefully designed interface with a real
BitTorrent v1/v2 engine, authenticated XPC, and recovery tests.

**Development build:** macOS 15+, Apple Silicon. This is an internal preview,
not a notarized public release. See [validation status](docs/VALIDATION.md) for
the distinction between implemented behavior and verified acceptance criteria.

## Build and run

Requires Xcode 26.3 command-line tools, Python 3.12+, and Rustup.
The pinned Rust toolchain is installed by Rustup if it is not available.

```sh
make build
open .build/SeedHop.app
```

The build script compiles the Rust core, generates matching UniFFI bindings,
compiles the native app and both hosts, bundles the Rust library, and signs
nested code from the inside out. No XcodeGen, Node.js, Homebrew, or web runtime
is required. Swift uses the Swift 6.2 compiler in Swift 5 language mode for the
Foundation XPC interfaces.

Local builds use ad-hoc signing and a development-only library-validation
exception. For certificate-backed validation:

```sh
python3 scripts/build.py --identity 'Apple Development: Your Name (TEAMID)'
```

Release builds omit development flags and the validation exception:

```sh
python3 scripts/build.py --release --identity 'Developer ID Application: Your Name (TEAMID)'
```

Notarization and distribution require your own Developer ID and Apple account.
No certificates, signing keys, credentials, or automatic service registrations
are added by the build script.

## Explore the design

```sh
.build/SeedHop.app/Contents/MacOS/SeedHop --design-preview
make render
```

Design-preview mode uses labeled sample data and cannot issue transfer commands.
The renderer writes 22 light/dark main-window, compact, empty, import, file-selection,
settings, multi-selection, empty-filter, recovery, and high-contrast screenshots under `.build/design/`. It renders app-owned views,
not the user's screen. See [design system](docs/DESIGN.md).

## Verify

```sh
make test
make lint
make ux-test
make interface-test
cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/fixture
.build/SeedHop.app/Contents/MacOS/SeedHop --verify-engine "$PWD/.build/fixture"
python3 scripts/interop.py
make faults
make v2-test
```

Tests need local socket access. The XPC smoke test uses an isolated library
inside the supplied fixture directory. The interoperability suite checks
single-file and multi-file transfers in both directions plus magnet metadata
and selective downloading. Each case launches a fresh qBittorrent profile and
closes its test processes afterward; it does not use your normal profile.
Use `python3 scripts/interop.py --repeat 2` to repeat all five scenarios, or
`--direction download --case magnet` to target a case. Logs and hashes are
retained under `.build/interop-*`. See [interoperability results](docs/INTEROPERABILITY.md).

`make faults` tests unavailable destinations and interrupted peer/tracker connections,
then fills a disposable 32 MiB HFS+ disk image to exercise real disk-full errors.
It verifies transfer recovery, exact downloaded bytes, and durable settings retries,
then detaches and deletes the image. Logs remain under `.build/faults-*`.
See [recovery results and scope](docs/RECOVERY.md).

`make ux-test` runs deterministic Swift presentation regressions with delayed
engine replies. The XPC smoke test also exercises the actual import presentation
model against the Rust helper. `make interface-test` uses fixture data to invoke native menu actions and verify
search/table first responders and library reopening after the last window closes.
It does not synthesize system keyboard input or run real transfers.
These checks do not replace a live keyboard,
pointer, or VoiceOver walkthrough. See [UX review](docs/UX-REVIEW.md).

## Structure

- `crates/seedhop-core`: engine adapter, safe storage, state ownership, commands,
  fixtures, and recovery tests.
- `crates/seedhop-ffi`: typed UniFFI interface and runtime ownership.
- `macOS/App`: design system, native interface, XPC client, settings, and renderer.
- `macOS/Host`: shared engine host for foreground XPC and optional launch agent.
- `macOS/Resources`: app icon artwork; all macOS icon sizes are generated at build time.
- `vendor/rqbit`: pinned upstream snapshot with documented, reviewable patches.
- `scripts`: reproducible builds, app icon, and independent peer test.

The app icon and its generation prompt are documented in [icon artwork](docs/ICON.md).

The foreground host is the default. Background mode and login restoration are
separate opt-ins. Downloads are stored in your chosen folder; library state is
under `~/Library/Application Support/SeedHop`. The app does not enable a local
HTTP control server. [BitTorrent v2 support](docs/BITTORRENT-V2.md) includes pure-v2
and hybrid transfers, seeding, magnets, file selection, and SHA-256 restart checks.
Encrypted peers, RSS, search, remote control,
and advanced queueing remain roadmap work.

## License

Apache-2.0. Upstream notices are retained. Dependency licenses are listed in
[third-party notices](THIRD_PARTY_NOTICES.md).
