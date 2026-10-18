#!/usr/bin/env python3
"""Copy Cargo.lock's registry artifacts from another local cache; never download."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tomllib


def prefix(name):
    if len(name) <= 2:
        return str(len(name))
    if len(name) == 3:
        return f"3/{name[0]}"
    return f"{name[:2]}/{name[2:4]}"


def indexed(path, version, checksum):
    if not path.is_file():
        return False
    for item in path.read_bytes().split(b"\0"):
        if item.startswith(b"{"):
            try:
                entry = json.loads(item)
            except (ValueError, UnicodeDecodeError):
                continue
            if entry.get("vers") == version and entry.get("cksum") == checksum:
                return True
    return False


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", type=Path, default=Path.home() / ".cargo/registry")
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    registry = "index.crates.io-1949cf8c6b5b557f"
    packages = tomllib.loads(args.lock.read_text())["package"]
    report = {"copied_crates": [], "copied_indexes": [], "missing": [], "checked": 0}
    for package in packages:
        if not package.get("source", "").startswith("registry+"):
            continue
        name, version, checksum = (package[k] for k in ("name", "version", "checksum"))
        report["checked"] += 1
        relative = Path("cache") / registry / f"{name}-{version}.crate"
        source, destination = args.source / relative, args.destination / relative
        if not destination.is_file() or digest(destination) != checksum:
            if not source.is_file() or digest(source) != checksum:
                report["missing"].append(str(relative))
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
                report["copied_crates"].append(str(relative))
        relative = Path("index") / registry / ".cache" / prefix(name) / name
        source, destination = args.source / relative, args.destination / relative
        if not indexed(destination, version, checksum):
            if not indexed(source, version, checksum):
                report["missing"].append(f"{relative} ({version})")
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                if destination.exists():
                    backup = args.report.parent / "registry-index-before" / prefix(name) / name
                    backup.parent.mkdir(parents=True, exist_ok=True)
                    if not backup.exists():
                        shutil.copy2(destination, backup)
                shutil.copy2(source, destination)
                report["copied_indexes"].append(str(relative))
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: len(v) if isinstance(v, list) else v for k, v in report.items()}))
    for missing in report["missing"]:
        print("MISSING " + missing)
    return 1 if report["missing"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
