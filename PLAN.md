# Milestone 0 — Establish a beautiful native experience and validate the engine

## Summary

Build a polished macOS application foundation alongside the engine proof of concept. **Visual design and user experience are primary product requirements throughout development, with acceptance criteria equal to correctness and reliability.**

The long-term goal remains a free, open-source torrent client with qBittorrent-level capabilities and a substantially better Mac experience. Public previews arrive incrementally.

Target macOS 15+ and Apple Silicon. Use SwiftUI, AppKit, and Rust with Hardened Runtime and direct distribution.

## Implementation

### Visual design and user experience

- Establish the design foundation before expanding features: typography, spacing, semantic colors, materials, icons, selection states, motion, and reusable components.
- Pursue a refined native Mac aesthetic with clear hierarchy, excellent typography, restrained materials, and carefully balanced information density.
- Design a cohesive main window: navigation sidebar, configurable transfer table, contextual toolbar, optional inspector, and unobtrusive transfer totals.
- Make everyday actions immediately understandable while placing advanced controls in contextual inspectors and settings.
- Produce polished designs for the main window, adding a torrent, file selection, transfer details, and settings. Include light and dark appearances.
- Implement the main window and add-torrent flow against the real Rust core during this milestone. Use explicitly identified fixtures to review other screen states.
- Treat loading, metadata resolution, empty lists, stalled transfers, errors, and recovery as designed experiences.
- Preserve selection, scroll position, and interaction stability during live updates. Use AppKit-backed tables or trees where they improve interaction and performance.
- Provide keyboard navigation, contextual menus, drag and drop, resizable layouts, VoiceOver support, sufficient contrast, and reduced-motion behavior.
- Explain operations in user terms. Show accurate progress and recovery actions without exposing unnecessary engine details.

### Engine and build foundation

- Pin `librqbit 9.0.1` to upstream commit `a499d2f243d124e144aef137afe7cb304a6e3f36`. Audit that revision rather than documentation labeled `latest`. [Upstream revision](https://github.com/ikatson/rqbit/commit/a499d2f243d124e144aef137afe7cb304a6e3f36).
- Use Rust 1.99.1, UniFFI 0.32.2, and Tokio 1.54.1; commit dependency lockfiles and match bindings-generator and runtime versions.
- Separate the Rust application core, torrent-engine adapter, and UniFFI exports.
- Maintain necessary engine fixes as focused patches against the pinned revision.

### Process ownership and lifecycle

- Reuse one Swift host module and Rust core in an embedded foreground XPC service and an optional background launch agent.
- Acquire an exclusive operating-system file lock before opening session state or transfer files.
- Authenticate XPC peers through code-signing requirements and keep engine operations outside the UI’s main actor.
- Switch hosts through controlled shutdown, persistence, termination, and restoration. Show a brief, understandable transition in the interface.

| Action | Required behavior |
|---|---|
| Close the last window | Transfers continue |
| Quit with background mode off | Save state and stop the foreground host |
| Quit with background mode on | Interface exits; background transfers continue |
| Quit and Stop Transfers | Persist stopped intent and stop engine activity |
| Change background mode | Perform a controlled host handoff |
| Background permission denied | Retain foreground operation with a clear explanation |
| Login with automatic resume off | Do not start transfers |
| Login with automatic resume on | Restore previously running transfers; retain paused torrents |

Background operation and resume-at-login are separate settings, both initially disabled. Scope crash-recovery intent to the current boot and graphical login session.

## Interfaces and persistence

- Expose asynchronous operations for adding magnets or torrent bytes, selecting files, setting limits, pausing/resuming, removing transfers, cancelling metadata resolution, reading snapshots, and shutting down.
- Return application-owned records for identity, state, progress, rates, file selection, and actionable errors. Include protocol and engine-instance versions in the XPC handshake.
- Use typed UniFFI records within the helper and versioned JSON payloads carried by XPC `Data` between processes.
- Poll snapshots once per second while the main window is visible; update presentation state without rebuilding unaffected rows.
- Keep persistence under Application Support, with the helper as sole writer. Use librqbit’s session store during this milestone; defer the broader application database.
- Distinguish user pause intent from temporary shutdown pauses. Verify durability before reporting a command as successfully saved.
- Preserve corrupted state for diagnosis. Audit persistence failures and shutdown completion rather than assuming engine cancellation guarantees a completed save. [Reviewed implementation](https://docs.rs/librqbit/latest/src/librqbit/session.rs.html).

## Test and acceptance criteria

**Design and UX**

- Review rendered screens in light and dark mode at compact and expanded window sizes.
- Verify readable hierarchy, consistent spacing, aligned controls, coherent iconography, and clear selection states.
- Complete adding, selecting files, pausing, resuming, inspecting, and removing a torrent using both pointer and keyboard.
- Validate VoiceOver, contrast, reduced motion, and understandable error recovery.
- Confirm live updates preserve selection and scrolling, and hashing does not block interaction.
- Require visual and interaction review before accepting each implemented screen.

**Engine and lifecycle**

- Use generated fixtures and controlled local swarms, with qBittorrent as an independent interoperability peer.
- Verify downloaded bytes, magnet metadata, multi-file selection, seeding, limits, duplicate detection, and removal with retained files.
- Test restarts, abrupt helper termination, competing startups, repeated host switches, permission denial, and logout/login.
- Test full disks, unavailable destinations, malformed paths, and symlink escapes.
- Record source evidence and fixture results for v2/hybrid, encryption, uTP, priorities, rechecks, relocation, and renaming.
- Run Rust tests and Clippy, Swift builds/tests, and signed lifecycle tests on macOS 15. Report unsupported formats explicitly.

The milestone passes only when transfer integrity, exclusive ownership, persistence, signed lifecycle behavior, and the implemented UI’s design and usability criteria pass.

## Assumptions and deliverables

- Hardened Runtime is enabled; App Sandbox is disabled.
- The first public preview supports v1, with explicit protocol gaps. Full protocol coverage remains required later.
- Use an existing Apple Development identity for signed helper validation when that gate resumes. Further signing and physical recovery checks were deferred by the user on 2026-10-25.
- Deliver the design foundation, screen designs, polished working main window and add flow, reproducible builds, fixtures, lifecycle results, and engine capability report.
- RSS, search, media playback, remote Web UI, and qBittorrent API compatibility remain subsequent milestones.
- **Every subsequent milestone includes design, implementation, and UX verification together. Visual quality is part of feature completion.**

## Transfer-control follow-up — 2026-10-25

Force Recheck → file priorities → basic queueing → relocation → local renaming
are implemented with engine, XPC, and native UI coverage. Rechecks and storage
changes preserve selection and user pause intent; queue suspension is separate
from user pause. See `docs/FILE-MANAGEMENT.md` for verification and remaining limits.

Physical sleep/wake, drive removal, interface loss, actual logout/login, and
further signing validation remain open, deferred at the user's request. Preliminary
certificate-backed handoff and simulated login-policy results are recorded in
`docs/VALIDATION.md`. Final local builds use ad-hoc signing. These deferrals do
not constitute milestone acceptance. Encryption and uTP remain separate protocol
projects and were not included in the file-management work.
