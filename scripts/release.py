#!/usr/bin/env python3
"""Build signed, notarized release assets. Secrets are read only from environment."""
import argparse
import base64
import contextlib
import hashlib
import json
import os
import pathlib
import plistlib
import re
import shlex
import signal
import subprocess
import tempfile
import time
import shutil
import xml.etree.ElementTree as ET

import build
import sparkle
import security_inventory

ROOT = build.ROOT
REPOSITORY = "ahmetaltun/SeedHop"
SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
SECRETS = ("APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD",
           "APPLE_SIGNING_IDENTITY", "APPLE_TEAM_ID", "SPARKLE_PRIVATE_KEY")


class ToolError(RuntimeError):
    def __init__(self, message, returncode):
        super().__init__(message)
        self.returncode = returncode


def run(*command, input=None, sensitive=False, stream=False, timeout=300, heartbeat=None):
    if sensitive and stream:
        raise ValueError("Credential-bearing commands cannot stream raw output")
    arguments = list(map(str, command))
    started = time.monotonic()
    try:
        if heartbeat:
            with subprocess.Popen(arguments, cwd=ROOT, stdin=subprocess.PIPE,
                                  stdout=None if stream else subprocess.PIPE,
                                  stderr=None if stream else subprocess.PIPE,
                                  start_new_session=True) as process:
                pending_input = input
                while True:
                    remaining = timeout - (time.monotonic() - started)
                    if remaining <= 0:
                        os.killpg(process.pid, signal.SIGKILL); process.communicate()
                        raise RuntimeError(f"{heartbeat} timed out after {timeout}s; inspect retained submission ID")
                    try:
                        stdout, stderr = process.communicate(input=pending_input, timeout=min(30, remaining))
                        result = subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)
                        break
                    except subprocess.TimeoutExpired:
                        pending_input = None
                        print(f"{heartbeat}: still running ({int(time.monotonic() - started)}s)", flush=True)
        else:
            result = subprocess.run(arguments, cwd=ROOT, input=input, timeout=timeout,
                                    stdout=None if stream else subprocess.PIPE,
                                    stderr=None if stream else subprocess.PIPE)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{pathlib.Path(arguments[0]).name} timed out after {timeout}s") from None
    if result.returncode:
        # Tool failures can echo credentials supplied as arguments. Never forward those outputs.
        detail = "" if sensitive or stream else (result.stdout + result.stderr).decode(errors="replace")[-4000:]
        raise ToolError(f"{pathlib.Path(str(command[0])).name} failed ({result.returncode}). {detail}", result.returncode)
    return result.stdout or b""


def stapler(action, path):
    for attempt in range(1, 4):
        try:
            run("xcrun", "stapler", action, path, timeout=120,
                heartbeat=f"Apple ticket {action}: {path.name}")
            return
        except ToolError as error:
            # stapler uses sysexits: host/service unavailable or temporary failure.
            # Invalid/missing tickets (65) and signature errors must still fail.
            if error.returncode not in (68, 69, 75) or attempt == 3:
                raise
            print(f"Apple ticket service temporarily unavailable; retry {attempt}/2 in {15 * attempt}s", flush=True)
            time.sleep(15 * attempt)


def validate_tag(tag, version):
    if tag != f"v{version}":
        raise ValueError(f"Tag must match workspace version: v{version}")


def validate_secrets(environ):
    missing = [name for name in SECRETS if not environ.get(name, "").strip()]
    if missing: raise ValueError("Missing secrets: " + ", ".join(missing))
    identity, team = environ["APPLE_SIGNING_IDENTITY"], environ["APPLE_TEAM_ID"]
    if not re.fullmatch(r"[A-Z0-9]{10}", team): raise ValueError("Invalid APPLE_TEAM_ID")
    if not identity.startswith("Developer ID Application:") or not identity.endswith(f"({team})"):
        raise ValueError("APPLE_SIGNING_IDENTITY must be Developer ID Application for APPLE_TEAM_ID")
    try:
        certificate = base64.b64decode("".join(environ["APPLE_CERTIFICATE"].split()), validate=True)
        seed = base64.b64decode(environ["SPARKLE_PRIVATE_KEY"].strip(), validate=True)
    except ValueError:
        raise ValueError("Certificate and Sparkle private key must be valid base64") from None
    if not certificate: raise ValueError("APPLE_CERTIFICATE is empty")
    if len(seed) != 32: raise ValueError("SPARKLE_PRIVATE_KEY must be a Sparkle-generated 32-byte seed in base64")
    # macOS's system LibreSSL cannot decode Ed25519 keys. CryptoKit is part of macOS.
    (ROOT / ".cache/swift").mkdir(parents=True, exist_ok=True)
    public = run("swift", "-module-cache-path", ROOT / ".cache/swift", ROOT / "scripts/SparklePublicKey.swift",
                 input=(environ["SPARKLE_PRIVATE_KEY"].strip() + "\n").encode(), sensitive=True).decode().strip()
    public_key = json.loads((ROOT / "macOS/Resources/Updates.json").read_text())["publicKey"]
    if public != public_key:
        raise ValueError("SPARKLE_PRIVATE_KEY does not match the committed public key")
    return certificate


