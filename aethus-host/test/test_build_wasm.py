"""Missing translation inputs must not replace a previous archive. Run with -I."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HOST = Path(__file__).resolve().parents[1]


@unittest.skipIf(os.name == "nt", "PS5 build scripts run on Linux")
class BuildInputTests(unittest.TestCase):
    def reject_bad_input(self, empty):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            code, runtime, work = (root / name for name in ("code", "runtime", "work"))
            for directory in (code, runtime, work, root / "re/ps5/tools", root / "vulkan/tooling"):
                directory.mkdir(parents=True)
            (root / "re/ps5/tools/cc.sh").write_text("exit 99\n")
            (root / "vulkan/tooling/prospero-clang18").write_text("exit 99\n")
            (code / "wenilla.h").write_text("fixture\n")
            for name in ("wasm-rt.h", "runtime.inc", "wasm-rt-impl.c", "wasm-rt-mem-impl.c", "wasm-rt-exceptions-impl.c"):
                (runtime / name).write_text("fixture\n")
            for index in range(128):
                if index == 127 and not empty:
                    continue
                (code / f"wenilla_{index}.c").write_text("" if index == 127 else "fixture\n")
            archive = work / "libwenilla.a"
            previous = b"previous verified archive\n"
            archive.write_bytes(previous)
            result = subprocess.run(
                ["bash", str(HOST / "ps5/build-wasm.sh"), str(code), str(runtime), str(work), "1"],
                env=dict(os.environ, AZEROTH_RE=str(root / "re"), PS5_VULKAN=str(root / "vulkan"),
                         PS5_PAYLOAD_SDK=str(root / "sdk")), capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("wenilla_127.c", result.stderr)
            self.assertEqual(archive.read_bytes(), previous)
            self.assertFalse(list((work / "obj-wasm").iterdir()))

    def test_missing_unit_preserves_previous_archive(self):
        self.reject_bad_input(empty=False)

    def test_empty_unit_preserves_previous_archive(self):
        self.reject_bad_input(empty=True)


if __name__ == "__main__":
    unittest.main()
