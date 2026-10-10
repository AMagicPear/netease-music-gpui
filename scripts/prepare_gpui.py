"""Download pinned GPUI crates and apply the local patches."""

import hashlib
import io
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
from urllib.request import urlopen

VERSION = "0.3.8"
CRATES = {
    "gpui-pre": "d7f0bc3ebba070f8981b3f8e905a12f47d27794a30d029e59cdb9edf7a4b3360",
    "gpui-pre-apple": "bfdd914311cd12894367c8b9908947bb0c27fd42f8b9a5ff7a0730551a83fa4c",
    "gpui-pre-wgpu": "439c5421cafe1a7cf68d0c0ae5cc82b26fad6b0e6345ac98d20acaf1f2463914",
}


def main():
    root = Path(__file__).resolve().parents[1]
    patches = sorted((root / "vendor/patches").glob("*.patch"))
    vendor = root / "vendor"
    stamp = vendor / ".gpui-patch-id"
    patch_id = hashlib.sha256(b"".join(p.read_bytes() for p in patches)).hexdigest()
    if (stamp.exists() and stamp.read_text() == patch_id
            and all((vendor / name / "Cargo.toml").exists() for name in CRATES)):
        print("GPUI patches are ready", flush=True)
        return

    # Prepare everything before replacing the generated source directories.
    with tempfile.TemporaryDirectory(prefix="gpui-", dir=vendor) as temporary:
        staging = Path(temporary)
        for name, checksum in CRATES.items():
            url = f"https://static.crates.io/crates/{name}/{name}-{VERSION}.crate"
            print(f"Downloading {name} {VERSION}", flush=True)
            with urlopen(url, timeout=60) as response:
                data = response.read()
            if hashlib.sha256(data).hexdigest() != checksum:
                raise RuntimeError(f"Checksum mismatch for {name}")
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
                # Only extract regular files from the expected package directory.
                for member in archive:
                    path = Path(member.name)
                    if path.is_absolute() or ".." in path.parts or path.parts[0] != f"{name}-{VERSION}":
                        raise RuntimeError(f"Unexpected archive path: {member.name}")
                    if member.isfile():
                        destination = staging / "vendor" / name / Path(*path.parts[1:])
                        destination.parent.mkdir(parents=True, exist_ok=True)
                        source = archive.extractfile(member)
                        if source is None:
                            raise RuntimeError(f"Cannot extract archive file: {member.name}")
                        with source, destination.open("wb") as output:
                            shutil.copyfileobj(source, output)
            # The Metal build script uses the sibling patched GPUI source instead.
            if name == "gpui-pre-apple":
                shutil.rmtree(staging / "vendor" / name / "vendor")
        for patch in patches:
            subprocess.run(["git", "apply", "--check", str(patch)], cwd=staging, check=True)
            subprocess.run(["git", "apply", str(patch)], cwd=staging, check=True)
        for name in CRATES:
            destination = vendor / name
            if destination.exists():
                shutil.rmtree(destination)
            shutil.move(staging / "vendor" / name, destination)
        stamp.write_text(patch_id)
    print("GPUI patches are ready")


if __name__ == "__main__":
    main()
