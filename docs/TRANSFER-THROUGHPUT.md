# Transfer throughput investigation — 2026-10-29

The reported symptom was approximately 1 MB/s from a magnet advertised with more
than 3,000 seeders. The initial investigation reproduced and fixed two engine
problems. A follow-up reproduced two more problems affecting upload availability
and speed presentation. Socket tests were initially blocked by approval-service
capacity; the service became available during the follow-up, allowing controlled
swarm tests and two bounded public-swarm samples. See the follow-up results below.

## Confirmed causes and changes

### Development builds imposed a substantial encryption CPU bottleneck

`make build` invokes `scripts/build.py` without `--release`. The script uses
Cargo's development profile, which previously left the transfer engine and RC4
stream cipher unoptimized. The saved local connection preference was `prefer`,
so encrypted payload performance is directly relevant, although the affected
connection's negotiated encryption could not be observed.

`encrypted_transport_throughput` transfers and verifies 16,777,216 bytes through
the production MSE/PE stream adapters over a bounded Tokio duplex stream. Both
ends execute the real cipher. Timing excludes negotiation and includes an
acknowledgment after complete receipt. It uses no disk or network sockets.

| Same fixture on this Apple Silicon machine | Elapsed | Decimal MB/s |
| --- | ---: | ---: |
| Original development profile | 7.120 s | 2.357 |
| Optimized engine and cipher packages | 0.102 s | 164.304 |

These are individual controlled measurements, approximately a 70-fold increase,
not public download speeds or a statistical latency distribution. A live peer
also pays for protocol processing, hashing, storage and network conditions.

`Cargo.toml` now sets development package optimization to level 2 for `librqbit`
and level 3 for `rc4` and `cipher`. Debug symbols and debug assertions remain
enabled. Release configuration and the user's encryption policy are unchanged.

### Existing transfers retained obsolete speed limits

An upload/download quota acquisition captured its limiter before waiting. Saving
a faster cap or removing the cap replaced the limiter for future acquisitions,
but an already waiting acquisition continued using the old quota. A 16 KiB block
waiting at 1 byte/s could therefore remain stalled for hours after unlimited mode
was selected. Upload scheduling and download requests both use this code; hash
responses use the same upload quota path.

The new regression starts a blocked 16 KiB acquisition and switches to either
unlimited or 1,000,000 bytes/s. It failed before the change and passes afterward
for both upload and download. The fix subscribes to limiter changes, cancels the
obsolete wait and applies the replacement quota to unfinished bytes. Batches
already charged are not charged again. Small limits still accept larger wire
blocks by charging them in batches.

Vendor patch: `vendor/patches/0026-seedhop-live-rate-limits.patch`.

## Other paths reviewed

| Area | Evidence and implication |
| --- | --- |
| Configured limits | Saved normal limits were `[0,0]`, meaning unlimited. No saved schedule existed. There is no default 1 MB/s cap. |
| Speed presentation | Estimators calculate bytes per second. The adapter's MiB/s-to-bytes conversion reverses the estimator conversion; it does not divide throughput by eight. |
| Download requests | The engine permits 128 outstanding 16 KiB blocks per peer, bounded further by advertised `reqq`. This is up to 2 MiB per peer, rather than a single-block round trip. |
| Peer availability | The default limit is 128 connections per torrent. Advertised swarm size does not show how many peers connected, unchoked this client, or supplied useful bytes. |
| Transport | TCP and uTP, encryption negotiation and fallback remain enabled. No transport was disabled to improve benchmark results. |
| Storage | Received blocks use positioned writes and per-piece hash verification. No unconditional file durability flush was found on every payload block. Real disk throughput remains unmeasured here. |
| Backpressure | Upload replies retain the existing per-peer response budget through socket writes. Quota batching and these bounds remain intact. |

