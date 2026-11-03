# Security hardening checkpoint — 2026-11-03

This increment implements the first security boundaries from the privacy research.
It is not a certification of anonymity or a completed independent security audit.
Two real IKEv2 disconnect/reconnect rounds have now run with user authorization.
The bounded results and capture limitations are recorded in
`results/ikev2-validation-2026-11-03.json`; broader VPN acceptance remains open.

## Delivered behavior

### Network protection

- Settings → Transfers can require an available VPN interface and a numeric DNS
  server. Direct routing remains the default for existing installations.
- The engine loads and validates the policy before restoring a session. The
  selected name, interface index, and address set must match. Interface changes
  latch the engine into a blocked state; restarting requires an explicit action.
- Peer TCP/uTP, listeners, DHT, HTTP trackers, UDP trackers and torrent URL imports
  use the selected interface. Protected DNS uses TCP to the chosen server through
  that interface, including HTTP hostnames, UDP trackers and DHT bootstrap names.
  There is no system-resolver fallback in protected mode.
- Saving first drains session-owned network work. The new policy applies through
  the existing process-exit/reconnect barrier. Settings remain available when
  startup is blocked, without starting an unprotected session to change them.
- Environment proxies are disabled. Explicit proxy configurations are rejected
  when combined with protected routing/destination policy until their coverage
  is verified. No proxy privacy mode is advertised.
- Updates are a separate application connection. They are not covered by the
  torrent interface setting, and the Settings explanation says so.

An interface name is not proof of a VPN provider's identity or trustworthiness.
This implementation pins the selected interface and addresses, not a provider
account. It has controlled loopback and signed XPC evidence; it does not yet have
real VPN packet-capture evidence across macOS sleep, interface reuse, split
tunneling, or IPv6-only routes. Do not claim universal leak prevention.

### Untrusted destinations and input

- Local and reserved destinations are blocked by default for peer connections,
  incoming peers, DHT traffic, UDP trackers, HTTP trackers, and torrent URL
  imports. DNS answers are filtered before connection, and redirects are checked
  again. HTTPS redirects cannot downgrade to HTTP in this restricted mode.
- The explicit “Allow local trackers and peers” option permits trusted LAN use.
  Test/lab callers opt in explicitly through `EngineConfig.allow_local_network`.
- HTTP tracker bodies are limited to 2 MiB; remotely fetched torrent metadata is
  limited to 32 MiB. XPC requests are limited to 48 MiB and eight in-flight
  requests. The existing signing requirements authenticate both ends of XPC.
- Fuzzing found a 108-byte metainfo input that overflowed `atoi` while parsing an
  integer. Bencoded integers and byte-string lengths now use checked parsing;
  the input is retained as a regression and curated fuzz seed. Containers are
  limited to 64 nesting levels to bound parser stack use.
- Metadata allocation permits are dropped after their dictionaries and pending
  proof buffers. The existing four-dictionary shared budget remains in place.

The address restriction is conservative, including transition/documentation
ranges. Some specialized LAN and IPv6 translation setups require further work.
It is an outbound destination boundary, not a replacement for App Sandbox.

### Files and local privacy

- New payload files and nested directories receive macOS quarantine before
  payload writes. Copies preserve existing quarantine; rename keeps its inode
  metadata. A destination that cannot retain quarantine fails rather than
  silently losing the marker.
- Mutable payload opens reject symlinks, non-regular files, and pre-existing
  multiply linked files. Non-blocking opens avoid hanging on a FIFO.
- New payload files use mode 0600 and new nested directories use 0700. Existing
  payload permissions are preserved. Application-owned state directories are
  owner-only; atomic settings writes enforce mode 0600.
- Notifications omit filenames by default. Users can explicitly enable them.
  Host system-log messages omit raw error descriptions. Existing diagnostic
  exports continue to use allowlisted fields.

Gatekeeper interaction when a user opens a downloaded application, unsupported
filesystems, broad filesystem race testing, private-session history, and a full
retention/backup review remain separate acceptance work. Quarantine is not an
antivirus verdict.

### Dependencies and release checks

- `crypto-bigint` is upgraded from 0.5.5 to 0.7.10 for RUSTSEC-2026-0387. The MSE
  adapter uses the new fixed-width Montgomery API and constant-time exponent
  path. Variable-time parameter construction uses only the public fixed modulus.
