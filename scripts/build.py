#!/usr/bin/env python3
"""Build a native .app using Xcode's compiler; no project generator is required."""
import argparse
import os
import pathlib
import plistlib
import json
import shutil
import subprocess
import hashlib
import platform
import re
import tomllib
import sparkle
import sandbox
from localization import LANGUAGES, check as check_localizations, normalize as normalize_localizations, pseudolocalize

ROOT = pathlib.Path(__file__).resolve().parent.parent
SIGNING_CONFIG = ROOT / '.build/signing-identity'
RUST_TARGET = "aarch64-apple-darwin"

def workspace_version():
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("Distribution requires a stable major.minor.patch workspace version.")
    return version

def build_number(value):
    if not re.fullmatch(r"[1-9]\d{0,17}", value):
        raise argparse.ArgumentTypeError("Build number must be a positive integer of at most 18 digits.")
    return value

def run(*args):
    print("+", " ".join(map(str,args)),flush=True)
    subprocess.run(list(map(str,args)),cwd=ROOT,check=True)

def write_plist(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(plistlib.dumps(value))

def verify_arm64(path):
    architectures = subprocess.check_output(["lipo", "-archs", str(path)], text=True).split()
    if architectures != ["arm64"]:
        raise RuntimeError(f"{path} must contain only arm64; found: {' '.join(architectures)}")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release",action="store_true")
    parser.add_argument("--build-number", type=build_number, default="1", help="Increasing distribution build number")
    parser.add_argument("--identity", default=os.environ.get('SEEDHOP_SIGNING_IDENTITY') or (SIGNING_CONFIG.read_text().strip() if SIGNING_CONFIG.exists() else '-'), help="Signing identity; defaults to SEEDHOP_SIGNING_IDENTITY, saved local identity, then ad-hoc")
    parser.add_argument("--save-identity", action="store_true", help="Save the validated identity locally for subsequent builds")
    parser.add_argument("--validation-root", type=pathlib.Path, help="Build an isolated development lifecycle fixture with a distinct service namespace and library")
    parser.add_argument("--skip-rust",action="store_true")
    parser.add_argument("--sandbox", action="store_true", help="Enable App Sandbox in a signed release or isolated validation build")
    parser.add_argument("--sandbox-team-id", help="Ten-character Apple Developer team ID matching the signing certificate")
    parser.add_argument("--update-localizations",action="store_true",help="Extract and sync English source strings into catalogs")
    parser.add_argument("--pseudolocalization",action="store_true",help="Include development-only expanded en-XA text")
    args = parser.parse_args()
    if args.release and args.pseudolocalization:
        parser.error("Pseudolocalization is only available in development builds.")
    if args.release and args.update_localizations:
        parser.error("Update catalogs in a development build before running release validation.")
    if args.release and not args.sandbox:
        parser.error("Release builds require --sandbox and --sandbox-team-id.")
    if args.sandbox and (not (args.release or args.validation_root) or args.identity == '-'):
        parser.error("Sandbox builds require a signing identity and --release or --validation-root.")
    if args.sandbox != bool(args.sandbox_team_id):
        parser.error("Use --sandbox and --sandbox-team-id together.")
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("SeedHop builds require Apple Silicon macOS and native arm64 tools. Run outside Rosetta.")
    if args.identity != '-':
        identities = subprocess.check_output(['security', 'find-identity', '-v', '-p', 'codesigning'], text=True)
        if args.identity not in identities:
            parser.error('The requested signing identity is not valid or its private key is unavailable in this keychain.')
    if args.release and not args.identity.startswith('Developer ID Application:'):
        parser.error('Release builds require a Developer ID Application signing identity.')
    if args.validation_root and (args.release or args.identity == '-'):
        parser.error('Lifecycle fixtures require a certificate-backed development build.')
    if args.save_identity:
        SIGNING_CONFIG.parent.mkdir(parents=True, exist_ok=True)
        SIGNING_CONFIG.write_text(args.identity + '\n')
    profile = "release" if args.release else "debug"
    target = ROOT/"target"/RUST_TARGET/profile
    generated = ROOT/"macOS/Generated"
    generated.mkdir(parents=True,exist_ok=True)
    (ROOT/".cache/swift").mkdir(parents=True,exist_ok=True)
    if not args.skip_rust:
        cargo = ["cargo","build","--locked","--target",RUST_TARGET,"--target-dir",ROOT/"target","-p","seedhop-ffi","--features","bindgen"]
        if args.release: cargo.append("--release")
        run(*cargo)
        run(target/"uniffi-bindgen","generate","--library",target/"libseedhop_ffi.dylib","--language","swift","--out-dir",generated)
    verify_arm64(target/"libseedhop_ffi.dylib")
    validation_root = args.validation_root.resolve() if args.validation_root else None
    validation_prefix = 'app.seedhop.sandbox.' if args.sandbox else 'app.seedhop.validation.'
    app_id = validation_prefix + hashlib.sha256(str(validation_root).encode()).hexdigest()[:12] if validation_root else 'app.seedhop.mac'
    try:
        sandbox_config = sandbox.configuration(app_id, args.sandbox_team_id) if args.sandbox else None
    except ValueError as error:
        parser.error(str(error))
    background_id = sandbox_config['group'] + '.agent' if sandbox_config else app_id + '.agent'
    app = (validation_root or ROOT/'.build') / 'SeedHop.app'
    contents = app/"Contents"
    for folder in ["MacOS","Frameworks","Resources","XPCServices/SeedHopEngine.xpc/Contents/MacOS","XPCServices/SeedHopPrivateEngine.xpc/Contents/MacOS","Library/LaunchAgents"]:
        (contents/folder).mkdir(parents=True,exist_ok=True)
    for name in ["LICENSE","THIRD_PARTY_NOTICES.md","vendor/UNICODE-LICENSE.txt"]:
        if (ROOT/name).exists(): shutil.copy2(ROOT/name,contents/"Resources"/pathlib.Path(name).name)
    license_root=contents/"Resources/ThirdPartyLicenses"
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--offline","--format-version","1"],cwd=ROOT,text=True))
    for package in metadata["packages"]:
        directory=pathlib.Path(package["manifest_path"]).parent
        for source in directory.iterdir():
            if source.is_file() and source.name.upper().startswith(("LICENSE","COPYING","NOTICE")):
                destination=license_root/(package["name"]+"-"+package["version"])
                destination.mkdir(parents=True,exist_ok=True); shutil.copy2(source,destination/source.name)
    supplemental_licenses = ROOT/"vendor/licenses"
    if supplemental_licenses.is_dir():
        shutil.copytree(supplemental_licenses, license_root, dirs_exist_ok=True)
    run("swift","-module-cache-path",ROOT/".cache/swift",ROOT/"scripts/Icon.swift",contents/"Resources/AppIcon.icns",ROOT/"macOS/Resources/AppIcon.png")
    base = {"CFBundleVersion":args.build_number,"CFBundleShortVersionString":workspace_version(),"LSMinimumSystemVersion":"15.0","LSArchitecturePriority":["arm64"],"LSRequiresNativeExecution":True,"CFBundleDevelopmentRegion":"en","CFBundleLocalizations":LANGUAGES + (["en-XA"] if args.pseudolocalization else []),"NSLocalNetworkUsageDescription":"SeedHop connects to torrent peers and trackers on networks you use."}
    write_plist(contents/"Info.plist",dict(base,CFBundleIdentifier=app_id,CFBundleName="SeedHop",CFBundleDisplayName="SeedHop",CFBundleExecutable="SeedHop",CFBundlePackageType="APPL",NSPrincipalClass="NSApplication",LSApplicationCategoryType="public.app-category.utilities",NSHighResolutionCapable=True,CFBundleIconFile="AppIcon",CFBundleURLTypes=[{"CFBundleURLName":"BitTorrent magnet link","CFBundleURLSchemes":["magnet"]}],CFBundleDocumentTypes=[{"CFBundleTypeName":"BitTorrent torrent","CFBundleTypeRole":"Viewer","LSHandlerRank":"Alternate","LSItemContentTypes":["org.bittorrent.torrent"],"CFBundleTypeExtensions":["torrent"]}],UTImportedTypeDeclarations=[{"UTTypeIdentifier":"org.bittorrent.torrent","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["torrent"],"public.mime-type":"application/x-bittorrent"}}]))
    updates = json.loads((ROOT/"macOS/Resources/Updates.json").read_text())
    if args.release and not updates.get("publicKey"):
        parser.error("A Sparkle public key is required for distribution.")
    app_plist = plistlib.loads((contents/"Info.plist").read_bytes())
    app_plist.update(SUFeedURL=updates["feedURL"], SUPublicEDKey=updates["publicKey"],
                     SUAllowsAutomaticUpdates=False, SUAutomaticallyUpdate=False,
                     SUEnableSystemProfiling=False)
    if sandbox_config:
        app_plist['SUEnableInstallerLauncherService'] = True
    write_plist(contents/"Info.plist", app_plist)
    sparkle_distribution = sparkle.distribution()
    sparkle.embed(sparkle_distribution, contents/"Frameworks/Sparkle.framework", args.identity)
    shutil.copy2(sparkle_distribution/"LICENSE", contents/"Resources/Sparkle-LICENSE")
    xpc = contents/"XPCServices/SeedHopEngine.xpc"
    private_xpc = contents/"XPCServices/SeedHopPrivateEngine.xpc"
    services = [(app_id+".engine",xpc), (app_id+".private-engine",private_xpc)]
    for identifier, service in services:
        write_plist(service/"Contents/Info.plist",dict(base,CFBundleIdentifier=identifier,CFBundleExecutable="SeedHopEngine",CFBundlePackageType="XPC!",XPCService={"ServiceType":"Application","RunLoopType":"NSRunLoop","JoinExistingSession":True}))
    for stale in (contents/"Library/LaunchAgents").glob("*.plist"):
        stale.unlink()
    write_plist(contents/f"Library/LaunchAgents/{background_id}.plist",{"Label":background_id,"BundleProgram":"Contents/MacOS/SeedHopAgent","ProgramArguments":["SeedHopAgent","--agent"],"MachServices":{background_id:True},"RunAtLoad":True,"KeepAlive":{"SuccessfulExit":False},"ThrottleInterval":5,"ProcessType":"Background","AssociatedBundleIdentifiers":[app_id]})
    shutil.copy2(target/"libseedhop_ffi.dylib",contents/"Frameworks/libseedhop_ffi.dylib")
    run("install_name_tool","-id","@rpath/libseedhop_ffi.dylib",contents/"Frameworks/libseedhop_ffi.dylib")
    common = ["swiftc","-target","arm64-apple-macosx15.0","-swift-version","5","-parse-as-library","-module-cache-path",ROOT/".cache/swift"]
    common += ["-O"] if args.release else ["-g"]
    if not args.release: common += ["-D","SEEDHOP_DEVELOPMENT"]
    if validation_root:
        configuration = validation_root / 'ValidationConfiguration.swift'
        configuration.write_text('import Foundation\nenum ValidationConfiguration {\n' +
            'static let app = ' + json.dumps(app_id) + '\n' +
            'static let root = ' + json.dumps(str(validation_root), ensure_ascii=False) + '\n}\n')
        common += ['-D', 'SEEDHOP_VALIDATION', configuration]
    if sandbox_config:
        sandbox_source = app.parent / 'SandboxConfiguration.swift'
        sandbox_source.write_text('enum SandboxConfiguration { static let group = ' + json.dumps(sandbox_config['group']) + ' }\n')
        common += ['-D', 'SEEDHOP_SANDBOX', sandbox_source]
    shared = sorted((ROOT/"macOS/Shared").glob("*.swift"))
    host = sorted((ROOT/"macOS/Host").glob("*.swift"))
    # Both executable locations find the single bundled, signed Rust dynamic library.
    ffi_args = ["-I",generated,"-Xcc",f"-fmodule-map-file={generated/'seedhop_ffiFFI.modulemap'}","-L",contents/"Frameworks","-lseedhop_ffi","-Xlinker","-rpath","-Xlinker","@executable_path/../Frameworks","-Xlinker","-rpath","-Xlinker","@executable_path/../../../../Frameworks"]
    for name,path in [("SeedHopEngine",xpc/"Contents/MacOS/SeedHopEngine"),("SeedHopAgent",contents/"MacOS/SeedHopAgent")]:
        run(*common,"-module-name",name,*shared,*host,generated/"seedhop_ffi.swift",*ffi_args,"-framework","Security","-o",path)
    shutil.copy2(xpc/"Contents/MacOS/SeedHopEngine",private_xpc/"Contents/MacOS/SeedHopEngine")
    extraction = ROOT/".build/localization-extraction"
    if extraction.exists(): shutil.rmtree(extraction)
    extraction.mkdir(parents=True)
    localization_args = ["-emit-localized-strings","-emit-localized-strings-path",extraction]
    run(*common,*localization_args,"-module-name","SeedHop",*shared,*sorted((ROOT/"macOS/App").glob("*.swift")),"-F",contents/"Frameworks","-framework","Sparkle","-Xlinker","-rpath","-Xlinker","@executable_path/../Frameworks","-framework","SwiftUI","-framework","AppKit","-framework","ServiceManagement","-framework","Security","-framework","UserNotifications","-o",contents/"MacOS/SeedHop")
    catalogs = ROOT/"macOS/Resources/Localization"
    if args.update_localizations:
        run("xcrun","xcstringstool","sync",catalogs/"Localizable.xcstrings","--stringsdata",*sorted(extraction.glob("*.stringsdata")))
        normalize_localizations(extraction)
    else:
        check_localizations(extraction=extraction)
    for stale in (contents/"Resources").glob("*.lproj"):
        shutil.rmtree(stale)
    for catalog in sorted(catalogs.glob("*.xcstrings")):
        run("xcrun","xcstringstool","compile",catalog,"--output-directory",contents/"Resources")
    for _, service in services:
        xpc_resources = service/"Contents/Resources"
        xpc_resources.mkdir(parents=True,exist_ok=True)
        for stale in xpc_resources.glob("*.lproj"): shutil.rmtree(stale)
        run("xcrun","xcstringstool","compile",catalogs/"InfoPlist.xcstrings","--output-directory",xpc_resources)
    if args.pseudolocalization:
        pseudo = ROOT/".build/pseudolocalization"
        pseudolocalize(pseudo)
        shutil.copytree(pseudo/"en-XA.lproj",contents/"Resources/en-XA.lproj")
    if not args.update_localizations: check_localizations(bundle=app,extraction=extraction)
    for binary in [contents/"MacOS/SeedHop", contents/"MacOS/SeedHopAgent", xpc/"Contents/MacOS/SeedHopEngine", contents/"Frameworks/libseedhop_ffi.dylib"]:
        verify_arm64(binary)
    debug_entitlements = ROOT/".build/debug.entitlements"
    write_plist(debug_entitlements,{"com.apple.security.cs.disable-library-validation":True})
    for name,path in [(app_id+".rust",contents/"Frameworks/libseedhop_ffi.dylib"),(app_id+".agent",contents/"MacOS/SeedHopAgent"),*services,(app_id,app)]:
        sign=["codesign","--force","--options","runtime","--sign",args.identity,"--identifier",name]
        if args.identity != "-": sign += ["--timestamp"]
        if not args.release and args.identity=="-" and path!=contents/"Frameworks/libseedhop_ffi.dylib": sign += ["--entitlements",debug_entitlements]
        if sandbox_config and path != contents/"Frameworks/libseedhop_ffi.dylib":
            entitlement_file = app.parent / (name.rsplit('.', 1)[-1] + '.entitlements')
            role = 'application' if path == app else 'private_engine' if path == private_xpc else 'engine'
            write_plist(entitlement_file, sandbox_config[role])
            sign += ['--entitlements', entitlement_file]
        run(*sign,path)
    if sandbox_config:
        signature = subprocess.check_output(['codesign', '-d', '--verbose=4', str(app)], stderr=subprocess.STDOUT, text=True)
        if 'TeamIdentifier=' + args.sandbox_team_id not in signature.splitlines():
            raise RuntimeError('Sandbox group team does not match the signing certificate; do not run this build.')
    run("codesign","--verify","--deep","--strict",app)
    print(f"Built {app}\nSigning: {args.identity}. Ad-hoc builds are for local development, not distribution.")

if __name__ == "__main__": main()
