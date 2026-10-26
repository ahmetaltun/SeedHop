# Milestone 0 validation

Status: **implemented development foundation; full milestone acceptance pending**.
No public distribution or notarization was performed.

## Protocol completion checkpoint — 2026-10-26

The requested BEP 52, simultaneous hybrid v1/v2, MSE/PE encryption and TCP/uTP
scope is implemented and verified. The final source passes 94 workspace Rust
tests (one opt-in disk-image test ignored), three isolated LSD tests, 95
independent protocol cases and four rebuilt packaged-native XPC smokes. Ad-hoc
deep/strict signatures and bundled license bytes pass. See
[`BEP52-CONFORMANCE.md`](BEP52-CONFORMANCE.md) and
[`v2-final-2026-10-26.json`](results/v2-final-2026-10-26.json).

This closes protocol implementation, not milestone release acceptance. Remaining
release work is certificate-backed signing/notarization and service identity,
physical login/sleep/drive/interface recovery, then UI accessibility acceptance.
The user-deferred gates below remain deferred; no public distribution occurred.

## Transfer-control follow-up — 2026-10-25

Force Recheck, file priorities, basic queueing, relocation, and local renaming
are implemented across engine, FFI, protocol-v2 XPC, and native UI. The workspace
suite passed 41 tests (one opt-in disk-image test excluded), Clippy with warnings
denied, 17 Swift presentation scenarios, native menu/focus checks, and the expanded
XPC smoke test with exact moved/renamed bytes and restart. See
[`FILE-MANAGEMENT.md`](FILE-MANAGEMENT.md) for behavior, tests, and transaction limits.

The unrestricted keychain exposed an existing Apple Development identity that
was hidden from the sandbox. Before the user's request to skip signing arrived,
a separate certificate-backed fixture passed three foreground/background handoff
cycles with exact bytes and running/paused transfers. Simulated changed-login
scope correctly kept the engine idle with resume off and restored running intent
with resume on, while preserving paused transfers; same-session crash scope also
restored with login resume off. The fixture uses separate service/library IDs and
unregisters its agent afterward. This was an earlier build, before the subsequent
file-control additions; it does not certify the final build or actual logout/login.

**User-deferred checks:** further certificate-backed validation, actual
logout/login, sleep/wake, physical removable-drive loss, and interface loss.
No external physical drive was attached. Local signing configuration was reset
to ad-hoc, and the final normal app is ad-hoc signed. MSE/PE encryption has since
been implemented and independently checked over TCP/uTP.
The subsequent uTP integration and protocol work are tracked in
`V2-COMPLETION.md`. The deferred gates remain open.

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
| Rust tests | PASS | 41 active tests across storage, recovery, faults, queue/file management, swarm, and v2 suites; opt-in disk-image test excluded from this follow-up |
| Rust formatting and Clippy | PASS | Workspace checks with warnings denied |
| Authenticated foreground XPC | PASS, ad-hoc | In-app `--verify-engine` smoke test |
| XPC metadata, paused add, limits, restart, restore, removal | PASS | Isolated smoke-test library |
| Import presentation model against real XPC/Rust | PASS, ad-hoc | Off-main file reading, real metadata, and paused addition through `ImportSession` |
| Swift UX presentation regressions | PASS | 17 controlled-transport scenarios; not a native input or accessibility certification |
| Native menu actions and focus | PASS, fixture window | Find focuses NSSearchField; Focus Transfer List focuses NSTableView; Show Library reopens after the last window closes; system key events and real transfers excluded |
| Exclusive ownership across processes | PASS | Competing startup rejected while child owns lease |
| Abrupt process death and restoration | PASS | Child killed after durable commands; lock and state recovered |
| Persistence failures reach callers | PASS | Read-only session directory fault |
| Corrupted JSON retained | PASS | Original malformed bytes preserved |
| Existing files protected; unrelated files retained | PASS | Explicit-reuse and deletion tests |
| Unsafe paths and symlink traversal preflight | PASS | Temporary-directory fixtures |
| Unavailable destination and retry | PASS | Missing add folder; paused destination disappearance; rejected restore preserves session bytes; reconnect and exact-byte download |
| Real disk-full payload and state recovery | PASS | Disposable 32 MiB HFS+ image; actual ENOSPC reaches snapshots/callers; Resume and settings retry verified |
| Interrupted peer and tracker connections | PASS | Active TCP relay connections closed; tracker requests rejected; automatic reconnection and exact-byte completion |
| Multi-file downloading | PASS | Controlled tracker swarm, nested paths, exact byte comparison |
| Metadata cancellation | PASS | Genuinely pending tracker request cancelled explicitly |
| Independent qBittorrent interoperability | PASS, both directions | 5 scenarios repeated twice against qBittorrent v5.3.0alpha1; 6,947,698 bytes compared exactly |
| Independent magnet metadata and file selection | PASS | qBittorrent supplied metadata; selected nested files verified; piece-aligned unselected file progress remained zero |
| v2/hybrid TCP, seeding, magnets and selection | PASS | Nine isolated qBittorrent scenarios; full identity, proofs, alignment and restart regression coverage |
| Visual rendering | PASS | 26 light/dark library, compact, import, files, priorities/rename inspector, queue settings, recovery, selection, and high-contrast renders |

Independent transfer SHA-256:
`631b84027d6b9e52b539c4e8373622d23032dfadc64d60af87339c9037e4f769`.
`scripts/interop.py` creates a separate qBittorrent profile and retains logs
under `.build/interop-*`; the summary is `.build/interop-result.json`.
The user's normal qBittorrent profile is not used.

