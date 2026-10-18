#!/usr/bin/env python3
"""Create an immutable PS5 auto-update feed from an already built title; never publish it.

Run Python with -I. The console streams individual content-addressed files and verifies their
sizes/SHA-256s before installing. Base game MPQs, saves and machine configuration are excluded.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
from urllib.parse import urlsplit

TITLE = "PPSA99320"
SAFE = re.compile(r"[A-Za-z0-9_.-]{1,96}\Z")
PATCH = re.compile(r"patch(?:-[0-9]{1,5})?\.MPQ\Z")
REQUIRED = {"eboot.bin", "js/boot.mjs", "web/wenilla.js", "sce_sys/param.json"}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def allowed(path):
    if len(path) > 240 or any(not SAFE.fullmatch(c) or c in (".", "..") or c.endswith(".") for c in path.split("/")):
        return False
    return (path in {"eboot.bin", "sce_module/libc.prx", "sce_sys/param.json", "sce_sys/icon0.png", "sce_sys/pic0.dds", "sce_sys/pic1.dds", "sce_sys/snd0.at9"}
            or path.startswith(("js/", "web/", "licenses/"))
            or path.startswith("Data/") and bool(PATCH.fullmatch(path[5:])))


def build_feed(app, data_dirs, version, sequence, output, base_url):
    if not SAFE.fullmatch(version) or version in (".", "..") or sequence <= 0:
        raise ValueError("Invalid version or sequence")
    parsed = urlsplit(base_url)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("Base URL must be an absolute HTTP(S) directory URL")
    if output.exists() and any(output.iterdir()):
        raise ValueError("Output must be new or empty; published releases are immutable")
    if json.loads((app / "sce_sys/param.json").read_text(encoding="utf-8"))["titleId"] != TITLE:
        raise ValueError("Wrong title metadata")
    with (app / "eboot.bin").open("rb") as stream:
        if stream.read(4) != (0x1D3D154F).to_bytes(4, "little"):
            raise ValueError("eboot.bin is not a PS5 SELF")
    paths = {}
    for path in app.rglob("*"):
        relative = path.relative_to(app).as_posix()
        if path.is_symlink():
            raise ValueError(f"Symlink in package: {relative}")
        if path.is_file() and allowed(relative):
            paths[relative] = path
    for directory in data_dirs:
        for path in directory.glob("patch*.MPQ"):
            if path.is_symlink():
                raise ValueError(f"Symlink patch: {path.name}")
            if PATCH.fullmatch(path.name):
                paths["Data/" + path.name] = path
    if not REQUIRED.issubset(paths):
        raise ValueError("Incomplete package")
    if len(paths) > 1024 or len({p.lower() for p in paths}) != len(paths):
        raise ValueError("Too many files or case alias")
    output.mkdir(parents=True, exist_ok=True)
    assets = []
    total = 0
    for relative, source in sorted(paths.items()):
        size = source.stat().st_size
        limit = 4 * 1024**3 if relative.startswith("Data/") else (512 * 1024**2 if relative in ("eboot.bin", "sce_module/libc.prx") else 16 * 1024**2)
        total += size
        if size > limit or total > 8 * 1024**3:
            raise ValueError(f"Update size limit: {relative}")
        sha = digest(source)
        blob = sha + ".bin"
        destination = output / blob
        if not destination.exists():
            shutil.copyfile(source, destination)
        if destination.stat().st_size != size or digest(destination) != sha:
            raise ValueError(f"Copied input changed: {relative}")
        assets.append({"path": relative, "url": base_url.rstrip("/") + "/" + blob, "size": size, "sha256": sha})
    identity = hashlib.sha256(json.dumps(assets, sort_keys=True).encode()).hexdigest()[:16]
    manifest = {"format": 1, "title_id": TITLE, "id": f"{version}-{sequence}-{identity}", "version": version,
                "sequence": sequence, "files": assets}
    encoded = (json.dumps(manifest, indent=2) + "\n").encode()
    (output / "update.json").write_bytes(encoded)
    return {"id": manifest["id"], "files": len(assets), "bytes": total,
            "manifest_sha256": hashlib.sha256(encoded).hexdigest(), "manifest_url": base_url.rstrip("/") + "/update.json"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--data", type=Path, action="append", default=[])
    parser.add_argument("--version", required=True)
    parser.add_argument("--sequence", type=int, required=True, help="Monotonically increasing release sequence, including host-only updates")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-url", required=True)
    args = parser.parse_args()
    print(json.dumps(build_feed(args.app, args.data, args.version, args.sequence, args.output, args.base_url), indent=2))


if __name__ == "__main__":
    main()
