#!/usr/bin/env python3
"""Export the locked dependency inventory without local paths or machine identifiers."""
import argparse
import hashlib
import json
import pathlib
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent


def inventory(root=ROOT):
    lock_bytes = (root / "Cargo.lock").read_bytes()
    lock = tomllib.loads(lock_bytes.decode())
    packages = [{key: package[key] for key in ("name", "version", "source", "checksum") if key in package}
                for package in lock["package"]]
    return {"schemaVersion": 1, "scope": "Cargo.lock (includes build, development and platform dependencies)",
            "lockSha256": hashlib.sha256(lock_bytes).hexdigest(),
            "packages": sorted(packages, key=lambda p: (p["name"], p["version"]))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(inventory(), indent=2) + "\n")
    print(f"Dependency inventory: {args.output}")


if __name__ == "__main__":
    main()
