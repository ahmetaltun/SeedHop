# BitTorrent v2 implementation

SeedHop imports, downloads, seeds, selects files, pauses/resumes, and restores
pure v2 and hybrid torrents. Both `.torrent` files and `btmh` magnets are supported.
Hybrid torrents use the v2 swarm and SHA-256 verification; this implementation
does not join both swarms simultaneously or claim simultaneous SHA-1/SHA-256
payload verification. Existing v1 torrents retain their v1 behavior.

Specification: [BEP 52](https://www.bittorrent.org/beps/bep_0052.html).
The focused engine delta is `vendor/patches/0002-seedhop-bep52.patch`, applied after
the reliability patch to the pinned rqbit revision.

## Engine integration

- Shared BEP 52 metadata, identity, Merkle proof and piece verification code lives
  in `vendor/rqbit/crates/librqbit_core/src/v2.rs`; the application reexports it.
- Full 256-bit identities are exposed to the app and saved in JSON session state.
  Only tracker/DHT/peer-wire lookup uses the required truncated 20-byte identity.
  A truncated-identity collision cannot silently return an unrelated torrent.
- File offsets follow v2 piece alignment, while piece sizes and progress exclude
  alignment gaps. Empty files occupy no pieces. File operations use authoritative
  absolute offsets and never read gap bytes from another file.
- Initial checks, downloads and restart checks verify SHA-256 Merkle pieces with
  a bounded 16 KiB read buffer, rather than allocating an entire piece in memory.
- The peer codec supports hash request (21), hashes (22), hash reject (23), and
  reject (16). Hash serving covers block and piece layers plus required uncle
  hashes. Requests are bounded to 512 base hashes and checked before allocation.
- Magnet metadata verifies the complete expected SHA-256 info hash. Missing
  external piece layers are requested in bounded ranges; each response must
  correlate with an outstanding request and prove its hashes against a root in
  the authenticated info dictionary. The complete metainfo, including layers,
  is persisted for restoration and metadata serving.
- Legacy `btih` hybrid magnets verify the original SHA-1 metadata commitment
  before upgrading to the derived SHA-256 identity. Hybrid peer handshakes may
  negotiate a v2 identity; padded final-block upload requests return virtual
  zero bytes without crossing into the next file.

Metadata parsing retains the 32 MiB input limit, 64-level nesting limit,
200,000-value limit, checked size arithmetic, shared identical-file layers, and
safe path limits (255 bytes/component, 4096 bytes/path, 32 MiB aggregate paths).
Symlinks and invalid UTF-8 names are rejected on this platform.

## Executed validation — October 25, 2026

All 32 active Rust tests pass, including 19 dedicated v2 tests. The separately
invoked disk-image fault test remains ignored in the ordinary Rust suite.
Coverage includes short blocks, per-file boundaries, empty-only torrents,
selection, full identity persistence, duplicate detection, corruption detected
by restart recheck, retained-file removal, malformed metadata and paths, shared
piece layers, multi-range magnet proofs, unsolicited/corrupt/truncated proofs,
and hash-wire messages split at every ring-buffer boundary.

`scripts/v2_interop.py` uses independent Python fixtures and an isolated
qBittorrent v5.3.0alpha1 / libtorrent 2.1.0.0 profile for each scenario. All nine pass:

| Format | Independent scenarios |
|---|---|
| Pure v2 | Download, upload, btmh magnet, selective download |
| Hybrid through v2 | Download, upload, btmh magnet, selective download, legacy btih magnet |

Every complete transfer compares all real file bytes exactly. Selective cases
compare the selected file and require zero progress for unselected files.
The fixtures include short files, nested paths, an empty file and duplicate
content sharing a pieces root. The user's normal qBittorrent profile is not used.

The rebuilt native app also passes its authenticated XPC import/pause/limits/
restart/restore/removal checks with both pure-v2 and hybrid fixtures. Twenty-two
app-owned light/dark fixture screens render successfully. The five existing
independent v1 cases also pass after the engine changes.
Retained results: `docs/results/v2-wire-2026-10-25.json` and
`docs/results/v1-after-v2-2026-10-25.json`.

```sh
make v2-test
python3 scripts/v2_interop.py --direction magnet --format pure
# Standalone metadata and block verification tools:
python3 scripts/v2_validation.py --without-qbit
cargo run -p seedhop-core --bin seedhop-lab -- inspect-v2 FILE.torrent
cargo run -p seedhop-core --bin seedhop-lab -- verify-v2-piece FILE.torrent INDEX PIECE.bin
```

## Scope

These are controlled TCP/HTTP-tracker interoperability results, not a claim that
every tracker, DHT network, peer extension or public swarm has been certified.
The existing DHT and UDP-tracker paths use the truncated v2 identity but have not
received independent live-v2-swarm validation. Encrypted peers, experimental uTP,
priorities, relocation and renaming remain separate work. Hybrid v1-only peers
cannot exchange payload with the selected v2 swarm. UI accessibility and
certificate-backed background lifecycle gates in `VALIDATION.md` remain open.
