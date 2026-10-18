import importlib.util
import json
import hashlib
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("package_update", Path(__file__).resolve().parents[1] / "tools/package_update.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PackageTests(unittest.TestCase):
    def test_expected_upstream_patches_are_complete_and_verified(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            body = b"expected patch"
            expected = [{"name": "patch-2.MPQ", "size": len(body), "sha256": hashlib.sha256(body).hexdigest()}]
            with self.assertRaisesRegex(ValueError, "missing or changed"):
                module.required_patches([root], expected)
            (root / "patch-2.MPQ").write_bytes(b"incorrect data")
            with self.assertRaisesRegex(ValueError, "missing or changed"):
                module.required_patches([root], expected)
            (root / "patch-2.MPQ").write_bytes(body)
            self.assertEqual(module.required_patches([root], expected), {"patch-2.MPQ": root / "patch-2.MPQ"})
            with self.assertRaisesRegex(ValueError, "identity"):
                module.required_patches([root], expected + expected)

    def test_feed_preserves_payload_and_excludes_private_data(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            app = root / "app"
            files = {"eboot.bin": (0x1D3D154F).to_bytes(4, "little") + b"test executable",
                     "sce_sys/param.json": b'{"titleId":"PPSA99320"}', "js/boot.mjs": b"boot",
                     "web/wenilla.js": b"web", "userdata/localStorage.json": b"private save",
                     "env.txt": b"private config", "Data/base.MPQ": b"base data", "Data/patch-5.MPQ": b"patch"}
            for name, body in files.items():
                path = app / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(body)
            output = root / "feed"
            receipt = module.build_feed(app, [], "0.97.0", 1, output, "http://192.168.0.123:8840")
            manifest = json.loads((output / "update.json").read_text())
            self.assertEqual({f["path"] for f in manifest["files"]}, {"eboot.bin", "sce_sys/param.json", "js/boot.mjs", "web/wenilla.js", "Data/patch-5.MPQ"})
            self.assertEqual(receipt["manifest_sha256"], module.digest(output / "update.json"))
            for f in manifest["files"]:
                self.assertEqual((output / (f["sha256"] + ".bin")).read_bytes(), files[f["path"]])
            with self.assertRaisesRegex(ValueError, "immutable"):
                module.build_feed(app, [], "0.97.0", 1, output, "http://192.168.0.123:8840")

            # A complete cloud feed must include the required patch even when a
            # different patch is already present in the built package.
            patch = b"required upstream patch"
            expected = [{"name": "patch-2.MPQ", "size": len(patch), "sha256": hashlib.sha256(patch).hexdigest()}]
            cloud_output = root / "cloud-feed"
            with self.assertRaisesRegex(ValueError, "missing or changed"):
                module.build_feed(app, [root], "0.97.0", 2, cloud_output, "https://example.com/release", expected)
            self.assertFalse((cloud_output / "update.json").exists())
            (root / "patch-2.MPQ").write_bytes(patch)
            module.build_feed(app, [root], "0.97.0", 2, cloud_output, "https://example.com/release", expected)
            cloud_manifest = json.loads((cloud_output / "update.json").read_text())
            cloud_patches = [f for f in cloud_manifest["files"] if f["path"].startswith("Data/")]
            self.assertEqual([f["path"] for f in cloud_patches], ["Data/patch-2.MPQ"])
            self.assertEqual((cloud_output / (cloud_patches[0]["sha256"] + ".bin")).read_bytes(), patch)

    def test_payload_path_policy(self):
        for path in ["../eboot.bin", "userdata/save", "env.txt", "Data/base.MPQ", "js/../save", "web/x:y", "web/x%2Fy"]:
            self.assertFalse(module.allowed(path), path)
        for path in ["eboot.bin", "js/boot.mjs", "web/snippets/game/inline0.js", "Data/patch.MPQ", "Data/patch-5.MPQ"]:
            self.assertTrue(module.allowed(path), path)


if __name__ == "__main__":
    unittest.main()
