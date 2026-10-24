# Milestone 0 validation

Status: **implemented development foundation; full milestone acceptance pending**.
No public distribution or notarization was performed.

## Environment

Apple Silicon; macOS 15.8.1; Xcode 26.3; Swift 6.2.4 compiler in Swift 5 mode;
Rust 1.99.1; UniFFI 0.32.2; Tokio 1.54.1; rqbit 9.0.1 at the recorded revision.
Local builds use ad-hoc signatures with Hardened Runtime. A debug-only library
validation exception is omitted from certificate-backed and release builds.

## Executed checks

| Check | Result | Evidence |
|---|---|---|
| App, foreground service, background agent build | PASS | `scripts/build.py` |
| Nested code signature verification | PASS, ad-hoc | `codesign --verify --deep --strict` |
| Rust tests | PASS | 10 tests across storage, recovery, and controlled swarm suites |
| Rust formatting and Clippy | PASS | Workspace checks with warnings denied |
| Authenticated foreground XPC | PASS, ad-hoc | In-app `--verify-engine` smoke test |
| XPC metadata, paused add, limits, restart, restore, removal | PASS | Isolated smoke-test library |
| Import presentation model against real XPC/Rust | PASS, ad-hoc | Off-main file reading, real metadata, and paused addition through `ImportSession` |
| Swift UX presentation regressions | PASS | 13 controlled-transport scenarios; not a native input or accessibility certification |
| Exclusive ownership across processes | PASS | Competing startup rejected while child owns lease |
| Abrupt process death and restoration | PASS | Child killed after durable commands; lock and state recovered |
| Persistence failures reach callers | PASS | Read-only session directory fault |
| Corrupted JSON retained | PASS | Original malformed bytes preserved |
| Existing files protected; unrelated files retained | PASS | Explicit-reuse and deletion tests |
| Unsafe paths and symlink traversal preflight | PASS | Temporary-directory fixtures |
| Multi-file downloading | PASS | Controlled tracker swarm, nested paths, exact byte comparison |
| Metadata cancellation | PASS | Genuinely pending tracker request cancelled explicitly |
| Independent qBittorrent interoperability | PASS, seeding direction | SeedHop → qBittorrent v5.3.0alpha1, 1 MiB |
| Visual rendering | PASS | 18 light/dark library, compact, empty, import, files, settings, multi-selection, filter, and recovery renders |

Independent transfer SHA-256:
`631b84027d6b9e52b539c4e8373622d23032dfadc64d60af87339c9037e4f769`.
`scripts/interop.py` creates a separate qBittorrent profile and retains logs
under `.build/interop-*`; the summary is `.build/interop-result.json`.
The user's normal qBittorrent profile is not used.

The reverse-direction qBittorrent seed attempt did not pass: the local test
instance closed even a separate Python plain-v1 handshake probe. Its cause
is not established. Downloading is verified by the controlled multi-file swarm;
bidirectional qBittorrent interoperability must not be inferred from the seeding test.

The UX follow-up and manual walkthrough checklist are in `UX-REVIEW.md`.
Cancellation and stale replies, action-target stability, removal confirmation,
overlapping file saves, duplicate adds, and file-read retries have executable
Swift regression coverage. These checks use a controlled transport; the separate
XPC smoke check exercises the real import model and Rust helper. Native UI
automation is unavailable in the current environment, so pointer/keyboard and
VoiceOver completion remain unverified.

## Remaining acceptance gates

- Configure an Apple Development identity and validate background registration,
  repeated host handoffs, revoked permission, and helper updates with certificate
  signatures. No valid identity was visible during setup.
- Exercise logout/login with automatic resume both off and on. Scoped boot/login
  run intent is implemented but not certified by the current tests.
- Complete keyboard-only and VoiceOver walkthroughs, Increase Contrast,
  Reduce Motion, window reopening, and long-library scrolling checks.
- Test actual disk-full faults, removable-drive disappearance, network interface
  loss, and sleep/wake on the target system.
- Add private-swarm, v2/hybrid, IPv6-only, and encrypted-peer fixtures before
  advertising those capabilities.

## Reproduce

```sh
make build
make test
make lint
make ux-test
make render
cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/fixture
.build/SeedHop.app/Contents/MacOS/SeedHop --verify-engine "$PWD/.build/fixture"
python3 scripts/interop.py
```

Native rendering, local sockets, and macOS service access require execution
outside restrictive tool sandboxes. Screenshots render app-owned views rather
than capturing the user's screen. Visual review does not replace accessibility tests.
