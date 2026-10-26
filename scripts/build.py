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

ROOT = pathlib.Path(__file__).resolve().parent.parent
SIGNING_CONFIG = ROOT / '.build/signing-identity'

def run(*args):
    print("+", " ".join(map(str,args)),flush=True)
    subprocess.run(list(map(str,args)),cwd=ROOT,check=True)

def write_plist(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(plistlib.dumps(value))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release",action="store_true")
    parser.add_argument("--identity", default=os.environ.get('SEEDHOP_SIGNING_IDENTITY') or (SIGNING_CONFIG.read_text().strip() if SIGNING_CONFIG.exists() else '-'), help="Signing identity; defaults to SEEDHOP_SIGNING_IDENTITY, saved local identity, then ad-hoc")
    parser.add_argument("--save-identity", action="store_true", help="Save the validated identity locally for subsequent builds")
    parser.add_argument("--validation-root", type=pathlib.Path, help="Build an isolated development lifecycle fixture with a distinct service namespace and library")
    parser.add_argument("--skip-rust",action="store_true")
    args = parser.parse_args()
    if args.identity != '-':
        identities = subprocess.check_output(['security', 'find-identity', '-v', '-p', 'codesigning'], text=True)
        if args.identity not in identities:
            parser.error('The requested signing identity is not valid or its private key is unavailable in this keychain.')
    if args.validation_root and (args.release or args.identity == '-'):
        parser.error('Lifecycle fixtures require a certificate-backed development build.')
    if args.save_identity:
        SIGNING_CONFIG.parent.mkdir(parents=True, exist_ok=True)
        SIGNING_CONFIG.write_text(args.identity + '\n')
    profile = "release" if args.release else "debug"
    target = ROOT/"target"/profile
    generated = ROOT/"macOS/Generated"
    generated.mkdir(parents=True,exist_ok=True)
    (ROOT/".cache/swift").mkdir(parents=True,exist_ok=True)
    if not args.skip_rust:
        cargo = ["cargo","build","--locked","-p","seedhop-ffi","--features","bindgen"]
        if args.release: cargo.append("--release")
        run(*cargo)
        run(target/"uniffi-bindgen","generate","--library",target/"libseedhop_ffi.dylib","--language","swift","--out-dir",generated)
    validation_root = args.validation_root.resolve() if args.validation_root else None
    app_id = 'app.seedhop.validation.' + hashlib.sha256(str(validation_root).encode()).hexdigest()[:12] if validation_root else 'app.seedhop.mac'
    app = (validation_root or ROOT/'.build') / 'SeedHop.app'
    contents = app/"Contents"
    for folder in ["MacOS","Frameworks","Resources","XPCServices/SeedHopEngine.xpc/Contents/MacOS","Library/LaunchAgents"]:
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
    iconset=ROOT/".build/AppIcon.iconset"
    run("swift","-module-cache-path",ROOT/".cache/swift",ROOT/"scripts/Icon.swift",iconset,ROOT/"macOS/Resources/AppIcon.png")
    run("iconutil","-c","icns",iconset,"-o",contents/"Resources/AppIcon.icns")
    base = {"CFBundleVersion":"1","CFBundleShortVersionString":"0.1.0","LSMinimumSystemVersion":"15.0","CFBundleDevelopmentRegion":"en","NSLocalNetworkUsageDescription":"SeedHop connects to torrent peers and trackers on networks you use."}
    write_plist(contents/"Info.plist",dict(base,CFBundleIdentifier=app_id,CFBundleName="SeedHop",CFBundleDisplayName="SeedHop",CFBundleExecutable="SeedHop",CFBundlePackageType="APPL",NSPrincipalClass="NSApplication",LSApplicationCategoryType="public.app-category.utilities",NSHighResolutionCapable=True,CFBundleIconFile="AppIcon",CFBundleURLTypes=[{"CFBundleURLName":"BitTorrent magnet link","CFBundleURLSchemes":["magnet"]}],CFBundleDocumentTypes=[{"CFBundleTypeName":"BitTorrent torrent","CFBundleTypeRole":"Viewer","LSHandlerRank":"Alternate","LSItemContentTypes":["org.bittorrent.torrent"],"CFBundleTypeExtensions":["torrent"]}],UTImportedTypeDeclarations=[{"UTTypeIdentifier":"org.bittorrent.torrent","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["torrent"],"public.mime-type":"application/x-bittorrent"}}]))
    xpc = contents/"XPCServices/SeedHopEngine.xpc"
    write_plist(xpc/"Contents/Info.plist",dict(base,CFBundleIdentifier=app_id+".engine",CFBundleExecutable="SeedHopEngine",CFBundlePackageType="XPC!",XPCService={"ServiceType":"Application","RunLoopType":"NSRunLoop","JoinExistingSession":True}))
    write_plist(contents/f"Library/LaunchAgents/{app_id}.agent.plist",{"Label":app_id+".agent","BundleProgram":"Contents/MacOS/SeedHopAgent","ProgramArguments":["SeedHopAgent","--agent"],"MachServices":{app_id+".agent":True},"RunAtLoad":True,"KeepAlive":{"SuccessfulExit":False},"ThrottleInterval":5,"ProcessType":"Background","AssociatedBundleIdentifiers":[app_id]})
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
    shared = sorted((ROOT/"macOS/Shared").glob("*.swift"))
    host = sorted((ROOT/"macOS/Host").glob("*.swift"))
    # Both executable locations find the single bundled, signed Rust dynamic library.
    ffi_args = ["-I",generated,"-Xcc",f"-fmodule-map-file={generated/'seedhop_ffiFFI.modulemap'}","-L",contents/"Frameworks","-lseedhop_ffi","-Xlinker","-rpath","-Xlinker","@executable_path/../Frameworks","-Xlinker","-rpath","-Xlinker","@executable_path/../../../../Frameworks"]
    for name,path in [("SeedHopEngine",xpc/"Contents/MacOS/SeedHopEngine"),("SeedHopAgent",contents/"MacOS/SeedHopAgent")]:
        run(*common,"-module-name",name,*shared,*host,generated/"seedhop_ffi.swift",*ffi_args,"-framework","Security","-o",path)
    run(*common,"-module-name","SeedHop",*shared,*sorted((ROOT/"macOS/App").glob("*.swift")),"-framework","SwiftUI","-framework","AppKit","-framework","ServiceManagement","-framework","Security","-framework","UserNotifications","-o",contents/"MacOS/SeedHop")
    debug_entitlements = ROOT/".build/debug.entitlements"
    write_plist(debug_entitlements,{"com.apple.security.cs.disable-library-validation":True})
    for name,path in [(app_id+".rust",contents/"Frameworks/libseedhop_ffi.dylib"),(app_id+".agent",contents/"MacOS/SeedHopAgent"),(app_id+".engine",xpc),(app_id,app)]:
        sign=["codesign","--force","--options","runtime","--sign",args.identity,"--identifier",name]
        if not args.release and args.identity=="-" and path!=contents/"Frameworks/libseedhop_ffi.dylib": sign += ["--entitlements",debug_entitlements]
        run(*sign,path)
    run("codesign","--verify","--deep","--strict",app)
    print(f"Built {app}\nSigning: {args.identity}. Ad-hoc builds are for local development, not distribution.")

if __name__ == "__main__": main()
