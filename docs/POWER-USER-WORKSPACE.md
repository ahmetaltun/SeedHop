# Power-user workspace

The library uses a compact, configurable transfer table and a resizable bottom
pane with General, Files, Peers, Trackers, and Activity tabs. The sidebar is
hideable. The status bar reports visible and selected counts, global rates, and
engine connectivity. A fixed-height attention strip keeps recovery and file
operations available without resizing the table on each progress update.

## Workspace behavior

- Default compact rows target 28 points; Comfortable Rows targets 40 points.
- Name stays visible. The View Options > Columns menu and native header menu
  control other columns; Restore Default Columns clears customization.
- Column IDs from the earlier table are retained. Sorting, density, tab,
  visibility, and divider positions persist. Torrent identity breaks sort ties.
- Queued and Checking filters supplement existing state filters. Tracker filters
  use parsed hosts, never credentials or URL tokens. Explicit filter changes
  clear hidden selection; live snapshots preserve selection and scroll anchors.
  Active filters and Clear Filters occupy the fixed status bar, preserving the
  table viewport when filtering. Filter results are reused until their inputs change.
- The Files tab retains native hierarchy, independent row/inclusion selection,
  search, priorities, rename, and last-included-file protection. Size, Progress,
  and Priority now have aligned columns.
- Existing commands and captured targets remain in use. Multi-selection shows
  totals and explicit group actions instead of one transfer's diagnostics.
- Rendering and acceptance fixtures use separate workspace preference storage.

## Data and protocol

Protocol version 3 adds a typed `torrentDetails` XPC request. Its section is one
of `general`, `files`, `peers`, `trackers`, or `activity`; `allPeers` includes known
connection attempts. Replies carry torrent ID, engine instance ID, observation
time, and requested section. The UI serializes requests, polls once per second
while visible, and rejects replies after selection, section, or instance changes.
The FFI bridge serializes typed Rust records to JSON; the host decodes typed
Swift records before returning the authenticated XPC reply. Incompatible helpers
report a version error instead of accepting an unknown request.

Shared engine payload atomics survive pause, queue changes, recheck, and file
operations. They count received and sent piece payload, separately from verified
content. Engine-owned `statistics.json` checkpoints totals, added/completed dates,
and seeding duration every 30 seconds and on orderly shutdown. Add persists its
initial accounting before committing membership. The shutdown barrier drains
initialization, live-peer, PEX, and session tasks before the final checkpoint.
An abrupt termination can lose statistics since the last successful checkpoint;
this is not a transactional traffic audit log.

Earlier transfers retain unknown lifetime history and dates. Their observed
session traffic is still available. Existing disk content never increments
network download totals. Lifetime ratio uses uploaded payload / downloaded
payload only when both totals are known and the denominator is nonzero.

ETA comes from the engine's estimator. Peer rates use monotonic counter deltas;
connection-generation IDs prevent reconnects from reusing a rate baseline.
Transport and payload encryption reflect the negotiated connection, not a
configured encryption preference.

Tracker communication callbacks report HTTP/UDP contact, response, warning,
error, and retry schedules. UDP address families have distinct endpoint records.
Missing swarm counts remain unknown, including HTTP responses that omit counts.
Tracker-reported counts are never summed into a unique swarm population.
Discovery enabled/disabled labels are separate from tracker results and do not
claim network health or incoming reachability.

## Activity and privacy

The app retains up to 900 traffic samples per observed transfer (15 minutes at
one sample per second) and 200 displayed events per transfer. Tracker failures
and warnings have an engine-owned 200-event ring, so opening Activity can recover
recent failures that occurred while another tab was selected. UI events also
cover observed transfer, recovery, and file-operation changes. History is held
in memory and is not reconstructed after an app restart.

Tracker addresses hide credentials, paths, and queries. Tracker messages redact
known credential/token fragments, including decoded fragments. Table identities
use opaque digests so different private URLs on one host remain distinct.
Copy Diagnostics uses an allowlist and omits torrent identifiers, personal paths,
tracker URLs, arbitrary remote messages, and peer endpoints. Copy Hash is a
separate explicit action. Peer encryption does not imply anonymity.

## Verification and acceptance

Current implementation evidence is recorded in `results/power-user-workspace-2026-10-28.json`.
The implementation is a development build, not a release candidate.

Controlled native menu/focus/file checks and the authenticated protocol-v3 XPC
smoke pass. The renderer produces 145 fixtures. Native geometry checks verify
16 fully visible compact rows at 1280×800, both row densities, divider persistence,
column persistence, and programmatic selection highlights.

Performance acceptance remains open: final runs 31–33 report zero table warnings
and pass every budget except intermittent 100-row filtering (99.80–103.31 ms p95
against 100 ms). All runs, including failures, are retained. See
[PERFORMANCE.md](PERFORMANCE.md) for the exact measurement boundary and findings.


Automated coverage includes accounting and unknown-history regressions, actual
loopback payload transfer and tracker results, pause/recheck/restart continuity,
late-response isolation, hidden polling, export privacy, native menu/focus/file
controls, and packaged XPC diagnostics. Performance fixtures include the existing
100/1,000-transfer and 10,000-file workloads plus 1,000 peers and 100 trackers.
Latency budgets retain their previous definitions; diagnostic layout/scroll/decode
budgets are 125/50/50 ms p95.

The renderer covers minimum (780×560), desktop (1280×800), and wide (1720×1000)
workspaces, all five tabs, light/dark and both high-contrast appearances, and
comfortable density. These are app-owned fixtures, not a recording of user input.

Manual acceptance remains separate. The task script and observation fields are in
[WORKSPACE-USABILITY.md](WORKSPACE-USABILITY.md):

- Physical keyboard and pointer input, including live scrolling and reopening.
- VoiceOver navigation and speech, system contrast, and Reduce Motion settings.
- Five representative power-user tasks: explain waiting, identify tracker
  failure, prioritize files, inspect seeding contribution, and recover storage.
  Record completion, errors, and repeated navigation. No user study was run by
  the automated fixtures.
- Signing/notarization and physical lifecycle gates in RELEASE-VALIDATION.md.

RSS, remote control, tags, scheduling, search plugins, tracker editing, peer
banning, forced announcements, country lookup, and piece maps remain outside
this change.
