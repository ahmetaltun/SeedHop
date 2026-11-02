"""Release failure gates; no Apple account, network or signing identity required."""
import argparse
import base64
import contextlib
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build
import release
import sparkle


class ReleaseTests(unittest.TestCase):
    def test_versions_and_build_numbers(self):
        release.validate_tag("v0.1.0", "0.1.0")
        for tag in ("0.1.0", "v1.0.0", "v0.1.0-beta", "../../master"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_tag(tag, "0.1.0")
        for number in ("0", "-1", "1.2", "01", "a", "1" * 19):
            with self.subTest(number=number), self.assertRaises(argparse.ArgumentTypeError):
                build.build_number(number)
        self.assertEqual(build.build_number("123"), "123")
        self.assertRegex(build.workspace_version(), r"^\d+\.\d+\.\d+$")

    def secrets(self):
        values = {name: "fixture" for name in release.SECRETS}
        values.update(APPLE_CERTIFICATE=base64.b64encode(b"certificate").decode(),
                      APPLE_TEAM_ID="ABCDEFGHIJ", APPLE_SIGNING_IDENTITY="Developer ID Application: Test (ABCDEFGHIJ)",
                      SPARKLE_PRIVATE_KEY=base64.b64encode(bytes(32)).decode())
        return values

    def test_missing_secrets_and_invalid_identity_fail_before_import(self):
        for key in release.SECRETS:
            values = self.secrets(); values.pop(key)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, key):
                release.validate_secrets(values)
        for identity in ("-", "Apple Development: Test (ABCDEFGHIJ)", "Developer ID Application: Test (OTHERTEAM1)"):
            values = self.secrets(); values["APPLE_SIGNING_IDENTITY"] = identity
            with self.subTest(identity=identity), self.assertRaisesRegex(ValueError, "Developer ID"):
                release.validate_secrets(values)

    def test_invalid_certificate_and_sparkle_key_rejected(self):
        for key in ("APPLE_CERTIFICATE", "SPARKLE_PRIVATE_KEY"):
            values = self.secrets(); values[key] = "not base64!"
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "base64"):
                release.validate_secrets(values)
        with patch.object(release, "run", return_value=bytes(44)):
            with self.assertRaisesRegex(ValueError, "does not match"):
                release.validate_secrets(self.secrets())

    def test_release_rejects_adhoc_signing(self):
        with patch("sys.argv", ["build.py", "--release", "--identity", "-"]), \
             patch.object(build.platform, "system", return_value="Darwin"), \
             patch.object(build.platform, "machine", return_value="arm64"), \
             contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            build.main()

    def test_notarization_rejection_retains_log_and_stops(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(release, "run", side_effect=[b'{"id":"submission"}', b'{"status":"Invalid","id":"submission"}', b'{"issues":[]}']) as run:
            root = pathlib.Path(directory)
            with self.assertRaisesRegex(ValueError, "not accepted"):
                release.notarize(root / "SeedHop.zip", self.secrets(), root)
            self.assertTrue((root / "SeedHop-notary-log.json").exists())
            self.assertTrue(all(call.kwargs["sensitive"] for call in run.call_args_list))

    def test_notarization_acceptance(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(release, "run", side_effect=[b'{"id":"submission"}', b'{"status":"Accepted","id":"submission"}', b'{"issues":[]}']):
            root = pathlib.Path(directory)
            self.assertEqual(release.notarize(root / "SeedHop.zip", self.secrets(), root), "submission")
            self.assertTrue((root / "SeedHop-notary-log.json").exists())

    def test_release_stages_reject_wrong_build_and_modified_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); app = root / "SeedHop.app"; app.mkdir()
            binary = app / "binary"; binary.write_bytes(b"signed fixture")
            path = root / "state.json"
            release.save_state(path, dict(commit="abc", buildNumber="2"), app, "build")
            self.assertEqual(release.load_state(path, app, "dmg", "abc", "2")["phase"], "build")
            for phase, commit, number in [("app", "abc", "2"), ("dmg", "other", "2"), ("dmg", "abc", "3")]:
                with self.subTest(phase=phase, commit=commit, number=number), self.assertRaisesRegex(ValueError, "mismatch"):
                    release.load_state(path, app, phase, commit, number)
            binary.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "changed"):
                release.load_state(path, app, "dmg", "abc", "2")

    def test_sensitive_commands_cannot_stream_and_timeouts_redact_credentials(self):
        with self.assertRaisesRegex(ValueError, "cannot stream"):
            release.run("security", sensitive=True, stream=True)
        with patch.object(release.subprocess, "run", side_effect=subprocess.TimeoutExpired(["secret-fixture"], 1)):
            with self.assertRaisesRegex(RuntimeError, "timed out") as caught:
                release.run("security", "secret-fixture", sensitive=True, timeout=1)
        self.assertNotIn("secret-fixture", str(caught.exception))

    def test_command_heartbeat_captures_output_and_enforces_deadline(self):
        self.assertEqual(release.run(sys.executable, "-c", "print('done')", heartbeat="Fixture"), b"done\n")
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaisesRegex(RuntimeError, "timed out"):
            release.run(sys.executable, "-c", "import time; time.sleep(10)",
                        timeout=0.05, heartbeat="Fixture", sensitive=True)

    def test_app_stapling_reuses_dmg_submission_without_uploading_again(self):
        state = dict(dmgSubmission="one-apple-submission")
        with tempfile.TemporaryDirectory() as directory, \
             patch.dict(os.environ, self.secrets()), \
             patch.object(release, "preflight", return_value=("0.1.0", "abc")), \
             patch.object(release, "validate_secrets", return_value=b"certificate"), \
             patch.object(release, "load_state", return_value=state), \
             patch.object(release, "save_state"), patch.object(release, "verify_app"), \
             patch.object(release, "run", return_value=b"") as run, \
             patch.object(release, "notarize") as notarize:
            release.package("v0.1.0", "2", pathlib.Path(directory), "app")
        notarize.assert_not_called()
        self.assertEqual(state["appSubmission"], "one-apple-submission")
        self.assertIn(("xcrun", "stapler", "staple", release.ROOT / ".build/SeedHop.app"),
                      [call.args for call in run.call_args_list])

    def test_stapling_retries_only_transient_apple_service_errors(self):
        failure = release.ToolError("Ticket service unavailable", 68)
        with patch.object(release, "run", side_effect=[failure, b""]) as run, \
             patch.object(release.time, "sleep") as sleep:
            release.staple(pathlib.Path("SeedHop.app"))
            self.assertEqual(run.call_count, 2)
            sleep.assert_called_once_with(15)
        for code, attempts in [(65, 1), (68, 3)]:
            with self.subTest(code=code), \
                 patch.object(release, "run", side_effect=release.ToolError("Controlled failure", code)) as run, \
                 patch.object(release.time, "sleep"), self.assertRaises(release.ToolError):
                release.staple(pathlib.Path("SeedHop.app"))
            self.assertEqual(run.call_count, attempts)

    def test_notarization_timeout_preserves_submission_id(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(release, "run", side_effect=[b'{"id":"submission"}', RuntimeError("Timed out")]):
            root = pathlib.Path(directory)
            with self.assertRaisesRegex(RuntimeError, "Timed out"):
                release.notarize(root / "SeedHop.zip", self.secrets(), root)
            self.assertEqual(json.loads((root / "SeedHop-notary.json").read_text())["id"], "submission")

    def test_keychain_cleanup_restores_search_list_after_import_failure(self):
        calls = []
        def command(*args, **kwargs):
            calls.append(args)
            if args[1:3] == ("list-keychains", "-d") and "-s" not in args:
                return b'"/Users/fixture/Library/Keychains/login.keychain-db"\n'
            if args[1] == "create-keychain": pathlib.Path(args[-1]).touch()
            if args[1] == "import": raise RuntimeError("Controlled import failure")
            return b""
        with tempfile.TemporaryDirectory() as directory, patch.object(release, "run", side_effect=command):
            with self.assertRaisesRegex(RuntimeError, "import failure"):
                with release.signing_keychain(pathlib.Path(directory), b"certificate", self.secrets()):
                    self.fail("Import failure entered signing context")
        self.assertIn(("security", "list-keychains", "-d", "user", "-s", "/Users/fixture/Library/Keychains/login.keychain-db"), calls)
        self.assertEqual(calls[-1][1], "delete-keychain")

    def test_credential_tool_errors_do_not_expose_raw_output(self):
        result = subprocess.CompletedProcess(["security"], 1, b"private output", b"password-fixture")
        with patch.object(release.subprocess, "run", return_value=result):
            with self.assertRaises(RuntimeError) as caught:
                release.run("security", "import", "fixture", sensitive=True)
        self.assertNotIn("password-fixture", str(caught.exception))
        self.assertNotIn("private output", str(caught.exception))

    def test_published_release_and_nonincreasing_build_rejected(self):
        commit = b"abc\n"
        published = dict(tag_name="v0.1.0", draft=False, prerelease=False, assets=[])
        with patch.object(build, "workspace_version", return_value="0.1.0"), \
             patch.object(release, "run", side_effect=[commit, commit, b"", json.dumps([[published]]).encode()]):
            with self.assertRaisesRegex(ValueError, "published release"):
                release.preflight("v0.1.0", "2")
        older = dict(tag_name="v0.0.1", draft=False, prerelease=False,
                     assets=[dict(name="release-metadata.json", browser_download_url="https://example.com/metadata")])
        with patch.object(build, "workspace_version", return_value="0.1.0"), \
             patch.object(release, "run", side_effect=[commit, commit, b"", json.dumps([[older]]).encode(), b'{"buildNumber":"10"}']):
            with self.assertRaisesRegex(ValueError, "Build number"):
                release.preflight("v0.1.0", "9")

    def test_sparkle_checksum_failure_prevents_extraction(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(sparkle, "ROOT", pathlib.Path(directory)):
            archive = pathlib.Path(directory) / ".cache/sparkle" / sparkle.VERSION / "distribution.tar.xz"
            archive.parent.mkdir(parents=True); archive.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "checksum"):
                sparkle.distribution()

    def test_appcast_size_version_signature_and_url(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); archive = root / "SeedHop-0.1.0-arm64.zip"; archive.write_bytes(b"archive")
            xml = '<rss xmlns:sparkle="' + release.SPARKLE_NS + '"><channel><item><sparkle:version>2</sparkle:version><sparkle:shortVersionString>0.1.0</sparkle:shortVersionString><sparkle:minimumSystemVersion>15.0</sparkle:minimumSystemVersion><enclosure url="https://github.com/ahmetaltun/SeedHop/releases/download/v0.1.0/SeedHop-0.1.0-arm64.zip" length="7" sparkle:edSignature="signed"/></item></channel></rss>'
            path = root / "appcast.xml"; path.write_text(xml)
            self.assertEqual(release.validate_appcast(path, archive, "v0.1.0", "2"), "signed")
            for source, replacement in [('length="7"', 'length="8"'), ('download/v0.1.0', 'download/v1.0.0'),
                                        ('>2<', '>1<'), ('sparkle:edSignature="signed"', '')]:
                path.write_text(xml.replace(source, replacement))
                with self.subTest(source=source), self.assertRaises(ValueError):
                    release.validate_appcast(path, archive, "v0.1.0", "2")


if __name__ == "__main__":
    unittest.main()
