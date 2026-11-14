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

VERSION = "0.1.15"
CRATES = {
    "gpui-fast": "a069272b90d908d15152711b38153edabcf67a5dcc490123197af7fd9ad78d9d",
    "gpui-fast-apple": "94395e1c8cc1e0795d3bac933d8701582eb3e6b274db09919dca24eab379d801",
    "gpui-fast-windows": "e07b0bad5ff42b905882f91797540299a6747a1a34d07696083e3be3bb580c67",
    "gpui-fast-wgpu": "9c02b5c6805781c8978d568df0bd62845a728bc9ac176ec8c4337b65a5daf41f",
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
            if name == "gpui-fast-apple":
                shutil.rmtree(staging / "vendor" / name / "src/fast/release_gpui")
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
