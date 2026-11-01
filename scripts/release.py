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
import subprocess
import tempfile
import xml.etree.ElementTree as ET

import build
import sparkle

ROOT = build.ROOT
REPOSITORY = "ahmetaltun/SeedHop"
SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
SECRETS = ("APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD",
           "APPLE_SIGNING_IDENTITY", "APPLE_TEAM_ID", "SPARKLE_PRIVATE_KEY")


def run(*command, input=None, sensitive=False):
    result = subprocess.run(list(map(str, command)), cwd=ROOT, input=input,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        # Tool failures can echo credentials supplied as arguments. Never forward those outputs.
        detail = "" if sensitive else result.stderr.decode(errors="replace")[-4000:]
        raise RuntimeError(f"{pathlib.Path(str(command[0])).name} failed ({result.returncode}). {detail}")
    return result.stdout


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
    public_der = run("openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER",
                     input=bytes.fromhex("302e020100300506032b657004220420") + seed, sensitive=True)
    public_key = json.loads((ROOT / "macOS/Resources/Updates.json").read_text())["publicKey"]
    if base64.b64encode(public_der[-32:]).decode() != public_key:
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
    response = json.loads(run("xcrun", "notarytool", "submit", path, *credentials,
                              "--wait", "--timeout", "45m", "--output-format", "json", sensitive=True))
    (output / f"{path.stem}-notary.json").write_text(json.dumps(response, indent=2) + "\n")
    if response.get("status") != "Accepted":
        if response.get("id"):
            log = run("xcrun", "notarytool", "log", response["id"], *credentials, sensitive=True)
            (output / f"{path.stem}-notary-log.json").write_bytes(log)
        raise ValueError(f"Notarization not accepted for {path.name}; inspect retained notary results")
    return response["id"]


def verify_app(app, team):
    run("codesign", "--verify", "--deep", "--strict", "--verbose=2", app)
    contents = app / "Contents"
    paths = [contents / "MacOS/SeedHop", contents / "MacOS/SeedHopAgent",
             contents / "XPCServices/SeedHopEngine.xpc", contents / "Frameworks/libseedhop_ffi.dylib"]
    framework = contents / "Frameworks/Sparkle.framework"
    paths += [framework, framework / "Versions/B/Autoupdate", framework / "Versions/B/Updater.app",
              framework / "Versions/B/XPCServices/Downloader.xpc", framework / "Versions/B/XPCServices/Installer.xpc"]
    for path in [app, *paths]:
        info = subprocess.run(["codesign", "-d", "--verbose=4", str(path)], capture_output=True, check=True).stderr.decode()
        if f"TeamIdentifier={team}" not in info or "Timestamp=" not in info or "runtime" not in info:
            raise ValueError(f"Missing team, timestamp or Hardened Runtime: {path}")
        entitlements = run("codesign", "-d", "--entitlements", ":-", path)
        values = plistlib.loads(entitlements) if entitlements.strip() else {}
        if values.get("com.apple.security.cs.disable-library-validation") or values.get("com.apple.security.get-task-allow"):
            raise ValueError(f"Development entitlement in release: {path}")
    for path in [contents / "MacOS/SeedHop", contents / "MacOS/SeedHopAgent",
                 contents / "XPCServices/SeedHopEngine.xpc/Contents/MacOS/SeedHopEngine",
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
        run("xcrun", "stapler", "validate", mounted_app)
        run("spctl", "--assess", "--type", "execute", mounted_app)
        if bundle_manifest(mounted_app) != bundle_manifest(app):
            raise ValueError("DMG app differs from the verified build")
        if os.readlink(mounts[0] / "Applications") != "/Applications":
            raise ValueError("DMG lacks the Applications shortcut")
    finally:
        for mount in mounts: run("hdiutil", "detach", mount)


def package(tag, number, output):
    version, commit = preflight(tag, number)
    certificate = validate_secrets(os.environ)
    if output.exists() and any(output.iterdir()): raise ValueError("Release output directory must be empty")
    output.mkdir(parents=True, exist_ok=True)
    identity = os.environ["APPLE_SIGNING_IDENTITY"]
    with tempfile.TemporaryDirectory(prefix="seedhop-release-") as temporary:
        directory = pathlib.Path(temporary)
        private_key = directory / "sparkle.key"
        private_key.write_text(os.environ["SPARKLE_PRIVATE_KEY"].strip()); private_key.chmod(0o600)
        with signing_keychain(directory, certificate, os.environ):
            # Fresh bundle avoids carrying development-only resources into distribution.
            app = ROOT / ".build/SeedHop.app"
            if app.exists():
                import shutil
                shutil.rmtree(app)
            run("python3", "scripts/build.py", "--release", "--identity", identity, "--build-number", number)
            verify_app(app, os.environ["APPLE_TEAM_ID"])
            submission = directory / "SeedHop-notarization.zip"
            run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, submission)
            app_submission = notarize(submission, os.environ, output)
            run("xcrun", "stapler", "staple", app)
            run("xcrun", "stapler", "validate", app)
            run("spctl", "--assess", "--type", "execute", "--verbose=4", app)
            updates = directory / "updates"; updates.mkdir()
            archive = updates / f"SeedHop-{version}-arm64.zip"
            run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
            stage = directory / "dmg"; stage.mkdir()
            run("ditto", app, stage / "SeedHop.app")
            (stage / "Applications").symlink_to("/Applications")
            dmg = output / f"SeedHop-{version}-arm64.dmg"
            run("hdiutil", "create", "-volname", "SeedHop", "-srcfolder", stage, "-format", "UDZO", dmg)
            run("codesign", "--force", "--timestamp", "--sign", identity, dmg)
            dmg_submission = notarize(dmg, os.environ, output)
            run("xcrun", "stapler", "staple", dmg)
            run("xcrun", "stapler", "validate", dmg)
            run("codesign", "--verify", "--strict", dmg)
            run("spctl", "--assess", "--type", "open", "--context", "context:primary-signature", dmg)
            verify_dmg(dmg, app, os.environ["APPLE_TEAM_ID"])
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
                                       str(private_key), str(corrupted), signature], capture_output=True)
            if rejected.returncode == 0: raise ValueError("Sparkle accepted a corrupted update archive")
            # Verify the exact extracted update payload, rather than only the build directory.
            extracted = directory / "extracted"; extracted.mkdir()
            run("ditto", "-x", "-k", archive, extracted)
            verify_app(extracted / "SeedHop.app", os.environ["APPLE_TEAM_ID"])
            run("xcrun", "stapler", "validate", extracted / "SeedHop.app")
            run("spctl", "--assess", "--type", "execute", extracted / "SeedHop.app")
            if bundle_manifest(extracted / "SeedHop.app") != bundle_manifest(app):
                raise ValueError("ZIP app differs from the verified build")
            import shutil
            shutil.copy2(archive, output / archive.name)
            shutil.copy2(appcast, output / "appcast.xml")
            metadata = dict(version=version, buildNumber=number, commit=commit, tag=tag,
                            architecture="arm64", minimumSystemVersion="15.0", sparkleVersion=sparkle.VERSION,
                            signingIdentity=identity, teamID=os.environ["APPLE_TEAM_ID"],
                            notarization=dict(app=app_submission, dmg=dmg_submission),
                            xcode=run("xcodebuild", "-version").decode().strip(),
                            rust=run("rustc", "--version").decode().strip(),
                            automatedVerification="passed", corruptUpdateRejected=True, manualAcceptance="pending")
            (output / "release-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
            files = sorted(path for path in output.iterdir() if path.is_file())
            (output / "SHA256SUMS").write_text("".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in files))
    print(f"Verified release assets: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--build-number", required=True, type=build.build_number)
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / ".build/release")
    args = parser.parse_args()
    try:
        package(args.tag, args.build_number, args.output.resolve())
    except (ValueError, RuntimeError) as error:
        raise SystemExit(str(error))
