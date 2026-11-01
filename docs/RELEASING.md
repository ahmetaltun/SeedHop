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
2. Merge the release changes into `master`. Review CI results.
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

The workflow runs CI without Apple secrets, then imports the certificate into a
temporary keychain. It restores the keychain search list and deletes temporary
signing material even on ordinary failures. GitHub destroys the runner after
cancellation. No personal keychain or signing identity is required on a developer's
machine to run ordinary CI.

Expected draft assets:

- `SeedHop-0.1.0-arm64.dmg`: signed, notarized disk image with an Applications shortcut.
- `SeedHop-0.1.0-arm64.zip`: notarized, stapled app; also used by Sparkle.
- `appcast.xml`: Sparkle-generated feed with the final ZIP's signature and size.
- `SHA256SUMS`: SHA-256 for every other asset.
- `release-metadata.json`: commit, build number, toolchains, identity, and notary submission IDs.
- Notarization JSON results, with rejection logs when available.

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

CI is automated evidence, not completion of the manual release gates in
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
