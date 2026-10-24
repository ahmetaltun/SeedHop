#!/usr/bin/env python3
"""Build a native .app using Xcode's compiler; no project generator is required."""
import argparse
import os
import pathlib
import plistlib
import json
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent

def run(*args):
    print("+", " ".join(map(str,args)),flush=True)
    subprocess.run(list(map(str,args)),cwd=ROOT,check=True)

def write_plist(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(plistlib.dumps(value))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release",action="store_true")
    parser.add_argument("--identity",default="-",help="Apple Development/Developer ID identity, or - for local ad-hoc builds")
    parser.add_argument("--skip-rust",action="store_true")
    args = parser.parse_args()
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
    app = ROOT/".build/SeedHop.app"
    contents = app/"Contents"
    for folder in ["MacOS","Frameworks","Resources","XPCServices/SeedHopEngine.xpc/Contents/MacOS","Library/LaunchAgents"]:
        (contents/folder).mkdir(parents=True,exist_ok=True)
    for name in ["LICENSE","THIRD_PARTY_NOTICES.md"]:
        if (ROOT/name).exists(): shutil.copy2(ROOT/name,contents/"Resources"/name)
    license_root=contents/"Resources/ThirdPartyLicenses"
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--offline","--format-version","1"],cwd=ROOT,text=True))
    for package in metadata["packages"]:
        directory=pathlib.Path(package["manifest_path"]).parent
        for source in directory.iterdir():
            if source.is_file() and source.name.upper().startswith(("LICENSE","COPYING","NOTICE")):
                destination=license_root/(package["name"]+"-"+package["version"])
                destination.mkdir(parents=True,exist_ok=True); shutil.copy2(source,destination/source.name)
    iconset=ROOT/".build/AppIcon.iconset"
    run("swift","-module-cache-path",ROOT/".cache/swift",ROOT/"scripts/Icon.swift",iconset,ROOT/"macOS/Resources/AppIcon.png")
    run("iconutil","-c","icns",iconset,"-o",contents/"Resources/AppIcon.icns")
    base = {"CFBundleVersion":"1","CFBundleShortVersionString":"0.1.0","LSMinimumSystemVersion":"15.0","CFBundleDevelopmentRegion":"en","NSLocalNetworkUsageDescription":"SeedHop connects to torrent peers and trackers on networks you use."}
    write_plist(contents/"Info.plist",dict(base,CFBundleIdentifier="app.seedhop.mac",CFBundleName="SeedHop",CFBundleDisplayName="SeedHop",CFBundleExecutable="SeedHop",CFBundlePackageType="APPL",NSPrincipalClass="NSApplication",LSApplicationCategoryType="public.app-category.utilities",NSHighResolutionCapable=True,CFBundleIconFile="AppIcon",CFBundleURLTypes=[{"CFBundleURLName":"BitTorrent magnet link","CFBundleURLSchemes":["magnet"]}],CFBundleDocumentTypes=[{"CFBundleTypeName":"BitTorrent torrent","CFBundleTypeRole":"Viewer","LSHandlerRank":"Alternate","LSItemContentTypes":["org.bittorrent.torrent"],"CFBundleTypeExtensions":["torrent"]}],UTImportedTypeDeclarations=[{"UTTypeIdentifier":"org.bittorrent.torrent","UTTypeConformsTo":["public.data"],"UTTypeTagSpecification":{"public.filename-extension":["torrent"],"public.mime-type":"application/x-bittorrent"}}]))
    xpc = contents/"XPCServices/SeedHopEngine.xpc"
    write_plist(xpc/"Contents/Info.plist",dict(base,CFBundleIdentifier="app.seedhop.mac.engine",CFBundleExecutable="SeedHopEngine",CFBundlePackageType="XPC!",XPCService={"ServiceType":"Application","RunLoopType":"NSRunLoop","JoinExistingSession":True}))
    write_plist(contents/"Library/LaunchAgents/app.seedhop.mac.agent.plist",{"Label":"app.seedhop.mac.agent","BundleProgram":"Contents/MacOS/SeedHopAgent","ProgramArguments":["SeedHopAgent","--agent"],"MachServices":{"app.seedhop.mac.agent":True},"RunAtLoad":True,"KeepAlive":{"SuccessfulExit":False},"ThrottleInterval":5,"ProcessType":"Background","AssociatedBundleIdentifiers":["app.seedhop.mac"]})
    shutil.copy2(target/"libseedhop_ffi.dylib",contents/"Frameworks/libseedhop_ffi.dylib")
    run("install_name_tool","-id","@rpath/libseedhop_ffi.dylib",contents/"Frameworks/libseedhop_ffi.dylib")
    common = ["swiftc","-target","arm64-apple-macosx15.0","-swift-version","5","-parse-as-library","-module-cache-path",ROOT/".cache/swift"]
    common += ["-O"] if args.release else ["-g"]
    if not args.release: common += ["-D","SEEDHOP_DEVELOPMENT"]
    shared = sorted((ROOT/"macOS/Shared").glob("*.swift"))
    host = sorted((ROOT/"macOS/Host").glob("*.swift"))
    # Both executable locations find the single bundled, signed Rust dynamic library.
    ffi_args = ["-I",generated,"-Xcc",f"-fmodule-map-file={generated/'seedhop_ffiFFI.modulemap'}","-L",contents/"Frameworks","-lseedhop_ffi","-Xlinker","-rpath","-Xlinker","@executable_path/../Frameworks","-Xlinker","-rpath","-Xlinker","@executable_path/../../../../Frameworks"]
    for name,path in [("SeedHopEngine",xpc/"Contents/MacOS/SeedHopEngine"),("SeedHopAgent",contents/"MacOS/SeedHopAgent")]:
        run(*common,"-module-name",name,*shared,*host,generated/"seedhop_ffi.swift",*ffi_args,"-framework","Security","-o",path)
    run(*common,"-module-name","SeedHop",*shared,*sorted((ROOT/"macOS/App").glob("*.swift")),"-framework","SwiftUI","-framework","AppKit","-framework","ServiceManagement","-framework","Security","-framework","UserNotifications","-o",contents/"MacOS/SeedHop")
    debug_entitlements = ROOT/".build/debug.entitlements"
    write_plist(debug_entitlements,{"com.apple.security.cs.disable-library-validation":True})
    for name,path in [("app.seedhop.mac.rust",contents/"Frameworks/libseedhop_ffi.dylib"),("app.seedhop.mac.agent",contents/"MacOS/SeedHopAgent"),("app.seedhop.mac.engine",xpc),("app.seedhop.mac",app)]:
        sign=["codesign","--force","--options","runtime","--sign",args.identity,"--identifier",name]
        if not args.release and args.identity=="-" and path!=contents/"Frameworks/libseedhop_ffi.dylib": sign += ["--entitlements",debug_entitlements]
        run(*sign,path)
    run("codesign","--verify","--deep","--strict",app)
    print(f"Built {app}\nSigning: {args.identity}. Ad-hoc builds are for local development, not distribution.")

if __name__ == "__main__": main()
