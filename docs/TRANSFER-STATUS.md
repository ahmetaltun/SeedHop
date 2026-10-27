# Transfer states and recovery actions

Roadmap step 4 completed on 2026-10-27. Rows, filters, the inspector, and aggregate
activity now share a presentation of engine facts. The raw engine states remain
compatible with existing transfer and protocol harnesses.

## State presentation

| Snapshot fact | Presentation and available action |
|---|---|
| Waiting, no connected peers | Finding peers; no inferred tracker or network failure |
| Waiting, connected peers | Waiting for data; show the connected count in details |
| Queued | Queue position and Move to Front of Queue |
| Checking | Checked bytes, separate from downloaded bytes; saved pause intent is explained |
| Downloading or seeding | Existing transfer progress and sharing controls |
| Paused | Resume when explicitly paused; an internally suspended running transfer says Preparing transfer |
| Active file operation | Operation phase and copied bytes override temporary paused/error states |
| Fatal error | Unknown progress rather than a misleading zero percent; typed guidance and Retry |
| Retained-file recovery | Cleanup needed, included in Needs attention, with persistent cleanup actions |
| Disconnected engine | Last-known labels and reconnect explanation; live rates are unavailable |

An orphaned cleanup notice remains visible even without a matching torrent row.
An empty attention view cannot claim that everything is healthy while that notice
remains. Search and navigation remain usable during recovery.

## Typed errors and Retry

The adapter walks typed error causes rather than matching localized strings.
It recognizes both `std::io::Error` and the `nix::errno::Errno` returned by rqbit's
Unix vectored writes. Disk space/quota, access, unavailable paths/devices, and
other known file errors receive separate guidance. Unknown causes and network
timeouts remain generic transfer errors; zero download speed does not establish
a failed tracker or network.

The additive `error_kind` field crosses UniFFI and becomes optional `errorKind`
in XPC snapshots. Older snapshots without the field still decode. Error details
remain available in a disclosure group.

The new `retry` command rechecks an errored torrent under the engine mutation
lock. It preserves its identity, selected files, priorities, destination, and
saved pause intent. A running torrent re-enters queue admission before it can
transfer data. A paused torrent stays paused. A delayed Retry against a healthy
transfer is a no-op. Missing destinations are rejected without creating replacement
folders. Conflicting retained-file cleanup must be resolved first.

Retry captures its target before asynchronous work, suppresses duplicate requests,
and retains the failure and choices when a command fails. Mixed selections send
Resume to paused transfers and Retry to failed transfers. The inspector's queue
action also captures its own transfer. Checking disables conflicting recheck,
resume, and move actions. Cleanup disables affected removal/recheck/retry and
all layout changes.

Show Folder opens the nearest existing parent when the destination is missing;
it never creates the missing destination. Move Files is offered in file error
details where the existing transaction can safely copy available originals.
Unavailable-source errors first require reconnecting/restoring the saved path.

## Evidence and limits

`results/transfer-status-2026-10-27.json` records source, binary, and log digests.
Focused verification passed:

- Fifteen core unit tests, including typed standard/Unix I/O causes and generic
  fallback; three management tests; one healthy stale-Retry integration test.
- Three controlled recovery tests, plus real payload/state ENOSPC and the new
  Retry test on a disposable 32 MiB HFS+ image. Both paused and running intent
  retain a high file priority, reject a missing destination, survive restart,
  obey queue admission, and finish with exact payload bytes. The image was detached.
- Twenty-seven controlled Swift regressions, including state labels, progress,
  cleanup attention, failed/delayed Retry, mixed commands, offline facts, older
  snapshot decoding, and finding existing parent folders.
- Packaged foreground XPC import/recheck, stale Retry, priorities, queue,
  move/rename progress/cancellation, byte verification, restart, and removal.
- Three native menu/focus checks, workspace Clippy with all targets/features,
  formatting, and an ad-hoc development build with verified bundle signatures.
- Forty-six rendered fixtures, including light/dark transfer errors, compact
  states, stale snapshots, cleanup attention, and orphan cleanup. Representative
  new screens were visually inspected; rendering does not certify native input.

The cancellation regression now waits for the queue's asynchronous position
assignment before taking its preservation baseline. This removes a test timing
race without changing queue behavior.

Permission/device error classification has typed unit coverage; the real new
failure/retry integration uses ENOSPC. Native pointer/key-event delivery,
VoiceOver, physical drive loss, sleep/wake, and public-swarm acceptance remain
separate gates. This step adds no protocol support or reachability guarantee.
RSS, remote control, distribution signing, notarization, and release remain deferred.
The next roadmap step is settings and accurate connection/encryption policy.
