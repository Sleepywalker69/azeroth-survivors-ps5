import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

HOST = Path(__file__).resolve().parents[1]
def load(name):
    spec = importlib.util.spec_from_file_location(name, HOST / "tools" / (name + ".py"))
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result)
    return result
cloud = load("cloud_release")
kit = load("buildkit")


def manifest(sequence=1):
    files = []
    for path in ["eboot.bin", "js/boot.mjs", "web/wenilla.js", "sce_sys/param.json"]:
        sha = hashlib.sha256(path.encode()).hexdigest()
        files.append({"path": path, "size": 10, "sha256": sha,
                      "url": cloud.RELEASES + f"ps5-0.97.0-{sequence}/" + sha + ".bin"})
    return {"format": 1, "title_id": "PPSA99320", "sequence": sequence, "id": "test", "version": "0.97.0", "files": files}


class CloudTests(unittest.TestCase):
    def test_unchanged_upstream_and_source_skip_build(self):
        info = {"version": "0.97.0", "web_id": "123", "patches": {"patches": []}}
        prior = {"publisher": {"upstream_key": cloud.upstream_key(info), "source": "a" * 40}}
        self.assertFalse(cloud.needs_build(info, prior, "a" * 40))
        self.assertTrue(cloud.needs_build(info, prior, "b" * 40))
        self.assertTrue(cloud.needs_build(info, prior, "a" * 40, True))
        changed = copy.deepcopy(info); changed["patches"]["patches"].append({"new": True})
        self.assertTrue(cloud.needs_build(changed, prior, "a" * 40))

    def test_feed_cannot_publish_private_paths_or_external_payloads(self):
        cloud.validate_manifest(manifest())
        for key, value in [("path", "userdata/save.json"), ("url", "https://evil.example/file"),
                           ("url", cloud.RELEASES + "ps5-x/../../file"), ("size", -1), ("sha256", "x" * 64)]:
            bad = manifest(); bad["files"][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError): cloud.validate_manifest(bad)
        bad = manifest(); bad["files"].append(copy.deepcopy(bad["files"][0]))
        with self.assertRaises(ValueError): cloud.validate_manifest(bad)

    def test_reuse_only_matching_previous_content(self):
        previous, current = manifest(1), manifest(2)
        current["files"][0]["sha256"] = "f" * 64
        original_url = current["files"][0]["url"]
        cloud.reuse_payloads(current, previous)
        self.assertEqual(current["files"][0]["url"], original_url)
        self.assertEqual(current["files"][1]["url"], previous["files"][1]["url"])
        self.assertTrue(current["id"].startswith("0.97.0-2-"))

    def test_missing_or_wrong_github_digest_blocks_promotion(self):
        for asset in [{}, {"size": 10, "digest": "sha256:" + "b" * 64, "state": "uploaded"},
                      {"size": 11, "digest": "sha256:" + "a" * 64, "state": "uploaded"}]:
            with self.assertRaises(ValueError): cloud.verified_asset(asset, 10, "a" * 64)
        cloud.verified_asset({"size": 10, "digest": "sha256:" + "a" * 64, "state": "uploaded"}, 10, "a" * 64)

    def test_bad_candidate_leaves_current_channel_untouched(self):
        with patch.object(cloud, "api") as api:
            with self.assertRaises(ValueError): cloud.promote_pointer({"id": 2}, {"id": 1}, 10, "a" * 64)
            api.assert_not_called()

    def test_failed_promotion_restores_previous_pointer(self):
        staged = {"id": 2, "size": 10, "digest": "sha256:" + "a" * 64, "state": "uploaded"}
        calls = []
        def mutate(path, method, **fields):
            calls.append((path, fields["name"]))
            if path.endswith("/2"): raise RuntimeError("injected rename failure")
            return {}
        with patch.object(cloud, "api", side_effect=mutate), self.assertRaisesRegex(RuntimeError, "injected"):
            cloud.promote_pointer(staged, {"id": 1}, 10, "a" * 64)
        self.assertEqual(calls, [("releases/assets/1", "update-previous-1.json"),
                                ("releases/assets/2", "update.json"), ("releases/assets/1", "update.json")])

    def test_successful_promotion_retains_old_pointer(self):
        staged = {"id": 2, "size": 10, "digest": "sha256:" + "a" * 64, "state": "uploaded"}
        with patch.object(cloud, "api", return_value={}) as api:
            cloud.promote_pointer(staged, {"id": 1}, 10, "a" * 64)
            self.assertEqual(api.call_count, 2)
            self.assertEqual(api.call_args.kwargs["name"], "update.json")

    def test_buildkit_rejects_archive_escape_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); archive = root / "bad.tar.gz"
            with tarfile.open(archive, "w:gz") as stream:
                entry = tarfile.TarInfo("../escape"); entry.size = 1
                stream.addfile(entry, io.BytesIO(b"x"))
            with self.assertRaises(ValueError): kit.unpack(archive, kit.digest(archive), root / "out")
            self.assertFalse((root / "escape").exists())


if __name__ == "__main__": unittest.main()
