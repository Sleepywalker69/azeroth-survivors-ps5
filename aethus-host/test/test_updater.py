"""Destructive-input and compatibility checks for the updater. Run with Python -I."""
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import Request

HOST = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("update_pc", HOST / "tools/update_pc.py")
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


class UpdaterTests(unittest.TestCase):
    def test_patch_plan_rejects_duplicates_and_combined_oversize(self):
        entry = {"name": "patch-2.MPQ", "size": 4 * 1024**3, "sha256": "a" * 64,
                 "parts": [{"url": "patch-2/part-000.bin", "size": 10}]}
        updater.check_patch_manifest({"patches": [entry]})
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            updater.check_patch_manifest({"patches": [entry, entry]})
        with self.assertRaisesRegex(ValueError, "Combined"):
            updater.check_patch_manifest({"patches": [entry, dict(entry, name="patch-3.MPQ")]})

    def test_explicit_native_converter_keeps_paths_as_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "native wasm2c"
            binary.write_bytes(b"fixture")
            with patch.dict(os.environ, {"WASM2C": str(binary)}):
                command = updater.wasm2c_command(root / "input with spaces.wasm", root / "output.c")
            self.assertEqual(command[0], binary.resolve())
            self.assertEqual(command[1], str(root / "input with spaces.wasm"))
            self.assertEqual(command[-1], str(root / "output.c"))

    @unittest.skipIf(os.name == "nt", "native Linux guard")
    def test_linux_rejects_windows_converter(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "wasm2c.exe"
            binary.write_bytes(b"fixture")
            with patch.dict(os.environ, {"WASM2C": str(binary)}), self.assertRaisesRegex(ValueError, "native wasm2c"):
                updater.wasm2c_command(Path("input.wasm"), Path("output.c"))

    def test_relative_paths_reject_escape_and_command_separators(self):
        for name in ("../x", "/x", "x/../../y", "x\\y", "https://evil/x", "x\r\nDELE /", "%2e%2e/x", "x//y", "x?y", "x#y"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                updater.safe_relative(name)
        self.assertEqual(updater.safe_relative("snippets/abc/inline0.js"), "snippets/abc/inline0.js")

    def test_redirect_cannot_reach_other_origin_or_plain_http(self):
        handler = updater.SameOriginRedirect()
        for target in ("http://wow.aethus.pl/x", "https://other.example/x", "http://127.0.0.1/"):
            with self.subTest(target=target), self.assertRaises(ValueError):
                handler.redirect_request(Request(updater.ORIGIN), None, 302, "Found", {}, target)

    def test_cached_game_passes_compatibility(self):
        with tempfile.TemporaryDirectory() as directory:
            updater.check_glue(HOST.parent / "aethus-web-0.96.0", Path(directory))

    def test_new_browser_import_body_stops_before_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            source = HOST.parent / "aethus-web-0.96.0"
            shutil.copytree(source / "snippets", web / "snippets")
            text = (source / "wenilla.js").read_text(encoding="utf-8")
            needle = "function __wbg_get_imports()"
            begin = text.index(needle)
            body = text.index("{", text.index(": function(", begin)) + 1
            text = text[:body] + "\n            navigator.unimplementedBrowserFunction();" + text[body:]
            (web / "wenilla.js").write_text(text, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Host review required"):
                updater.check_glue(web, root)
            self.assertTrue((root / "unreviewed-glue.json").is_file())

    def test_changed_string_helper_stops_before_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            source = HOST.parent / "aethus-web-0.96.0"
            shutil.copytree(source / "snippets", web / "snippets")
            text = (source / "wenilla.js").read_text(encoding="utf-8")
            text = text.replace("if (realloc === undefined) {", "if (realloc === null) {", 1)
            (web / "wenilla.js").write_text(text, encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unreviewed passStringToWasm0"):
                updater.check_glue(web, root)

    def test_changed_snippet_stops_before_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            source = HOST.parent / "aethus-web-0.96.0"
            shutil.copy2(source / "wenilla.js", web / "wenilla.js")
            shutil.copytree(source / "snippets", web / "snippets")
            snippet = next((web / "snippets").rglob("*.js"))
            snippet.write_text(snippet.read_text() + "\nnavigator.unimplementedBrowserFunction();")
            with self.assertRaisesRegex(ValueError, "Host review required"):
                updater.check_glue(web, root)

    def test_patch_parts_concatenate_decode_and_verify(self):
        payload = b"MPQ\x1a" + bytes(range(256)) * 300
        packed = gzip.compress(payload)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            parts = [root / "a", root / "b"]
            split = len(packed) // 2
            parts[0].write_bytes(packed[:split])
            parts[1].write_bytes(packed[split:])
            entry = {"name": "patch-6.MPQ", "size": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}
            updater.assemble_patch(parts, root / "result.MPQ", entry)
            self.assertEqual((root / "result.MPQ").read_bytes(), payload)
            entry["sha256"] = "0" * 64
            with self.assertRaises(ValueError):
                updater.assemble_patch(parts, root / "bad.MPQ", entry)
            self.assertFalse((root / "bad.MPQ").exists())

    def test_patch_expansion_cannot_exceed_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            packed = root / "bomb"
            packed.write_bytes(gzip.compress(b"MPQ\x1a" + b"x" * 100000))
            entry = {"name": "patch-6.MPQ", "size": 1024, "sha256": "0" * 64}
            with self.assertRaises(ValueError):
                updater.assemble_patch([packed], root / "bad.MPQ", entry)
            self.assertFalse((root / "bad.MPQ").exists())

    def test_manifest_cannot_name_arbitrary_files(self):
        entry = {"name": "patch-10.MPQ", "size": 10, "sha256": "a" * 64,
                 "parts": [{"url": "patch-10/part-000.bin", "size": 10}]}
        updater.check_patch(entry)
        entry["name"] = "../eboot.bin"
        with self.assertRaises(ValueError):
            updater.check_patch(entry)


if __name__ == "__main__":
    unittest.main(verbosity=2)