def preflight(tag, number):
    version = build.workspace_version()
    validate_tag(tag, version)
    build.build_number(number)
    commit = run("git", "rev-parse", f"refs/tags/{tag}^{{commit}}").decode().strip()
    if run("git", "rev-parse", "HEAD").decode().strip() != commit:
        raise ValueError("Checkout must point to the requested tag")
    run("git", "merge-base", "--is-ancestor", commit, "origin/master")
    releases = json.loads(run("gh", "api", f"repos/{REPOSITORY}/releases", "--paginate", "--slurp"))
    for release in (item for page in releases for item in page):
        if release["tag_name"] == tag and not release["draft"]:
            raise ValueError("Refusing to overwrite a published release")
        if release["draft"] or release["prerelease"]: continue
        old_version = release["tag_name"].removeprefix("v")
        if re.fullmatch(r"\d+\.\d+\.\d+", old_version):
            if tuple(map(int, old_version.split("."))) >= tuple(map(int, version.split("."))):
                raise ValueError("Release version must exceed every published stable version")
        asset = next((a for a in release["assets"] if a["name"] == "release-metadata.json"), None)
        if asset:
            previous = json.loads(run("curl", "--fail", "--location", "--silent", "--show-error", asset["browser_download_url"]))
            if int(number) <= int(previous["buildNumber"]):
                raise ValueError("Build number must exceed every published stable build")
    return version, commit


@contextlib.contextmanager
def signing_keychain(directory, certificate, environ):
    keychain = directory / "release.keychain-db"
    old_search = shlex.split(run("security", "list-keychains", "-d", "user").decode())
    password = os.urandom(32).hex()
    p12 = directory / "certificate.p12"
    p12.write_bytes(certificate); p12.chmod(0o600)
    try:
        run("security", "create-keychain", "-p", password, keychain, sensitive=True)
        run("security", "set-keychain-settings", "-lut", "21600", keychain)
        run("security", "unlock-keychain", "-p", password, keychain, sensitive=True)
        run("security", "list-keychains", "-d", "user", "-s", keychain, *old_search)
        run("security", "import", p12, "-k", keychain, "-P", environ["APPLE_CERTIFICATE_PASSWORD"],
            "-T", "/usr/bin/codesign", "-T", "/usr/bin/security", sensitive=True)
        run("security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", password,
            keychain, sensitive=True)
        identities = run("security", "find-identity", "-v", "-p", "codesigning", keychain).decode()
        if f'"{environ["APPLE_SIGNING_IDENTITY"]}"' not in identities:
            raise ValueError("Imported certificate lacks a valid matching Developer ID private key")
        yield
    finally:
        try:
            run("security", "list-keychains", "-d", "user", "-s", *old_search)
        finally:
            if keychain.exists(): run("security", "delete-keychain", keychain, sensitive=True)


def notarize(path, environ, output):
    credentials = ["--apple-id", environ["APPLE_ID"], "--password", environ["APPLE_PASSWORD"],
                   "--team-id", environ["APPLE_TEAM_ID"]]
    print(f"Submitting {path.name} for notarization", flush=True)
    submission = json.loads(run("xcrun", "notarytool", "submit", path, *credentials,
                               "--output-format", "json", sensitive=True, timeout=600,
                               heartbeat=f"Uploading {path.name} to Apple"))
    # Preserve the ID before waiting: Apple continues processing after a timeout.
    (output / f"{path.stem}-notary.json").write_text(json.dumps(submission, indent=2) + "\n")
    print(f"Notarization submission: {submission['id']}", flush=True)
    response = json.loads(run("xcrun", "notarytool", "wait", submission["id"], *credentials,
                              "--timeout", "45m", "--output-format", "json", sensitive=True,
                              timeout=2760, heartbeat=f"Apple notarization {submission['id']}"))
    (output / f"{path.stem}-notary.json").write_text(json.dumps(response, indent=2) + "\n")
    print(f"Apple notarization result: {response.get('status', 'Unknown')}", flush=True)
    # Apple's log can include warnings even for an accepted submission.
    if response.get("id"):
        log = run("xcrun", "notarytool", "log", response["id"], *credentials, sensitive=True)
        (output / f"{path.stem}-notary-log.json").write_bytes(log)
    if response.get("status") != "Accepted":
        raise ValueError(f"Notarization not accepted for {path.name}; inspect retained notary results")
    return response["id"]


