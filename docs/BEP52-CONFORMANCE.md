# BitTorrent v2 completion audit

Scope: BEP 52 metainfo, discovery and peer-wire behavior; simultaneous hybrid
v1/v2 swarms; MSE/PE encryption; TCP and uTP. Optional extensions are included
where required by the supported magnet/discovery paths. Certificate-backed
signing, physical macOS lifecycle and UI accessibility remain separate release
gates in `VALIDATION.md`.

Primary references: [BEP 52](https://www.bittorrent.org/beps/bep_0052.html),
[BEP 9](https://www.bittorrent.org/beps/bep_0009.html),
[BEP 10](https://www.bittorrent.org/beps/bep_0010.html), and
[BEP 27](https://www.bittorrent.org/beps/bep_0027.html).
Status: **implemented and verified for the requested scope and documented client
bounds**. The final checkpoint passes 94 workspace Rust tests, three isolated LSD
tests, 95 independent protocol cases and four packaged native XPC smokes.
See [the final evidence](results/v2-final-2026-10-26.json). This is implementation
validation, not certification of every public network or physical release gate.
Historical source hashes and commands are in the dated result records linked
from `V2-COMPLETION.md`.

| Requirement | Evidence |
|---|---|
| Canonical bencoding and full original info bytes | `v2.rs` parser rejects noncanonical input; root v2 tests cover ordering, duplicate/truncated data and identity. |
| Meta-version checked before semantic layout validation | Patch 0022 checks the meta version before version-specific serde fields on both file imports and acquired info dictionaries. Future-version dictionaries with missing or malformed current-layout fields report the unsupported version; the parser and application preview have a regression test. |
| Piece length, file order, per-file alignment, empty files | Independent fixtures plus Rust transfer/recheck/selection tests. |
| File tree structure and symlink handling | File/directory collisions and unsupported symlink storage rejected. |
| Invalid UTF-8, reserved and unsafe filenames | Patch 0015 escapes unsafe/invalid bytes, shortens long components, and separates sibling names after normalization/full case folding. Valid Unicode remains readable; completed strings bypass legacy encoding and BOM stripping. Hybrid equality still uses raw bytes. Actual-filesystem and independent v1/v2 transfer tests cover traversal, reserved names, Unicode/case aliases, directories and virtual padding. Versioned complete path maps preserve restarts; older sessions migrate their original storage names without moving files. |
| Merkle roots, layer shape, short blocks, zero padding, duplicate roots | Independent tree fixtures, hostile proof rejection and shared-root tests. |
| Full hashes and truncated wire identity | v2/hybrid imports, same-wire-prefix rejection, dual-hash commitment tests. |
| Tracker/DHT identity and IPv6 | UDP/KRPC/IPv6 loopback hardening; hybrid cases exercise both swarm IDs. Public/routed-network evidence remains distinct. |
| Private dictionary acquisition window | Patch 0012, private raw peers and independent causally gated DHT/proof fixtures with a public positive control. |
| Private PEX/LSD and tracker policy | Patch 0019 omits private PEX advertisements and tests hostile PEX with public positive controls plus outbound suppression against a real outgoing source. Acquisition connections omit PEX. Optional/additive ID updates, zero disable/re-enable and owned sender tasks have wire regressions. Patch 0020 gives streams distinct registrations, fans out bounded queues, shares one periodic worker per hash/port and cancels it after the last announcing owner drops. Real datagrams and actual worker release have three production-source tests; production-session public/private controls cover pure-v2 and both hybrid identities. Pending replies retain announcer cancellation. Only first private tracker used, so no tracker failover is claimed. Caller initial peers are an explicit manual override. |
| Metadata IDs, requests, sizes and extension updates | Patch 0013, all-buffer-split codec test and four independent metadata-wire cases. Patch 0017 rejects out-of-range metadata indices, including the piece-count boundary and u32::MAX, then serves both valid blocks on the same connection. Patch 0019 accepts omitted optional fields, merges additive IDs and prevents disabled ID zero from becoming an extension handshake; raw peers prove disable/re-enable without disconnect. |
| Metadata resource bounds and useful progress | Patch 0014 shares four acquisition slots across the session, admits eight attempts per lookup, remembers 4096 endpoints, and bounds DHT/LSD endpoint queues to 1024. Useful-progress/attempt deadlines are 20/120 seconds. Shared-capacity, duplicate-endpoint and keepalive-only recovery tests plus a delayed useful-progress control pass. These bounds cover active acquisition, not caller-retained completed previews or every internal DHT task. |
| Connection handshake and bitfield ordering | Patch 0005 plus TCP/uTP and mixed-wire interoperability tests. |
| Interest independent of choke state | Patch 0016 separates interest updates from waits for unchoking/request capacity. Raw peers prove HAVE, selection, other-peer completion and unselected stream start/seek/drop transitions. Three independent outgoing cases prove choked partial-peer interest changes in pure-v2 and both hybrid swarms. |
| Request/piece/reject correlation and cancel duty | Patch 0005 exact request/cancel ledger; patch 0016 cancels/releases unneeded or stale owned requests and gates enqueueing against current ownership/choke/window state. Legacy choke retirement and v2 explicit-reject duty have separate wire tests. Patch 0017 holds the existing piece lock through verification/publication and checks it for priority reservations; a storage-gated second-peer regression proves no duplicate reservation during verification. |
| Hash request response and correlation | Patch 0022 preserves structurally valid request/reject parameters for semantic handling and correlation, while hashes replies retain strict parameter/size bounds. Raw pure-v2 and hybrid peers receive exact correlated rejects for unsupported lengths, layers, indices and roots, then obtain a valid proof on the same connection. Every ring-buffer split is tested. Malformed frames remain errors. Patch 0017 covers verified duplicate-root storage, metadata-only piece layers and required leaf proofs after payload upload. |
| Hash availability, rate limits and serving cost | Patch 0017 permits payload reads only from verified pieces, using any verified identical-root copy. Thirty-two reply jobs per peer bound queued/in-flight hash, metadata and upload work; permits last through socket flush. Metadata acquisition rejection queues share the same bound. Patch 0018 charges replies/rejects through shared torrent/session upload limiters, handles blocks exceeding a low burst without failure, and serves only required leaf/piece base layers. Raw peers prove shared rates and required proofs after an optional-layer reject. Leaf-proof work scales with requested blocks and piece size; no fixed wall-time or external-disk latency guarantee is claimed. |
| Simultaneous hybrid layout and verification | Patch 0007 plus independent BEP 3-only and mixed-peer transfers; both SHA-1 and SHA-256 required before HAVE. |
| Legacy-only hybrid magnets | Patch 0010 independent complete/partial/corrupt/padding/private/inspection cases; layer roots authenticated before membership. |
| Dual-hash magnets | Patch 0011 races both identities, verifies both commitments before proofs/payload; independent legacy-only/mixed/qBittorrent transport cases. |
| Legacy layer-fetch peer-state and resource audit | Patch 0021 tracks useful interest while choked, retires and releases choked requests by closing their attempt, and removes per-block shared wakeups/scans. Five causal cases cover btih/dual partial peers, corrupt late payload, claim restart and slow useful progress past a stale timer. Sixteen prior legacy regressions pass. Eight workers, one block each, endpoint/reply bounds and progress deadlines remain. The final transport matrix and all sixteen legacy regressions pass against patch 0022. |
| MSE/PE policy and stream framing | Patch 0008 malformed-handshake/policy/key-binding tests and independent TCP/uTP required-RC4 matrices. |
| uTP fallback and impaired transfer recovery | Patch 0006 fallback plus independent loss/reorder/blackout/reconnection evidence. |
| Persistence and app integration | Rust restore/recheck/file-operation coverage and repaired small-stack FFI initialization. The final ad-hoc native build passes four pure-v2/hybrid XPC smokes, including sanitized filenames, exact file-operation bytes and restart. Deep/strict signature and bundled-license byte checks pass. |
| Reproducible source | Twenty-two ordered vendor patches reconstruct current source, reverse cleanly and reapply byte-identically. The final evidence records implementation, patch, source-tree and packaged binary hashes. |

The final transport matrix covers pure-v2 and hybrid download, upload, btmh
magnet and selection, plus hybrid btih and dual-hash magnets, over TCP/uTP with
plaintext/required RC4: forty independent cases. Six additional hybrid cases
prove simultaneous v1/v2 payload, mixed magnets and legacy-only peers. Four
impaired uTP cases cover forced legacy hybrid wire and encrypted hybrid wire in
both directions. Fourteen
discovery/hostile cases, five private-metadata cases, five metadata-wire cases,
sixteen legacy-layer cases and five causal legacy-flow cases complete the final
95-case checkpoint. Every case passed against the final source.

Supported-client bounds remain explicit: metadata is capped at 32 MiB; piece
length is a power of two from 16 KiB through 2 GiB; the storage chunk index is
32-bit; mapped path components are at most 248 bytes and complete paths at most
4096 bytes. Hash serving supports the required leaf/piece base layers and at most
512 base hashes per request. Frames, reply jobs, endpoint queues, concurrent
metadata attempts and progress deadlines remain bounded. Optional intermediate
hash layers can receive correlated rejects. These limits do not claim support
for unlimited torrents, optional extensions or arbitrary storage latency.

Certificate-backed signing/notarization, actual login/sleep/physical drive and
interface recovery, and UI accessibility remain separate release gates in
`VALIDATION.md`. Public/routed-network evidence remains distinct from the
controlled interoperability results. No protocol completion work remains within
the requested scope.
