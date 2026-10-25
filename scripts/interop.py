#!/usr/bin/env python3
"""Reproducible v1 interoperability using an isolated qBittorrent profile per case."""
import argparse
import base64
import contextlib
import datetime
import hashlib
import http.server
import json
import os
import pathlib
import select
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
QBIT = pathlib.Path("/Applications/qbittorrent.app/Contents/MacOS/qbittorrent")
LAB = ROOT / "target/debug/seedhop-lab"


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, list):
        return b"l" + b"".join(bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        return b"d" + b"".join(bencode(key) + bencode(value[key]) for key in sorted(value)) + b"e"
    raise TypeError(type(value))


def fixture(work, multifile, announce):
    # Metainfo is generated independently of SeedHop's Rust encoder.
    if multifile:
        files = [("first.bin", bytes(i % 251 for i in range(256 * 1024))),
                 ("docs/second.bin", bytes(i % 173 for i in range(96 * 1024))),
                 ("optional/third.bin", bytes(i % 139 for i in range(128 * 1024 + 147)))]
    else:
        files = [("seedhop-fixture.bin", bytes(i % 251 for i in range(1024 * 1024)))]
    content = b"".join(data for _, data in files)
    info = {b"name": b"payload" if multifile else files[0][0].encode(),
            b"piece length": 16384,
            b"pieces": b"".join(hashlib.sha1(content[at:at + 16384]).digest()
                                for at in range(0, len(content), 16384))}
    if multifile:
        info[b"files"] = [{b"length": len(data), b"path": [part.encode() for part in name.split("/")]}
                          for name, data in files]
    else:
        info[b"length"] = len(content)
    info_hash = hashlib.sha1(bencode(info)).hexdigest()
    torrent = work / "fixture.torrent"
    torrent.write_bytes(bencode({b"announce": announce.encode(), b"info": info}))
    seed_parent = work / "seed"
    seed_root = seed_parent / "payload" if multifile else seed_parent
    for name, data in files:
        path = seed_root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return torrent, info_hash, files, seed_parent, seed_root


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def two_ports():
    with socket.socket() as first, socket.socket() as second:
        first.bind(("127.0.0.1", 0))
        second.bind(("127.0.0.1", 0))
        return first.getsockname()[1], second.getsockname()[1]


class Tracker:
    def __init__(self, work):
        self.peer = 0
        self.info_hash = ""
        self.lock = threading.Lock()
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                query = urllib.parse.urlsplit(self.path)
                values = {urllib.parse.unquote_to_bytes(key): urllib.parse.unquote_to_bytes(value.replace("+", " "))
                          for pair in query.query.split("&") if "=" in pair
                          for key, value in [pair.split("=", 1)]}
                advertised = int(values.get(b"port", b"0"))
                valid = query.path == "/announce" and values.get(b"info_hash", b"").hex() == owner.info_hash
                endpoint = owner.peer if valid and advertised != owner.peer else 0
                peers = socket.inet_aton("127.0.0.1") + struct.pack("!H", endpoint) if endpoint else b""
                body = bencode({b"interval": 1, b"peers": peers}) if valid else bencode({b"failure reason": b"Unknown fixture"})
                with owner.lock, (work / "tracker.jsonl").open("a") as log:
                    log.write(json.dumps({"time": time.monotonic(), "port": advertised,
                                          "validInfoHash": valid, "returnedPort": endpoint,
                                          "userAgent": self.headers.get("User-Agent", "")}) + "\n")
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                with contextlib.suppress(BrokenPipeError, ConnectionResetError):
                    self.wfile.write(body)

            def log_message(self, *_):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/announce"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)


class QBittorrent:
    def __init__(self, work, executable):
        self.work = work
        self.web, self.peer = two_ports()
        self.process = None
        self.log = (work / "qbit.log").open("w+")
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.base = f"http://127.0.0.1:{self.web}/api/v2"
        profile = work / "profile"
        conf = profile / "qBittorrent/config"
        conf.mkdir(parents=True)
        salt = os.urandom(16)
        password = base64.b64encode(salt).decode() + ":" + base64.b64encode(
            hashlib.pbkdf2_hmac("sha512", b"seedhop-local-test", salt, 100000)).decode()
        (conf / "qBittorrent.ini").write_text(f"""[LegalNotice]
Accepted=true
[Preferences]
WebUI\\Enabled=true
WebUI\\Address=127.0.0.1
WebUI\\Port={self.web}
WebUI\\Username=seedhop-test
WebUI\\Password_PBKDF2=@ByteArray({password})
WebUI\\LocalHostAuth=false
WebUI\\CSRFProtection=false
WebUI\\HostHeaderValidation=false
Connection\\UPnP=false
[BitTorrent]
Session\\DHTEnabled=false
Session\\UPnPEnabled=false
Session\\LSDEnabled=false
Session\\PeXEnabled=false
Session\\Port={self.peer}
Session\\Interface=lo0
Session\\InterfaceName=lo0
Session\\InterfaceAddress=127.0.0.1
""")
        self.process = subprocess.Popen([str(executable), "--confirm-legal-notice", "--no-splash",
                                         f"--profile={profile}", f"--webui-port={self.web}",
                                         f"--torrenting-port={self.peer}"],
                                        env=dict(os.environ, QT_QPA_PLATFORM="cocoa"), stdout=self.log, stderr=self.log)

    def start(self):
        deadline = time.monotonic() + 25
        while True:
            try:
                self.version = self.api("/app/version").decode()
                break
            except OSError:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError(f"Isolated qBittorrent failed to start; see {self.work}")
                time.sleep(0.2)
        self.api("/app/setPreferences", {"json": json.dumps({
            "encryption": 2, "bittorrent_protocol": 1, "queueing_enabled": False,
            "max_ratio": -1, "max_seeding_time": -1, "max_inactive_seeding_time": -1,
            "dht": False, "pex": False, "lsd": False, "upnp": False,
            "resolve_peer_countries": False})})
        self.build = json.loads(self.api("/app/buildInfo"))
        write_json(self.work / "build-info.json", self.build)
        prefs = json.loads(self.api("/app/preferences"))
        keys = ["encryption", "bittorrent_protocol", "queueing_enabled", "dht", "pex", "lsd", "upnp",
                "max_ratio", "max_seeding_time", "current_network_interface", "current_interface_address"]
        write_json(self.work / "peer-settings.json", {key: prefs.get(key) for key in keys})
        if prefs.get("encryption") != 2 or prefs.get("dht") or prefs.get("pex") or prefs.get("lsd"):
            raise RuntimeError("qBittorrent did not apply the isolated test settings")

    def api(self, path, values=None):
        data = urllib.parse.urlencode(values).encode() if values is not None else None
        with self.http.open(self.base + path, data=data, timeout=3) as response:
            return response.read()

    def add(self, torrent, destination):
        boundary = "seedhop-" + os.urandom(8).hex()
        body = b""
        for name, value in {"savepath": str(destination), "stopped": "false", "paused": "false",
                            "autoTMM": "false", "skip_checking": "false", "root_folder": "true"}.items():
            body += f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
        body += (f'--{boundary}\r\nContent-Disposition: form-data; name="torrents"; filename="fixture.torrent"\r\n'
                 'Content-Type: application/x-bittorrent\r\n\r\n').encode()
        body += torrent.read_bytes() + f"\r\n--{boundary}--\r\n".encode()
        request = urllib.request.Request(self.base + "/torrents/add", data=body,
                                         headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
        with self.http.open(request, timeout=5) as response:
            added = response.read().strip()
        if added.startswith(b"{"):
            outcome = json.loads(added)
            if outcome.get("failure_count", 0) or outcome.get("success_count") != 1:
                raise RuntimeError(f"qBittorrent rejected fixture: {added[:300]!r}")
        elif added not in (b"Ok.", b"Ok", b""):
            raise RuntimeError(f"Unexpected add response: {added[:300]!r}")

    def complete(self, info_hash):
        deadline = time.monotonic() + 40
        while True:
            items = json.loads(self.api("/torrents/info?hashes=" + info_hash))
            if len(items) == 1 and items[0]["progress"] == 1 and items[0]["state"] in ("stalledUP", "uploading", "forcedUP"):
                write_json(self.work / "qbit-complete.json", items)
                return items[0]
            if time.monotonic() >= deadline:
                raise RuntimeError(f"qBittorrent did not complete: {items!r}")
            time.sleep(0.2)

    def ready_peer(self, info_hash):
        # API progress can be 1 before the peer listener is ready for this torrent.
        attempts = []
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", self.peer), timeout=1) as connection:
                    connection.sendall(b"\x13BitTorrent protocol" + bytes(8) + bytes.fromhex(info_hash)
                                       + b"-SH0001-" + os.urandom(12))
                    response = b""
                    while len(response) < 68:
                        chunk = connection.recv(68 - len(response))
                        if not chunk:
                            break
                        response += chunk
                valid = len(response) == 68 and response[:20] == b"\x13BitTorrent protocol" and response[28:48].hex() == info_hash
                attempts.append({"responseBytes": len(response), "valid": valid})
                if valid:
                    write_json(self.work / "handshake-readiness.json", attempts)
                    return len(attempts)
            except OSError as error:
                attempts.append({"error": str(error)})
            time.sleep(0.25)
        write_json(self.work / "handshake-readiness.json", attempts)
        raise RuntimeError("qBittorrent's torrent is complete but no valid peer handshake arrived within 30 seconds")

    def diagnostics(self):
        for name, path in [("qbit-after.json", "/torrents/info"), ("peer-blocks.json", "/log/peers"),
                           ("qbit-events.json", "/log/main")]:
            with contextlib.suppress(OSError):
                (self.work / name).write_bytes(self.api(path))

    def close(self):
        stop(self.process)
        self.log.close()


def run_case(direction, kind, executable):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"interop-{direction}-{kind}-", dir=ROOT / ".build"))
    print(f"RUN {direction}/{kind}: {work}", flush=True)
    report = {"case": kind, "direction": "SeedHop → qBittorrent" if direction == "upload" else "qBittorrent → SeedHop",
              "artifacts": str(work), "result": "FAIL"}
    tracker = Tracker(work)
    qbit = None
    engine = None
    try:
        torrent, info_hash, files, seed_parent, seed_root = fixture(work, kind != "single", tracker.url)
        tracker.info_hash = info_hash
        downloads = work / "downloads"
        downloads.mkdir()
        selected = [0, 2] if kind == "magnet" else list(range(len(files)))
        with (work / "seedhop.log").open("w") as engine_log, (work / "seedhop-snapshot.json").open("w+") as snapshots:
            qbit = QBittorrent(work, executable)
            qbit.start()
            report.update(peer=qbit.version, peerBuild=qbit.build, infoHash=info_hash)
            if direction == "upload":
                engine = subprocess.Popen([str(LAB), "seed", str(torrent), str(work / "state"), str(seed_root)],
                                          stdout=subprocess.PIPE, stderr=engine_log, text=True,
                                          env=dict(os.environ, SEEDHOP_DIAGNOSTICS="1"))
                readable, _, _ = select.select([engine.stdout], [], [], 15)
                if not readable:
                    raise RuntimeError("SeedHop did not verify the seed fixture within 15 seconds")
                ready = engine.stdout.readline()
                snapshots.write(ready)
                snapshots.flush()
                tracker.peer = json.loads(ready)["listenPort"]
                qbit.add(torrent, downloads)
                item = qbit.complete(info_hash)
                output = pathlib.Path(item["content_path"]) if len(files) > 1 else pathlib.Path(item["content_path"]).parent
            else:
                tracker.peer = qbit.peer
                qbit.add(torrent, seed_parent)
                qbit.complete(info_hash)
                report["handshakeAttempts"] = qbit.ready_peer(info_hash)
                source = "magnet:?" + urllib.parse.urlencode({"xt": "urn:btih:" + info_hash, "tr": tracker.url}) if kind == "magnet" else str(torrent)
                command = "download-magnet" if kind == "magnet" else "download"
                engine = subprocess.Popen([str(LAB), command, source, str(work / "state"), str(downloads),
                                           ",".join(map(str, selected))], stdout=snapshots, stderr=engine_log,
                                          env=dict(os.environ, SEEDHOP_DIAGNOSTICS="1"))
                engine.wait(timeout=85 if kind == "magnet" else 55)
                if engine.returncode:
                    raise RuntimeError(f"SeedHop download exited {engine.returncode}; see {work / 'seedhop.log'}")
                snapshots.seek(0)
                snapshot = json.load(snapshots)
                item = next(t for t in snapshot["torrents"] if t["id"] == info_hash)
                if item["state"] != "seeding" or [f["index"] for f in item["files"] if f["selected"]] != selected:
                    raise RuntimeError("Completed snapshot did not preserve the requested file selection")
                if kind == "magnet" and item["files"][1]["downloaded"] != 0:
                    raise RuntimeError("The piece-aligned unselected file was downloaded")
                output = downloads
            verified = []
            for index in selected:
                name, expected = files[index]
                actual = (output / name).read_bytes()
                if actual != expected:
                    raise RuntimeError(f"Byte comparison failed for {name}")
                verified.append({"name": name, "bytes": len(actual), "sha256": hashlib.sha256(actual).hexdigest()})
            report.update(result="PASS", files=verified, bytesVerified=sum(file["bytes"] for file in verified),
                          source="magnet metadata" if kind == "magnet" else "torrent file",
                          transport="plain v1 TCP; loopback tracker and peer traffic")
    except Exception as error:
        report["error"] = str(error)
    finally:
        stop(engine)
        if qbit is not None:
            qbit.diagnostics()
            qbit.close()
        tracker.close()
        write_json(work / "result.json", report)
    print(f"{report['result']}: {direction}/{kind}" + (" — " + report["error"] if "error" in report else ""), flush=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--direction", choices=["both", "upload", "download"], default="both")
    parser.add_argument("--case", choices=["all", "single", "multi", "magnet"], default="all")
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--qbit", type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be at least 1")
    if not args.qbit.is_file():
        parser.error("Install qBittorrent or supply its executable with --qbit")
    cases = [(direction, kind) for direction in ["upload", "download"] for kind in ["single", "multi", "magnet"]
             if (args.direction == "both" or args.direction == direction)
             and (args.case == "all" or args.case == kind) and not (direction == "upload" and kind == "magnet")]
    if not cases:
        parser.error("The magnet case tests SeedHop metadata resolution and requires download direction")
    (ROOT / ".build").mkdir(exist_ok=True)
    summary = {"schemaVersion": 1, "result": "RUNNING", "cases": [],
               "startedUTC": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    destination = ROOT / ".build/interop-result.json"
    write_json(destination, summary)
    subprocess.run(["cargo", "build", "--locked", "--offline", "-p", "seedhop-core", "--bin", "seedhop-lab"], cwd=ROOT, check=True)
    for repeat in range(args.repeat):
        for direction, kind in cases:
            report = run_case(direction, kind, args.qbit.resolve())
            report["run"] = repeat + 1
            summary["cases"].append(report)
            write_json(destination, summary)
    summary["result"] = "PASS" if all(case["result"] == "PASS" for case in summary["cases"]) else "FAIL"
    summary["bytesVerified"] = sum(case.get("bytesVerified", 0) for case in summary["cases"])
    write_json(destination, summary)
    print(f"{summary['result']}: {len(summary['cases'])} cases, {summary['bytesVerified']} bytes verified; {destination}")
    raise SystemExit(0 if summary["result"] == "PASS" else 1)


if __name__ == "__main__":
    main()
