"""Pinned Sparkle distribution used by the custom native build."""
import hashlib
import pathlib
import subprocess
import tarfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
VERSION = "2.10.1"
SHA256 = "1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27"
URL = f"https://github.com/sparkle-project/Sparkle/releases/download/{VERSION}/Sparkle-{VERSION}.tar.xz"


def distribution():
    directory = ROOT / ".cache/sparkle" / VERSION
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / "distribution.tar.xz"
    if not archive.exists():
        temporary = archive.with_suffix(".download")
        try:
            urllib.request.urlretrieve(URL, temporary)
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise ValueError("Sparkle distribution checksum mismatch; remove the cached archive and retry.")
    # Re-extract the verified archive rather than trusting mutable cached binaries.
    with tarfile.open(archive) as source:
        source.extractall(directory, filter="data")
    return directory


def embed(directory, destination, identity):
    """Preserve framework symlinks; retain each helper's upstream entitlements."""
    subprocess.run(["ditto", str(directory / "Sparkle.framework"), str(destination)], check=True)
    version = destination / "Versions/B"
    paths = [version / "XPCServices/Downloader.xpc", version / "XPCServices/Installer.xpc",
             version / "Autoupdate", version / "Updater.app", destination]
    for path in paths:
        result = subprocess.run(["codesign", "-d", "--entitlements", ":-", str(path)],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
        if path.suffix in (".xpc", ".app"):
            import plistlib
            executable = plistlib.loads((path / "Contents/Info.plist").read_bytes())["CFBundleExecutable"]
            binary = path / "Contents/MacOS" / executable
        else:
            binary = version / "Sparkle" if path == destination else path
        architectures = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).split()
        if "arm64" not in architectures: raise ValueError(f"Sparkle binary has no arm64 slice: {binary}")
        if architectures != ["arm64"]:
            thin = binary.with_name(binary.name + ".arm64")
            subprocess.run(["lipo", str(binary), "-thin", "arm64", "-output", str(thin)], check=True)
            thin.chmod(binary.stat().st_mode)
            thin.replace(binary)
        command = ["codesign", "--force", "--options", "runtime", "--sign", identity]
        if identity != "-": command.append("--timestamp")
        entitlements = directory / "signing.entitlements"
        try:
            if result.stdout.strip():
                entitlements.write_bytes(result.stdout)
                command += ["--entitlements", str(entitlements)]
            subprocess.run([*command, str(path)], check=True)
        finally:
            entitlements.unlink(missing_ok=True)


if __name__ == "__main__":
    print(distribution())
