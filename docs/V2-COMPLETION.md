# BitTorrent v2 completion work

Scope requested on October 25, 2026: BEP 52, simultaneous hybrid v1/v2
swarms, peer encryption, and uTP. This scope is implemented and verified as of
October 26, 2026; see [the final evidence](results/v2-final-2026-10-26.json) and
[requirement audit](BEP52-CONFORMANCE.md). Earlier sections record historical
checkpoints and their then-open work.
The scope includes real interoperability in each transport and both directions;
enabling an option alone is not evidence that the transport works.

## Current wire corrections

The first completion patch addresses actual peer behavior beyond successful
happy-path transfers:

- Unavailable or invalid v2 upload requests receive a correlated `reject`.
  A following valid request remains serviceable on the same connection.
- Metadata-only v2 connections explicitly reject payload and hash requests,
  and close on unsolicited payload.
- Availability bitfields must appear first (the BEP 10 extension handshake is
  handled separately); duplicate/late bitfields and nonzero spare bits close
  the connection.
- `not interested` updates the peer's interest state.
- Canceled requests are tracked by exact identity and multiplicity. Unrelated
  data cannot consume a generic tolerance for late canceled requests. The
  ledger closes an unresponsive connection at 4096 unanswered cancellations.
- Correlated rejects release the affected piece for another peer, cancel its
  other pending chunks, and keep the connection usable for different pieces.
  A fresh `have` can advertise the rejected piece again. Availability changes
  wake the piece requester rather than waiting for its polling timeout.

Raw TCP tests in `crates/seedhop-core/tests/v2.rs` cover pure-v2 and hybrid upload
responses, bitfield validation, metadata-phase responses, and download recovery
after a reject. The production cancellation ledger is also compiled directly
into that integration target to exercise exact matching, repeated cancellations,
and the capacity bound without rqbit's standalone CLI/UI dev dependencies.

## uTP integration and metadata recovery

The app now binds TCP and uTP on the same announced port. Patch 0006 retries an
outgoing connection over uTP when TCP connects but its BitTorrent handshake fails
(a uTP-only peer may accept TCP before rejecting it). It does not bypass SOCKS,
retry a wrong torrent identity, or retry a self connection. Metadata discovery
now makes at most three attempts for transient connection/handshake/I/O failures
with bounded backoff; invalid metadata and Merkle proofs are not retried by this
policy. A raw-peer regression drops the first handshake, supplies authenticated
metadata on the second connection, and checks that list-only resolution creates
neither files nor library membership.

