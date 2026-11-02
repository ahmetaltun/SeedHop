"""Create a Finder-ready installer without GUI automation."""
import argparse
import pathlib
import subprocess
import tempfile

import dmgbuild
from ds_store import DSStore

ROOT = pathlib.Path(__file__).resolve().parent.parent


def settings(app, background):
    return dict(
        format="UDZO", filesystem="HFS+", files=[str(app)],
        symlinks={"Applications": "/Applications"},
        icon=str(app / "Contents/Resources/AppIcon.icns"), background=str(background),
        window_rect=((160, 160), (720, 460)), default_view="icon-view",
        icon_locations={"SeedHop.app": (200, 256), "Applications": (520, 256)},
        icon_size=104, text_size=14, label_pos="bottom", arrange_by=None,
        show_status_bar=False, show_tab_view=False, show_toolbar=False,
        show_pathbar=False, show_sidebar=False, show_icon_preview=False,
        include_icon_view_settings=True, include_list_view_settings=False,
    )


def create(app, output):
    app = app.resolve()
    if app.name != "SeedHop.app" or not (app / "Contents/Resources/AppIcon.icns").is_file():
        raise ValueError("Expected a built SeedHop.app with its icon")
    with tempfile.TemporaryDirectory(prefix="seedhop-dmg-artwork-") as temporary:
        artwork = pathlib.Path(temporary)
        subprocess.run(["swift", "-module-cache-path", str(ROOT / ".cache/swift"),
                        str(ROOT / "scripts/DMGArtwork.swift"), str(artwork)], check=True)
        background = artwork / "background.tiff"
        subprocess.run(["tiffutil", "-cathidpicheck", str(artwork / "background.png"),
                        str(artwork / "background@2x.png"), "-out", str(background)], check=True)
        dmgbuild.build_dmg(str(output), "SeedHop", settings=settings(app, background))


def verify_layout(mount):
    if not (mount / ".background.tiff").is_file() or not (mount / ".VolumeIcon.icns").is_file():
        raise ValueError("Installer artwork or volume icon missing")
    with DSStore.open(str(mount / ".DS_Store"), "r") as store:
        window = store["."]["bwsp"]
        icons = store["."]["icvp"]
        if window["WindowBounds"] != "{{160, 160}, {720, 460}}":
            raise ValueError("Installer window dimensions changed")
        if any(window[key] for key in ("ShowToolbar", "ShowSidebar", "ShowStatusBar", "ShowPathbar", "ShowTabView")):
            raise ValueError("Installer window contains unwanted Finder controls")
        if icons["backgroundType"] != 2 or icons["iconSize"] != 104 or icons["arrangeBy"] != "none":
            raise ValueError("Installer background or icon settings missing")
        for name, position in {"SeedHop.app": (200, 256), "Applications": (520, 256)}.items():
            if store[name]["Iloc"] != position:
                raise ValueError(f"Installer icon misplaced: {name}")
    print("Installer artwork, window layout, and icon positions verified", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--verify-mounted", type=pathlib.Path)
    args = parser.parse_args()
    if args.verify_mounted:
        verify_layout(args.verify_mounted)
    elif args.app and args.output:
        create(args.app, args.output.resolve())
    else:
        parser.error("Supply --app and --output, or --verify-mounted")
