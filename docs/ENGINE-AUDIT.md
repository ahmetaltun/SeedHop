# Engine audit — rqbit 9.0.1

Baseline: `a499d2f243d124e144aef137afe7cb304a6e3f36`.
`vendor/SOURCE.json` records archive provenance. Local deltas are recorded in
`vendor/patches/0001-seedhop-reliability.patch`,
`vendor/patches/0002-seedhop-bep52.patch`,
`vendor/patches/0003-seedhop-metadata-guard.patch`,
`vendor/patches/0004-seedhop-file-controls.patch`,
`vendor/patches/0005-seedhop-v2-wire-completion.patch`,
`vendor/patches/0006-seedhop-utp-fallback.patch`,
`vendor/patches/0007-seedhop-hybrid-swarms.patch`,
`vendor/patches/0008-seedhop-peer-encryption.patch`,
`vendor/patches/0009-seedhop-magnet-proof-discovery.patch`,
`vendor/patches/0010-seedhop-legacy-payload-layers.patch`, and
`vendor/patches/0011-seedhop-dual-hash-magnets.patch` and
`vendor/patches/0012-seedhop-private-metadata-teardown.patch` and
`vendor/patches/0013-seedhop-metadata-wire-conformance.patch` and
`vendor/patches/0014-seedhop-metadata-resource-bounds.patch` and
`vendor/patches/0015-seedhop-v2-filename-mapping.patch` and
`vendor/patches/0016-seedhop-peer-interest-state.patch` and
`vendor/patches/0017-seedhop-authenticated-serving.patch` and
`vendor/patches/0018-seedhop-hash-rate-policy.patch` and
`vendor/patches/0019-seedhop-extension-discovery-policy.patch` and
`vendor/patches/0020-seedhop-lsd-subscription-lifetime.patch` and
`vendor/patches/0021-seedhop-legacy-layer-peer-state.patch` and
`vendor/patches/0022-seedhop-final-wire-boundaries.patch`; upstream's license is retained.

`vendor/patches/0024-seedhop-magnet-peer-scheduling.patch` keeps magnet discovery
streams polled through a bounded candidate queue and tries fresh peers before
retrying failed connections. The existing four shared metadata dictionary buffers,
per-peer retry limit, and private-metadata isolation remain in place.

`vendor/patches/0025-seedhop-magnet-probe-budgets.patch` separates up to 64
shared socket probes from the four dictionary buffers. Metadata-only socket and
read waits are capped at three seconds; useful metadata progress must continue
within five seconds. Handshake negotiation retains time for encryption and
TCP/uTP fallback, and waiting for a dictionary buffer does not count as peer
inactivity. Payload-transfer timeouts and encryption policy remain unchanged.
Validation is recorded in [`results/magnet-resolution-2026-10-28.json`](results/magnet-resolution-2026-10-28.json).

BEP 52, simultaneous hybrid swarms, MSE/PE encryption and TCP/uTP are implemented
and verified. The final checkpoint and supported-client bounds are recorded in
`BEP52-CONFORMANCE.md` and `results/v2-final-2026-10-26.json`. All twenty-two
patches reconstruct current source and reverse/reapply byte-identically.

## Reliability changes

1. `session.rs`: retain JoinHandles for session-owned tasks; expose
   `stop_and_wait`; await cancellation instead of sleeping for one second.
   Propagate metadata persistence and session deletion failures to callers.
   Retain the user-selected destination root when deleting torrent files.
2. `session_persistence/json.rs`: sync session data before atomic rename,
   sync the containing directory, atomically replace torrent metadata, roll back
   failed in-memory updates, and propagate metadata-file failures.
3. `storage/filesystem/fs.rs`: Unix file creation traverses directory
   descriptors with `openat`/`mkdirat` and `O_NOFOLLOW`, including intermediate
   directories. This closes a symlink race between preflight and file creation.
   File removal also uses descriptor-relative traversal and surfaces failures.

The session barrier covers session-owned tasks. Torrent/peer work also exists
outside that list; SeedHop therefore destroys the owned Tokio runtime before
releasing the library lease. A timeout is not used to abandon blocking disk work.