- The pinned uTP 0.7.0 source is vendored to require `ringbuf` 0.5.2 for
  RUSTSEC-2026-0293. Its wire implementation is unchanged. Provenance is in
  `vendor/UTP-SOURCE.json`; rqbit deltas remain recorded as patches.
- The native lockfile audit reports zero known vulnerabilities. The remaining
  `basic-toml` unmaintained-package warning is not suppressed.
- Pull requests, `dev`/`master` pushes, and a weekly GitHub workflow run dependency
  scanning, native boundary tests, Clippy, and bounded torrent/DNS parser fuzzing.
  Workflow activation and hosted execution require pushing these changes.
- Releases include a lockfile dependency inventory in their checksummed assets.
  This inventory includes development/build/platform dependencies; it is not a
  claim that every listed package is present in the shipped binary, nor a full
  SPDX/CycloneDX SBOM or signed build-provenance attestation.

## Compatibility and recovery

Native protocol version is 10. Existing libraries retain their torrent state,
pause intent, file priorities, queue order and encryption preference.

Explicitly saving network settings writes a version-2 connection-settings guard
before writing the route policy. Previous clients accept only version 1 and will
therefore refuse to start instead of ignoring protection. A pending-save marker
blocks new clients after an interrupted save. It clears only after a complete,
durable policy write. Peer-encryption changes preserve this guard.

After an explicit network-policy save, missing, corrupt, unavailable or partially
saved protection never selects a
weaker route automatically. Use the current app's Settings to save a new choice.
Do not manually delete the guard or downgrade its version to bypass the failure.
A storage failure that prevents even the initial guard write cannot establish a
durable new preference; the save reports failure and the current engine stays
stopped. No successful-save claim is made in that case.

## Validation and remaining order

Executed results are recorded in `results/security-hardening-2026-11-03.json`.
Controlled tests cover routing policy persistence, missing interfaces, stopped
listeners, LAN opt-in, quarantine/copy behavior, hard links/FIFOs, parser overflow,
encryption interoperability, v2/hybrid transfers, and native presentation.
Certificate-signed XPC fixtures use a separate application identity and library;
public discovery is disabled in those fixtures.

The next unfinished boundaries are:

1. Extend the initial IKEv2 captures to process-attributed captures, IPv6,
   sleep/wake, split tunneling, and failures where the interface remains present.
   Keep provider-wide and “no leaks” release claims gated on those results.
2. App Sandbox production activation. An isolated development implementation now
   includes folder grants, shared storage, migration and rollback; see
   [sandbox migration](SANDBOX-MIGRATION.md). App Sandbox remains **disabled in
   production**. Runtime tests are deferred. Signed login/background, relocation,
   external-drive, preference migration and update acceptance must pass before
   enabling it for existing users.
3. Complete retention controls/private sessions. Development-only
   [local activity clearing](LOCAL-PRIVACY.md) now covers engine events, app
   timelines and native recent-item/notification cleanup. Separate development
   controls now offer optional saved-statistics retention with a version-4 guard
   and pending-purge recovery; resume metadata remains. Runtime acceptance is
   deferred. Formal SBOM,
   independently verifiable build provenance and release-key recovery exercises
   also remain.
4. Strict proxy mode only after DNS/UDP/incoming coverage is proved; unsupported
   transports must remain blocked without direct fallback.
5. Independent security review and public disclosure/response procedures. No
   independent audit has been commissioned or represented as complete.

Native pointer/keyboard and VoiceOver walkthroughs remain manual. No production
library, VPN connection, login registration, or published release was changed by
the controlled validation fixtures. In the separate real IKEv2 test, the user
disconnected and reconnected their VPN twice. That test used isolated engine
state and downloads; it did not change production library preferences.

## Primary references

- [Apple networking: interface binding](https://developer.apple.com/forums/thread/798519)
- [Apple App Sandbox](https://developer.apple.com/documentation/security/protecting-user-data-with-app-sandbox)
- [Apple quarantine keys](https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/LaunchServicesKeys.html)
- [BEP 27: private torrents](https://www.bittorrent.org/beps/bep_0027.html)
- [OWASP SSRF prevention](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
- [Sparkle security changes](https://sparkle-project.org/documentation/security-and-reliability/)
- [RustSec advisory database](https://github.com/RustSec/advisory-db)
