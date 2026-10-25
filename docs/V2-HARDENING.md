# V2 hardening checkpoint — October 25, 2026

The next feature is explicit Force Recheck. Keep file priorities, queueing,
relocation, and renaming behind the remaining hardening checks. Encryption and
uTP are separate protocol projects. This checkpoint does not declare release
readiness.

## Changes

- Bind the app's TCP listener on the engine's dual-stack unspecified address.
  Its former explicit IPv4 binding prevented incoming IPv6 connections.
- Add an isolated discovery harness with independent Python BEP 15 UDP tracker
  and BEP 5 KRPC fixtures, plus IPv6 HTTP tracker responses. DHT fixtures have
  no trackers and advertise only the isolated qBittorrent seed. The lab's
  `probe-dht` command configures a local bootstrap on the same vendored Session
  discovery implementation used by the app; it does not change the app's DHT
  configuration or certify its public bootstrap hosts.
- Add patch `0003-seedhop-metadata-guard.patch` after the existing reliability
  and BEP 52 patches. Repeated metadata after the info dictionary completes now
  returns a peer error instead of unwrapping cleared state. An independent
  attacker replays the metadata before Merkle proofs complete. The fixture
  requires the rejection diagnostic, no panic, and exact bytes from a healthy
  qBittorrent peer afterward.
- Extend the controlled Rust TCP relay to mutate actual payload blocks and hash
  responses. Counters track injected frames written to the client. Both pure v2
  and hybrid tests verify rejection and recovery.

## Executed checks

| Coverage | Result and limits |
|---|---|
| UDP tracker discovery | Pure v2 and hybrid download exact bytes; logs verify the truncated SHA-256 wire identity |
| DHT discovery | Pure v2 and hybrid download exact bytes using an independent local KRPC node and qBittorrent seed; public swarm/bootstrap behavior remains unverified |
| IPv6 | Pure v2 and hybrid download and upload over `::1`, IPv6 HTTP tracker connections and `peers6`; incoming and outgoing peers covered, routed IPv6-only operation unverified |
| Private torrents | Pure v2 and hybrid exact-byte tracker downloads; with DHT enabled and bootstrapped, trackerless private torrents stay live and emit neither `get_peers` nor `announce_peer` during the observation window |
| Metadata replay | Both formats reject the replay without a panic, then finish through a healthy independent peer |
| Corrupt peer data | Both formats receive altered wire blocks without advancing verified progress, survive a TCP/tracker outage, and recover automatically |
| Partial restart | Both formats restart after an interrupted, partially verified transfer, retain nonzero partial progress, and finish with exact bytes |
| Malicious hash proofs | Both formats reject altered magnet proofs without files or membership; cancellation and a fresh valid attempt recover. Existing tests cover unsolicited, truncated and incorrect proof ranges |
| Missing destination | Both formats reject Resume and restore while a destination is missing, preserve saved session bytes, avoid recreating the path, and complete after reconnection. Directory rename simulates disappearance; this is not a physical unplug test |
| Existing v2 interoperability | Nine TCP/HTTP tracker download, upload, magnet and selection cases pass after these changes |
| Vendor patch integrity | Reverse all three patches in a temporary copy, reapply in order, and compare the resulting vendored tree byte-for-byte |

The independent peer is qBittorrent v5.3.0alpha1 / libtorrent 2.1.0.0.
Each case uses a new profile, with DHT/PEX/LSD disabled in qBittorrent and local
fixtures supplying discovery. The user's ordinary profile is not accessed.
Private PEX/LSD suppression is source-reviewed; dedicated malicious PEX and
multicast packet-capture tests remain outside this checkpoint. Private magnet
privacy before metadata is known is not certified.

The Rust workspace has 35 active tests, including 22 v2 tests; the existing
real-ENOSPC test remains ignored unless run on its disposable mounted image.
Formatting and Clippy with warnings denied pass. One v1 local-swarm test timed
out during an earlier run concurrent with multiple independent swarms; its
isolated rerun passed. The final retained workspace run is recorded separately.

Retained evidence: `results/v2-hardening-2026-10-25.json`. Packet logs, peer
settings, fixture files, downloaded bytes, and diagnostics remain at the listed
`.build/v2-hardening-*` paths. Rust and discovery logs are retained in `.build`.

## Release gates still open

- Actual macOS sleep/wake, physical drive removal during I/O, and interface loss.
  Controlled outages, missing paths, and process restart do not certify these.
- Certificate-backed background registration, repeated host handoffs, revoked
  permission, helper updates, and logout/login restoration with automatic
  resume both off and on. The current `security find-identity -v -p codesigning`
  check reports zero valid identities visible to this session.
- Accessibility and manual native interaction acceptance in `VALIDATION.md`.

Force Recheck has not been added in this checkpoint. After hardening, implement
it across engine, FFI/XPC, and UI with progress, cancellation/error handling,
selection and user-pause preservation. Then take file priorities, queueing,
relocation, and renaming in that order.

## Reproduce

```sh
make v2-hardening
python3 scripts/v2_hardening.py --mode dht
python3 scripts/v2_hardening.py --mode duplicate-metadata
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
make v2-test
```

Local socket and isolated GUI peer tests require execution outside a restrictive
sandbox. No public tracker/DHT test is implied by these results.
