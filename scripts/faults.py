#!/usr/bin/env python3
"""Run controlled recovery tests and real ENOSPC on a disposable 32 MiB image."""

import datetime
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]


def main():
    if platform.system() != "Darwin":
        raise SystemExit("The disk-image fault runner requires macOS and hdiutil.")
    build = ROOT / ".build"
    build.mkdir(exist_ok=True)
    run = Path(tempfile.mkdtemp(prefix="faults-", dir=build))
    report = {
        "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "volume_size_mib": 32,
        "checks": [],
    }

    def execute(name, command, env=None, timeout=180):
        start = time.monotonic()
        result = subprocess.run(
            command, cwd=ROOT, env=env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout,
        )
        print(result.stdout, end="", flush=True)
        log = run / f"{name}.log"
        log.write_text(result.stdout)
        report["checks"].append({
            "name": name, "command": command, "exit_code": result.returncode,
            "seconds": round(time.monotonic() - start, 3),
            "log": str(log.relative_to(ROOT)),
        })
        result.check_returncode()

    image = run / "faults.dmg"
    mount = run / "volume"
    mounted = False
    try:
        execute("controlled-recovery", [
            "cargo", "test", "-p", "seedhop-core", "--test", "faults", "--locked",
            "--", "--nocapture",
        ])
        execute("create-image", [
            "hdiutil", "create", "-size", "32m", "-fs", "HFS+",
            "-volname", "SeedHopFaults", str(image),
        ])
        mount.mkdir()
        execute("attach-image", [
            "hdiutil", "attach", "-nobrowse", "-mountpoint", str(mount), str(image),
        ])
        mounted = True
        execute("real-disk-full", [
            "cargo", "test", "-p", "seedhop-core", "--test", "faults", "--locked",
            "real_disk_full_payload_and_state_writes_recover", "--", "--ignored", "--exact", "--nocapture",
        ], env={**os.environ, "SEEDHOP_FAULT_VOLUME": str(mount)})
        execute("real-disk-full-retry", [
            "cargo", "test", "-p", "seedhop-core", "--test", "status", "--locked",
            "real_disk_full_retry_preserves_intent_queue_choices_and_destination", "--", "--ignored", "--exact", "--nocapture",
        ], env={**os.environ, "SEEDHOP_FAULT_VOLUME": str(mount)})
        report["passed"] = True
    finally:
        if mounted:
            try:
                execute("detach-image", ["hdiutil", "detach", str(mount)], timeout=30)
                mounted = False
            except (subprocess.SubprocessError, OSError) as error:
                report["cleanup_error"] = str(error)
                report["passed"] = False
                print(f"Test image remains mounted at {mount}; detach it with hdiutil.", flush=True)
        if not mounted:
            image.unlink(missing_ok=True)
        report.setdefault("passed", False)
        report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        output = run / "result.json"
        output.write_text(json.dumps(report, indent=2) + "\n")
        (build / "faults-result.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"Recovery evidence: {output}", flush=True)
    if not report["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