def verify_app(app, team):
    run("codesign", "--verify", "--deep", "--strict", "--verbose=2", app)
    contents = app / "Contents"
    private_engine = contents / "XPCServices/SeedHopPrivateEngine.xpc"
    shared = [app, contents / "MacOS/SeedHop", contents / "MacOS/SeedHopAgent", contents / "XPCServices/SeedHopEngine.xpc"]
    paths = [*shared, private_engine, contents / "Frameworks/libseedhop_ffi.dylib"]
    framework = contents / "Frameworks/Sparkle.framework"
    paths += [framework, framework / "Versions/B/Autoupdate", framework / "Versions/B/Updater.app",
              framework / "Versions/B/XPCServices/Downloader.xpc", framework / "Versions/B/XPCServices/Installer.xpc"]
    for path in paths:
        info = subprocess.run(["codesign", "-d", "--verbose=4", str(path)], capture_output=True, check=True).stderr.decode()
        if f"TeamIdentifier={team}" not in info or "Timestamp=" not in info or "runtime" not in info:
            raise ValueError(f"Missing team, timestamp or Hardened Runtime: {path}")
        entitlements = run("codesign", "-d", "--entitlements", ":-", path)
        values = plistlib.loads(entitlements) if entitlements.strip() else {}
        if values.get("com.apple.security.cs.disable-library-validation") or values.get("com.apple.security.get-task-allow"):
            raise ValueError(f"Development entitlement in release: {path}")
        groups = values.get("com.apple.security.application-groups", [])
        if path in shared and (not values.get("com.apple.security.app-sandbox") or f"{team}.app.seedhop.mac.shared" not in groups):
            raise ValueError(f"Missing SeedHop sandbox entitlements: {path}")
        if path == private_engine and (not values.get("com.apple.security.app-sandbox") or groups or values.get("com.apple.security.files.bookmarks.app-scope")):
            raise ValueError(f"Invalid private engine entitlements: {path}")
    for path in [contents / "MacOS/SeedHop", contents / "MacOS/SeedHopAgent",
                 contents / "XPCServices/SeedHopEngine.xpc/Contents/MacOS/SeedHopEngine",
                 contents / "XPCServices/SeedHopPrivateEngine.xpc/Contents/MacOS/SeedHopEngine",
                 contents / "Frameworks/libseedhop_ffi.dylib"]:
        build.verify_arm64(path)


def validate_appcast(path, archive, tag, number):
    root = ET.parse(path).getroot()
    items = root.findall("./channel/item")
    if len(items) != 1: raise ValueError("Expected one stable full-update appcast item")
    item = items[0]
    enclosure = item.find("enclosure")
    if enclosure is None: raise ValueError("Appcast has no download")
    expected = f"https://github.com/{REPOSITORY}/releases/download/{tag}/{archive.name}"
    if enclosure.get("url") != expected or int(enclosure.get("length", "0")) != archive.stat().st_size:
        raise ValueError("Appcast download URL or size mismatch")
    def value(key):
        return item.findtext(f"{{{SPARKLE_NS}}}{key}") or enclosure.get(f"{{{SPARKLE_NS}}}{key}")
    if value("version") != number or value("shortVersionString") != tag[1:]:
        raise ValueError("Appcast version mismatch")
    if not enclosure.get(f"{{{SPARKLE_NS}}}edSignature"):
        raise ValueError("Appcast lacks an Ed25519 signature")
    if item.findtext(f"{{{SPARKLE_NS}}}minimumSystemVersion") not in ("15.0", "15.0.0"):
        raise ValueError("Appcast has incorrect macOS requirement")
    return enclosure.get(f"{{{SPARKLE_NS}}}edSignature")