Request pipelining and choke state are required parts of the
[BitTorrent peer protocol](https://www.bittorrent.org/beps/bep_0003.html).
No connection-limit increase, verification bypass, cipher-policy downgrade, or
speculative transport rewrite was needed for the confirmed fixes.

## Validation and reproduction

Passed checks:

- Four existing socket-free MSE tests: encrypted partial writes and initial
  payloads, encryption policy and invalid keys, header-only policy, malformed
  handshake fields.
- Existing low-burst upload/download regression.
- New pending-limit replacement regression, covering both directions and both
  replacement modes.
- The explicit encrypted transport throughput fixture and byte verification.
- Workspace Clippy across all targets with warnings denied, formatting and diff
  checks, and vendor patch reverse-application validation.
- The native development app rebuilt and passed deep, strict signature
  verification. `iconutil` rejected the regenerated iconset, so packaging reused
  the existing app icon; no icon or build-script source change was needed.

Commands from the repository root:

```sh
cargo test --offline --locked -p seedhop-core --test throughput changing_limits
cargo test --offline --locked -p seedhop-core --test throughput encrypted_transport_throughput -- --ignored --nocapture
cargo test --offline --locked -p seedhop-core --test mse -- --skip engine_binds_mse --skip magnet_metadata
cargo test --offline --locked -p seedhop-core --test v2 transfer_rate_limits_accept_blocks_larger_than_the_burst
```

The retained `loopback_throughput` fixture exercises a 16 MiB transfer between
real engines, including seeding, positioned writes, hashing and byte verification.
It could not execute because socket binding was denied. Run it after restoring
socket access:

```sh
cargo test --offline --locked -p seedhop-core --test throughput loopback_throughput -- --ignored --nocapture
```

The new lab command records one-second samples of payload totals, upload/download
rates, negotiated transport/encryption and peer state, then shuts the engine down
at the configured deadline. Use a disposable state/download directory; partial
payload files remain for inspection. The download directory must already exist.
Metadata resolution has a separate 30-second timeout. The transfer duration
defaults to 45 seconds and accepts 5–300 seconds.

```sh
cargo build --offline --locked -p seedhop-core --bin seedhop-lab
mkdir -p .build/throughput-public/downloads
SEEDHOP_TRANSFER_TIMEOUT_SECS=20 target/debug/seedhop-lab benchmark-magnet "$MAGNET" \
  "$PWD/.build/throughput-public/state" "$PWD/.build/throughput-public/downloads" \
  > .build/throughput-public/result.json
```

`benchmark-magnet` enables discovery and honors `SEEDHOP_PEER_ENCRYPTION`
(`allow`, `disabled`, `prefer`, `require`); the default is `allow`. Set it to
`prefer` when comparing against the saved local preference. Peer details in the
result can contain public endpoint addresses; raw results are kept under ignored
`.build/`, not committed.

Actual ISP capacity, VPN effects, router reachability and physical disk behavior
remain unverified. The public observations below cover two short runs on this
machine and do not establish universal or sustained public-swarm performance.

## Follow-up: bursty rates and zero uploads

### Empty peers were incorrectly excluded from HAVE announcements

An empty peer may omit its initial bitfield under BEP 3 and BEP 52. The engine
treated an absent bit as if the peer already had that piece, suppressing HAVE
announcements as new pieces finished verification. Such a peer could remain
unaware that SeedHop had anything to upload. The fix suppresses an announcement
only when the peer actually advertised that piece; missing availability is no
longer interpreted as possession.

The real-wire regression failed before the fix waiting for a HAVE. It now passes
for v2, hybrid/v2 and hybrid/v1 connections, both with an omitted bitfield and an
explicit empty bitfield. Each case downloads and verifies new pieces in SeedHop,
checks the empty peer receives HAVE, then requests a block from SeedHop and checks
the returned bytes and upload accounting.

### The five-second rate estimator was being fed ten samples per second

The per-torrent updater supplied samples every 100 ms to an estimator retaining
five per-second samples. This reduced its steady-state window to about half a
second. Ordinary block bursts therefore produced large peaks followed by zeros,
including while the multi-second upload average remained nonzero. The session
updater already used a one-second cadence.

Per-torrent sampling now also runs once per second. Both download and upload
display their intended approximately five-second average. This changes reporting,
not the number of transferred bytes or the bandwidth available from remote peers.
A real-peer regression failed before the fix after a 1.5-second quiet gap. It now
checks the upload remains visible after that gap and returns to zero after six
seconds without further uploads; cumulative payload accounting remains exact.

These changes and peer diagnostics are retained in
`vendor/patches/0027-seedhop-upload-announcements-and-rate-cadence.patch`.
Peer diagnostics now expose remote interest, advertised piece count, pending
download requests, connection error count and piece-steal count. Existing Swift
decoders can ignore the added JSON fields.

### Controlled transfer and supplied-magnet observations

The 16 MiB real-engine loopback fixture completed in 0.162 seconds, reporting
103.797 MB/s. All 16,777,216 bytes were uploaded, downloaded, hashed and compared.
This verifies the local transfer paths; it is not an Internet throughput result.

Two fresh, isolated 20-second samples used the supplied magnet with encryption
set to `prefer`. The first run, before these follow-up fixes, downloaded
63,908,801 bytes and uploaded 327,680 bytes. Its half-second rates oscillated
between large bursts and lows. After the follow-up fixes, another fresh run
downloaded 93,026,178 bytes and uploaded zero bytes. Its final five-second download
rate was approximately 6.80 MB/s, with late samples around 6.67–7.38 MB/s.
The runs encountered different peers and startup conditions, so their byte totals
are not a controlled throughput improvement comparison.

In the latter run, every observed peer with an advertised bitfield had all 1,025
pieces in the torrent; metadata independently confirmed that piece count. Every
observed peer reported `interested=false`. Zero upload was therefore consistent
with no connected peer requesting SeedHop's data. Seeders already possessing the
complete torrent cannot create upload demand. A large seeder count does not imply
there will be downloaders needing this client's pieces.

Only two piece steals and no recorded errors among the observed live peer
snapshots appeared in the latter run. Pending downloads remained pipelined.
These observations do not support a speculative peer-scheduler rewrite.

Raw endpoint-bearing logs remain in ignored `.build/throughput-current/` and
`.build/throughput-followup/`. Aggregate results are in
`results/transfer-throughput-2026-10-29.json`. A long-duration public seeding test,
incoming reachability test and ISP/VPN comparison remain outside this evidence.

Follow-up validation passed all 84 tests across the v2/hybrid, MSE, diagnostics
and swarm integration suites, plus the explicit loopback throughput fixture.
Workspace Clippy across all targets with warnings denied, formatting, diff checks,
vendor patch reverse-application validation and the rebuilt app's deep, strict
signature verification also passed. Packaging reused the existing icon as above.