The initial reverse-direction attempt failed. The follow-up reproduced a startup
interval where qBittorrent reported complete data but rejected plain handshakes.
The harness now waits for a valid 68-byte handshake with the expected info hash.
Both transfer directions, multi-file transfers, and magnet/selective downloading
then passed in two complete repeated suites. This does not establish the internal
cause of every historical reset; no production engine protocol patch was needed.
See `INTEROPERABILITY.md` and `results/interop-2026-10-25.json` for evidence and scope.

The recovery follow-up found and fixed two Resume issues: missing destinations
were accepted, and running intent prevented errored transfers from restarting.
`scripts/faults.py` now exercises both cases, real disk-full payload/state writes,
and interrupted peer/tracker connections. See `RECOVERY.md` and
`results/recovery-2026-10-25.json` for retained results and scope.

The UX follow-up and manual walkthrough checklist are in `UX-REVIEW.md`.
Cancellation and stale replies, action-target stability, removal confirmation,
overlapping file saves, duplicate adds, and file-read retries have executable
Swift regression coverage. These checks use a controlled transport; the separate
XPC smoke check exercises the real import model and Rust helper. Native UI
automation is unavailable in the current environment, so pointer/keyboard and
VoiceOver completion remain unverified. The subsequent app-owned native menu
checks verify search/table focus and fixture-window reopening; they do not
certify physical keyboard input, VoiceOver, or reopening with a real transfer.

The October 26 encryption build initially failed its foreground XPC smoke.
Crash reports subsequently identified stack exhaustion while constructing the
uTP dispatcher on the host dispatch queue, after successful authentication.
FFI initialization now runs on a Tokio worker and passes a 512 KiB host-stack
regression. The rebuilt ad-hoc app passes deep/strict signature verification
and both pure-v2/hybrid XPC import, recheck, priority/queue, rename/relocation,
exact-byte, restart and shutdown checks. See
[the evidence](results/v2-native-stack-2026-10-26.json). That native checkpoint
passed 76 Rust tests (54 v2), with one opt-in disk-image test ignored.
The filename checkpoint additionally verifies regular and raw-name pure-v2/hybrid
native imports and a byte-identical bundled Unicode data license; see
[the filename evidence](results/v2-filenames-2026-10-26.json).
The later engine serving checkpoint passed 84 Rust tests (62 v2), including
metadata-boundary, verified-hash and gated verification-race regressions; see
[the serving evidence](results/v2-serving-2026-10-26.json). Its native package
will be rebuilt after the remaining engine fixes. The hash-rate checkpoint
passes 87 Rust tests (65 v2), including shared rate and low-burst handling; see
[the hash-rate evidence](results/v2-hash-rate-2026-10-26.json).
The extension-policy checkpoint passes 91 Rust tests (69 v2), including
private PEX suppression and repeated extension enable/disable/re-enable; see
[the extension evidence](results/v2-extensions-2026-10-26.json).
The LSD checkpoint passes 92 workspace Rust tests (70 v2), plus three
production-source LSD tests for shared subscriptions, worker lifetime and full
queue isolation; see [the LSD evidence](results/v2-lsd-2026-10-26.json).
The legacy-fetch checkpoint retains 92 passing workspace tests and adds five
independent causal cases plus sixteen legacy regressions; see
[the legacy-flow evidence](results/v2-legacy-flow-2026-10-26.json).
Certificate-backed signing and physical lifecycle validation remain deferred.

## Remaining acceptance gates

- Validate the final build's certificate-backed lifecycle, revoked permission,
  and helper updates. Further signing work was deferred by the user; earlier
  isolated handoff results are recorded above.
- Exercise logout/login with automatic resume both off and on. Scoped boot/login
  run intent is implemented but not certified by the current tests.
- Complete keyboard-only and VoiceOver walkthroughs, Increase Contrast,
  Reduce Motion, reopening during real transfers, and long-library scrolling checks.
- Test physical removable-drive disappearance, network interface loss, and
  sleep/wake on the target system. Controlled missing paths, TCP/tracker outages,
  and actual disk-full writes on a disposable HFS+ image now pass; these do not
  certify physical unplugging, APFS behavior, or system sleep/wake.
- Validate routed IPv6-only networks and public-swarm discovery. Controlled
  private DHT suppression, local DHT/UDP discovery, IPv6 transfers in both
  directions, malicious proofs, corrupt peer data, and partial restart now have
  dedicated v2/hybrid fixtures. See `V2-HARDENING.md`. MSE/PE encryption on TCP/uTP
  has dedicated interoperability and hostile-handshake checks; remaining uTP
  network/recovery validation is tracked in `V2-COMPLETION.md`.
- Complete the user-deferred physical and actual login checks before declaring
  recovery acceptance complete. Force Recheck, priorities, basic queueing,
  relocation, and renaming are implemented; see `FILE-MANAGEMENT.md`.

## Reproduce

```sh
make build
make test
make lint
make ux-test
make interface-test
make render
cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/fixture
.build/SeedHop.app/Contents/MacOS/SeedHop --verify-engine "$PWD/.build/fixture"
python3 scripts/interop.py
python3 scripts/interop.py --repeat 2
make faults
```

Native rendering, local sockets, and macOS service access require execution
outside restrictive tool sandboxes. Screenshots render app-owned views rather
than capturing the user's screen. Visual review does not replace accessibility tests.
