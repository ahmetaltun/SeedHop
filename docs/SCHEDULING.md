# Weekly transfer scheduling

This release includes engine-owned weekly scheduling. RSS and remote control
are cancelled, rather than deferred release features.

Open Settings → Transfers → Weekly schedule. Add one or more windows, select
their starting weekdays, enter 24-hour start/end times, and choose bandwidth
limits or suspension of downloads and seeding. Enable scheduling and save.
Changes take effect without restarting the engine. Reload saved discards the
local draft; failed saves retain the draft and report an actionable error.

## Behavior

- Up to 32 non-overlapping windows, with independent download/upload caps.
  Limits use decimal KB/s; 0 means unlimited. A window replaces normal caps.
- Weekdays refer to the start of a window. For example, Monday 22:00–06:00
  includes Tuesday until 06:00. Equal times mean 24 hours from the start.
  Start is inclusive and end is exclusive, including across Sunday/Monday.
- Overlap is rejected even when scheduling is disabled, so enabling saved rules
  cannot introduce ambiguous precedence. Adjacent windows are allowed.
- Outside windows, or when disabled, normal bandwidth limits and queue admission
  apply. Editing normal limits during a window saves them for later without
  overriding the active window. Settings distinguishes normal and effective caps.
- Suspension blocks peer transfers for downloads and completed seeders. It
  preserves manual pause choices, identities, selected files, priorities and
  queue order. Transfers display “Scheduled suspension” rather than suggesting
  they are waiting for a download slot. Manual Resume still respects suspension.
- File verification and active file operations can finish. Window transitions
  wait for the current engine mutation; they cannot interrupt a layout commit.
  Read-only metadata inspection does not block automatic evaluation.
- The engine evaluates the current local calendar on its existing 250 ms timer.
  It reevaluates after clock/timezone changes or wake rather than replaying
  missed events. DST repeats the current local-time window; skipped local times
  cause no deferred action. Only the currently applicable window matters.
- Scheduling runs wherever the existing engine runs, including background mode.
  It does not create a service, launch the app, wake the Mac, or enable login
  resume. An engine that is stopped performs no scheduled work.

## Alternative bandwidth limits

Settings → Transfers provides a second download/upload profile. The bottom-right
switch selects it immediately. Click the adjacent tortoise to open the bandwidth
window, which shows saved profiles and effective caps. The switch stays disabled
until at least one alternative cap is greater than zero. While enabled, alternative limits replace
both global and scheduled caps. Schedule suspension and manual pause intent still
apply. Switching off restores the current scheduled caps, or global caps outside
windows. Saving global limits while alternative limits are active preserves them
for later. Both profiles use decimal KB/s; 0 means unlimited.

The engine atomically saves alternative caps and the selected mode together in
`alternative-limits.json`. A missing file defaults to no alternative caps and alternative mode off.
Previously saved unlimited alternative profiles load with alternative mode off. Existing global limits remain in `limits.json`.

## Persistence and compatibility

`schedule.json` is an atomically written version-1 record in the library state
directory. Missing settings mean scheduling is off. Corrupt, unreadable, invalid,
or future settings cause startup to fail without replacing the original record.
The existing `limits.json` remains authoritative for normal bandwidth limits.
Schedule changes do not rewrite transfer pause intent or queue order.

Restoration with scheduling enabled initially queues restored transfers before
peer activity; the current window and ordinary queue policy then decide admission.
Failed application is retried by the engine and exposed as a persistent scheduling
warning. Disabling a schedule retains its windows for later use.

The native interface, host and UniFFI bindings use protocol version 4. Settings,
snapshot status, normal limits and effective limits cross the authenticated XPC
boundary. No separate scheduler process, dependency, database or migration is
needed.

## Verification

Focused Rust tests cover weekly/overnight/full-day boundaries, week wrap,
adjacent windows, overlap rejection, deterministic clock jumps, active and normal
caps, download/seeding suspension, paused intent, selections/priorities, queue
order, new imports, restart, failed-save retry, corrupt-state preservation,
concurrent snapshot readers, and evaluation during read-only metadata inspection.
Controlled Swift regressions cover draft validation, overflow, delayed saves,
failed-save retry and schedule-specific transfer guidance. The packaged native
XPC check covers settings, effective/normal caps, failed-save preservation,
restart and disabling the schedule.

These are automated checks. They do not certify physical input, VoiceOver
speech, signing, or physical lifecycle behavior. Signing and physical lifecycle
tests are outside this development pass at the user's request.
