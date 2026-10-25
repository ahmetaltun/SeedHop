# Engine audit — rqbit 9.0.1

Baseline: `a499d2f243d124e144aef137afe7cb304a6e3f36`.
`vendor/SOURCE.json` records archive provenance. Local deltas are recorded in
`vendor/patches/0001-seedhop-reliability.patch` and
`vendor/patches/0002-seedhop-bep52.patch`; upstream's license is retained.

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
| v2/hybrid | Shared BEP 52 metadata, SHA-256 verification, aligned storage, hash wire messages and magnet layers | Enabled through the v2 swarm; nine independent transfer/magnet/selection cases pass. See `BITTORRENT-V2.md` |
| Encryption | No MSE/RC4 handshake implementation found in reviewed protocol/connection sources | Roadmap gap |
| uTP | `listen.rs`, `librqbit-utp` | Experimental upstream; disabled here |
| Rarest-first and priorities | Upstream README documents sequential-only selection | Roadmap gap |
| Recheck, relocation, renaming | No corresponding public Session operation in pinned revision | Restart verification only; explicit operations deferred |
| NAT-PMP | No implementation established by audit | Not exposed |

## Remaining audit work

Private-swarm discovery suppression, SOCKS/DNS coverage, interface loss,
IPv6-only operation, broader disk/filesystem faults, and hybrid interoperability
require dedicated fixtures before being advertised. Static review is not an
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
magnet identities. Hybrids select v2 rather than simultaneous dual-swarm operation.
See `BITTORRENT-V2.md` and `results/v2-wire-2026-10-25.json`.
