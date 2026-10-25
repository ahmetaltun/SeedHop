#!/usr/bin/env python3
"""Independent BEP 52 fixtures, Rust piece checks, and isolated qBittorrent validation.

This certifies metainfo/content verification, not v2 peer transfers in SeedHop.
"""
import argparse
import contextlib
import datetime
import hashlib
import json
import pathlib
import subprocess
import tempfile
from zoneinfo import ZoneInfo

from interop import LAB, QBIT, ROOT, QBittorrent, bencode, write_json


def digest(data):
    return hashlib.sha256(data).digest()


def tree_root(hashes):
    while len(hashes) > 1:
        hashes = [digest(hashes[i] + hashes[i + 1]) for i in range(0, len(hashes), 2)]
    return hashes[0]


def fixture(work, hybrid, announce=None):
    piece_length = 32768
    # Three real pieces, a short block, an empty file and a shared pieces root.
    files = [("a.bin", bytes(i % 251 for i in range(70001))),
             ("docs/b.bin", b"small file"), ("empty", b""),
             ("same.bin", bytes(i % 251 for i in range(70001)))]
    tree, layers, v1_files, content = {}, {}, [], bytearray()
    for file_index, (path, data) in enumerate(files):
        props = {b"length": len(data)}
        if data:
            hashes = [digest(data[i:i + 16384]) for i in range(0, len(data), 16384)]
            hashes.extend([bytes(32)] * ((1 << (len(hashes) - 1).bit_length()) - len(hashes)))
            root = tree_root(hashes)
            props[b"pieces root"] = root
            if len(data) > piece_length:
                layer = [tree_root(hashes[i:i + 2]) for i in range(0, len(hashes), 2)]
                layers[root] = b"".join(layer[:(len(data) + piece_length - 1) // piece_length])
        node = tree
        parts = path.encode().split(b"/")
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = {b"": props}
        v1_files.append({b"length": len(data), b"path": parts})
        content.extend(data)
        padding = (-len(content)) % piece_length
        if padding:
            v1_files.append({b"attr": b"p", b"length": padding,
                             b"path": [b".pad", f"{file_index}_{padding}".encode()]})
            content.extend(bytes(padding))
    info = {b"file tree": tree, b"meta version": 2, b"name": b"payload",
            b"piece length": piece_length}
    if hybrid:
        info.update({b"files": v1_files, b"pieces": b"".join(
            hashlib.sha1(content[i:i + piece_length]).digest()
            for i in range(0, len(content), piece_length))})
    torrent = work / "fixture.torrent"
    metainfo = {b"info": info, b"piece layers": layers}
    if announce:
        metainfo[b"announce"] = announce.encode()
    torrent.write_bytes(bencode(metainfo))
    # qBittorrent prepends the advisory name for a rootless multi-file tree.
    seed = work / "seed"
    for path, data in files:
        target = seed / "payload" / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    return torrent, files, info


def run_case(parent, hybrid, qbit_executable):
    work = parent / ("hybrid" if hybrid else "v2")
    work.mkdir()
    with contextlib.ExitStack() as cleanup:
        return validate_case(work, hybrid, qbit_executable, cleanup)


def validate_case(work, hybrid, qbit_executable, cleanup):
    torrent, files, info = fixture(work, hybrid)
    parsed = json.loads(subprocess.check_output([str(LAB), "inspect-v2", str(torrent)]))
    expected_hash = digest(bencode(info))
    assert bytes(parsed["infoHash"]) == expected_hash
    assert parsed["hybrid"] == hybrid
    assert parsed["totalBytes"] == sum(len(data) for _, data in files)
    index = 0
    for path, data in files:
        for start in range(0, len(data), 32768):
            piece = work / "piece.bin"
            piece.write_bytes(data[start:start + 32768])
            subprocess.run([str(LAB), "verify-v2-piece", str(torrent), str(index), str(piece)],
                           check=True, capture_output=True)
            index += 1
    assert parsed["pieceCount"] == index
    result = {"format": "hybrid" if hybrid else "v2", "infoHashV2": expected_hash.hex(),
              "verifiedPieces": index, "verifiedBytes": parsed["totalBytes"],
              "metainfoAndPieces": "PASS"}
    if qbit_executable:
        peer = QBittorrent(work, qbit_executable)
        cleanup.callback(peer.close)
        try:
            peer.start()
            peer.add(torrent, work / "seed")
            # This qBittorrent build indexes hybrids by the truncated v2 identity.
            item = peer.complete(expected_hash[:20].hex())
            assert item["infohash_v2"] == expected_hash.hex()
            if hybrid:
                assert item["infohash_v1"] == hashlib.sha1(bencode(info)).hexdigest()
            result.update(qBittorrent=peer.version, libtorrent=peer.build.get("libtorrent"),
                          independentContentRecheck="PASS")
        finally:
            peer.diagnostics()
    write_json(work / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--without-qbit", action="store_true")
    parser.add_argument("--qbit", type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    if not args.without_qbit and not args.qbit.is_file():
        parser.error("Install qBittorrent, pass --qbit, or use --without-qbit")
    subprocess.run(["cargo", "build", "-p", "seedhop-core", "--bin", "seedhop-lab", "--locked"], cwd=ROOT, check=True)
    work = pathlib.Path(tempfile.mkdtemp(prefix="v2-validation-", dir=ROOT / ".build"))
    results = [run_case(work, hybrid, None if args.without_qbit else args.qbit) for hybrid in [False, True]]
    report = {"date": datetime.datetime.now(ZoneInfo("Europe/Minsk")).date().isoformat(), "scope": "Standalone BEP 52 metadata and payload verification; transfer suite is v2_interop.py", "cases": results}
    write_json(work / "result.json", report)
    write_json(ROOT / ".build/v2-validation-result.json", report)
    print(json.dumps(report, indent=2))
    print(f"Retained fixtures and logs: {work}")


if __name__ == "__main__":
    main()
