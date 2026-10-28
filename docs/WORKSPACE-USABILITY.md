# Workspace usability acceptance

Status: not run. Automated fixtures and renders do not establish completion of
these tasks by representative torrent power users.

Use a separate macOS test account, a dedicated download folder, and disposable
transfer files. Import real test torrents for tasks that change files or transfer
state. `--design-preview` contains labeled sample data and cannot issue commands.
Record the build digest, macOS version, window size, density, previous torrent
client, keyboard preferences, and any assistive technology used. Identify
participants with anonymous codes.

Start at 1280×800 with compact rows and the details pane visible. Give the task
prompt without naming a tab or control. Observe the first attempt before offering
help. Preserve errors and unsuccessful attempts in the record.

| Task | Participant prompt | Evidence of completion |
| --- | --- | --- |
| Waiting | Explain what is currently known about why this transfer is waiting. | Finds transfer state, connected peers, and discovery observations; distinguishes observations from an unproven root cause. |
| Tracker failure | Identify the tracker that failed and whether that establishes that the whole transfer has failed. | Identifies the failed endpoint and last result; checks other discovery paths without summing tracker counts into a unique swarm. |
| File priority | Prioritize the requested file, exclude an unrelated file, and verify the resulting selection. | Changes the intended protocol file indices, confirms priorities and inclusion, and preserves unrelated files and the last included file. |
| Seeding contribution | Explain how much this transfer has uploaded and what its share ratio means. | Finds lifetime/session labels; distinguishes network traffic from verified content and explains unknown or zero-denominator ratios. |
| Storage recovery | Recover a transfer whose disposable destination is unavailable. | Finds the persistent notice, inspects the destination, restores storage, and uses the appropriate recovery action without losing file choices or pause intent. |

Prepare both a failed tracker with another working discovery path and a transfer
with no connected peers. Do not describe the second condition as proof of tracker
failure. Use a controlled unavailable destination for recovery. Physical external
drive removal and background lifecycle certification remain separate gates.

For every attempt, record:

- Participant code, task, fixture state, start/end time, and completion duration.
- Outcome: independent completion, completion with help, or incomplete.
- Errors, incorrect conclusions, unintended actions, and recovery steps.
- Tab changes, repeated navigation, failed searches, and focus loss.
- Help given and the participant's explanation of the observed state.

Run pointer, keyboard-only, VoiceOver, Reduce Motion, system contrast, live
scrolling, and reopening checks separately. Record the actual settings and input
method. A menu action invoked by a self-test does not prove physical key delivery
or VoiceOver speech.

Store observations alongside the implementation evidence. Report what participants
did, including failed attempts; do not infer measured usability from screenshots
or automated success alone.