def bundle_manifest(app):
    return {str(path.relative_to(app)): ("link:" + os.readlink(path) if path.is_symlink()
            else hashlib.sha256(path.read_bytes()).hexdigest())
            for path in sorted(app.rglob("*")) if path.is_file() or path.is_symlink()}


def verify_dmg(dmg, app, team):
    attached = plistlib.loads(run("hdiutil", "attach", "-readonly", "-nobrowse", "-plist", dmg))
    mounts = [pathlib.Path(entity["mount-point"]) for entity in attached["system-entities"] if "mount-point" in entity]
    try:
        if len(mounts) != 1: raise ValueError("Expected one mounted release volume")
        mounted_app = mounts[0] / "SeedHop.app"
        verify_app(mounted_app, team)
        # The stapled outer DMG carries the ticket for its nested app. Keep the
        # signed image immutable; staple the standalone app later for ZIP users.
        run("spctl", "--assess", "--type", "execute", mounted_app)
        if bundle_manifest(mounted_app) != bundle_manifest(app):
            raise ValueError("DMG app differs from the verified build")
        if os.readlink(mounts[0] / "Applications") != "/Applications":
            raise ValueError("DMG lacks the Applications shortcut")
        run("python3", "scripts/dmg.py", "--verify-mounted", mounts[0], stream=True)
    finally:
        for mount in mounts: run("hdiutil", "detach", mount)


PHASES = ("build", "dmg", "app", "finalize")


def save_state(path, state, app, phase):
    state.update(phase=phase, manifest=bundle_manifest(app))
    path.write_text(json.dumps(state, indent=2) + "\n")


def load_state(path, app, phase, commit, number):
    state = json.loads(path.read_text())
    previous = PHASES[PHASES.index(phase) - 1]
    if state.get("phase") != previous or state.get("commit") != commit or state.get("buildNumber") != number:
        raise ValueError("Release phase, commit, or build number mismatch; start a fresh build")
    if state.get("manifest") != bundle_manifest(app):
        raise ValueError("App changed between release stages")
    return state


