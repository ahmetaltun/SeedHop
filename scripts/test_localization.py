"""Focused validation of the release guard, independent of macOS UI automation."""
import copy
import json
import pathlib
import tempfile
import unittest
import subprocess
import sys
from unittest.mock import patch
import localization


class LocalizationChecks(unittest.TestCase):
    def test_reordered_placeholders_preserve_argument_identity(self):
        self.assertEqual(localization.signature("%lld files · %@"), localization.signature("%2$@ · %1$lld files"))
        self.assertNotEqual(localization.signature("%lld files · %@"), localization.signature("%@ · %lld files"))
        self.assertEqual(localization.signature("%lld%%"), localization.signature("%lld%%"))

    def test_missing_translation_and_bad_plural_fail_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            entry = {"comment": "File count.", "localizations": {
                language: {"variations": {"plural": {category: {"stringUnit": {"state": "translated", "value": "%lld files"}}
                                                     for category in localization.PLURALS[language]}}}
                for language in localization.LANGUAGES}}
            catalog = {"sourceLanguage": "en", "strings": {"%lld files": entry}, "version": "1.0"}
            with patch.object(localization, "CATALOGS", root):
                localization.write(root / "Localizable.xcstrings", catalog)
                localization.write(root / "translation-status.json", {"sourceDigest": localization.source_digest()})
                localization.check()
                broken = copy.deepcopy(catalog)
                del broken["strings"]["%lld files"]["localizations"]["ru"]["variations"]["plural"]["few"]
                localization.write(root / "Localizable.xcstrings", broken)
                with self.assertRaisesRegex(ValueError, "incorrect plural categories"): localization.check()
                broken = copy.deepcopy(catalog)
                del broken["strings"]["%lld files"]["localizations"]["ja"]
                localization.write(root / "Localizable.xcstrings", broken)
                with self.assertRaisesRegex(ValueError, "language coverage"): localization.check()
                broken = copy.deepcopy(catalog)
                broken["strings"]["%lld files"]["localizations"]["de"]["variations"]["plural"]["one"]["stringUnit"]["value"] = "%@ Datei"
                localization.write(root / "Localizable.xcstrings", broken)
                with self.assertRaisesRegex(ValueError, "placeholder mismatch"): localization.check()

    def test_english_wording_change_marks_existing_translations_for_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            entry = {"comment": "Native guidance.", "localizations": {language: {"stringUnit": {"state": "translated", "value": "Old text"}}
                                                                       for language in localization.LANGUAGES}}
            catalog = {"sourceLanguage": "en", "strings": {"Stable metadata key": entry}, "version": "1.0"}
            with patch.object(localization, "CATALOGS", root):
                localization.write(root / "Localizable.xcstrings", catalog)
                localization.write(root / "translation-status.json", {"entries": {"Localizable.xcstrings": {
                    "Stable metadata key": localization.entry_digest(entry, "Stable metadata key")}}})
                entry["localizations"]["en"]["stringUnit"]["value"] = "Revised text"
                localization.write(root / "Localizable.xcstrings", catalog)
                localization.normalize(root / "empty-extraction")
                updated = localization.read(root / "Localizable.xcstrings")["strings"]["Stable metadata key"]
                self.assertEqual(updated["localizations"]["de"]["stringUnit"]["state"], "needs_review")
                self.assertEqual(updated["localizations"]["en"]["stringUnit"]["state"], "translated")

    def test_release_cannot_bypass_guards_with_development_flags(self):
        for flag in ["--update-localizations", "--pseudolocalization"]:
            result = subprocess.run([sys.executable, str(localization.ROOT / "scripts/build.py"), "--release", flag], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


if __name__ == "__main__": unittest.main()
