# rqbit source provenance

`rqbit/` is the source archive of https://github.com/ikatson/rqbit at
`a499d2f243d124e144aef137afe7cb304a6e3f36` (v9.0.1), Apache-2.0.
The upstream LICENSE is retained. SeedHop patches are described in
`docs/ENGINE-AUDIT.md`; the ordered patches are in `vendor/patches/` (reliability, BEP 52, metadata replay guard, transfer/file controls, v2 wire corrections, uTP fallback, simultaneous hybrid swarms, MSE/PE encryption, magnet proof discovery/private tracker isolation, legacy payload-derived Merkle layers, dual-hash magnet discovery, early private metadata teardown and single-response DHT recursion, BEP 9 metadata wire conformance, shared metadata resource bounds, filename sanitization and saved-path migration, live interest and choke handling, authenticated serving and verification ownership, shared hash rate policy, additive extension IDs and private PEX policy, owned LSD subscriptions and announcement lifetime, legacy layer-fetch peer state and progress, semantic hash rejects and meta-version priority, native payload/peer/tracker diagnostics, a complete live-task shutdown barrier, and bounded magnet discovery with fresh peers prioritized before failed-peer retries, and separate socket-probe/dictionary budgets with metadata-only timeouts). Desktop/web assets are upstream references, not used by
the native app. No upstream instruction file is a product dependency.

Unicode filename collision data is licensed under the notice in `UNICODE-LICENSE.txt`, which is bundled with the native app.

Patch `0026-seedhop-live-rate-limits.patch` wakes pending payload and hash quota
waits when upload/download limits change, including when a limit becomes unlimited.

Patch `0027-seedhop-upload-announcements-and-rate-cadence.patch` restores HAVE
announcements to peers omitting their empty bitfield, matches per-torrent speed
sampling to the estimator's per-second window, and exposes upload-demand and
download-pipeline diagnostics.

Patch `0028-seedhop-piece-selection-options.patch` persists per-torrent sequential
and first/last-piece preferences, selects rarest available pieces by default with
randomized ties, and tracks connected-peer availability across bitfield, HAVE,
reject, and disconnect events. File priority and explicit stream priority remain
authoritative. It also repairs an obsolete unit-test call to the v2-aware metadata
helper so focused upstream library tests compile.

Patch `0029-seedhop-selected-file-storage.patch` keeps unchecked payload paths
unopened. Shared-piece boundary bytes live in per-torrent hidden part storage.
Selecting a file later links its sparse part data into place without replacing an
existing file. Removal deletes the owned part rather than an unrelated unchecked
file at the payload path.

Patch `0030-seedhop-runtime-piece-preferences.patch` makes the existing per-torrent
sequential and first/last-piece preferences mutable without restarting peers or
changing transfer intent. It reads both flags consistently for new piece
acquisitions and restores prior preferences if session persistence fails.

Patch `0031-seedhop-piece-map-diagnostics.patch` exposes read-only piece maps
for the selected torrent's General tab. At most 512 ordered bins preserve
verified and in-flight counts and the minimum connected-peer availability in
each range. Distributed copies exclude local files; paused transfers retain
verified progress but report no live availability. Checking and error states
report no map. This does not change piece selection or transfer behavior.

Patch `0032-seedhop-piece-source-gaps.patch` adds exact per-bin counts of pieces
missing locally and absent from every connected peer. Paused observations omit
this count. Grouping never infers a gap from unrelated local/remote minima.

Patch `0033-seedhop-force-reannounce.patch` adds ephemeral, generation-scoped
controls to the existing tracker monitors. HTTP regular and minimum intervals
remain distinct; manual requests respect minimum intervals, a 60-second local
cooldown, and retry backoff. UDP manual requests retain the advertised interval.
Tracker lifecycle events are committed after successful requests, with payload
baselines covering fast completion after metadata discovery. A hash-indexed weak
registry and monitor-drop cleanup avoid retaining retired streams or scanning the
whole session for each torrent snapshot. Manual transports share four slots.

Patch `0036-seedhop-private-payload-capabilities.patch` adds an opt-in pinned
filesystem factory for private payloads. It creates unknown files exclusively,
reopens only recorded file identities, and retains directory descriptors instead
of resolving a replacement path for new opens. Quarantine, owner-only modes,
non-blocking opens and single-link checks remain required. Initial partial-file
selection uses index-only boundary storage inside each exclusive transfer root;
it does not write the torrent hash into those filenames. Later membership changes
and payload deletion are rejected. Ordinary filesystem storage keeps its existing
behavior. Runtime acceptance is deferred; this patch is not a filesystem-race or
private-session certification.
