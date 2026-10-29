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
