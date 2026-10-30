"""Download pinned GPUI crates and apply the local patches."""

import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
from urllib.error import URLError
from urllib.request import urlopen

VERSION = "0.3.8"
CRATES = {
    "gpui-pre": "d7f0bc3ebba070f8981b3f8e905a12f47d27794a30d029e59cdb9edf7a4b3360",
    "gpui-pre-apple": "bfdd914311cd12894367c8b9908947bb0c27fd42f8b9a5ff7a0730551a83fa4c",
    "gpui-pre-windows": "371f9bb13f97e3bea2f8b8e5e81232bb74d735153c244ae67f7400e743925b1c",
    "gpui-pre-wgpu": "439c5421cafe1a7cf68d0c0ae5cc82b26fad6b0e6345ac98d20acaf1f2463914",
}

# Mirrors serve byte-identical crates, so the pinned SHA-256 still applies.
# domestic mirrors are listed first because static.crates.io can be very slow
# or drop the connection mid-transfer, which looks like the script hanging.
SOURCES = (
    os.environ.get("GPUI_CRATE_SOURCE")
    or "https://mirrors.ustc.edu.cn/crates.io/crates/{name}/{name}-{version}.crate",
    "https://static.crates.io/crates/{name}/{name}-{version}.crate",
)
DOWNLOAD_TIMEOUT = float(os.environ.get("GPUI_DOWNLOAD_TIMEOUT") or 30)
CHUNK = 1 << 16


def download(name, checksum):
    """Return the crate bytes, trying each source until one matches the checksum."""
    problems = []
    for template in SOURCES:
        url = template.format(name=name, version=VERSION)
        print(f"Downloading {name} {VERSION} from {url}", flush=True)
        try:
            with urlopen(url, timeout=DOWNLOAD_TIMEOUT) as response:
                # Read in chunks so a stalled socket trips the timeout instead of hanging.
                chunks = []
                while chunk := response.read(CHUNK):
                    chunks.append(chunk)
            data = b"".join(chunks)
        except (URLError, OSError) as error:
            problems.append(f"{url}: {error}")
            print(f"  failed: {error}", flush=True)
            continue
        digest = hashlib.sha256(data).hexdigest()
        if digest != checksum:
            problems.append(f"{url}: checksum {digest}")
            print(f"  checksum mismatch: {digest}", flush=True)
            continue
        return data
    raise RuntimeError(
        "Cannot download "
        + name
        + "; set GPUI_CRATE_SOURCE to a working mirror template.\n  "
        + "\n  ".join(problems)
    )


def main():
    root = Path(__file__).resolve().parents[1]
    patches = sorted((root / "vendor/patches").glob("*.patch"))
    vendor = root / "vendor"
    stamp = vendor / ".gpui-patch-id"
    patch_id = hashlib.sha256(
        Path(__file__).read_bytes() + b"".join(p.read_bytes() for p in patches)
    ).hexdigest()
    if (stamp.exists() and stamp.read_text() == patch_id
            and all((vendor / name / "Cargo.toml").exists() for name in CRATES)):
        print("GPUI patches are ready", flush=True)
        return

    # Prepare everything before replacing the generated source directories.
    with tempfile.TemporaryDirectory(prefix="gpui-", dir=vendor) as temporary:
        staging = Path(temporary)
        # Isolate git apply from the parent repository, which can silently skip patches.
        subprocess.run(["git", "init", "--quiet", str(staging)], check=True)
        for name, checksum in CRATES.items():
            data = download(name, checksum)
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
            # `core.autocrlf=true` (the Windows default) checks the patches out with
            # CRLF, but the extracted crate sources use LF, so `git apply` would fail
            # to match context lines. Normalize the copy we hand to git.
            normalized = staging / patch.name
            normalized.write_bytes(patch.read_bytes().replace(b"\r\n", b"\n"))
            for arguments in (["--check"], []):
                subprocess.run(
                    ["git", "apply", *arguments, str(normalized)], cwd=staging, check=True
                )
            normalized.unlink()
        for name in CRATES:
            destination = vendor / name
            if destination.exists():
                shutil.rmtree(destination)
            shutil.move(staging / "vendor" / name, destination)
        stamp.write_text(patch_id)
    print("GPUI patches are ready")


if __name__ == "__main__":
    main()
