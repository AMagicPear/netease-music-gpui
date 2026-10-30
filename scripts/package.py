"""Build a native app; Windows distributions are self-contained portable ZIPs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import zipfile


def windows_architecture(host):
    architecture = host.split("-", 1)[0]
    try:
        return {"x86_64": "x64", "aarch64": "arm64", "i686": "x86"}[architecture]
    except KeyError:
        raise ValueError(f"Unsupported Windows architecture: {host}") from None


def find_windows_runtime(host, explicit=None):
    """Use the official app-local release CRT, never DLLs from System32 or debug CRT."""
    architecture = windows_architecture(host)
    if explicit is not None:
        candidates = [Path(explicit)]
    else:
        roots = []
        if os.environ.get("VCToolsRedistDir"):
            roots.append(Path(os.environ["VCToolsRedistDir"]))
        vswhere = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
        if vswhere.is_file():
            installations = subprocess.check_output(
                [str(vswhere), "-all", "-products", "*", "-property", "installationPath"],
                text=True, encoding="utf-8",
            ).splitlines()
            for installation in installations:
                roots.extend((Path(installation) / "VC/Redist/MSVC").glob("**/Microsoft.VC*.CRT"))
        candidates = []
        for root in roots:
            if root.name.endswith(".CRT"):
                if root.parent.name == architecture:
                    candidates.append(root)
            else:
                candidates.extend(root.glob(f"{architecture}/Microsoft.VC*.CRT"))
        def version(path):
            return tuple(int(part) for part in path.parent.parent.name.split(".") if part.isdigit())
        candidates.sort(key=version, reverse=True)
    required = {"vcruntime140.dll", "msvcp140.dll"}
    if architecture in ("x64", "arm64"):
        required.add("vcruntime140_1.dll")
    for directory in candidates:
        if all((directory / name).is_file() for name in required):
            return directory
    raise RuntimeError(
        f"Cannot find the official {architecture} VC release runtime. Install Visual Studio C++ "
        "build tools, or pass --runtime-dir <arch>/Microsoft.VC*.CRT."
    )


def archive_windows(folder, archive):
    """Retain the top-level folder so extracting the ZIP keeps all resources together."""
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for path in sorted(folder.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(folder.parent).as_posix())
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_name(archive.name + ".sha256")
    checksum.write_text(f"{digest}  {archive.name}\n", encoding="utf-8")


def add_windows_distribution(output, host, version, runtime):
    for dll in sorted(runtime.glob("*.dll")):
        shutil.copy2(dll, output / dll.name)
    (output / "README.txt").write_text(
        f"网易云音乐 GPUI 练习项目 {version} ({host})\n\n"
        "支持 Windows 10/11。请先完整解压 ZIP，再运行 netease-music-gpui.exe。\n"
        "请保留 EXE、assets 和同目录 DLL；无需安装 Python、Rust 或管理员权限。\n"
        "VC 运行库以应用本地 DLL 随包提供。\n\n"
        "登录会话和播放记录保存在 %APPDATA%/NeteaseMusicGpui/，\n"
        "封面和音频缓存保存在 %LOCALAPPDATA%/NeteaseMusicGpui/Cache/。\n"
        "升级时先从托盘退出旧程序，再把新包解压到新目录；个人数据保留。\n"
        "这是练习项目，与网易官方客户端无关。\n",
        encoding="utf-8-sig",
    )
    (output / "build-info.json").write_text(json.dumps({
        "name": "netease-music-gpui", "version": version, "target": host,
        "runtime": sorted(dll.name for dll in runtime.glob("*.dll")),
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


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
    elif platform == "win32":
        binary = output / executable.name
        resources = output / "assets"
    else:
        raise ValueError("Packaging is supported on macOS and Windows only")
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, binary)
    shutil.copytree(root / "assets", resources)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-dir", type=Path, help="Windows official VC release CRT directory")
    args = parser.parse_args()
    if sys.platform not in ("darwin", "win32"):
        raise SystemExit("Run this script on macOS or Windows to build for that platform")
    root = Path(__file__).resolve().parents[1]
    subprocess.run([sys.executable, str(root / "scripts/prepare_gpui.py")], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root
    ))
    package = next(item for item in metadata["packages"] if item["name"] == "netease-music-gpui")
    host = next(line.removeprefix("host: ") for line in subprocess.check_output(
        ["rustc", "-vV"], text=True
    ).splitlines() if line.startswith("host: "))
    runtime = find_windows_runtime(host, args.runtime_dir) if sys.platform == "win32" else None
    subprocess.run(["cargo", "build", "--locked", "--release", "--bin", package["name"],
                    "--target", host], cwd=root, check=True)
    target = Path(metadata["target_directory"])
    executable = target / host / "release" / (package["name"] + (".exe" if sys.platform == "win32" else ""))
    destination = target / "bundle" / ("NetEase Music.app" if sys.platform == "darwin" else package["name"])
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        output = Path(temporary) / destination.name
        assemble(root, executable, output, sys.platform, package["version"])
        if runtime is not None:
            add_windows_distribution(output, host, package["version"], runtime)
        if sys.platform == "darwin":
            # 本地 ad-hoc 签名；正式分发时再使用 Developer ID 并公证。
            subprocess.run(["codesign", "--force", "--sign", "-", str(output)], check=True)
        if destination.exists():
            # Refuse to recursively replace a symlink pointing outside the bundle directory.
            if destination.resolve().parent != destination.parent.resolve():
                raise RuntimeError(f"Refusing to replace bundle outside output directory: {destination}")
            shutil.rmtree(destination)
        shutil.move(output, destination)
    print(destination)
    if sys.platform == "win32":
        archive = destination.parent / f"{package['name']}-{package['version']}-{host}.zip"
        archive_windows(destination, archive)
        print(archive)
        print(archive.with_name(archive.name + ".sha256"))


if __name__ == "__main__":
    main()
