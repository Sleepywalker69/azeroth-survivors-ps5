#!/usr/bin/env python3
"""Check, build and publish the Aethus PS5 release channel. Run Python with -I.

Check reads metadata only. Prepare explicitly downloads approved web/patch inputs unless
--web supplies a local candidate. Publish is a separate command and verifies every asset
before switching the channel manifest. No Windows executable is downloaded.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

HOST = Path(__file__).resolve().parents[1]
REPOSITORY = "Sleepywalker69/azeroth-survivors-ps5"
CHANNEL = "ps5-update"
RELEASES = f"https://github.com/{REPOSITORY}/releases/download/"
FEED = RELEASES + CHANNEL + "/update.json"


def module(name):
    spec = importlib.util.spec_from_file_location(name, HOST / "tools" / (name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


class GitHubRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        url = urlsplit(newurl)
        if url.scheme != "https" or url.netloc not in ("github.com", "release-assets.githubusercontent.com"):
            raise ValueError("Release download redirected outside GitHub HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def current_feed():
    try:
        with build_opener(GitHubRedirect()).open(Request(FEED, headers={"User-Agent": "AethusPS5Publisher/1.0"}), timeout=60) as response:
            data = response.read(1024 * 1024 + 1)
    except HTTPError as error:
        if error.code == 404: return None
        raise
    if len(data) > 1024 * 1024:
        raise ValueError("Published manifest exceeds 1 MiB")
    value = json.loads(data)
    validate_manifest(value)
    return value


def validate_manifest(value):
    packaging = module("package_update")
    if value.get("format") != 1 or value.get("title_id") != "PPSA99320":
        raise ValueError("Wrong published title or manifest format")
    if not isinstance(value.get("sequence"), int) or value["sequence"] <= 0:
        raise ValueError("Invalid published release sequence")
    files = value.get("files")
    if not isinstance(files, list) or not 0 < len(files) <= 1024:
        raise ValueError("Invalid manifest file list")
    seen = set()
    total = 0
    for file in files:
        name, sha, size, url = (file[k] for k in ("path", "sha256", "size", "url"))
        if not packaging.allowed(name) or name.lower() in seen or not re.fullmatch(r"[0-9a-f]{64}", sha):
            raise ValueError("Invalid manifest payload path or digest")
        seen.add(name.lower())
        limit = 4 * 1024**3 if name.startswith("Data/") else (512 * 1024**2 if name in ("eboot.bin", "sce_module/libc.prx") else 16 * 1024**2)
        if not isinstance(size, int) or not 0 <= size <= limit:
            raise ValueError("Invalid manifest payload size")
        total += size
        suffix = url.removeprefix(RELEASES)
        if url == suffix or not re.fullmatch(r"ps5-[A-Za-z0-9_.-]+/" + sha + r"\.bin", suffix):
            raise ValueError("Payload is outside this repository's immutable release assets")
    if total > 8 * 1024**3 or not packaging.REQUIRED.issubset(seen):
        raise ValueError("Incomplete or oversized manifest")


def upstream_key(info):
    selected = {k: info[k] for k in ("version", "web_id", "patches")}
    return hashlib.sha256(json.dumps(selected, sort_keys=True).encode()).hexdigest()


def needs_build(info, previous, source, force=False):
    publisher = (previous or {}).get("publisher", {})
    return force or publisher.get("upstream_key") != upstream_key(info) or publisher.get("source") != source


def check(args):
    if not re.fullmatch(r"[0-9a-f]{40}", args.source):
        raise ValueError("Source revision must be a full commit hash")
    info = module("update_pc").upstream_info()
    previous = current_feed()
    plan = {"upstream": info, "source": args.source, "previous": previous,
            "changed": needs_build(info, previous, args.source, args.force)}
    save(args.plan, plan)
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            stream.write("changed=" + str(plan["changed"]).lower() + "\n")
    print(json.dumps({"version": info["version"], "build_needed": plan["changed"], "assets": info["assets"]}, indent=2))


def reuse_payloads(manifest, previous):
    if previous:
        validate_manifest(previous)
        old = {(f["path"], f["size"], f["sha256"]): f["url"] for f in previous["files"]}
        for file in manifest["files"]:
            file["url"] = old.get((file["path"], file["size"], file["sha256"]), file["url"])
    identity = hashlib.sha256(json.dumps(manifest["files"], sort_keys=True).encode()).hexdigest()[:16]
    manifest["id"] = f"{manifest['version']}-{manifest['sequence']}-{identity}"


def prepare(args):
    updater = module("update_pc")
    plan = json.loads(args.plan.read_text())
    updater.check_patch_manifest(plan["upstream"]["patches"])
    if not plan["changed"]:
        raise ValueError("No build requested by the plan")
    args.evidence.mkdir(parents=True, exist_ok=True)
    if args.web:
        web = args.web.resolve()
        if json.loads((web / "latest.json").read_text())["version"] != plan["upstream"]["version"]:
            raise ValueError("Local candidate version differs from the plan")
    else:
        if not args.download:
            raise ValueError("Prepare requires --download or an offline --web candidate")
        web = updater.prepare_download(plan["upstream"], args.evidence, args.data)
    data_dirs = args.data + ([web / "Data"] if (web / "Data").is_dir() else [])
    expected_patches = plan["upstream"]["patches"]["patches"]
    packaging = module("package_update")
    packaging.required_patches(data_dirs, expected_patches)
    app = Path(updater.build(web, args.evidence, args.jobs, "zen2"))
    sequence = int(datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S"))
    if sequence <= (plan["previous"] or {}).get("sequence", 0):
        raise ValueError("Clock/release sequence does not increase")
    version = plan["upstream"]["version"]
    tag = f"ps5-{version}-{sequence}"
    # The upstream manifest owns patch-2 onward. Existing base game files are not bundled.
    packaging.build_feed(app, data_dirs, version, sequence, args.output, RELEASES + tag, expected_patches)
    manifest_path = args.output / "update.json"
    manifest = json.loads(manifest_path.read_text())
    reuse_payloads(manifest, plan["previous"])
    manifest["publisher"] = {"source": plan["source"], "upstream_key": upstream_key(plan["upstream"]),
        "wasm_sha256": updater.sha256(web / "wenilla_bg.wasm"), "toolchain_sha256": os.environ.get("AETHUS_BUILD_KIT_SHA256", "")}
    validate_manifest(manifest)
    save(manifest_path, manifest)
    save(args.evidence / "publish-plan.json", {"repository": REPOSITORY, "tag": tag,
        "source": plan["source"], "manifest_sha256": updater.sha256(manifest_path),
        "new_assets": sorted({f["sha256"] + ".bin" for f in manifest["files"] if f["url"].startswith(RELEASES + tag + "/")}),
        "channel": FEED})
    print("Prepared " + tag + "; publication is separate")


class GitHubFailure(RuntimeError):
    def __init__(self, result):
        self.not_found = "(HTTP 404)" in result.stderr
        super().__init__(f"GitHub command failed ({result.returncode}): {result.stderr.strip()}")


def gh(*args):
    result = subprocess.run(["gh", *map(str, args)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if result.returncode:
        raise GitHubFailure(result)
    return result.stdout


def api(path, method="GET", **fields):
    args = ["api", f"repos/{REPOSITORY}/" + path, "--method", method]
    for key, value in fields.items():
        if isinstance(value, bool): args += ["-F", f"{key}={str(value).lower()}"]
        elif isinstance(value, int): args += ["-F", f"{key}={value}"]
        else: args += ["-f", f"{key}={value}"]
    return json.loads(gh(*args))


def create_release(tag, source, title, notes, draft):
    # The creation response is authoritative even while GitHub's tag/list
    # endpoints have not yet made a new draft visible.
    return api("releases", "POST", tag_name=tag, target_commitish=source,
               name=title, body=notes, draft=draft, make_latest="false")


def assets(release_id):
    result = []
    page = 1
    while True:
        batch = api(f"releases/{release_id}/assets?per_page=100&page={page}")
        result.extend(batch)
        if len(batch) < 100: return {asset["name"]: asset for asset in result}
        page += 1


def release_by_tag(tag):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", tag):
        raise ValueError("Invalid release tag")
    try:
        return api("releases/tags/" + tag)
    except GitHubFailure as error:
        if not error.not_found: raise
    # GitHub's tag endpoint returns only published releases. A newly created
    # draft is visible through the authenticated release list instead.
    page = 1
    while True:
        batch = api(f"releases?per_page=100&page={page}")
        for release in batch:
            if release["tag_name"] == tag: return release
        if len(batch) < 100: return None
        page += 1


def verified_asset(asset, size, sha):
    if asset.get("size") != size or asset.get("digest") != "sha256:" + sha or asset.get("state") != "uploaded":
        raise ValueError("GitHub asset size/digest verification failed: " + asset.get("name", "missing"))


def promote_pointer(staged, old, size, sha):
    verified_asset(staged, size, sha)
    if old:
        api(f"releases/assets/{old['id']}", "PATCH", name=f"update-previous-{old['id']}.json")
    try:
        api(f"releases/assets/{staged['id']}", "PATCH", name="update.json")
    except Exception:
        if old: api(f"releases/assets/{old['id']}", "PATCH", name="update.json")
        raise


def publish(args):
    updater = module("update_pc")
    plan = json.loads((args.evidence / "publish-plan.json").read_text())
    manifest_path = args.feed / "update.json"
    manifest = json.loads(manifest_path.read_text())
    validate_manifest(manifest)
    tag, source = plan["tag"], plan["source"]
    if (plan["repository"] != REPOSITORY or not re.fullmatch(r"ps5-[A-Za-z0-9_.-]+", tag)
            or not re.fullmatch(r"[0-9a-f]{40}", source) or updater.sha256(manifest_path) != plan["manifest_sha256"]):
        raise ValueError("Publication plan changed or has invalid identity")
    previous = current_feed()
    if previous and previous["sequence"] >= manifest["sequence"]:
        if previous == manifest:
            print("Channel already contains this exact release"); return
        raise ValueError("Published channel advanced; refusing overwrite")
    # Validate every local blob before creating anything on GitHub.
    new_assets = {}
    for file in manifest["files"]:
        if file["url"].startswith(RELEASES + tag + "/"):
            path = args.feed / (file["sha256"] + ".bin")
            if path.is_symlink() or not path.is_file() or path.stat().st_size != file["size"] or updater.sha256(path) != file["sha256"]:
                raise ValueError("Local release blob mismatch: " + file["path"])
            new_assets[path.name] = path
    release = release_by_tag(tag)
    notes = args.evidence / "release-notes.txt"
    notes.write_text(f"Aethus {manifest['version']} for PPSA99320.\nSource: {source}\n"
                     "Built from the reviewed web-host interface. Saves and base game files are preserved.\n")
    if release is None:
        release = create_release(tag, source, f"Aethus PS5 {manifest['version']}", notes.read_text(), True)
    existing = assets(release["id"])
    for name, path in new_assets.items():
        if name not in existing:
            gh("release", "upload", tag, path, "--repo", REPOSITORY)
    if "update.json" not in existing:
        gh("release", "upload", tag, manifest_path, "--repo", REPOSITORY)
    all_assets = assets(release["id"])
    for name, path in new_assets.items():
        verified_asset(all_assets[name], path.stat().st_size, updater.sha256(path))
    verified_asset(all_assets["update.json"], manifest_path.stat().st_size, updater.sha256(manifest_path))
    # Previously published content is reused only after checking its GitHub digest again.
    remote = {}
    for file in manifest["files"]:
        asset_tag = file["url"][len(RELEASES):].split("/")[0]
        if asset_tag == tag: continue
        if asset_tag not in remote:
            remote[asset_tag] = assets(api("releases/tags/" + asset_tag)["id"])
        verified_asset(remote[asset_tag][file["sha256"] + ".bin"], file["size"], file["sha256"])
    if release["draft"]:
        gh("release", "edit", tag, "--repo", REPOSITORY, "--draft=false", "--latest=false")
    # Uploads can take time. Recheck before changing the channel, including for local runs.
    current = current_feed()
    if current and current["sequence"] >= manifest["sequence"]:
        if current == manifest:
            print("Another run already promoted this exact release"); return
        raise ValueError("Channel advanced during upload; refusing promotion")
    channel = release_by_tag(CHANNEL)
    if channel is None:
        channel = create_release(CHANNEL, source, "PS5 update channel", notes.read_text(), False)
    candidate = args.evidence / f"update-{manifest['sequence']}.json"
    shutil.copyfile(manifest_path, candidate)
    channel_assets = assets(channel["id"])
    if candidate.name not in channel_assets:
        gh("release", "upload", CHANNEL, candidate, "--repo", REPOSITORY)
    channel_assets = assets(channel["id"])
    staged = channel_assets[candidate.name]
    # Upload + verify first, then switch names. Retain the old pointer for recovery.
    old = channel_assets.get("update.json")
    promote_pointer(staged, old, candidate.stat().st_size, updater.sha256(candidate))
    verified_asset(assets(channel["id"])["update.json"], candidate.stat().st_size, updater.sha256(candidate))
    save(args.evidence / "published.json", {"tag": tag, "manifest_sha256": updater.sha256(candidate), "channel": FEED})
    print("Published and verified " + FEED)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("check")
    p.add_argument("--source", required=True)
    p.add_argument("--plan", type=Path, required=True)
    p.add_argument("--force", action="store_true")
    p = sub.add_parser("prepare")
    p.add_argument("--plan", type=Path, required=True)
    p.add_argument("--evidence", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--web", type=Path)
    p.add_argument("--download", action="store_true")
    p.add_argument("--data", type=Path, action="append", default=[])
    p.add_argument("--jobs", type=int, default=4)
    p = sub.add_parser("publish")
    p.add_argument("--feed", type=Path, required=True)
    p.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    {"check": check, "prepare": prepare, "publish": publish}[args.command](args)


if __name__ == "__main__":
    main()