Fast resume is disabled in this milestone. Disk contents are rechecked after
restart instead of trusting stale piece maps. DHT persistence is disabled so
the engine does not write into rqbit's separate application directories.

## Application boundary

- Exclusive `flock` ownership precedes session restoration and file access.
- Session state is the authority for membership, selection, and user pauses.
  Internal shutdown pauses do not replace persisted user intent.
- Limits are saved by atomic replacement and syncing before acknowledgment.
- Unreadable JSON is retained in place; startup surfaces an actionable error.
- Existing destinations and metadata paths are checked before restore.
- Resume revalidates destinations before changing intent; an errored transfer
  restarts and rechecks even if its previous intent already said running.
- Explicit reuse is required for existing files; deletion enumerates torrent
  files and preserves unrelated files.

## Capabilities

| Capability | Pinned-source evidence | Milestone behavior |
|---|---|---|
| v1 metainfo and magnets | `librqbit_core/torrent_metainfo.rs`, `session.rs` | Enabled; metadata resolution cancellable |
| TCP, HTTP/UDP trackers | `listen.rs`, `tracker_comms` | Enabled |
| DHT | `dht`, `SessionOptions` | Enabled in app; disabled in deterministic tests |
| File selection, limits, pause | `session.rs`, `limits.rs` | Exposed through core and UI |
| Session persistence | `session_persistence/json.rs` | Enabled with reliability patches |
| Streaming primitives | `ManagedTorrent::stream` | Not exposed in this milestone |
| v2/hybrid | Shared BEP 52 metadata, dual hash verification, aligned storage, hash wire messages and magnet layers | Patch 0007 joins both hybrid swarms; patch 0009 separates metadata/proof discovery and fixes private trackers; patch 0010 derives layers from legacy payload; patch 0011 discovers metadata through either hash while enforcing both commitments in dual-hash magnets; patch 0012 stops public discovery immediately after private dictionary authentication and fixes duplicate recursive DHT RPCs. Final conformance and interoperability evidence is tracked in `V2-COMPLETION.md` |
| Encryption | Patch 0008, `mse.rs`, session/peer connection integration | MSE/PE on TCP/uTP; Allow (default), Prefer, Require and Disabled policies. Require rejects bare/header-only peers; independent interoperability is recorded in `V2-COMPLETION.md` |
| uTP | `listen.rs`, `librqbit-utp`, patch 0006 | App enables TCP/uTP; TCP handshake failure can retry uTP. Completion and independent evidence tracked in `V2-COMPLETION.md` |
| File priorities; rarest-first | Patch 0004 orders requests by persistent file priority | Low/Normal/High exposed; rarest-first remains separate work |
| Recheck, queueing, relocation, renaming | Patch 0004 and application storage transactions | Exposed across core, FFI/XPC, and native UI; see `FILE-MANAGEMENT.md` |
| NAT-PMP | No implementation established by audit | Not exposed |

## Remaining audit work

SOCKS/DNS coverage, interface loss, routed IPv6-only operation, and broader
disk/filesystem faults require dedicated fixtures before being advertised.
Controlled private-swarm suppression, loopback IPv6, UDP trackers, DHT discovery,
and hybrid transfers are covered in `V2-HARDENING.md`. Static review is not an
interoperability result. See `VALIDATION.md` for executed checks.

## Independent peer follow-up

The local qBittorrent suite now verifies plain v1 TCP transfers in both directions,
nested multi-file payloads, magnet metadata, and selective downloading. The five
scenarios passed twice against the installed qBittorrent v5.3.0alpha1 / libtorrent
2.1.0.0 build. Only the standalone lab runner and Python harness needed extension;
the production engine adapter and pinned upstream patches were unchanged.
See `INTEROPERABILITY.md` for the readiness finding, exact scope, and hashes.

## Recovery follow-up

The controlled recovery suite verifies missing destinations without recreating
their paths or altering saved state, active TCP interruption plus failed HTTP
tracker requests, and automatic reconnection with exact-byte completion.
A disposable 32 MiB HFS+ volume supplies actual ENOSPC for payload writes and
atomic limits persistence. Freeing space permits Resume and settings retry;
updated settings survive restart. These checks required two application Resume
fixes and no additional upstream patches. Physical drive removal, interface loss,
APFS faults, and sleep/wake remain separate gates. See `RECOVERY.md`.

