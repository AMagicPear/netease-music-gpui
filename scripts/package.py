"""Build a native release and copy it with its assets into target/bundle."""

import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile


def assemble(root, executable, output, platform, version):
    if platform == "darwin":
        contents = output / "Contents"
        binary = contents / "MacOS" / executable.name
        resources = contents / "Resources" / "assets"
        contents.mkdir(parents=True)
        resources.parent.mkdir()
        shutil.copy2(root / "packaging/163Music.icns", resources.parent / "163Music.icns")
        with (contents / "Info.plist").open("wb") as file:
            plistlib.dump({
                "CFBundleExecutable": executable.name,
                "CFBundleIconFile": "163Music.icns",
                "CFBundleIdentifier": "dev.netease-music-gpui",
                "CFBundleName": "NetEase Music",
                "CFBundleDisplayName": "NetEase Music",
                "CFBundleDevelopmentRegion": "en",
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": version,
                "CFBundleVersion": version,
                # 与官方客户端一致声明 orpheus scheme，网页「点击打开客户端」
                # 按钮可以唤起本应用；不主动抢系统默认处理器。
                "CFBundleURLTypes": [
                    {
                        "CFBundleTypeRole": "Viewer",
                        "CFBundleURLName": "orpheus",
                        "CFBundleURLSchemes": ["orpheus"],
                    }
                ],
                "NSHighResolutionCapable": True,
            }, file)
        for language, name in (("en", "NetEase Music"), ("zh-Hans", "网易云音乐"), ("zh-Hant", "网易云音乐")):
            localized = resources.parent / f"{language}.lproj"
            localized.mkdir()
            with (localized / "InfoPlist.strings").open("wb") as file:
                plistlib.dump({"CFBundleName": name, "CFBundleDisplayName": name}, file)
    elif platform in ("win32", "linux"):
        binary = output / executable.name
        resources = output / "assets"
    else:
        raise ValueError("Packaging is supported on macOS, Windows and Linux only")
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, binary)
    shutil.copytree(root / "assets", resources)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", metavar="TRIPLE",
                        help="override the cargo target triple, e.g. x86_64-apple-darwin on macOS")
    args = parser.parse_args()
    if sys.platform not in ("darwin", "win32", "linux"):
        raise SystemExit("Run this script on macOS, Windows or Linux to build for that platform")
    root = Path(__file__).resolve().parents[1]
    subprocess.run([sys.executable, str(root / "scripts/prepare_gpui.py")], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root
    ))
    package = next(item for item in metadata["packages"] if item["name"] == "netease-music-gpui")
    host = next(line.removeprefix("host: ") for line in subprocess.check_output(
        ["rustc", "-vV"], text=True
    ).splitlines() if line.startswith("host: "))
    build_target = args.target or host
    if sys.platform != "darwin" and build_target != host:
        raise SystemExit("Cross compilation is only supported on macOS")
    subprocess.run(["cargo", "build", "--locked", "--release", "--bin", package["name"],
                    "--target", build_target], cwd=root, check=True)
    target = Path(metadata["target_directory"])
    executable = target / build_target / "release" / (package["name"] + (".exe" if sys.platform == "win32" else ""))
    destination = target / "bundle" / ("NetEase Music.app" if sys.platform == "darwin" else package["name"])
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        output = Path(temporary) / destination.name
        assemble(root, executable, output, sys.platform, package["version"])
        if sys.platform == "darwin":
            # 本地 ad-hoc 签名；正式分发时再使用 Developer ID 并公证。
            subprocess.run(["codesign", "--force", "--sign", "-", str(output)], check=True)
        if destination.exists():
            shutil.rmtree(destination)
        shutil.move(output, destination)
    print(destination)


if __name__ == "__main__":
    main()
