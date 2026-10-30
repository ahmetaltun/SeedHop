#!/usr/bin/env python3
"""Offline catalog validation and development pseudolocalization for the custom build."""
import argparse
import collections
import hashlib
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
CATALOGS = ROOT / "macOS/Resources/Localization"
LANGUAGES = ["en", "de", "es", "fr", "it", "pt-BR", "tr", "ru", "uk", "pl", "ja", "ko", "zh-Hans"]
PLURALS = {language: {"one", "other"} for language in LANGUAGES}
PLURALS.update({language: {"one", "few", "many", "other"} for language in ["ru", "uk", "pl"]})
PLURALS.update({language: {"one", "many", "other"} for language in ["es", "fr", "it", "pt-BR"]})
PLURALS.update({language: {"other"} for language in ["ja", "ko", "zh-Hans"]})
TOKEN = re.compile(r"%%|%(?:(\d+)\$)?(@|lld|llu|ld|lu|d|u|f)")
VERBATIM = {"SeedHop", "0", "09:00", "17:00", "0.1.0 · Milestone 0", "magnet:?xt=urn:btih:…", "KB/s", "TCP and uTP (UDP)", "BitTorrent v1 + v2"}
COUNT_KEYS = {"%lld files · %@", "%lld matching files", "%lld need attention", "%lld transfers", "%lld transfers selected",
              "%u peers connected; this snapshot shows no download activity.", "Remove %lld transfers?",
              "These %lld transfers will be removed. Their files stay on your Mac unless you choose to delete them."}
COUNT_KEYS.update("%lld " + state for state in ["checking", "connected", "downloading", "paused", "queued", "scheduled", "seeding", "waiting"])


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


def signature(value):
    """Track argument positions as well as types, so reordering must be explicit."""
    result = []; index = 0
    for token in TOKEN.finditer(value):
        if token.group() == "%%": continue
        index += 1
        result.append((int(token[1]) if token[1] else index, token[2]))
    return collections.Counter(result)


def units(node):
    if "stringUnit" in node: yield node["stringUnit"]
    for value in node.values():
        if isinstance(value, dict): yield from units(value)


def english_source(entry, key):
    def text_only(node):
        if isinstance(node, dict): return {name: text_only(value) for name, value in node.items() if name != "state"}
        return node
    return text_only(entry.get("localizations", {}).get("en", key))