The independent suite supports `--transport tcp` and `--transport utp`. It sets
qBittorrent's protocol preference to TCP-only or uTP-only and reads it back before
starting the case; encryption remains explicitly disabled in these fixtures.
Protocol selection follows the [qBittorrent API documentation](https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-%28qBittorrent-5.0%29).
The uTP protocol reference is [BEP 29](https://www.bittorrent.org/beps/bep_0029.html).
All nine existing v2/hybrid scenarios have passed separately over forced TCP and
forced uTP, with exact real-file byte comparisons. Encryption, routed networks,
loss/reordering and forced-uTP interruption recovery remain unverified here.

Final evidence: [v2-completion-2026-10-25.json](results/v2-completion-2026-10-25.json).
The serial workspace run passes 49 tests (33 in the v2 target); the opt-in
disk-image fault test remains ignored. Check, Clippy with warnings denied,
formatting, Python compilation and six-patch reverse/reapply verification pass.
Both final independent matrices pass all nine cases. The retained record checks
every peer's forced protocol/encryption settings and verifies actual uTP stream
diagnostics, and includes source hashes. An intermediate concurrent v1 swarm
timeout passed in isolation and in the final serial run. Logs remain in
`.build/v2-completion-*.log` and each case's evidence folder.

## Simultaneous hybrid swarm follow-up

Patch 0007 carries the discovery hash alongside each endpoint, announces both
hybrid identities, and chooses the correct wire identity per connection. It
supports incoming v1-only peers, optional v1-to-v2 upgrade, and one immediate
alternate-identity attempt when an initial endpoint declines the first identity.
Both hash commitments are checked during initial/restart checks and downloads.
A piece that passes SHA-256 but contradicts the SHA-1 commitment fails the
torrent instead of being shared between inconsistent swarms.

Legacy upload requests can cover real bytes, a padded final block, or entire
virtual padding blocks. Legacy download requests retain their full wire lengths
in the request/cancel ledger; response padding must be zero before real bytes
are written. The parser records the exact optional final-padding extent. Tracker
`left` uses that swarm's verified piece sizes. PEX retains the negotiated swarm
identity and ignores dropped endpoints as discovery candidates. Private flags
learned during magnet resolution stop the public discovery stream before live
transfer begins; privacy before the flag is known remains a separate audit item.

`scripts/v2_hybrid_interop.py` supplies an independent Python BEP 3-only seed and
leecher plus an isolated qBittorrent v2 peer. All four scenarios pass: v1-only
download, mixed-swarm download, mixed-swarm btmh magnet, and v1-only upload.
Mixed cases mask piece zero from the v2 peer and offer it only through the v1
tracker identity; recorded payload requests must prove contribution by both.
Rust regressions cover padded/unpadded final files, pure padding blocks, nonzero
padding rejected before disk writes, inconsistent commitments, alternate identity
fallback, and ring-buffer splits while stripping virtual padding.

The older evidence above remains a checkpoint for patches 0001–0006. The new
record is [v2-hybrid-2026-10-25.json](results/v2-hybrid-2026-10-25.json): 54 Rust
tests pass (38 in the v2 target), with the opt-in disk-image test ignored. The
four independent hybrid cases, all 18 TCP/uTP regression cases, and seven hybrid
hardening cases pass. UDP and local DHT captures prove that both hashes were
queried. The mixed fixture filters tracker evidence to SeedHop's own announces,
checks swarm-specific `left`, and verifies the full app identity and real-byte
progress. Check, Clippy with warnings denied, formatting, Python compilation and
seven-patch reverse/reapply checks pass. Logs are `.build/v2-hybrid-*.log`.

## Peer encryption follow-up

Patch 0008 adds incoming and outgoing MSE/PE to both TCP and uTP, using
`crypto-bigint` for the protocol's fixed DH group and RustCrypto RC4 rather than
custom cryptographic primitives. This is legacy BitTorrent interoperability:
the mandated 768-bit DH, SHA-1 and RC4 do not provide TLS or peer authentication.
The wire reference is [libtorrent's MSE implementation](https://github.com/arvidn/libtorrent/blob/RC_2_0/src/bt_peer_connection.cpp)
and [DH/RC4 implementation](https://github.com/arvidn/libtorrent/blob/RC_2_0/src/pe_crypto.cpp).

`SessionOptions.peer_encryption` and `Engine::open_with_peer_encryption` expose
four policies. The app's default `Allow` accepts MSE and tries plaintext first,
then MSE on a fresh connection. `Prefer` reverses that order; `Disabled` only
accepts plaintext; `Require` accepts only RC4 payload encryption. Header-only PE
is accepted by Allow/Prefer but never counted as encrypted. Require rejects
both a bare handshake and a header-only cipher selection. The lab exposes these
policies through `SEEDHOP_PEER_ENCRYPTION`; no native policy selector is added.
Retries retain SOCKS routing and do not bypass an identity mismatch or a self
connection.

Negotiations use bounded synchronization/padding (512 bytes), bounded initial
payload (65535 bytes), public-key validation, existing handshake timeouts and
separate limits of eight incoming and eight outgoing crypto negotiations.
Incoming stream-key lookup is bound to the subsequent BitTorrent identity before
hybrid upgrade normalization. Buffered cipher writes preserve state across
partial/Pending writes and flush each frame; fragmented handshake reads now
collect all 68 bytes. Tests exercise maximum initial data and padding, invalid
keys and negotiation fields, header-only policy, cross-torrent key misuse,
fragmented initial handshakes, and encrypted legacy hybrid padding uploads.

The encryption checkpoint is recorded in
[the MSE results](results/v2-mse-2026-10-26.json): 59 Rust tests pass (five MSE
and 38 v2 tests), with one opt-in disk-image test ignored. All 18 forced-RC4
qBittorrent cases and 18 plaintext regressions pass across TCP/uTP, as do the two
policy fallback cases, four independent hybrid cases and seven hybrid hardening
cases. Peer settings are read back and actual RC4 negotiations are required in
the encrypted cases. A DHT fixture now withholds its payload endpoint until both
hybrid lookups arrive, avoiding shutdown of the tiny transfer before the second
lookup is scheduled. Check, Clippy with warnings denied, formatting, Python
compilation, forward vendor reconstruction and eight-patch reverse/reapply
verification pass. Logs are `.build/mse-*.log`. Full protocol completion remains
subject to the requirements below.

## uTP loss, reordering and interruption recovery

`scripts/v2_utp_impairment.py` places a UDP-only relay between the clients. It
drops every 37th data packet, delays every 11th by 25 ms, and blocks traffic for
350 ms after 80 data packets. Captures record packet types, connection IDs,
sequences and actions; counters require actual reordering, data retransmission,
outage drops and subsequent successful forwarding. Every case compares both
real files byte-for-byte, totaling 2,097,343 bytes.

All eight pure-v2/hybrid upload/download cases pass with plaintext or required
RC4. Two additional hybrid cases advertise only the SHA-1 tracker endpoint and
clear the v2 upgrade bit in both plaintext handshakes, proving legacy-wire
uploads and downloads over impaired uTP. Both handshake hashes must match the
expected SHA-1 identity. This fixture requires a complete plaintext handshake
in a single uTP data packet and does not intercept encrypted legacy handshakes.

A repeat encrypted transfer exceeded the lab's old 45-second deadline while the
engine was waiting to reconnect. The final stress fixture uses a bounded
120-second deadline, through a lab-only environment override; production
timeouts/backoff are unchanged. The final eight-case run passes, including
reconnection when a stream expires. This validates local recovery; routed
networks, physical interface changes and sleep/wake remain separate checks.

Evidence: [v2-utp-recovery-2026-10-26.json](results/v2-utp-recovery-2026-10-26.json).
Reproduce with `python3 scripts/v2_utp_impairment.py` and
`python3 scripts/v2_utp_impairment.py --wire v1 --format hybrid --encryption off`.

## Legacy magnet proof discovery and private trackers

Patch 0009 separates SHA-1-authenticated dictionary acquisition from v2 proof
provider discovery. A btih hybrid metadata peer can decline v2 handshakes and
MSE; the session uses the committed full SHA-256 identity to find a different
provider. That provider must supply the identical dictionary and authenticated
Merkle layers. Files no larger than a piece resolve from the legacy connection
alone, since their roots require no external piece layer.

Once a btih dictionary reveals its private flag, the resolver drops the public
discovery stream before searching for proofs and discards unauthenticated public
candidates. The already-authenticated provider can be retried, and new discovery
uses only the torrent's private tracker policy. Private torrents no longer
inherit session-wide extra trackers with zero or one explicit tracker. The
first tracker is retained before conversion to an unordered set, so the two
hybrid swarms and later reconnects keep the same choice.

Evidence: [v2-magnet-discovery-2026-10-26.json](results/v2-magnet-discovery-2026-10-26.json).
The final workspace run passes 61 tests (40 v2), with the opt-in disk-image test
ignored. All five independent hybrid cases pass, including a btih magnet whose
legacy metadata peer refuses v2 connections, with direct captures of Merkle
requests to the separate v2 peer and payload contribution from both swarms.
Four qBittorrent btih magnet cases pass over TCP/uTP with plaintext/required RC4,
and all seven hybrid hardening cases pass. Check, Clippy with warnings denied,
formatting, Python compilation and nine-patch reconstruction/roundtrip checks
pass. Logs are `.build/v2-magnet-*.log`.

Large-file hybrids in an exclusively legacy swarm still need a way to derive
layers from authenticated v1 payload. This patch does not provide that fallback.
The remaining privacy audit also includes discovery while a btmh connection has
received its dictionary but is still fetching proofs.

## Payload-derived layers in exclusively legacy swarms

Patch 0010 adds a delayed fallback for btih hybrid magnets. After allowing two
seconds for cheap v2 proofs, the resolver can fetch legacy payload in 16 KiB
blocks and derive the missing layers. It checks exact request identity and
length, virtual padding zeros and each complete SHA-1 piece before accepting
its block hashes. A completed layer must match the committed SHA-256 file root;
contradictory hybrid commitments fail resolution before membership or payload
files are created. No unverified layer can become usable metainfo.

The scheduler shares completed layer progress among up to eight peers, releases
claims on disconnect/cancellation, and retries transient transport failures at
most three times. Bad hashes and padding are rejected without discarding other
peers' verified work. Identical files share layers and can supply them through
different wire indices, including an unpadded final piece. No entire piece or
file payload is buffered: workers retain block hashes and one bounded block,
while metainfo/layer sizes retain the existing 32 MiB bounds. The address queue
is bounded to 1024 pending and 4096 remembered endpoints, and stalled workers
have a payload-progress deadline. The full peer-state/resource audit remains
open below.

This fallback streams payload during metadata inspection and discards those
bytes. A later transfer currently downloads them again; the authenticated
metainfo and derived layers are persisted normally after addition. This is a
bandwidth cost when no v2 proof source is available, not a claim of payload
caching during previews. `seedhop-lab inspect-magnet` exposes side-effect-free
inspection to the fixture harness.

Evidence: [v2-legacy-layers-2026-10-26.json](results/v2-legacy-layers-2026-10-26.json).
The workspace passes 62 Rust tests (41 v2), with one opt-in disk-image test
ignored. Eight independent Python legacy-peer cases cover a complete seed,
complementary partial peers, corrupt-data and nonzero-padding recovery, private
magnets, inconsistent commitments, and two previews with no files/membership.
The preview layers must exactly equal the independently generated layers.
Five mixed/legacy hybrid regressions, four TCP/uTP plaintext/required-RC4 btih
magnet regressions and seven hybrid hardening cases pass. Check, Clippy with
warnings denied, formatting, Python compilation and ten-patch forward/roundtrip
verification pass. Logs are `.build/v2-legacy-layers-*.log`.

The fallback currently applies to the btih metadata path. Magnets containing
both btih and btmh still need cross-swarm metadata discovery while preserving
both commitments. The btmh private-flag discovery window and the broader
conformance/privacy audit also remain open.

## Dual-hash magnet discovery

Patch 0011 preserves both `btih` and `btmh` commitments and starts metadata
lookups through both supplied swarm identities. A valid result from either path
cancels the other lookup, and the winning discovery identity is retained when
peers enter the live transfer. Legacy dictionary acquisition, separate v2
proofs and the payload-derived fallback all continue to enforce both original
commitments.

The wire hash and full expected hashes now have separate fields. Both hashes
are checked as soon as the complete dictionary arrives, before requesting
proofs or payload. A dual-hash result must also be genuine hybrid metadata;
matching digest values of a v1-only or v2-only dictionary do not satisfy this
requirement. Conflicting duplicate topics are rejected while identical repeats
are allowed. This follows the [BEP 9 magnet format](https://www.bittorrent.org/beps/bep_0009.html),
which permits both exact topics when they describe the same hybrid torrent.

Evidence: [v2-dual-magnet-2026-10-26.json](results/v2-dual-magnet-2026-10-26.json).
The final workspace passes 64 Rust tests (43 v2), with one opt-in disk-image test
ignored. A raw peer supplies dictionary bytes with an incorrect SHA-1, an
incorrect SHA-256 beyond the truncated wire ID, or matching digests of each
non-hybrid format. All four are rejected with zero proof/payload requests,
files or membership. Eight independent legacy-only cases pass with both
commitments, as do four qBittorrent v2-only discovery cases over TCP/uTP with
plaintext/required RC4. Six independent hybrid cases include a dual-hash mixed
swarm whose payload requires both peers, and seven hybrid hardening cases pass.
Check, Clippy with warnings denied, formatting, Python compilation and
eleven-patch reconstruction/roundtrip checks pass. Logs are
`.build/v2-dual-magnet-*.log`.

The default independent transport matrix now contains ten cases: four pure-v2
and six hybrid cases, including the new dual-hash magnet. Older evidence files
retain their original nine-case matrices as historical checkpoints. The private
flag discovery window remains open, including cancellation of the other lookup
once either authenticated dictionary reveals that the torrent is private.

## Private metadata discovery teardown

Patch 0012 publishes the private flag as soon as the complete dictionary passes
all supplied magnet commitments and is decoded. Both dual-hash lookup lanes
share this one-way state. They immediately drop public discovery streams and
cancel unrelated metadata connections, without waiting for Merkle proofs.
Subsequent proof/legacy-layer lookups pin the exact SHA-256 dictionary and use
only the first explicit torrent tracker, the authenticated provider and peers
explicitly supplied by the caller. Session extra trackers, DHT and LSD are
excluded. Caller-supplied peers remain a deliberate manual override; this is
not a claim that those endpoints were returned by a private tracker.

A forged private dictionary cannot trigger this transition before its full
hashes are verified. The coordinator also handles a result and a privacy notice
becoming ready together, so it cannot return the old public discovery stream.
Datagrams already sent before authentication cannot be recalled. Tests hold
DHT replies until after teardown and then introduce a new descendant and
announce token: no subsequent recursive lookup or announce is sent.

The public positive control exposed an upstream DHT bug: each recursive request
was sent twice and only its second response was processed. The same patch now
processes the original response once. The control proves this fixture can
recurse and announce when the dictionary is public.

Evidence: [v2-private-metadata-2026-10-26.json](results/v2-private-metadata-2026-10-26.json).
The workspace passes 66 Rust tests (45 v2), with one opt-in disk-image test
ignored. Two new raw-peer tests exercise pure/dual private teardown and reject
unauthenticated private dictionaries while still resolving through public
sources. Five independent Python cases cover the public control, private pure
v2, hybrid btmh, dual and legacy btih magnets. Every private case closes its
unrelated connections before proof release, sends zero descendant DHT queries
or late announces, and sends zero session extra-tracker requests after the
barrier. Byte-exact payload transfers finish after proof release. Check, Clippy
with warnings denied and twelve-patch reconstruction/roundtrip checks pass.
Remaining privacy work includes hostile PEX, private tracker failure policy and
public-network discovery validation; full completion is still in progress.

## Metadata wire conformance

Patch 0013 implements BEP 9 extensibility: unknown metadata message IDs are
ignored, including signed or large IDs with no known fields and opaque future
payload. The bounded frame parser preserves the next message and accepts future
header keys beyond the old guessed header prefix. Known data messages must keep
`total_size` equal to the extension handshake; zero/oversized dictionaries are
rejected before allocation. Metadata rejects fail the provider promptly, and
requests received during acquisition receive a correlated reject.

Repeated BEP 10 extension-handshake updates no longer allocate a new dictionary
or replay the request set. Metadata/proof connections remain choked and not
interested in payload; the legacy payload fallback still expresses its own
interest, and normal transfer unchoking is unchanged.

Evidence: [v2-metadata-wire-2026-10-26.json](results/v2-metadata-wire-2026-10-26.json).
The workspace passes 67 Rust tests (46 v2), with one opt-in disk-image test
ignored. Four independent Python cases cover future IDs/header keys, handshake
updates and request rejection, plus prompt recovery after changed sizes, zero
sizes and metadata rejects. All five private discovery cases, six hybrid swarm
cases, eight dual legacy-layer cases and four dual-magnet TCP/uTP plaintext/RC4
cases pass. Check, Clippy with warnings denied, formatting and thirteen-patch
reconstruction/roundtrip checks pass.

The native development app rebuilt and verified its ad-hoc signature. Crash
reports now identify the XPC failure: mutual signature requirements succeed,
but the helper exhausts its small dispatch-queue stack while constructing the
uTP dispatcher during synchronous FFI initialization. This must be fixed before
native pure-v2/hybrid integration is considered validated.

## Native XPC initialization repaired

Crash reports showed the helper authenticating the client and then exhausting
its small macOS dispatch-queue stack while constructing the uTP dispatcher.
`Runtime::block_on` polls its root future on the caller, even on a multithreaded
runtime. FFI initialization now spawns `Engine::open` on an owned Tokio worker
and waits for its result. uTP remains enabled; signing requirements are unchanged.

Evidence: [v2-native-stack-2026-10-26.json](results/v2-native-stack-2026-10-26.json).
A regression initializes real TCP/uTP listeners, snapshots and shuts down from
a 512 KiB host thread. The workspace passes 68 Rust tests (46 v2), with one
opt-in disk-image test ignored. Check, Clippy with warnings denied and formatting
pass. The rebuilt ad-hoc app passes deep/strict signature verification and both
native pure-v2 and hybrid XPC smoke tests: authenticated connection, import,
Force Recheck, priorities, queueing, rename/relocation with exact bytes, restart,
paused selection, removal and shutdown. These are development integration checks;
certificate-backed signing and physical lifecycle certification remain deferred
separate gates. The full protocol audit is still open.

## Metadata acquisition resource bounds

Patch 0014 replaces 128 connections per lookup with four acquisition slots
shared across the session, including concurrent previews and both hybrid
lookup lanes. Each lookup admits eight futures and remembers 4096 endpoints;
repeated caller endpoints are deduplicated. Old inactive history can be evicted
so a full history does not permanently exclude new candidates. DHT and LSD
endpoint queues are bounded to 1024; full queues drop excess discovery hints
instead of accumulating them behind occupied connection slots. Tracker endpoint
queues were already bounded.

Only newly accepted metadata blocks and authenticated correlated Merkle-hash
responses reset the 20-second progress deadline. Keepalives, extension updates
and unrelated messages cannot hold a slot indefinitely. A 120-second attempt
deadline additionally bounds trickle progress. These are acquisition bounds;
caller-retained finished previews and every internal DHT task are not included
in a claim of whole-process memory certification. Four maximum-size raw peer
dictionary buffers occupy at most 128 MiB, excluding decoded structures/layers.

A raw-peer regression proves concurrent lookups share the four-slot budget,
duplicates do not consume additional slots, and four peers sending keepalives
but no metadata are dropped after the progress deadline so a queued healthy
provider can resolve without files or membership. An independent two-block
provider takes about 24 seconds while supplying useful progress and keepalives;
it succeeds, proving the timer is not a fixed 20-second transfer cutoff.
The first slow control omitted keepalives and hit the existing 10-second socket
idle timeout; that fixture error was corrected without changing the production
deadlines.

Evidence and final checks are recorded in
[v2-metadata-resources-2026-10-26.json](results/v2-metadata-resources-2026-10-26.json).
The final workspace passes 69 Rust tests (47 v2), with one opt-in disk-image
test ignored. Five metadata-wire cases (including slow progress), five private
discovery cases, six hybrid cases, seven hardening cases, eight legacy-layer
cases and four dual-magnet transport/cipher cases pass. Check, Clippy with
warnings denied, formatting, Python compilation and fourteen-patch source
reconstruction/roundtrip checks pass.

Filename sanitization, live peer-state/hash-serving and discovery ownership
remain open in the completion audit. The final native app must be rebuilt after
the remaining engine changes; the previous native-stack checkpoint remains
historical evidence for its recorded sources.

## Filename sanitization and storage-map compatibility

Patch 0015 maps BEP 52 names into safe storage components without altering
original info bytes, file order, piece indices or payload commitments. Invalid
UTF-8 (including overlong encodings), separators, control bytes, literal escape
markers and trailing dots/spaces are escaped. Long components are bounded with
a full SHA-256 suffix derived from the original bytes. Reserved device names
use a separate escaped namespace. Normal Unicode names, including emoji, remain
readable. Siblings that would alias after Unicode normalization/full case
folding or removal of default-ignorable characters get distinct deterministic
suffixes. Files and directories use the same per-parent collision policy.
Complete storage components are at most 248 UTF-8 bytes and composed paths
retain the 4096-byte and aggregate allocation limits.

Hybrid layout equality now compares the original raw path components, before
storage mapping. A renamed/sanitized v1 path cannot substitute for its original
v2 spelling. The shared adapter also stops re-decoding finished UTF-8 storage
strings through a guessed v1 encoding; it preserves even an initial U+FEFF.
Optional container names derived from the advisory info name are sanitized,
so an unsafe advisory name does not reject otherwise valid content or escape
the default destination.

Every new v2 transfer persists a complete, versioned index-to-path map. Restoring
older sessions reproduces their historical UTF-8 validation and legacy encoding
guess, preserves existing rename overrides, and pins those original on-disk
paths once. It does not move existing files or create replacement names during
an upgrade. Missing/incomplete current maps, invalid version fields and future
versions fail before disk I/O. Unknown optional storage backends are not
certified by the app's JSON-persistence tests.

Evidence: [v2-filenames-2026-10-26.json](results/v2-filenames-2026-10-26.json).
Independent raw-byte fixtures exercise pure files/magnets, hybrid files, btih
magnets and dual-hash magnets. Files must have distinct filesystem inodes and
exact payload bytes; v2 uploads and legacy uploads including virtual padding
are read back independently. Rust tests cover raw commitment equality,
file/directory alias mapping, restart/recheck, old-session migration and corrupt
map rejection. Native regular/raw-name pure-v2/hybrid smokes use the actual
reported import path, then verify recheck, rename/relocation and restart.

The final workspace passes 76 Rust tests (54 v2), with one opt-in disk-image
test ignored. Five independent filename cases, six hybrid cases, eight legacy
layer cases, five private discovery cases and four dual-magnet transport/cipher
cases pass. Four native regular/raw pure-v2/hybrid XPC smokes and deep/strict
ad-hoc signature verification pass. Check, Clippy with warnings denied,
formatting, Python compilation and fifteen-patch reconstruction/roundtrip
verification pass.

Unicode normalization/case-folding dependencies are recorded in Cargo.lock and
THIRD_PARTY_NOTICES.md. Unicode's data license is retained in
`vendor/UNICODE-LICENSE.txt`, bundled in Resources and checked byte-for-byte.
Peer-state/hash-serving and discovery-ownership work remain open.

## Live peer interest, selection and choke handling

Patch 0016 keeps interest independent of the payload requester waiting for an
unchoke, request capacity or rate allowance. A peer is useful only when its
advertised pieces intersect missing selected pieces or current streaming demand.
Piece/HAVE/selection notifications refresh this state. A 250 ms fallback covers
stream starts, seeks and drops, which currently lack the shared notification.
Interest messages are emitted only on changes.

Requests for pieces no longer wanted are cancelled through the existing exact
ledger and their ownership is released. Requeueing no longer reselects an
unselected piece. Before enqueueing payload, the peer, selection/ownership,
choke state and request window are checked together; interest enters the writer
queue before the request. Selection and request-window updates also release
their locks before waking peer work, avoiding inverted lock ordering.

A legacy choke now retires outstanding requests and releases their piece claims;
otherwise a one-request window could remain full forever after unchoking.
V2 deliberately retains outstanding requests until piece/reject responses,
preserving BEP 52 explicit-rejection duty. Tests distinguish both behaviors and
prove cancelled late pieces/rejects remain correlated without writing unwanted
data or losing the connection.

The initial full suite hit an intermittent corruption-fixture gate: four
pipelined blocks were counted after the receiver could disconnect on its first
bad piece. An isolated rerun passed. The fixture can now explicitly batch those
four corrupted blocks before delivery; both complete-piece and recovery
assertions remain unchanged. The deterministic targeted and full runs pass.

Evidence: [v2-interest-2026-10-26.json](results/v2-interest-2026-10-26.json).
The workspace passes 80 Rust tests (58 v2), with one opt-in disk-image test
ignored. Four new raw-peer tests cover selection, other-peer completion,
streaming and legacy/v2 choke rules. Three independent outgoing cases cover
pure-v2, hybrid v2 wire and hybrid legacy wire. Six hybrid regressions, eight
legacy-layer cases, five private-discovery cases and four dual-magnet TCP/uTP
plaintext/required-RC4 cases pass. Two forced-legacy plaintext uTP cases and two
negotiated encrypted uTP cases recover under loss, reordering and blackout.
Check, Clippy with warnings denied, formatting, Python compilation and
sixteen-patch reconstruction/reverse/reapply pass.

This engine checkpoint does not rebuild the native app. The filename checkpoint
retains the last native package evidence; final native validation follows the
remaining serving/discovery fixes. Hash/metadata serving boundaries, verification
ownership and discovery lifetime/extension policy remain open. No additional
transport or feature project is included in this checkpoint.

The detailed requirement checklist is [BEP52-CONFORMANCE.md](BEP52-CONFORMANCE.md).

## Authenticated serving and verification ownership

Patch 0017 rejects metadata indices at or above the block count without slicing
or closing the peer. Valid requests still return the original info bytes. The
raw-peer regression reproduces the previous panic and checks both valid blocks
after invalid requests on pure-v2 and hybrid connections.

Hash serving now checks verified piece availability before reading payload.
Files sharing a root can use any verified copy with the same length, including
an unselected first copy and a selected second copy. Authenticated piece-layer
metadata remains serviceable without payload. A successful payload upload keeps
its required leaf proof available. The negative control fails when the old
unverified-storage read path is restored.

Each live peer admits at most 32 reply jobs across hash/metadata responses,
rejects and rate-limited payload uploads. Metadata and legacy-layer acquisition
connections also bound their rejection replies. Permits survive queueing, disk
work and socket write/flush; a slow reader applies backpressure. A 256-request
metadata pipeline drains with exact bytes while another peer remains responsive.
This bounds pending reply work; it does not yet establish a hash byte/CPU rate.
The remaining hash rate-limit and serving-cost review stays explicit below.

The existing per-piece lock now remains held through verification and HAVE
publication. Streaming priority reservations check that lock before taking a
piece, just as stealing already did. A storage gate holds verification while a
second peer offers the same piece. The old reservation path requests the piece
again; the fix waits and returns exact stream and disk bytes. No new production
verification state machine or hash cache is introduced.

Evidence: [v2-serving-2026-10-26.json](results/v2-serving-2026-10-26.json).
All 84 Rust tests pass (62 v2), with one opt-in disk-image test ignored. Three
independent interest cases, five metadata-wire cases, eight dual-identity legacy
layer cases and required-RC4 hybrid uploads over TCP and uTP pass. Check, Clippy
with warnings denied, formatting, diff checks and seventeen-patch
reconstruction/reverse/reapply pass. The native package remains at the filename
checkpoint until the remaining engine audit is complete.

## Hash rate policy and bounded serving layers

Patch 0018 charges live hash responses and hash rejects through the existing
torrent and session upload limiters. Both limits are shared across peers,
matching payload policy. Two raw peers requesting eight authenticated 96-byte
proofs now wait under a shared 256 B/s burst; the previous implementation
answered them in about 26 ms. Separate cases exercise torrent and session rates.

That check exposed an existing limiter boundary: a valid transfer block larger
than the configured burst failed with `InsufficientCapacity`. Upload and
download charging now split that amount into batches supported by the same
limiter, preserving the complete byte charge. The low-burst regression waits
successfully instead of disconnecting the caller.

Live hash serving supports the required leaf and piece base layers. Optional
other base layers receive correlated rejects, followed by successful required
proofs on the same connection. This avoids optional large-subtree work. Leaf
proof computation still scales with requested blocks and piece size; ancestors
at the stored piece layer use authenticated metadata. Existing request-size
validation, reply budgets and blocking-work admission remain in place. No new
production hash cache or independent rate-control subsystem is added.

Evidence: [v2-hash-rate-2026-10-26.json](results/v2-hash-rate-2026-10-26.json).
All 87 Rust tests pass (65 v2), with one opt-in disk-image test ignored.
Required-RC4 hybrid uploads to qBittorrent pass over TCP and uTP. Check, Clippy
with warnings denied, formatting, diff checks and eighteen-patch
reconstruction/reverse/reapply pass. Discovery lifetime/extension policy,
legacy-fetch peer state and final native/transport validation remain open.

## Extension updates and private PEX policy

Patch 0019 makes optional extension IDs serializable and accepts handshakes
without an `m` dictionary. Later dictionaries merge changes into each peer's
existing IDs. Zero disables an extension, and positive IDs can re-enable it.
Neither metadata nor PEX serializes with ID zero. Queued replies for disabled
extensions are discarded before socket output, preserving the peer connection.
A raw peer keeps metadata ID 11 through PEX-only and request-window updates,
disables it without losing payload rejection responses, and re-enables ID 13.

Each live public peer owns one cancellable PEX sender. Repeated updates do not
spawn duplicates; disabling cancels it, and re-enabling creates one replacement.
A real outgoing source and a keepalive-capable observer exercise repeated
enable/disable/re-enable updates. The negative control emits five exchanges;
the fix emits exactly one on the observer's ID 9. The private control emits none.

Private live handshakes omit PEX. They also omit `ut_metadata` and its size,
matching the existing policy of not serving private metadata. Metadata and
legacy-layer acquisition handshakes omit PEX because those connections do not
participate in peer exchange. Pure-v2/hybrid raw peers show a public PEX endpoint
is connected while the private counterpart is ignored.

The independent private fixture initially failed because it indexed the optional
metadata ID in every handshake, including private live connections. Its private
and legacy peers now honor optional/additive IDs while retaining metadata request
correlation and exact-byte checks. Private handshakes also assert PEX is absent.
The original fixture failure and corrected rerun remain separate evidence logs.

Evidence: [v2-extensions-2026-10-26.json](results/v2-extensions-2026-10-26.json).
All 91 Rust tests pass (69 v2), with one opt-in disk-image test ignored. Five
independent private-discovery cases, five metadata-wire cases, eight dual-identity
legacy-layer cases and four dual-magnet TCP/uTP plaintext/required-RC4 cases pass.
Check, Clippy with warnings denied, formatting, Python compilation, diff checks
and nineteen-patch reconstruction/reverse/reapply pass. LSD subscription and
announcement lifetime, legacy-fetch peer state and final native validation remain
open. No tracker failover is claimed; private torrents retain one tracker, and
caller-provided initial peers remain an explicit manual override.

## LSD subscription and announcement lifetime

Patch 0020 gives each LSD stream an owned subscription ID. Streams sharing an
info hash receive independent endpoint deliveries; adding or dropping one no
longer replaces or unregisters the other. Each subscriber retains its bounded
1024-entry queue, and a full queue does not block another consumer.

Periodic workers are shared by info hash and announced port. Listen-only
subscriptions create none. Dropping the last announcing subscriber cancels its
worker even while a listen-only stream remains. Replies also retain the relevant
announcer token, so an owner disappearing while a reply is pending cancels that
reply. Other subscriptions and ports remain independent.

Three tests compile the actual production LSD source through an isolated
manifest, avoiding unrelated rqbit desktop development dependencies. They use
real IPv6 loopback datagrams to prove fan-out, surviving registration and delivery
past a full subscriber queue. Owned Arc counts prove one shared worker and release
of the actual periodic future after its last announcing owner disappears. An
omitted-cancellation negative control times out at that release assertion.
The isolated build also exposed a core derive import relying on root feature
unification; it now uses the existing `serde_derive` dependency directly.

A production-session regression injects public and private LSD offers. Pure-v2
and both hybrid identities connect with the expected wire identity in the public
case; private offers never initiate connections. Source review confirms private
sessions do not register LSD/DHT sources. This is controlled loopback evidence,
not physical-interface or routed-network certification.

The first full run stalled in a raw encrypted fixture that reached an unrelated
IPv4 loopback service sharing the chosen IPv6 wildcard port under macOS address
reuse. Sampling and socket ownership identified the mismatch. Only that test was
terminated. Raw fixture handshakes now have deadlines and prefer the intended
IPv6 listener. A deliberately overlapping IPv4 decoy proves plaintext/encrypted
hybrid transfers avoid the wrong service. Production MSE negotiation already had
a timeout. The diagnosed run and clean rerun remain separate evidence.

Evidence: [v2-lsd-2026-10-26.json](results/v2-lsd-2026-10-26.json).
All 92 workspace Rust tests pass (70 v2), with one opt-in disk-image test ignored;
three production-source LSD tests also pass. Root and isolated LSD Clippy,
check, formatting, Python compilation, diff checks and twenty-patch
reconstruction/reverse/reapply pass. Legacy-fetch peer state and the final
native/transport completion audit remain open.

## Legacy layer-fetch peer state and progress

Patch 0021 starts legacy layer-fetch connections uninterested and updates interest
from their useful advertised pieces, independent of choking. A piece claimed by
another peer remains useful until accepted into layer derivation. Once that source
provides the observer's only useful piece, the observer becomes uninterested while
still choked; a new useful HAVE restores interest before unchoking and requesting.

Per-block progress no longer broadcasts to every worker. Valid blocks update a
local timestamp and queue their next block directly. Shared piece acceptance and
claim release wake shared work; availability/choke changes wake their own
scheduler. Timer expiry rechecks the latest timestamp before declaring an idle
peer. Duplicate HAVEs and repeated choke state do not cause unnecessary rescans.
Existing candidate selection remains in place; no new cache or pipeline is added.

The audit retains eight workers, one outstanding 16 KiB block per worker, a
20-second useful-progress deadline, three attempts per endpoint, a 4096-endpoint
history, a 1024-endpoint pending queue and bounded rejection replies. Keepalives
do not refresh useful progress. Choking an outstanding legacy request closes
that attempt and releases its claim; socket closure discards late responses.

Five independent causal cases exercise btih and dual-hash interest transitions,
choke plus corrupt late payload, replacement-provider restart, and slow useful
progress. The latter takes over 22 seconds while delivering valid blocks and
keepalives, and must finish without reconnecting or duplicating requests. Negative
controls catch unconditional interest, requests after choking and stale deadline
restarts. Every successful inspection checks exact derived layers and identity,
creates no payload files and leaves no session membership.

Evidence: [v2-legacy-flow-2026-10-26.json](results/v2-legacy-flow-2026-10-26.json).
All 92 workspace Rust tests pass (70 v2), with one opt-in disk-image test ignored.
The five causal cases and sixteen existing legacy regressions across both magnet
identities pass. Check, Clippy with warnings denied, formatting, Python compilation,
diff checks and twenty-one-patch reconstruction/reverse/reapply pass.
The final review still needs to distinguish hash-request framing from unsupported
semantic parameters, confirm future-version error priority, and rebuild/revalidate
the native package and final transport matrix.

## Final wire-boundary corrections — 2026-10-26

Patch 0022 distinguishes framing errors from unsupported hash-request parameters.
The codec preserves the exact 48-byte request/reject tuple for semantic handling;
invalid roots, layers, lengths and indices receive correlated hash rejects.
Hash data replies still require valid parameters, whole 32-byte hashes and the
bounded response size before allocation. Pure-v2 and hybrid raw peers request
unsupported parameters and then receive a valid authenticated proof on the same
connection. The codec regression covers every ring-buffer split.

Both file imports and acquired info dictionaries check `meta version` before
current-layout serde validation. Version 3 dictionaries missing piece length or
containing malformed current-version fields now report the unsupported version.
The generic parser, acquired-info preflight and application preview share this
regression. Before-fix negative controls reproduced both the premature peer EOF
and the misleading missing-piece-length error.

The final source passes all 94 workspace Rust tests, including 72 v2 tests, with
one opt-in disk-image test ignored. Check and Clippy with warnings denied pass.
All twenty-two ordered vendor patches reconstruct the current source, reverse
cleanly and reapply byte-identically. The final checkpoint also passes 95
independent protocol cases, three isolated LSD tests and four packaged native XPC smokes. The native app was rebuilt from
this source with ad-hoc signing. Deep/strict signature verification and bundled
Unicode/license byte checks pass. Formatting, Python compilation and diff checks
pass. [Final evidence](results/v2-final-2026-10-26.json) retains current source
and binary hashes, commands, detailed cases and log hashes.

## Completion status and separate release gates

| Requested scope | Final status |
|---|---|
| BEP 52 metainfo and wire protocol | Implemented. Requirement-by-requirement audit, 72 dedicated v2 Rust tests and independent malformed/hostile/discovery fixtures pass. Meta-version priority and correlated hash rejection close the final boundary gaps. |
| Simultaneous hybrid v1/v2 swarms | Implemented. Shared storage, virtual padding and dual SHA-1/SHA-256 verification are exercised by independent legacy-only, mixed-swarm, btih/dual magnet and forced legacy-wire uTP cases. |
| Peer encryption | MSE/PE with Disabled, Allow, Prefer and Require policies is implemented. Policy/key-binding/malformed-negotiation tests and required-RC4 transfers over TCP and uTP pass. |
| TCP and uTP | Forty final qBittorrent cases cover both formats, both transports, plaintext/required RC4 and supported import/selection paths. Four impaired hybrid uTP cases cover forced legacy and encrypted wire in both directions. |
| Discovery, privacy and legacy layers | Fourteen discovery/hostile cases, five private-metadata cases, five metadata-wire cases, sixteen legacy-layer regressions and five causal legacy-flow cases pass. Private torrents use a single stable first tracker; failover is not claimed. Caller initial peers remain an explicit manual override. |
| Persistence and application integration | All 94 workspace Rust tests pass; one opt-in disk-image test is ignored. Three isolated production LSD tests and four rebuilt native XPC smokes pass, including filename mapping, exact file bytes, file operations and restart. |
| Reproducible vendor source | All twenty-two patches reconstruct current vendor source and reverse/reapply byte-identically. Final source, patch, source-tree, binary and log hashes are retained. |

All requested protocol implementation work is complete within the documented
client bounds. Implementation and evidence were committed at `56ffc38` on
October 26, 2026. Further feature expansion is outside this completion checkpoint.

The user has resumed release validation, starting with accessibility, followed
by distribution signing/notarization and actual lifecycle/recovery acceptance.
See [`RELEASE-VALIDATION.md`](RELEASE-VALIDATION.md) for current evidence and
the target-system walkthrough. Remaining gates are:

1. Certificate-backed package signing/notarization and final background-service identity validation.
2. Physical sleep/wake, login, removable-drive and network-interface loss/recovery validation.
3. UI accessibility acceptance and the remaining release checklist in `VALIDATION.md`.

Those release gates remain open and do not change the completed protocol scope.
