"""Regression checks for Apple Silicon-only app packaging."""
import contextlib
import io
import pathlib
import tempfile
import unittest
from unittest.mock import patch

import build


class ArchitectureTests(unittest.TestCase):
    def test_debug_and_release_pin_rust_target_and_output(self):
        for flags in ([], ["--release", "--sandbox", "--sandbox-team-id", "ABCDEFGHIJ"]):
            with self.subTest(flags=flags), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                with patch("sys.argv", ["build.py", "--identity", "Developer ID Application: Test (ABCDEFGHIJ)" if flags else "-", *flags]), \
                     patch.object(build, "ROOT", root), \
                     patch.object(build.subprocess, "check_output", return_value="Developer ID Application: Test (ABCDEFGHIJ)"), \
                     patch.object(build.platform, "system", return_value="Darwin"), \
                     patch.object(build.platform, "machine", return_value="arm64"), \
                     patch.object(build, "run", side_effect=InterruptedError) as run:
                    with self.assertRaises(InterruptedError):
                        build.main()
                    command = run.call_args.args
                    self.assertEqual(command[command.index("--target") + 1], "aarch64-apple-darwin")
                    self.assertEqual(command[command.index("--target-dir") + 1], root / "target")
                    self.assertEqual("--release" in command, bool(flags))

    def test_skip_rust_rejects_wrong_library_before_packaging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with patch("sys.argv", ["build.py", "--identity", "-", "--skip-rust"]), \
                 patch.object(build, "ROOT", root), \
                 patch.object(build.platform, "system", return_value="Darwin"), \
                 patch.object(build.platform, "machine", return_value="arm64"), \
                 patch.object(build.subprocess, "check_output", return_value="x86_64\n") as inspect, \
                 patch.object(build, "run") as run:
                with self.assertRaisesRegex(RuntimeError, "must contain only arm64"):
                    build.main()
                self.assertEqual(inspect.call_args.args[0][-1],
                                 str(root / "target/aarch64-apple-darwin/debug/libseedhop_ffi.dylib"))
                self.assertFalse((root / ".build/SeedHop.app").exists())
                run.assert_not_called()

    def test_arm64_binary_is_accepted(self):
        with patch.object(build.subprocess, "check_output", return_value="arm64\n"):
            build.verify_arm64(pathlib.Path("engine.dylib"))

    def test_other_architectures_are_rejected(self):
        for architectures in ("x86_64\n", "x86_64 arm64\n", "arm64 x86_64\n", ""):
            with self.subTest(architectures=architectures):
                with patch.object(build.subprocess, "check_output", return_value=architectures):
                    with self.assertRaisesRegex(RuntimeError, "must contain only arm64"):
                        build.verify_arm64(pathlib.Path("engine.dylib"))

    def test_unsupported_hosts_fail_before_building(self):
        for system, machine in (("Darwin", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(system=system, machine=machine):
                with patch("sys.argv", ["build.py", "--identity", "-"]), \
                     patch.object(build.platform, "system", return_value=system), \
                     patch.object(build.platform, "machine", return_value=machine), \
                     patch.object(build, "run") as run, \
                     contextlib.redirect_stderr(io.StringIO()) as stderr:
                    with self.assertRaises(SystemExit) as error:
                        build.main()
                    self.assertEqual(error.exception.code, 2)
                    self.assertIn("require Apple Silicon macOS", stderr.getvalue())
                    run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
