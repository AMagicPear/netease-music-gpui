"""Packaging checks use temporary files; they never build or start the application."""

import hashlib
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
import zipfile

from package import add_windows_distribution, archive_windows, assemble, find_windows_runtime


class PackageTests(unittest.TestCase):
    def test_windows_zip_keeps_assets_runtime_and_manifest_together(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "音乐 project"
            (root / "assets/images").mkdir(parents=True)
            (root / "assets/images/封面.png").write_bytes(b"cover")
            (root / ".env").write_text("private", encoding="utf-8")
            executable = root / "netease-music-gpui.exe"
            executable.write_bytes(b"test executable")
            runtime = root / "x64/Microsoft.VC143.CRT"
            runtime.mkdir(parents=True)
            for name in ("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"):
                (runtime / name).write_bytes(b"runtime")
            (runtime / "not-a-runtime.txt").write_text("skip")
            self.assertEqual(find_windows_runtime("x86_64-pc-windows-msvc", runtime), runtime)
            output = root / "bundle/netease-music-gpui"
            assemble(root, executable, output, "win32", "0.1.0")
            add_windows_distribution(output, "x86_64-pc-windows-msvc", "0.1.0", runtime)
            archive = output.parent / "music.zip"
            archive_windows(output, archive)
            with zipfile.ZipFile(archive) as zipped:
                names = zipped.namelist()
                self.assertIn("netease-music-gpui/assets/images/封面.png", names)
                self.assertIn("netease-music-gpui/netease-music-gpui.exe", names)
                self.assertIn("netease-music-gpui/vcruntime140.dll", names)
                self.assertIn("netease-music-gpui/msvcp140.dll", names)
                self.assertFalse(any(".env" in name or "not-a-runtime" in name for name in names))
                info = json.loads(zipped.read("netease-music-gpui/build-info.json"))
                self.assertEqual(info["target"], "x86_64-pc-windows-msvc")
            self.assertEqual(archive.with_name("music.zip.sha256").read_text().split()[0],
                             hashlib.sha256(archive.read_bytes()).hexdigest())

    def test_incomplete_runtime_fails_instead_of_shipping_a_broken_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(RuntimeError):
                find_windows_runtime("x86_64-pc-windows-msvc", temporary)

    def test_macos_resource_layout_and_protocol_registration_are_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "assets").mkdir()
            (root / "assets/resource").write_bytes(b"asset")
            (root / "packaging").mkdir()
            (root / "packaging/163Music.icns").write_bytes(b"icon")
            executable = root / "music"
            executable.write_bytes(b"executable")
            output = root / "Music.app"
            assemble(root, executable, output, "darwin", "0.1.0")
            self.assertTrue((output / "Contents/MacOS/music").is_file())
            self.assertTrue((output / "Contents/Resources/assets/resource").is_file())
            with (output / "Contents/Info.plist").open("rb") as file:
                info = plistlib.load(file)
            self.assertEqual(info["CFBundleURLTypes"][0]["CFBundleURLSchemes"], ["orpheus"])


if __name__ == "__main__":
    unittest.main()