def entry_digest(entry, key):
    return hashlib.sha256(json.dumps(english_source(entry, key), sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def source_digest():
    source = {}
    for path in sorted(CATALOGS.glob("*.xcstrings")):
        source[path.name] = {key: english_source(entry, key)
                            for key, entry in read(path)["strings"].items() if entry.get("shouldTranslate", True)}
    return hashlib.sha256(json.dumps(source, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def normalize(extraction):
    locations = collections.defaultdict(set)
    for path in extraction.glob("*.stringsdata"):
        data = read(path)
        if "SelfTest" in data["source"] or "PreviewFixtures" in data["source"]: continue
        for entry in data.get("tables", {}).get("Localizable", []):
            locations[entry["key"]].add(pathlib.Path(data["source"]).name)
    path = CATALOGS / "Localizable.xcstrings"
    catalog = read(path)
    provenance = CATALOGS / "translation-status.json"
    previous = read(provenance).get("entries", {}).get(path.name, {}) if provenance.exists() else {}
    for key in list(catalog["strings"]):
        entry = catalog["strings"][key]
        if entry.get("extractionState") == "stale":
            del catalog["strings"][key]; continue
        if key in VERBATIM or not re.search(r"[A-Za-z]", TOKEN.sub("", key)):
            entry["shouldTranslate"] = False
        entry.setdefault("comment", "")
        if not entry["comment"]:
            files = ", ".join(sorted(locations.get(key, {"Native UI"})))
            entry["comment"] = f"SeedHop macOS UI ({files})."
            if signature(key):
                entry["comment"] += " Preserve placeholder types and positions; use positional placeholders when reordering. Text arguments may be verbatim user names, paths, or formatted quantities."
            if "Delete" in key:
                entry["comment"] += " Destructive action: deletes downloaded files, not just the library entry."
        entry.setdefault("localizations", {}).setdefault("en", {"stringUnit": {"state": "translated", "value": key}})
        if key in previous and previous[key] != entry_digest(entry, key):
            for language, node in entry["localizations"].items():
                if language != "en":
                    for unit in units(node): unit["state"] = "needs_review"
    write(path, catalog)
    # Metadata uses stable keys rather than English sentences, so wording changes need explicit revision flags too.
    info = CATALOGS / "InfoPlist.xcstrings"
    if info.exists():
        metadata = read(info)
        before = read(provenance).get("entries", {}).get(info.name, {}) if provenance.exists() else {}
        for key, entry in metadata["strings"].items():
            if key in before and before[key] != entry_digest(entry, key):
                for language, node in entry.get("localizations", {}).items():
                    if language != "en":
                        for unit in units(node): unit["state"] = "needs_review"
        write(info, metadata)


def check(bundle=None, extraction=None):
    errors = []; required = 0
    for path in sorted(CATALOGS.glob("*.xcstrings")):
        catalog = read(path)
        if catalog.get("sourceLanguage") != "en": errors.append(f"{path.name}: source language must be en")
        for key, entry in catalog["strings"].items():
            label = f"{path.name}: {key}"
            if entry.get("extractionState") == "stale": errors.append(f"{label}: stale source")
            if not entry.get("shouldTranslate", True): continue
            required += 1
            if not entry.get("comment"): errors.append(f"{label}: missing translator context")
            localizations = entry.get("localizations", {})
            if set(localizations) != set(LANGUAGES): errors.append(f"{label}: language coverage differs from supported languages")
            source_values = list(units(localizations.get("en", {})))
            source = source_values[0]["value"] if source_values else key
            for language in LANGUAGES:
                node = localizations.get(language, {})
                variants = node.get("variations", {}).get("plural")
                if key in COUNT_KEYS and variants is None:
                    errors.append(f"{label} [{language}]: count requires native plural forms")
                if variants is not None and set(variants) != PLURALS[language]:
                    errors.append(f"{label} [{language}]: incorrect plural categories")
                values = list(units(node))
                if not values: errors.append(f"{label} [{language}]: no translation")
                for unit in values:
                    value = unit.get("value", "")
                    if unit.get("state") != "translated" or not value: errors.append(f"{label} [{language}]: translation needs review")
                    if signature(source) != signature(value): errors.append(f"{label} [{language}]: placeholder mismatch")
                    if source.count("\n") != value.count("\n"): errors.append(f"{label} [{language}]: newline mismatch")
                    for term in ["SeedHop", "BitTorrent", "**", "`", "⌘", "⌥"]:
                        if source.count(term) != value.count(term): errors.append(f"{label} [{language}]: changed protected token {term}")
        if bundle:
            for language in LANGUAGES:
                folder = bundle / "Contents/Resources" / f"{language}.lproj"
                if not any(folder.glob(path.stem + ".strings*")):
                    errors.append(f"{folder}: missing compiled {path.stem}")
    if extraction:
        catalog_keys = set(read(CATALOGS / "Localizable.xcstrings")["strings"])
        for path in extraction.glob("*.stringsdata"):
            data = read(path)
            if "SelfTest" in data["source"] or "PreviewFixtures" in data["source"]: continue
            for entry in data.get("tables", {}).get("Localizable", []):
                if entry["key"] not in catalog_keys: errors.append(f"{data['source']}: missing catalog key {entry['key']}")
    provenance = CATALOGS / "translation-status.json"
    if not provenance.exists() or read(provenance).get("sourceDigest") != source_digest():
        errors.append("English source changed: update translations and translation provenance")
    if errors: raise ValueError("\n".join(errors))
    print(f"Localization: {required} required entries, {len(LANGUAGES)} languages; coverage and formats passed.")


def pseudolocalize(output):
    output.mkdir(parents=True, exist_ok=True)
    accents = str.maketrans("aeiouAEIOU", "áëïôüÁËÏÔÜ")
    for path in sorted(CATALOGS.glob("*.xcstrings")):
        catalog = read(path)
        for key, entry in catalog["strings"].items():
            english = entry.get("localizations", {}).get("en", {"stringUnit": {"state": "translated", "value": key}})
            node = json.loads(json.dumps(english))
            if entry.get("shouldTranslate", True):
                for unit in units(node):
                    value = unit["value"]; parts = []; last = 0
                    for match in TOKEN.finditer(value):
                        parts += [value[last:match.start()].translate(accents), match.group()]; last = match.end()
                    parts.append(value[last:].translate(accents))
                    unit["value"] = "[!! " + "".join(parts) + " ~" * max(2, len(value) // 7) + " !!]"
            entry["localizations"] = {"en-XA": node}
        generated = output / path.name
        write(generated, catalog)
        subprocess.run(["xcrun", "xcstringstool", "compile", str(generated), "--language", "en-XA", "--output-directory", str(output)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "normalize", "record-ai-provenance", "pseudolocalize"])
    parser.add_argument("--bundle", type=pathlib.Path)
    parser.add_argument("--extraction", type=pathlib.Path, default=ROOT / ".build/localization-extraction")
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / ".build/pseudolocalization")
    args = parser.parse_args()
    if args.command == "normalize": normalize(args.extraction)
    elif args.command == "record-ai-provenance":
        write(CATALOGS / "translation-status.json", {"sourceDigest": source_digest(), "sourceLanguage": "en", "languages": {
            language: {"origin": "AI draft", "humanReviewed": False} for language in LANGUAGES if language != "en"}, "entries": {
                path.name: {key: entry_digest(entry, key) for key, entry in read(path)["strings"].items() if entry.get("shouldTranslate", True)}
                for path in sorted(CATALOGS.glob("*.xcstrings"))}})
    elif args.command == "pseudolocalize": pseudolocalize(args.output)
    else: check(args.bundle, args.extraction)


if __name__ == "__main__":
    try: main()
    except (ValueError, KeyError, json.JSONDecodeError) as error:
        raise SystemExit(str(error))
