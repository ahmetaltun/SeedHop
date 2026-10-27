# Connection settings and engine restart

Roadmap step 5, completed on 2026-10-27, exposes the existing engine's peer-encryption policies and makes
connection information match the configured listener. It does not add another
protocol implementation, advanced transport switches, or a reachability probe.

## Policies

| Choice | Peer connection behavior |
|---|---|
| Disabled | Plaintext peer connections only |
| Allow encrypted peers (default) | Accept encrypted peers; try plaintext first, then encryption on a fresh outgoing connection |
| Prefer encrypted peers | Try encryption first, allowing a fresh plaintext connection if it fails |
| Require encryption | Require encrypted peer payloads; reject plaintext and header-only modes |

These are the existing rqbit MSE/PE policies. Requiring encryption can reduce
available peers. Legacy peer encryption does not hide the client's IP address or
encrypt tracker/discovery traffic. The settings explain those limits directly.

## Saving and applying

The engine owns a versioned `connection-settings.json` in the library state
directory. A missing file preserves the previous default, Allow. Malformed data,
unknown policy/version, and unreadable settings fail startup without overwriting
the saved file or guessing a weaker policy. The save command validates its input
under the existing mutation lock and uses the flushed atomic-write helper.

Snapshots expose both active and saved policies through UniFFI and XPC. A save
changes only the next-start policy: session/listener policy is immutable after
construction. The app refreshes the saved choice after the command; it does not
optimistically claim that the current session changed. A failed save remains
retryable. If a directory flush fails after rename, the engine reflects the
readable saved value while still reporting the durability error.

Settings show a pending-restart explanation and an explicit Restart engine to
apply button. Choosing the current active policy again cancels a pending change.
No restart happens merely because a policy was selected.

Restart uses the existing shutdown/reconnect ownership barrier and the same
foreground/background host mode. It does not send `explicitStop`, so it preserves
each torrent's pause intent rather than pausing the library. Reconnect reads the
saved policy, restores file choices/priorities/queue positions, and rechecks files.
The app retains selection and synchronizes its existing login-resume preference.
Imports, active file operations, policy/queue saves, and in-flight transfer actions
block this restart action. Duplicate restart/save requests are suppressed.

If shutdown is rejected, the active connection and pending preference remain. If
reconnection fails after shutdown, the app keeps the last library and policy
snapshot and allows a later reconnect. It does not switch host modes or silently
roll back to a different policy.

## Connection information

The configured listener supports TCP and uTP over UDP on the reported listening
port. The UI calls it a listening port, not proof of incoming reachability; routers,
firewalls, and the network may block connections. A disconnected snapshot does
not claim that its old port is active. The formats row includes v1, v2, and hybrid.
Older XPC snapshots without policy fields remain decodable, with policy editing
disabled rather than guessing an active setting.

## Validation and remaining gates

Portable evidence is in `results/connection-settings-2026-10-27.json`.

- Four settings integration tests cover all four policies across restart,
  paused/running/queued intent, high file priority, rejected/invalid saves,
  corrupt/unknown/unreadable settings, and the real listener. A saved Require
  policy still permits plaintext before restart; after restart plaintext is
  rejected and an encrypted handshake succeeds.
- The five existing MSE tests still pass, including fragmented handshakes,
  initial payload preservation, malformed inputs, header-only policy, and
  binding an encrypted connection to the correct torrent.
- Core unit/management regressions, controlled Swift presentation tests, packaged
  foreground XPC policy-save failure/retry and restart, three native menu/focus
  checks, Clippy, formatting, and the ad-hoc development build pass. There are
  fifteen core unit tests, three management tests, and 32 Swift regressions.
- Fifty-two rendered fixtures include light/dark pending policy, save errors, and offline
  state. Native rendering is separate from physical keyboard and VoiceOver
  acceptance.

The real listener check uses controlled TCP loopback. Existing uTP and public-swarm
protocol evidence is unchanged; this increment does not recertify every transport
or public network. The background restart route has controlled presentation
coverage and reuses the existing host handoff; real login, physical sleep/wake,
network/drive loss, native pointer/key-event delivery, and VoiceOver remain
separate acceptance gates. Distribution signing, notarization, release, RSS,
and remote control remain deferred.

The packaged cancellation observer initially exhausted a fixed poll count during
preparation. A bounded elapsed-time window with paced polling now observes actual
copying; both move/rename cancellation cases pass. This fixture adjustment does not
change production file-operation or protocol behavior.
