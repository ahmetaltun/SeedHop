# Releases and software updates

SeedHop ships directly to users as a Developer ID signed, notarized Apple Silicon
app for macOS 15 or later. GitHub Actions builds a draft release. Publishing that
draft remains a separate, manual acceptance step.

## Credentials

Configure these repository Actions secrets. Never put their values in issues,
workflow inputs, logs, or source files.

| Secret | Format |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12`, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that `.p12` |
| `APPLE_SIGNING_IDENTITY` | Exact `Developer ID Application: Name (TEAMID)` identity |
| `APPLE_TEAM_ID` | Ten-character Apple developer team ID matching the certificate |
| `APPLE_ID` | Apple account with access to that developer team |
| `APPLE_PASSWORD` | Apple app-specific password for `notarytool` |
| `SPARKLE_PRIVATE_KEY` | Sparkle-exported base64 Ed25519 seed; must match the committed public key |

The SeedHop Sparkle key was generated under login-keychain account
`app.seedhop.mac`. Its public key and feed URL are in
`macOS/Resources/Updates.json`. The private key is retained in the login keychain
and GitHub Actions secrets. Back up that key through a secure credential store;
GitHub cannot return a secret's value after upload.

For a new installation of this pipeline, use the pinned Sparkle distribution:

```sh
python3 scripts/sparkle.py
.cache/sparkle/2.10.1/bin/generate_keys --account app.seedhop.mac
```

Generate a key only when none exists. Export it with `generate_keys -x` into a
protected temporary file, upload that file with `gh secret set SPARKLE_PRIVATE_KEY
< private.key`, and remove the export. Commit only the displayed public key.
Do not casually regenerate the key: existing installations trust the key embedded
in their installed app. Follow Sparkle's key-rotation procedure when rotation is
necessary.

## Create a draft

1. Set `[workspace.package].version` in `Cargo.toml` to a stable `major.minor.patch`
   version. The app's About screen and bundle metadata derive from that version.
2. Run tests, formatting, Clippy, packaging, localization, and native UI checks
   locally before committing. Merge the validated release changes into `master`.
   GitHub Actions runs only the release workflow; branch pushes and pull requests
   do not run tests or builds.
3. Tag the intended commit and push the tag:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. Open the Release workflow. For retries, rerun it or use **Run workflow** with
   the existing tag. The workflow requires that commit to be on `master` and
   requires the tag to match its workspace version.

The release workflow's run number supplies `CFBundleVersion`. Keep this workflow
and its run-number history intact. The preflight rejects a build number that is
not greater than all published stable release metadata. Reruns may refresh draft
assets; a published release cannot be overwritten. Do not publish an older draft
after a newer stable version, or manually mark an older release as latest.

The workflow validates the tag and version, then builds the release app once.
It caches Rust dependencies, build outputs, and the checksum-verified Sparkle
distribution between runs. It does not run the local test suite or native UI
fixtures. Signing, notarization, archive integrity, and update-signature checks
remain required release gates.

GitHub shows four separate stages: **Build and sign optimized app**, **Create,
sign, notarize, and verify DMG**, **Staple and verify app using the DMG submission**,
and **Create signed ZIP, appcast, and checksums**. Compiler output streams immediately. The build
and Apple requests print a heartbeat every 30 seconds, with elapsed time and
the notarization submission ID where available. Each command and stage has a
deadline. A cold optimized build can take several minutes; silence is no longer
the only indication that work continues.

The build cache is saved before notarization, so an Apple failure does not discard
compiled dependencies. Each stage checks the preceding stage, commit, build
number, and app hashes. The final stage also checks the DMG hash. Only temporary
signing keychains hold imported private identities; neither those keychains nor
the exported Sparkle key enter the cache or uploaded artifacts.

Only the signed DMG is submitted to Apple. Notarizing that outermost container
also generates a ticket for the unchanged signed app inside it. The workflow
staples and verifies the DMG, mounts it, verifies the nested app and exact bundle
contents, then staples the standalone app using the same submission's ticket.
The final ZIP is created from that stapled app and signed for Sparkle afterward.
There is no temporary notarization ZIP and no second Apple upload or queue wait.

The DMG remains immutable after signing except for its supported outer ticket
attachment. Its nested app relies on the DMG's ticket, following Apple's standard
container flow. The ZIP contains a directly stapled app for offline checks.
Both payloads retain full signature and Gatekeeper checks. Metadata records
`strategy: outermost-dmg`, `submissionCount: 1`, and the shared submission ID.
Transient ticket-service connection failures receive at most two stapling retries
with 15- and 30-second delays. Invalid or missing tickets still stop the release.
Retries fetch the existing ticket; they never resubmit to notarization.

The workflow checks out the requested tag, including its release scripts.
Rerunning an old tag therefore uses its old scripts. New pipeline code must be
included in the tagged commit. Never move a published tag; an unpublished draft
tag may be corrected deliberately before rebuilding and reviewing its assets.

Apple processing time is outside the runner's control. A wait expires after
45 minutes, with a separate hard process deadline. Download the retained
notarization diagnostics and use the recorded ID to inspect that submission;
Apple continues processing after a client timeout. Inspect notarization logs
for warnings even after acceptance. Do not repeatedly submit the same payload
merely because Apple's queue is slow.

References: [Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
and [GitHub's dependency caching guidance](https://docs.github.com/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows).

The workflow imports the certificate into a temporary keychain.
It restores the keychain search list and deletes temporary
signing material even on ordinary failures. GitHub destroys the runner after
cancellation. No personal keychain or signing identity is required on a developer's
machine to run local tests.

Expected draft assets:

- `SeedHop-0.1.0-arm64.dmg`: signed, notarized disk image with an Applications shortcut.
- `SeedHop-0.1.0-arm64.zip`: notarized, stapled app; also used by Sparkle.
- `appcast.xml`: Sparkle-generated feed with the final ZIP's signature and size.
- `SHA256SUMS`: SHA-256 for every other asset.
- `release-metadata.json`: commit, build number, toolchains, identity, and notary submission IDs.
- Notarization JSON results, with rejection logs when available.

The DMG opens a 720 × 460 Finder icon window with mint artwork, large app and
Applications icons, and a drag-to-install arrow. The background contains 1× and
2× representations. Finder chrome and automatic icon arrangement are disabled.
The release uses checksum-pinned `dmgbuild` tools to write Finder metadata without
GUI automation, then verifies the mounted layout and unchanged app signatures.
Artwork is generated by `scripts/DMGArtwork.swift` before the DMG is signed and
notarized; the app bundle itself is never changed for installer styling.

For a local installer preview, install `scripts/requirements-dmg.txt` into an
isolated Python environment with `pip install --require-hashes --only-binary=:all:`
and `-r scripts/requirements-dmg.txt`. Run that environment's Python with
`scripts/dmg.py --app .build/SeedHop.app --output .build/SeedHop-preview.dmg`.

Download workflow artifacts to inspect a failed run. Notarization failures stop
release uploads; diagnostics are retained separately. Tool errors involving
credentials deliberately suppress raw output. Check certificate type, private-key
presence, team, certificate password, and Apple app-specific password first.
An unavailable pinned Xcode or mismatched Sparkle checksum is a hard failure;
do not silently substitute an unreviewed dependency or compiler.

## Update behavior

The app uses Sparkle 2.10.1 with a checksum-pinned binary distribution. The native
build preserves framework links and helper entitlements, keeps only arm64 code,
and signs nested components before signing the app. Sparkle's license is bundled.

The app menu provides **Check for Updates…**. General settings provides an
automatic-check toggle. Sparkle asks for permission to check automatically;
installation always requires approval. Development builds and fixture launches
do not start the updater or contact the public feed.

The public feed is:

```text
https://github.com/ahmetaltun/SeedHop/releases/latest/download/appcast.xml
```

Every stable release includes its own feed. Enclosures use version-specific
GitHub Release URLs, so downloads never depend on a moving latest ZIP URL. Drafts
and prereleases do not become the stable feed. Before the first stable release,
manual checks may report the feed unavailable; this does not affect transfers.
The first `0.1.0` release includes the updater, allowing later releases to update it.

Only installation starts update shutdown. Checking, downloading, canceling a
download, and feed errors leave the engine running. Installation is deferred
while an import, file change, settings save, or host handoff is active. The app
shuts down without pausing running transfers, waits for the host process to exit,
then unregisters its launch agent before replacement. A durable marker restores
the background preference after relaunch. If macOS requires renewed background
permission, foreground transfers remain available and the marker allows retry.
Failure to prepare cancels termination and attempts to reconnect the existing
session. Normal Quit continues to honor the background-transfer setting.

## Acceptance before publication

Automated packaging checks do not complete the manual release gates in
`docs/RELEASE-VALIDATION.md`. Test the exact downloaded artifact and retain its
hash, macOS version, and observations.

- Extract ZIP and mount DMG; verify the app launches from Applications, signatures,
  Gatekeeper, stapled tickets, helper authentication, and exact transfer bytes.
- Use two Developer ID signed builds and a controlled test HTTPS appcast. Use a
  disposable macOS user account to isolate the real library and preferences. For
  isolated
  tests, set the test feed in an uncommitted build configuration before signing;
  never edit an already signed bundle or publish test builds to the stable feed.
  Verify foreground and background upgrades, no duplicate helper, preservation of
  running/paused intent, selected files, queue, paths, and settings after relaunch.
- Exercise import/file-operation deferral, failed shutdown, denied background
  permission, cancellation, offline feed, and a deliberately corrupted update ZIP.
  Rejected or canceled updates must leave the current app usable.
- Record keyboard/VoiceOver, actual logout/login, sleep/wake, physical drive loss,
  and real network-interface recovery using the existing acceptance checklist.

Publish the accepted draft as a stable release and mark it latest. Then fetch the
public feed URL, confirm its version and enclosure, and test a manual update check
from the preceding installed build. Preserve the published bytes; corrections
require a new version and build.
