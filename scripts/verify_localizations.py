#!/usr/bin/env python3
"""Exercise actual bundled language selection in isolated development launches."""
import argparse
import json
import pathlib
import subprocess
from localization import CATALOGS, LANGUAGES, ROOT, check


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=pathlib.Path, default=ROOT / ".build/SeedHop.app")
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / ".build/localization-results.json")
    parser.add_argument("--render", action="store_true", help="Render 30 light/dark fixtures for each language")
    args = parser.parse_args()
    check(bundle=args.bundle)
    catalog = json.loads((CATALOGS / "Localizable.xcstrings").read_text())
    cases = [(language, language, "en_US") for language in LANGUAGES]
    cases += [("unsupported", "ar,he", "en_US"), ("preferred-order", "fr,de", "en_US"), ("regional", "fr", "de_DE")]
    results = []
    for name, preferred, region in cases:
        process = subprocess.run([str(args.bundle / "Contents/MacOS/SeedHop"), "--verify-localization",
                                  "-AppleLanguages", f"({preferred})", "-AppleLocale", region],
                                 capture_output=True, text=True, timeout=45)
        if process.returncode:
            raise SystemExit(f"{name}: {process.stderr or process.stdout}")
        report = json.loads(process.stdout)
        expected = "en" if name == "unsupported" else "fr" if name in {"preferred-order", "regional"} else name
        assert report["language"] == expected, (name, report)
        expected_remove = catalog["strings"]["Remove"]["localizations"][expected]["stringUnit"]["value"]
        assert report["remove"] == expected_remove, (name, "Wrong bundled translation", report)
        assert report["englishFallback"] == "en", (name, report)
        assert set(report["supported"]) == set(LANGUAGES), (name, report)
        for state, labels in report["statusSamples"].items():
            plural = catalog["strings"]["%lld " + state]["localizations"][expected]["variations"]["plural"]
            for count, label in zip([1,2,5,11,21], labels):
                if expected in {"ja","ko","zh-Hans"}: category = "other"
                elif expected in {"ru","uk"}:
                    category = "one" if count % 10 == 1 and count % 100 != 11 else "few" if count % 10 in {2,3,4} and count % 100 not in {12,13,14} else "many"
                elif expected == "pl": category = "one" if count == 1 else "few" if count % 10 in {2,3,4} and count % 100 not in {12,13,14} else "many"
                else: category = "one" if count == 1 else "other"
                value = plural[category]["stringUnit"]["value"]
                assert label == value.replace("%lld",str(count)).replace("%1$lld",str(count)), (name,state,count,label,value)
        if name == "fr": assert report["regionalNumber"] == "1,234.5", (name, "US regional formatting lost", report)
        if name == "regional": assert report["regionalNumber"] == "1.234,5", (name, "German regional formatting lost", report)
        report["case"] = name
        if args.render and name in LANGUAGES:
            directory = args.output.parent / "localization-renders" / name
            render = subprocess.run([str(args.bundle / "Contents/MacOS/SeedHop"), "--render-localizations",
                                     "-AppleLanguages", f"({preferred})", "-AppleLocale", region, "--output", str(directory)],
                                    capture_output=True, text=True, timeout=90)
            if render.returncode: raise SystemExit(f"{name} render: {render.stderr or render.stdout}")
            count = len(list(directory.glob("*.png")))
            assert count == 30, (name, "Missing localization renders", count)
            report["renders"] = count
        results.append(report)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"cases": results, "manualInputAndVoiceOver": "not tested"}, ensure_ascii=False, indent=2) + "\n")
    print(f"Bundled localization: {len(results)} language/region launches passed; {sum(x['checks'] for x in results)} assertions.")


if __name__ == "__main__": main()
