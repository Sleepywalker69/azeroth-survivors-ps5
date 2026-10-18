#!/usr/bin/env python3
"""Check, download, validate, build and deploy Aethus web releases on Windows or Linux.

Default operation checks metadata only. --download explicitly authorizes the files in the
printed plan. No Windows executable is fetched. Cargo always runs offline. Use Python -I.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
from urllib.parse import urljoin, urlsplit, unquote
from urllib.request import Request, HTTPRedirectHandler, build_opener
import zlib

HOST = Path(__file__).resolve().parents[1]
REPOS = HOST.parent
ORIGIN = "https://wow.aethus.pl/"
LATEST = ORIGIN + "exe/latest.json"
PATCHES = ORIGIN + "exe/data/patches.json"
UA = "AethusHostUpdater/1.0"
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?\Z")


class SameOriginRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        target = urlsplit(newurl)
        if target.scheme != "https" or target.netloc != "wow.aethus.pl":
            raise ValueError("Upstream redirected outside its HTTPS origin")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".new")
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def load_tool(name):
    # Only load our own tool source; -I deliberately excludes the working directory.
    path = HOST / ("gen/parse_glue.py" if name == "parse_glue" else f"tools/{name}.py")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def safe_relative(name):
    decoded = unquote(name)
    path = PurePosixPath(decoded)
    if (not name or name != decoded or path.is_absolute() or "\\" in name or
            any(part in ("", ".", "..") for part in name.split("/")) or
            any(ord(c) < 32 for c in name) or ":" in name or "?" in name or "#" in name):
        raise ValueError(f"Unsafe relative URL/path: {name!r}")
    return name


def request(url, method="GET"):
    if urlsplit(url).scheme != "https" or urlsplit(url).netloc != "wow.aethus.pl":
        raise ValueError(f"Unexpected upstream URL: {url}")
    response = build_opener(SameOriginRedirect()).open(Request(url, method=method, headers={
        "User-Agent": UA, "Accept-Encoding": "identity", "Cache-Control": "no-cache",
    }), timeout=60)
    if urlsplit(response.url).scheme != "https" or urlsplit(response.url).netloc != "wow.aethus.pl":
        response.close()
        raise ValueError("Upstream redirected outside its HTTPS origin")
    if response.headers.get("Content-Encoding", "identity") not in ("identity", ""):
        response.close()
        raise ValueError("Server ignored Accept-Encoding: identity; refusing encoded content")
    return response


def get_json(url):
    with request(url) as response:
        data = response.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("Upstream manifest exceeds 1 MiB")
    return json.loads(data)


def upstream_info():
    latest = get_json(LATEST)
    patches = get_json(PATCHES)
    check_patch_manifest(patches)
    if not VERSION.fullmatch(latest.get("version", "")):
        raise ValueError("Invalid upstream version")
    assets = []
    for name in ("wenilla_bg.wasm", "wenilla.js"):
        with request(ORIGIN + name, "HEAD") as response:
            headers = response.headers
            assets.append({"name": name, "url": ORIGIN + name,
                "size": int(headers.get("x-decoded-length") or headers.get("Content-Length") or 0),
                "etag": headers.get("ETag", ""), "modified": headers.get("Last-Modified", "")})
    identity = hashlib.sha256(json.dumps(assets, sort_keys=True).encode()).hexdigest()[:12]
    return {"version": latest["version"], "web_id": identity, "assets": assets,
            "latest": latest, "patches": patches}


def download(url, destination, limit, expected_size=0):
    if destination.exists():
        raise ValueError(f"Refusing to overwrite downloaded input: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    part = destination.with_name(destination.name + ".part")
    count = 0
    with request(url) as response, part.open("wb") as stream:
        declared = int(response.headers.get("Content-Length") or 0)
        if declared > limit:
            raise ValueError(f"Content-Length exceeds limit: {url}")
        while chunk := response.read(1024 * 1024):
            count += len(chunk)
            if count > limit:
                raise ValueError(f"Download exceeds limit: {url}")
            stream.write(chunk)
    if expected_size and count != expected_size:
        raise ValueError(f"Size mismatch for {url}: {count} != {expected_size}")
    part.replace(destination)
    return {"path": str(destination), "url": url, "size": count, "sha256": sha256(destination)}


def imports(path):
    text = path.read_text(encoding="utf-8")
    return re.findall(r'^import\s+.*?\s+from\s+[\'"]([^\'"]+)[\'"]', text, re.M)


def glue_signature(web):
    records = load_tool("parse_glue").parse(web / "wenilla.js")
    if len(records) < 100:
        raise ValueError("The glue parser found too few imports; format requires review")
    shapes = {}
    for record in records:
        name = re.sub(r"_[0-9a-f]{16}$", "", record["name"])
        shape = {k: record[k] for k in ("params", "handle_error", "body")}
        # wasm-bindgen records numeric function indexes in standalone comments. They are
        # regenerated for every wasm, and have no executable meaning in the JS glue.
        shape["body"] = re.sub(r"(?m)^\s*// Cast intrinsic for `[^\n]*`\.\s*$", "", shape["body"]).strip()
        normalized = json.dumps(shape, sort_keys=True)
        key = name + ":" + hashlib.sha256(normalized.encode()).hexdigest()
        shapes[key] = record["name"]
    snippets = {}
    for dependency in imports(web / "wenilla.js"):
        if dependency == "wasi_snapshot_preview1":
            continue
        if not dependency.startswith("./snippets/"):
            raise ValueError(f"Unreviewed module dependency: {dependency}")
        relative = safe_relative(dependency[2:])
        source = web / relative
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"Missing snippet: {relative}")
        snippets[relative] = sha256(source)
    return {"imports": shapes, "snippets": snippets}


def check_glue(web, evidence):
    allowed = json.loads((HOST / "tools/host-compatibility.json").read_text())
    current = glue_signature(web)
    unknown = [name for shape, name in current["imports"].items() if shape not in allowed["imports"]]
    known_snippets = set(allowed["snippets"].values())
    unknown.extend(path for path, digest in current["snippets"].items() if digest not in known_snippets)
    if unknown:
        dump(evidence / "unreviewed-glue.json", {"unknown": unknown, "candidate": current})
        raise ValueError("Host review required for new/changed glue imports or snippets: " + ", ".join(unknown[:12]))
    # Validate the exact helper before spending time translating/compiling the wasm.
    load_tool("prepare_glue").optimize((web / "wenilla.js").read_text(encoding="utf-8"))
    print(f"Compatibility gate passed: {len(current['imports'])} known glue imports", flush=True)


def check_patch(entry):
    if not re.fullmatch(r"patch-(?:[2-9]|[1-9][0-9]+)\.MPQ", entry.get("name", "")):
        raise ValueError("Unsafe patch name")
    if not isinstance(entry.get("size"), int) or not 0 < entry["size"] <= 4 * 1024**3:
        raise ValueError("Invalid patch size")
    if not re.fullmatch(r"[0-9a-f]{64}", entry.get("sha256", "")):
        raise ValueError("Invalid patch digest")
    if not isinstance(entry.get("parts"), list) or not 0 < len(entry["parts"]) <= 256:
        raise ValueError("Invalid patch parts")
    for part in entry["parts"]:
        safe_relative(part["url"])
        if not isinstance(part.get("size"), int) or not 0 < part["size"] <= 128 * 1024**2:
            raise ValueError("Invalid patch part size")


def check_patch_manifest(manifest):
    entries = manifest.get("patches")
    if not isinstance(entries, list) or len(entries) > 128:
        raise ValueError("Invalid patch manifest count")
    names, decoded, downloaded = set(), 0, 0
    for entry in entries:
        check_patch(entry)
        if entry["name"] in names:
            raise ValueError("Duplicate patch name")
        names.add(entry["name"])
        decoded += entry["size"]
        downloaded += sum(part["size"] for part in entry["parts"])
    if decoded > 6 * 1024**3 or downloaded > 6 * 1024**3:
        raise ValueError("Combined patches exceed the 6 GiB download/install budget")


def assemble_patch(parts, destination, entry):
    """Aethus multipart archives: concatenate, then decode gzip/zlib or plain MPQ."""
    with parts[0].open("rb") as first:
        magic = first.read(4)
    decoder = None if magic == b"MPQ\x1a" else zlib.decompressobj(47)
    count = 0
    pending = destination.with_name(destination.name + ".part")
    with pending.open("wb") as output:
        for part in parts:
            with part.open("rb") as source:
                while block := source.read(1024 * 1024):
                    if decoder:
                        block = decoder.decompress(block, entry["size"] - count + 1)
                        if decoder.unconsumed_tail:
                            raise ValueError("Patch expands beyond declared size")
                    count += len(block)
                    if count > entry["size"]:
                        raise ValueError("Patch expands beyond declared size")
                    output.write(block)
        if decoder and (not decoder.eof or decoder.unused_data):
            raise ValueError("Truncated or trailing compressed patch data")
    if count != entry["size"] or sha256(pending) != entry["sha256"]:
        raise ValueError(f"Patch checksum/size mismatch: {entry['name']}")
    with pending.open("rb") as stream:
        if stream.read(4) != b"MPQ\x1a":
            raise ValueError("Decoded patch is not an MPQ archive")
    pending.replace(destination)


def prepare_download(info, evidence, data_dirs):
    check_patch_manifest(info["patches"])
    folder = REPOS / ("aethus-web-" + info["version"] + "-" + info["web_id"])
    if (folder / "download.json").exists():
        receipt = json.loads((folder / "download.json").read_text())
        for file in receipt["files"]:
            if sha256(folder / file["relative"]) != file["sha256"]:
                raise ValueError("Previously downloaded release was changed")
        return folder
    folder.mkdir(exist_ok=True)
    dump(folder / "latest.json", info["latest"])
    dump(folder / "patches.json", info["patches"])
    receipt = {"upstream": info, "files": []}
    progress = folder / "download-progress.json"
    if progress.exists():
        receipt = json.loads(progress.read_text())
        if receipt["upstream"] != info:
            raise ValueError("Incomplete candidate belongs to different upstream metadata")
    def fetch(url, relative, limit, expected_size=0):
        old = next((file for file in receipt["files"] if file["relative"] == relative), None)
        destination = folder / relative
        if old:
            if not destination.is_file() or destination.stat().st_size != old["size"] or sha256(destination) != old["sha256"]:
                raise ValueError(f"Interrupted download input changed: {relative}")
            return
        result = download(url, destination, limit, expected_size)
        receipt["files"].append(dict(result, relative=relative))
        dump(progress, receipt)
    for asset in info["assets"]:
        limit = (256 if asset["name"].endswith(".wasm") else 2) * 1024**2
        fetch(asset["url"], asset["name"], limit, asset["size"])
    with (folder / "wenilla_bg.wasm").open("rb") as stream:
        if stream.read(8) != b"\0asm\x01\0\0\0":
            raise ValueError("Invalid wasm header/version")
    for dependency in sorted(set(imports(folder / "wenilla.js"))):
        if dependency == "wasi_snapshot_preview1":
            continue
        if not dependency.startswith("./snippets/"):
            raise ValueError("Unreviewed glue dependency: " + dependency)
        relative = safe_relative(dependency[2:])
        fetch(urljoin(ORIGIN, relative), relative, 2 * 1024**2)
    check_glue(folder, evidence)
    target = folder / "Data"
    target.mkdir(exist_ok=True)
    for entry in info["patches"]["patches"]:
        check_patch(entry)
        local = next((directory / entry["name"] for directory in data_dirs
                      if (directory / entry["name"]).is_file() and
                      (directory / entry["name"]).stat().st_size == entry["size"] and
                      sha256(directory / entry["name"]) == entry["sha256"]), None)
        if local:
            print("Cached patch verified: " + str(local), flush=True)
            continue
        parts = []
        for index, part in enumerate(entry["parts"]):
            path = folder / "parts" / entry["name"] / f"{index:04d}.bin"
            download(urljoin(PATCHES, part["url"]), path, part["size"], part["size"])
            parts.append(path)
        assemble_patch(parts, target / entry["name"], entry)
    # Detect an upstream release switching while its unversioned web files were fetched.
    after = upstream_info()
    if after["web_id"] != info["web_id"] or after["version"] != info["version"] or after["patches"] != info["patches"]:
        raise ValueError("Upstream changed during download; retain this candidate and retry a fresh release")
    dump(folder / "download.json", receipt)
    return folder


def run(command, log, env=None):
    print("RUN " + subprocess.list2cmdline([str(arg) for arg in command]), flush=True)
    with log.open("wb") as output:
        result = subprocess.run([str(arg) for arg in command], stdout=output, stderr=subprocess.STDOUT, env=env)
    if result.returncode:
        print(log.read_text(encoding="utf-8", errors="replace")[-8000:])
        raise RuntimeError(f"Command exited {result.returncode}; full output: {log}")


def wsl_path(path):
    if os.name != "nt":
        return str(path.resolve())
    return subprocess.check_output(["wsl.exe", "-e", "wslpath", "-u", str(path.resolve())], text=True).strip()


def wasm2c_command(wasm, output):
    override = os.environ.get("WASM2C")
    if override:
        compiler = Path(override).resolve(strict=True)
    elif os.name == "nt":
        compiler = REPOS / "tools/wabt/wabt-1.0.42/bin/wasm2c.exe"
    else:
        native = shutil.which("wasm2c")
        compiler = Path(native) if native else REPOS / "tools/wabt/wabt-1.0.42/bin/wasm2c"
    if not compiler.is_file():
        raise ValueError("Native wasm2c is missing; set WASM2C to its executable path")
    if os.name != "nt" and compiler.suffix.lower() == ".exe":
        raise ValueError("Linux builds require native wasm2c, not a Windows executable")
    return [compiler, str(wasm), "-n", "wenilla", "--num-outputs=128", "-o", str(output)]


def build(web, evidence, jobs, cpu="generic"):
    wasm = web / "wenilla_bg.wasm"
    version = json.loads((web / "latest.json").read_text())["version"]
    if not VERSION.fullmatch(version):
        raise ValueError("Invalid local version")
    check_glue(web, evidence)
    identity = version + "-" + sha256(wasm)[:12]
    c_flags = {"generic": "-O2", "zen2": "-O3 -march=znver2 -ffp-contract=off"}[cpu]
    work_identity = identity + ("-zen2" if cpu == "zen2" else "")
    directory = REPOS / "aethus-build" / identity
    directory.mkdir(parents=True, exist_ok=True)
    translation = directory / "wasm2c"
    translation.mkdir(exist_ok=True)
    generated = directory / "gen"
    generated.mkdir(exist_ok=True)
    marker = directory / "translation.json"
    command = wasm2c_command(wasm, translation / "wenilla.c")
    expected = {"wasm": sha256(wasm), "wasm2c": sha256(command[0]), "outputs": 128}
    if not marker.exists() or json.loads(marker.read_text()) != expected:
        run(command, evidence / "wasm2c.log")
        if len(list(translation.glob("wenilla_*.c"))) != 128 or not (translation / "wenilla.h").is_file():
            raise ValueError("Incomplete wasm2c output")
        dump(marker, expected)
    bridge = generated / "bridge_gen.c"
    run([sys.executable, "-I", HOST / "gen/gen_bridge.py", translation / "wenilla.h", bridge], evidence / "bridge.log")
    if os.name == "nt":
        home = subprocess.check_output(["wsl.exe", "-e", "printenv", "HOME"], text=True).strip()
        work = home + "/ps5/aethus-releases/" + work_identity
        command = ["wsl.exe", "-e", "env", "AETHUS_BUILD=" + wsl_path(directory),
            "AETHUS_WEB=" + wsl_path(web), "AETHUS_WORK=" + work, "JOBS=" + str(jobs),
            "AETHUS_C_OPT_FLAGS=" + c_flags,
            "bash", wsl_path(HOST / "ps5/build.sh")]
    else:
        releases = Path(os.environ.get("AETHUS_RELEASES", str(Path.home() / "ps5/aethus-releases")))
        work = str(releases / work_identity)
        command = ["env", "AETHUS_BUILD=" + str(directory), "AETHUS_WEB=" + str(web),
            "AETHUS_WORK=" + work, "JOBS=" + str(jobs), "AETHUS_C_OPT_FLAGS=" + c_flags,
            "bash", str(HOST / "ps5/build.sh")]
    run(command, evidence / "ps5-build.log")
    app = work + "/dist/PPSA99320"
    eboot_hash = (subprocess.check_output(["wsl.exe", "-e", "sha256sum", app + "/eboot.bin"], text=True).split()[0]
                  if os.name == "nt" else sha256(Path(app) / "eboot.bin"))
    dump(evidence / "build.json", {"version": version, "identity": identity, "app_wsl": app,
        "web": str(web), "wasm_sha256": expected["wasm"], "eboot_sha256": eboot_hash,
        "cpu": cpu, "c_flags": c_flags})
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--download", action="store_true", help="Authorize downloading the new web build, snippets and missing patches")
    parser.add_argument("--web", type=Path, help="Use this already downloaded web build, completely offline")
    parser.add_argument("--data", type=Path, action="append", default=[])
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--deploy", action="store_true")
    parser.add_argument("--title-closed", action="store_true")
    parser.add_argument("--host", default="192.168.0.86")
    parser.add_argument("--port", type=int, default=2121)
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--cpu", choices=("generic", "zen2"), default="generic",
                        help="C optimization profile; separate object directories preserve the comparison build")
    parser.add_argument("--feed-output", type=Path, help="Prepare a console auto-update feed after building; does not publish")
    parser.add_argument("--feed-base-url", help="Directory URL at which the prepared feed will be served")
    parser.add_argument("--sequence", type=int, help="Increasing console release sequence, including host-only updates")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if args.web and (args.check or args.download):
        parser.error("--web is offline and cannot be combined with --check or --download")
    if args.deploy and (not args.build or not args.title_closed):
        parser.error("--deploy requires --build and --title-closed")
    if any(v is not None for v in (args.feed_output, args.feed_base_url, args.sequence)):
        if not args.build or not all(v is not None for v in (args.feed_output, args.feed_base_url, args.sequence)):
            parser.error("feed preparation requires --build --feed-output --feed-base-url --sequence")
        if args.sequence <= 0:
            parser.error("--sequence must be positive")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    evidence = args.evidence or REPOS / "aethus-build/update-receipts" / stamp
    evidence.mkdir(parents=True, exist_ok=True)
    data_dirs = [path.resolve() for path in args.data]
    if (REPOS / "aethus-data").is_dir():
        data_dirs.append(REPOS / "aethus-data")
    if args.web:
        web = args.web.resolve()
    else:
        info = upstream_info()
        dump(evidence / "upstream-plan.json", info)
        print(json.dumps({"version": info["version"], "web_id": info["web_id"], "assets": info["assets"]}, indent=2))
        if not args.download:
            print("Metadata check complete. To download this release use --download; no executable was fetched.")
            return
        web = prepare_download(info, evidence, data_dirs)
    check_glue(web, evidence)
    if (web / "Data").is_dir():
        data_dirs.append(web / "Data")
    if args.build:
        app = build(web, evidence, args.jobs, args.cpu)
        if args.feed_output:
            command = (["wsl.exe", "-e"] if os.name == "nt" else []) + ["python3", "-I",
                wsl_path(HOST / "tools/package_update.py"), "--app", app,
                "--output", wsl_path(args.feed_output), "--base-url", args.feed_base_url,
                "--version", json.loads((web / "latest.json").read_text())["version"], "--sequence", str(args.sequence)]
            for directory in data_dirs:
                command += ["--data", wsl_path(directory)]
            run(command, evidence / "feed.log")
            dump(evidence / "feed.json", json.loads((evidence / "feed.log").read_text()))
        if args.deploy:
            command = (["wsl.exe", "-e"] if os.name == "nt" else []) + ["python3", "-I",
                wsl_path(HOST / "tools/deploy_ps5.py"), "--app", app, "--host", args.host,
                "--port", str(args.port), "--evidence", wsl_path(evidence), "--title-closed"]
            for directory in data_dirs:
                command += ["--data", wsl_path(directory)]
            run(command, evidence / "deploy.log")
    print("Updater complete; receipts: " + str(evidence), flush=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError) as error:
        print("Updater stopped: " + str(error), file=sys.stderr)
        raise SystemExit(1)
