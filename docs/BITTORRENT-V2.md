# BitTorrent v2 implementation

The requested protocol scope is implemented and verified: BEP 52, simultaneous
hybrid v1/v2 swarms, MSE/PE encryption, TCP and uTP. The final October 26 checkpoint
passes 94 workspace Rust tests, three isolated LSD tests, 95 independent protocol
cases and four rebuilt native XPC smokes. One opt-in disk-image test remains
ignored. See [the requirement audit](BEP52-CONFORMANCE.md),
[implementation checkpoints](V2-COMPLETION.md) and
[final evidence](results/v2-final-2026-10-26.json). Twenty-two ordered vendor
patches reconstruct current source and reverse/reapply byte-identically.
Certificate-backed signing, physical lifecycle/network recovery and UI
accessibility remain separate release gates.

SeedHop imports, downloads, seeds, selects files, pauses/resumes, and restores
pure v2 and hybrid torrents. Both `.torrent` files and `btmh` magnets are supported.
Hybrid torrents announce both identities and share storage across v1 and v2
connections. Every hybrid piece must pass both SHA-1 and SHA-256 verification
before it is advertised or counted as complete. Existing v1 torrents retain
their v1 behavior.

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
  Hybrid checks also verify SHA-1, synthesizing the validated BEP 47 padding
  without storing padding files or reading bytes from adjacent files.
- The peer codec supports hash request (21), hashes (22), hash reject (23), and
  reject (16). Hash serving covers block and piece layers plus required uncle
  hashes. Requests are bounded to 512 base hashes and checked before allocation.
- Magnet metadata verifies the complete expected SHA-256 info hash. Missing
  external piece layers are requested in bounded ranges; each response must
  correlate with an outstanding request and prove its hashes against a root in
  the authenticated info dictionary. The complete metainfo, including layers,
  is persisted for restoration and metadata serving.
- Legacy `btih` hybrid magnets verify the original SHA-1 metadata commitment
  before discovering proof providers using the derived SHA-256 identity. The
  metadata peer can be strictly v1-only; files no larger than a piece need no
  external proof connection. When no v2 proof provider answers, btih hybrids
  can derive missing layers from SHA-1-verified legacy payload, checking complete
  layers against the committed SHA-256 file roots before accepting the metainfo.
  Dual-hash magnets discover metadata through either swarm and check both full
  commitments, plus the hybrid format, before requesting proofs or payload.
  Hybrid peer handshakes may
  negotiate a v2 identity; padded final-block upload requests return virtual
  zero bytes without crossing into the next file.

Metadata parsing retains the 32 MiB input limit, 64-level nesting limit,
200,000-value limit, checked size arithmetic, shared identical-file layers, and
safe path limits (248 bytes per newly mapped component, 4096 bytes/path, and
32 MiB aggregate paths). Invalid UTF-8 and reserved names are mapped to safe
components; unsupported symlink storage is rejected on this platform.

## Executed validation — October 25, 2026

All 35 active Rust tests pass, including 22 dedicated v2 tests. The separately
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

The additional controlled discovery and hostile-peer results are documented in
[V2 hardening](V2-HARDENING.md). UDP trackers, local DHT discovery, IPv6 transfers
in both directions, and private torrents have dedicated fixtures. These do not
certify a public DHT swarm, every tracker, or all peer extensions. The subsequent
[completion work](V2-COMPLETION.md) enables uTP and validates the nine independent
scenarios over forced uTP as well as TCP. Patch 0008 adds MSE/PE encryption and
four explicit policies on both transports; see the completion record. Priorities, queueing,
Force Recheck, relocation, and renaming are covered in `FILE-MANAGEMENT.md`.
Hybrid v1-only peers can upload and download payload when authenticated v2
metadata/layers are available. Btih hybrid magnets also resolve through an
exclusively legacy swarm by deriving missing layers from payload. Dual-hash
magnets also discover metadata through either swarm. The completed conformance audit and separate network release gates are tracked
in `V2-COMPLETION.md`.
UI accessibility, physical sleep/drive recovery, and certificate-backed
background lifecycle gates in `VALIDATION.md` remain open.

Filename handling maps invalid/reserved components and filesystem aliases into
deterministic safe storage paths while preserving original info bytes and raw
hybrid-layout equality. Valid Unicode names stay readable. Complete versioned
path maps preserve restarts, and older sessions retain their existing files and
renames on upgrade. See the filename checkpoint in `V2-COMPLETION.md`.
