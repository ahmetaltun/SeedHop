#!/usr/bin/env python3
"""Check the fixed step-7 budgets against controlled debug fixture measurements."""
import argparse
import json
import math
from pathlib import Path


def evaluate(core, ui):
    expected = {(100, 1), (1000, 1), (1, 10000)}
    for result in (core, ui):
        if not result.get("fixture") or result.get("profile") != "debug":
            raise ValueError("Expected controlled debug fixture measurements")
        if {(case["transfers"], case["filesPerTransfer"]) for case in result["cases"]} != expected:
            raise ValueError("Missing or unexpected fixture shapes")
    if ui.get("profilingOnly"):
        raise ValueError("A diagnostic profiling run is not a standard budget run")
    checks = []

    def latency(name, value, limit):
        checks.append({"name": name, "value": value, "limit": limit,
                       "passed": math.isfinite(value) and 0 <= value <= limit})

    for case in core["cases"]:
        name = f"core {case['transfers']}/{case['filesPerTransfer']}"
        limit = 25 if case["filesPerTransfer"] > 1 else 20 if case["transfers"] == 1000 else 5
        latency(name + " snapshot p95 ms", case["snapshot"]["p95Ms"], limit)
        latency(name + " encode p95 ms", case["encode"]["p95Ms"], 100)
    for case in ui["cases"]:
        count = case["transfers"]
        name = f"UI {count}/{case['filesPerTransfer']}"
        limits = [("jsonDecode", 50), ("jsonEncode", 50), ("modelReads", 25),
                  ("unchangedSnapshotLayout", 20),
                  ("changingSnapshotLayout", 100 if count == 100 else 125),
                  ("programmaticScrollLayout", 50)]
        for field, limit in limits:
            latency(name + " " + field + " p95 ms", case[field]["p95Ms"], limit)
        if count > 1:
            latency(name + " filter p95 ms", case["filterUntilNativeRows"]["p95Ms"],
                    100 if count == 100 else 125)
        latency(name + " unchanged publications", case["unchangedPublicationsPerSnapshot"], 0)
        flags = ["selectionStableDuringUpdates", "scrollAnchorStableDuringUpdates"]
        if count > 1:
            flags.append("filterClearsHiddenSelection")
        for flag in flags:
            checks.append({"name": name + " " + flag, "passed": case[flag] is True})
    return {"budgetDefinition": "docs/PERFORMANCE.md", "checks": checks,
            "passed": all(check["passed"] for check in checks)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--ui", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = evaluate(json.loads(args.core.read_text()), json.loads(args.ui.read_text()))
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    for check in result["checks"]:
        if not check["passed"]:
            print("FAIL:", check)
    print(f"{'PASS' if result['passed'] else 'FAIL'}: {len(result['checks'])} applicable budget/stability checks")
    raise SystemExit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