## BEP 52 follow-up

The focused second patch adds v2 identities, aligned piece/file accounting,
SHA-256 download and restart checks, hash proof exchange, authenticated magnet
layers, and persisted full hashes. Nine qBittorrent cases pass for pure v2 and
hybrids through the v2 swarm, including uploads, downloads, selection and both
magnet identities. That checkpoint selected the v2 swarm; patch 0007 subsequently
adds simultaneous hybrid swarms and dual payload verification.
See `BITTORRENT-V2.md` and `results/v2-wire-2026-10-25.json`.

## V2 hardening review

The app listener now uses the engine's dual-stack binding instead of explicitly
restricting incoming peers to IPv4. Patch 0003 rejects repeated metadata data
after the info dictionary has completed; the former unwrap could panic a peer
task while v2 layer proofs were pending. The independent replay fixture requires
the rejection diagnostic, no panic, and exact-byte completion from a healthy
peer. The original BEP 52 patch remains intact and reviewable.

Review covered full and truncated identities at discovery boundaries, private
DHT/LSD/PEX suppression in source, correlated and root-authenticated magnet
proofs, SHA-256 payload checks, and persisted identity/selection checks. This is
a focused source review, not a complete security audit. Executed coverage and
remaining release gates are recorded in `V2-HARDENING.md`.


## Transfer and file controls

Patch 0004 adds in-place full rechecking, persistent file-priority order, queue
suspension separate from user pause, and local storage layouts. Hash completion
uses current pause intent, and fatal errors from old peer generations cannot
overwrite a later check, queue suspension, or layout change. Rechecks retain the
same managed torrent, full identity, selection, and persistent membership.

The native JSON store retains queue state, priorities, and local names. Storage
opens use mapped names; removal also uses the mapped paths. Original metainfo
and peer piece offsets remain unchanged. File-priority changes rebuild live
request order; a recorded wire Request verifies priority ordering. The native
app does not use the optional PostgreSQL store.

Application storage transactions quiesce peers, exclusively copy and flush new
files, persist the new layout, and only then remove originals. Descriptor-relative
opens and cleanup reject symlink traversal. Copy/save failure retains source
bytes; interrupted cleanup may leave duplicates. See `FILE-MANAGEMENT.md`.

## Native diagnostics and accounting — 2026-10-28

Patch `0023-seedhop-native-diagnostics.patch` adds shared per-torrent payload
atomics, negotiated peer encryption and connection-generation IDs, and tracker
outcome callbacks. HTTP omitted swarm counts remain optional. UDP address-family
outcomes remain separate. Tracker observations do not alter announce scheduling
or merge swarm counts. Session shutdown now retains and drains live, PEX,
initialization, and fatal-error tasks before core accounting checkpoints.
Completed task handles are pruned without hiding join failures.

Core-owned statistics and protocol-v3 XPC presentation are documented in
`POWER-USER-WORKSPACE.md`. Controlled loopback tests validate received payload,
plaintext TCP connection telemetry, omitted tracker counts, preserved pause and
recheck totals, and restart continuity. Existing protocol regression tests remain
in the workspace suite; physical lifecycle and release gates remain separate.

## Transfer throughput — 2026-10-29

Patch `0026-seedhop-live-rate-limits.patch` replaces a captured limiter with an
observable limiter generation. Pending upload/download acquisitions wake when
settings or scheduled caps change. Already charged batches remain charged; only
the unfinished bytes use the replacement quota. Unlimited mode releases waiters.
The regression failed before the patch and passes afterward for both directions,
with replacement by either an increased cap or unlimited mode.

The application's default development build previously compiled the transfer
engine and RC4 stream cipher without optimization. A disk-free, network-free
16 MiB encrypted stream fixture measured 2.357 MB/s before package optimization
and 164.304 MB/s afterward. Development builds now optimize `librqbit`, `rc4` and
`cipher`, retaining debug symbols and assertions. Encryption policy, payload
verification, resource bounds and transport fallback remain unchanged.

See `TRANSFER-THROUGHPUT.md` for scope, reproduction and the blocked network gates.