def package(tag, number, output, phase="all"):
    if phase == "all":
        for part in PHASES:
            package(tag, number, output, part)
        return
    started = time.monotonic()
    print(f"Starting release stage: {phase}", flush=True)
    version, commit = preflight(tag, number)
    certificate = validate_secrets(os.environ)
    app = ROOT / ".build/SeedHop.app"
    state_path = ROOT / ".build/release-state.json"
    if phase == "build":
        if output.exists() and any(output.iterdir()):
            raise ValueError("Release output directory must be empty")
        output.mkdir(parents=True, exist_ok=True)
        state = dict(commit=commit, buildNumber=number)
    else:
        state = load_state(state_path, app, phase, commit, number)
    identity = os.environ["APPLE_SIGNING_IDENTITY"]
    with tempfile.TemporaryDirectory(prefix="seedhop-release-") as temporary:
        directory = pathlib.Path(temporary)
        private_key = directory / "sparkle.key"
        private_key.write_text(os.environ["SPARKLE_PRIVATE_KEY"].strip()); private_key.chmod(0o600)
        signing = signing_keychain(directory, certificate, os.environ) if phase in ("build", "dmg") else contextlib.nullcontext()
        with signing:
            if phase == "build":
                print("Signing identity imported; building optimized app", flush=True)
                # Fresh bundle avoids carrying development-only resources into distribution.
                app = ROOT / ".build/SeedHop.app"
                if app.exists():
                    shutil.rmtree(app)
                run("python3", "-u", "scripts/build.py", "--release", "--sandbox", "--sandbox-team-id", os.environ["APPLE_TEAM_ID"], "--identity", identity, "--build-number", number,
                    stream=True, timeout=2400, heartbeat="Optimized native build")
                verify_app(app, os.environ["APPLE_TEAM_ID"])
                print("Nested signatures, entitlements, timestamps, and architecture verified", flush=True)
            elif phase == "app":
                # The DMG submission already generated this app's ticket.
                # Stapling adds it to the standalone bundle without rebuilding
                # or re-signing either the app or the already verified DMG.
                stapler("staple", app)
                stapler("validate", app)
                verify_app(app, os.environ["APPLE_TEAM_ID"])
                run("spctl", "--assess", "--type", "execute", "--verbose=4", app)
                print("App ticket from DMG submission stapled and verified; no second upload required", flush=True)
                state["appSubmission"] = state["dmgSubmission"]
            elif phase == "dmg":
                dmg = output / f"SeedHop-{version}-arm64.dmg"
                run("python3", "-u", "scripts/dmg.py", "--app", app, "--output", dmg,
                    stream=True, timeout=600, heartbeat="Creating branded installer")
                run("codesign", "--force", "--timestamp", "--sign", identity, dmg)
                dmg_submission = notarize(dmg, os.environ, output)
                stapler("staple", dmg)
                stapler("validate", dmg)
                run("codesign", "--verify", "--strict", dmg)
                run("spctl", "--assess", "--type", "open", "--context", "context:primary-signature", dmg)
                verify_dmg(dmg, app, os.environ["APPLE_TEAM_ID"])
                print("DMG ticket, mounted app, and installer layout verified", flush=True)
                state["dmgSubmission"] = dmg_submission
            else:
                app_submission, dmg_submission = state["appSubmission"], state["dmgSubmission"]
                dmg = output / f"SeedHop-{version}-arm64.dmg"
                expected = state["dmgHash"]
                if hashlib.sha256(dmg.read_bytes()).hexdigest() != expected:
                    raise ValueError("DMG changed between release stages")
                updates = directory / "updates"; updates.mkdir()
                archive = updates / f"SeedHop-{version}-arm64.zip"
                run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
                distribution = sparkle.distribution()
                run(distribution / "bin/generate_appcast", "--ed-key-file", private_key, "--maximum-deltas", "0",
                    "--download-url-prefix", f"https://github.com/{REPOSITORY}/releases/download/{tag}/",
                    "--link", f"https://github.com/{REPOSITORY}/releases/tag/{tag}", updates, sensitive=True)
                appcast = updates / "appcast.xml"
                signature = validate_appcast(appcast, archive, tag, number)
                run(distribution / "bin/sign_update", "--verify", "--ed-key-file", private_key, archive, signature, sensitive=True)
                corrupted = directory / "corrupted.zip"
                corrupted.write_bytes(archive.read_bytes() + b"corrupted-update-fixture")
                rejected = subprocess.run([str(distribution / "bin/sign_update"), "--verify", "--ed-key-file",
                                           str(private_key), str(corrupted), signature], capture_output=True, timeout=60)
                if rejected.returncode == 0: raise ValueError("Sparkle accepted a corrupted update archive")
                # Verify the exact extracted update payload, rather than only the build directory.
                extracted = directory / "extracted"; extracted.mkdir()
                run("ditto", "-x", "-k", archive, extracted)
                verify_app(extracted / "SeedHop.app", os.environ["APPLE_TEAM_ID"])
                stapler("validate", extracted / "SeedHop.app")
                run("spctl", "--assess", "--type", "execute", extracted / "SeedHop.app")
                if bundle_manifest(extracted / "SeedHop.app") != bundle_manifest(app):
                    raise ValueError("ZIP app differs from the verified build")
                shutil.copy2(archive, output / archive.name)
                shutil.copy2(appcast, output / "appcast.xml")
                metadata = dict(version=version, buildNumber=number, commit=commit, tag=tag,
                                architecture="arm64", minimumSystemVersion="15.0", sparkleVersion=sparkle.VERSION,
                                signingIdentity=identity, teamID=os.environ["APPLE_TEAM_ID"],
                                notarization=dict(app=app_submission, dmg=dmg_submission,
                                                  strategy="outermost-dmg", submissionCount=1),
                                xcode=run("xcodebuild", "-version").decode().strip(),
                                rust=run("rustc", "--version").decode().strip(),
                                automatedVerification="passed", corruptUpdateRejected=True, manualAcceptance="pending")
                (output / "release-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
                (output / "dependencies.json").write_text(json.dumps(security_inventory.inventory(), indent=2) + "\n")
                files = sorted(path for path in output.iterdir() if path.is_file())
                (output / "SHA256SUMS").write_text("".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in files))
    if phase == "dmg":
        state["dmgHash"] = hashlib.sha256(dmg.read_bytes()).hexdigest()
    save_state(state_path, state, app, phase)
    print(f"Completed release stage: {phase} ({int(time.monotonic() - started)}s)", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("all", *PHASES), default="all")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--build-number", required=True, type=build.build_number)
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / ".build/release")
    args = parser.parse_args()
    try:
        package(args.tag, args.build_number, args.output.resolve(), args.phase)
    except (ValueError, RuntimeError) as error:
        raise SystemExit(str(error))
