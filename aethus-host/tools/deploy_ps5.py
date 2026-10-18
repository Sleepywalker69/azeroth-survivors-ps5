#!/usr/bin/env python3
"""Deploy PPSA99320 over FTP, verifying each staged file and retaining replaced files.

The caller must ensure the title is closed. Existing userdata and extra Data files are kept.
No process control or deletion is used. Executable permissions are set before promotion.
"""
import argparse
from datetime import datetime, timezone
import ftplib
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import time


TITLE = "PPSA99320"


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def remote_digest(ftp, path):
    digest = hashlib.sha256()
    count = 0
    def consume(data):
        nonlocal count
        count += len(data)
        digest.update(data)
    ftp.retrbinary("RETR " + path, consume, blocksize=1024 * 1024)
    return count, digest.hexdigest()


def remote_size(ftp, path):
    ftp.voidcmd("TYPE I")
    try:
        return ftp.size(path)
    except ftplib.error_perm as error:
        if str(error).startswith("550"):
            return None
        raise


def mkdirs(ftp, path):
    current = ""
    for part in PurePosixPath(path).parts:
        if part == "/":
            continue
        current += "/" + part
        try:
            ftp.mkd(current)
        except ftplib.error_perm:
            # A denied mkdir is acceptable only if the directory already exists.
            previous = ftp.pwd()
            ftp.cwd(current)
            ftp.cwd(previous)


def package_files(app, data_dirs):
    param = json.loads((app / "sce_sys/param.json").read_text())
    if param.get("titleId") != TITLE:
        raise ValueError(f"Package title is not {TITLE}")
    files = [("sce_sys/icon0.png", app / "sce_sys/icon0.png"), ("eboot.bin", app / "eboot.bin")]
    for directory in ("sce_sys", "sce_module", "js", "web", "licenses"):
        if not (app / directory).is_dir():
            raise ValueError(f"Missing package directory: {directory}")
        for file in sorted((app / directory).rglob("*")):
            relative = file.relative_to(app).as_posix()
            if file.is_file() and relative != "sce_sys/icon0.png":
                files.append((relative, file))
    patches = {}
    for directory in data_dirs:
        for file in directory.iterdir():
            if file.is_file() and re.fullmatch(r"patch(?:-(?:[2-9]|[1-9][0-9]+))?\.mpq", file.name, re.I):
                patches[file.name.lower()] = file
    for name in sorted(patches, key=lambda name: 0 if name == "patch.mpq" else int(name[6:-4])):
        files.append(("Data/" + patches[name].name, patches[name]))
    for relative, source in files:
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"Missing or symlinked package input: {source}")
        if any(c in relative for c in "\r\n\\") or ".." in PurePosixPath(relative).parts:
            raise ValueError(f"Unsafe package path: {relative}")
    return files


def deploy(app, data_dirs, host, port, evidence, env_file=None):
    files = package_files(app, data_dirs)
    if env_file is not None:
        if not env_file.is_file() or env_file.is_symlink():
            raise ValueError("Invalid environment file")
        if b"\x00" in env_file.read_bytes():
            raise ValueError("Environment file contains NUL bytes; restore it before deployment")
        files.insert(next((i for i, (name, _) in enumerate(files) if name.startswith("Data/")), len(files)), ("env.txt", env_file))
    evidence.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    root = "/data/homebrew/" + TITLE
    backup = "/data/aethus-backups/" + TITLE + "/" + stamp
    receipt = {"title": TITLE, "host": host, "port": port, "backup": backup, "files": [], "complete": False}
    receipt_path = evidence / ("deploy-" + stamp + ".json")
    def save():
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    save()
    try:
        with ftplib.FTP() as ftp:
            ftp.connect(host, port, timeout=120)
            ftp.login()
            for relative, source in files:
                expected = (source.stat().st_size, sha256(source))
                target = root + "/" + relative
                old_size = remote_size(ftp, target)
                old_digest = None
                if old_size == expected[0]:
                    _, old_digest = remote_digest(ftp, target)
                    if old_digest == expected[1] and relative != "sce_sys/icon0.png":
                        receipt["files"].append({"path": relative, "size": expected[0], "sha256": expected[1], "action": "unchanged-verified"})
                        save()
                        print("UNCHANGED " + relative, flush=True)
                        continue
                mkdirs(ftp, str(PurePosixPath(target).parent))
                staged = target + ".aethus-" + stamp + ".part"
                print(f"UPLOAD {relative} ({expected[0]} bytes)", flush=True)
                start = time.monotonic()
                with source.open("rb") as stream:
                    ftp.storbinary("STOR " + staged, stream, blocksize=1024 * 1024)
                actual = remote_digest(ftp, staged)
                if actual != expected:
                    raise ValueError(f"FTP readback mismatch for {relative}: {actual} != {expected}")
                if relative == "eboot.bin":
                    ftp.sendcmd("SITE CHMOD 755 " + staged)
                    listing = []
                    ftp.retrlines("LIST " + staged, listing.append)
                    if not listing or not listing[0].startswith("-rwxr-xr-x"):
                        raise ValueError("Staged executable permissions did not verify")
                entry = {"path": relative, "size": expected[0], "sha256": expected[1], "action": "staged-verified", "staged": staged}
                receipt["files"].append(entry)
                save()
                if old_size is not None:
                    if old_digest is None:
                        _, old_digest = remote_digest(ftp, target)
                    saved = backup + "/" + relative
                    mkdirs(ftp, str(PurePosixPath(saved).parent))
                    entry.update(backup=saved, old_size=old_size, old_sha256=old_digest)
                    save()
                    ftp.rename(target, saved)
                    entry["action"] = "old-file-backed-up"
                    save()
                try:
                    ftp.rename(staged, target)
                except Exception:
                    if entry.get("backup"):
                        ftp.rename(entry["backup"], target)
                        entry["action"] = "rename-failed-old-restored"
                        save()
                    raise
                if remote_size(ftp, target) != expected[0]:
                    raise ValueError(f"Published size mismatch for {relative}")
                entry["action"] = "installed-verified"
                entry["seconds"] = round(time.monotonic() - start, 2)
                save()
                print(f"VERIFIED {relative} {expected[1]} ({entry['seconds']} s)", flush=True)
        receipt["complete"] = True
        save()
    except Exception as error:
        receipt["error"] = str(error)
        save()
        raise
    print("Deployment receipt: " + str(receipt_path), flush=True)
    return receipt_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--data", type=Path, action="append", default=[])
    parser.add_argument("--host", default="192.168.0.86")
    parser.add_argument("--port", type=int, default=2121)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--env-file", type=Path, help="Explicit environment replacement, backed up like the other files")
    parser.add_argument("--title-closed", action="store_true", required=True)
    args = parser.parse_args()
    deploy(args.app, args.data, args.host, args.port, args.evidence, args.env_file)


if __name__ == "__main__":
    main()
