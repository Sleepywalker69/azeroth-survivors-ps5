#!/usr/bin/env python3
"""Package or verify the existing Linux PS5 toolchain. Never download. Run Python -I."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
from urllib.request import HTTPRedirectHandler, Request, build_opener


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def safe_name(name):
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and all(p not in ("", ".", "..") for p in name.split("/")) and "\\" not in name


def pack(args):
    if args.output.exists():
        raise ValueError("Build kit already exists; choose a new output")
    selections = []
    def add(source, target):
        source = Path(source)
        if not source.exists():
            raise ValueError(f"Missing build input: {source}")
        selections.append((source, target))
    for part in ("bin", "target/include", "target/lib", "toolchain", "ldscripts", ".ps5-sdk-revision"):
        add(args.sdk / part, "sdk/" + part)
    for part in (".deps/native/radv-release", "build/host/ps5-native-tool", "tooling/native",
                 "tooling/psbc/ps5-pie-unwind.ld", "tooling/prospero-clang18", "tools/radv-link.sh",
                 "runtime", "vendor/ps5/sdk/stubs", "LICENSE", "NOTICE.md"):
        add(args.vulkan / part, "vulkan/" + part)
    add(args.wasm2c, "wabt/bin/wasm2c")
    add(args.wasm_runtime, "wabt/wasm2c")
    add(args.wasm_runtime.parent / "LICENSE", "wabt/LICENSE")
    add(args.quickjs, "quickjs")
    # No credentials, Cargo configuration, git checkouts, registry sources or shell profiles.
    registry = "index.crates.io-1949cf8c6b5b557f"
    add(args.cargo_home / "registry/cache" / registry, "cargo/registry/cache/" + registry)
    add(args.cargo_home / "registry/index" / registry / ".cache", "cargo/registry/index/" + registry + "/.cache")
    add(args.cargo_home / "registry/index" / registry / "config.json", "cargo/registry/index/" + registry + "/config.json")
    add(args.rust_toolchain, "rust")
    manifest = {"format": 1, "sdk_revision": (args.sdk / ".ps5-sdk-revision").read_text().strip(),
        "vulkan_revision": subprocess.check_output(["git", "-C", str(args.vulkan), "rev-parse", "HEAD"], text=True).strip(),
        "rust_version": subprocess.check_output([str(args.rust_toolchain / "bin/rustc"), "--version"], text=True).strip(),
        "wasm2c_version": subprocess.check_output([str(args.wasm2c), "--version"], text=True).strip(), "files": {}}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_name(args.output.name + ".part")
    with tarfile.open(temporary, "w:gz", compresslevel=1, dereference=False) as archive:
        for source, target in selections:
            def include(info, base=source, dest=target):
                if not safe_name(info.name) or info.name in manifest["files"]:
                    raise ValueError("Invalid or duplicate kit entry: " + info.name)
                relative = PurePosixPath(info.name).relative_to(dest)
                original = base.joinpath(*relative.parts)
                if info.issym():
                    # SDK and toolchain links may only point within the selected subtree.
                    boundary = base if base.is_dir() else base.parent
                    resolved = original.resolve(strict=True)
                    if os.path.isabs(info.linkname) or not resolved.is_relative_to(boundary.resolve()):
                        raise ValueError("External build-kit symlink: " + str(original))
                    record = {"symlink": info.linkname}
                elif info.isfile():
                    record = {"size": info.size, "sha256": digest(original), "mode": info.mode & 0o777}
                elif info.isdir():
                    record = {"directory": True}
                else:
                    raise ValueError("Special build-kit entry: " + info.name)
                manifest["files"][info.name] = record
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 0
                return info
            archive.add(source, arcname=target, filter=include)
        encoded = (json.dumps(manifest, indent=2) + "\n").encode()
        info = tarfile.TarInfo("buildkit.json")
        info.size = len(encoded); info.mode = 0o644
        archive.addfile(info, io.BytesIO(encoded))
    temporary.replace(args.output)
    receipt = {k: v for k, v in manifest.items() if k != "files"}
    receipt.update(name=args.output.name, bytes=args.output.stat().st_size, sha256=digest(args.output), files=len(manifest["files"]))
    args.output.with_name(args.output.name + ".json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


def unpack(archive_path, expected, output):
    if not re.fullmatch(r"[0-9a-f]{64}", expected) or digest(archive_path) != expected:
        raise ValueError("Build-kit SHA-256 mismatch")
    if output.exists():
        raise ValueError("Extraction directory must be new")
    with tarfile.open(archive_path, "r:gz") as archive:
        entries = archive.getmembers()
        names = [m.name for m in entries]
        if len(names) != len(set(names)) or any(not safe_name(n) for n in names):
            raise ValueError("Unsafe or duplicate archive path")
        if sum(m.size for m in entries) > 3 * 1024**3 or len(entries) > 100000:
            raise ValueError("Build-kit extraction limit exceeded")
        # Python's data filter rejects links outside output, devices and dangerous modes.
        archive.extractall(output, filter="data")
    manifest = json.loads((output / "buildkit.json").read_text())
    if manifest.get("format") != 1 or set(names) != set(manifest["files"]) | {"buildkit.json"}:
        raise ValueError("Build-kit inventory mismatch")
    for name, entry in manifest["files"].items():
        path = output.joinpath(*PurePosixPath(name).parts)
        if "symlink" in entry:
            valid = path.is_symlink() and os.readlink(path) == entry["symlink"] and path.resolve().is_relative_to(output.resolve())
        elif entry.get("directory"):
            valid = path.is_dir() and not path.is_symlink()
        else:
            valid = (path.is_file() and not path.is_symlink() and path.stat().st_size == entry["size"]
                     and digest(path) == entry["sha256"])
            if valid:
                path.chmod(entry["mode"])
        if not valid:
            raise ValueError("Build-kit verification failed: " + name)
    print(f"Verified {len(manifest['files'])} build-kit entries")


class KitRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        from urllib.parse import urlsplit
        url = urlsplit(newurl)
        if url.scheme != "https" or url.netloc not in ("github.com", "release-assets.githubusercontent.com"):
            raise ValueError("Build-kit redirect left GitHub HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def fetch(config_path, archive_path, output):
    config = json.loads(config_path.read_text())
    if (config.get("format") != 1 or not re.fullmatch(r"[0-9a-f]{64}", config.get("sha256", ""))
            or not isinstance(config.get("size"), int) or not 0 < config["size"] <= 2 * 1024**3
            or not re.fullmatch(r"https://github\.com/Sleepywalker69/azeroth-survivors-ps5/releases/download/ps5-buildkit-[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\.tar\.gz", config.get("url", ""))):
        raise ValueError("Invalid pinned build-kit configuration")
    if not archive_path.exists():
        archive_path.parent.mkdir(parents=True, exist_ok=True)
        temporary = archive_path.with_name(archive_path.name + ".part")
        count = 0
        with build_opener(KitRedirect()).open(Request(config["url"], headers={"User-Agent": "AethusPS5Builder/1.0"}), timeout=60) as response, temporary.open("wb") as stream:
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                if count > config["size"]: raise ValueError("Build-kit download exceeds pinned size")
                stream.write(chunk)
        if count != config["size"] or digest(temporary) != config["sha256"]:
            raise ValueError("Build-kit download size/SHA-256 mismatch")
        temporary.replace(archive_path)
    elif archive_path.stat().st_size != config["size"]:
        raise ValueError("Cached build-kit size mismatch")
    unpack(archive_path, config["sha256"], output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("pack")
    for name in ("sdk", "vulkan", "wasm2c", "wasm-runtime", "quickjs", "cargo-home", "rust-toolchain", "output"):
        p.add_argument("--" + name, type=Path, required=True)
    p = commands.add_parser("unpack")
    p.add_argument("--archive", type=Path, required=True)
    p.add_argument("--sha256", required=True)
    p.add_argument("--output", type=Path, required=True)
    p = commands.add_parser("fetch")
    p.add_argument("--config", type=Path, required=True)
    p.add_argument("--archive", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "pack": pack(args)
    elif args.command == "unpack": unpack(args.archive, args.sha256, args.output)
    else: fetch(args.config, args.archive, args.output)


if __name__ == "__main__":
    main()
