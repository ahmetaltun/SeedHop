#!/usr/bin/env python3
"""Hybrid btih magnets with exclusively legacy metadata and payload providers."""
import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
import threading
import urllib.parse
from interop import ROOT, LAB, Tracker, bencode, write_json
from v2_validation import fixture, digest
from v2_hardening import bdecode
from v2_hybrid_interop import V1Seed, wire_content


def run_case(mode, identity='btih'):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'legacy-layers-{identity}-{mode}-', dir=ROOT / '.build'))
    print(f'RUN {work}', flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, True, tracker.url, private=mode == 'private')
        expected_layers = bdecode(torrent.read_bytes())[b'piece layers']
        if mode == 'inspect-unpadded':
            assert b'p' in info[b'files'][-1][b'attr']
            info[b'files'].pop()
            data = dict(files)
            unpadded = b''.join(bytes(item[b'length']) if b'p' in item.get(b'attr', b'') else data[b'/'.join(item[b'path']).decode()] for item in info[b'files'])
            info[b'pieces'] = b''.join(hashlib.sha1(unpadded[i:i + info[b'piece length']]).digest() for i in range(0, len(unpadded), info[b'piece length']))
        content = wire_content(info, files)
        if mode == 'inconsistent':
            props = info[b'file tree'][b'a.bin'][b'']
            props[b'pieces root'] = bytes([props[b'pieces root'][0] ^ 1]) + props[b'pieces root'][1:]
        v1, v2 = hashlib.sha1(bencode(info)).hexdigest(), digest(bencode(info)).hex()
        tracker.info_hash, tracker.accepted_hashes = v2[:40], {v1}
        count = len(info[b'pieces']) // 20
        available = range(0, count, 2) if mode == 'partial' else range(4, 7) if mode == 'inspect-unpadded' else None
        def padding_fault(index, begin, data):
            return data[:-1] + bytes([data[-1] ^ 1]) if index == 2 and begin == 0 else data
        fault = padding_fault if mode == 'padding' else mode == 'corrupt'
        first = V1Seed(info, content, metadata=True, available=available, corrupt=fault)
        cleanup.callback(first.close)
        seeds = [first]
        tracker.peers_by_hash = {v1: [first.port]}
        if mode in ('partial', 'corrupt', 'padding'):
            second = V1Seed(info, content, metadata=True, available=range(1, count, 2) if mode == 'partial' else None)
            cleanup.callback(second.close)
            seeds.append(second)
            if mode == 'partial':
                tracker.peers_by_hash[v1].append(second.port)
            else:
                # Introduce a healthy provider only after bad payload has
                # actually been delivered during layer acquisition.
                def recover():
                    if first.corrupted.wait(20):
                        tracker.peers_by_hash[v1] = [first.port, second.port]
                control = threading.Thread(target=recover, daemon=True)
                control.start()
                cleanup.callback(lambda: control.join(timeout=1))
        parameters = [('xt', 'urn:btih:' + v1), ('tr', tracker.url)]
        if identity == 'dual':
            parameters.insert(1, ('xt', 'urn:btmh:1220' + v2))
        source = 'magnet:?' + urllib.parse.urlencode(parameters)
        downloads = work / 'downloads'
        downloads.mkdir()
        with (work / 'seedhop.log').open('w') as log:
            command = 'inspect-magnet' if mode.startswith('inspect') else 'download-magnet'
            result = subprocess.run([str(LAB), command, source, str(work / 'state'), str(downloads)], stdout=subprocess.PIPE, stderr=log, text=True, timeout=75, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
            log.write(result.stdout)
        diagnostics = (work / 'seedhop.log').read_text()
        if mode == 'inconsistent':
            assert result.returncode != 0 and 'commitments are inconsistent' in diagnostics
            assert not any(p.is_file() for p in downloads.rglob('*'))
            session = work / 'state/session/session.json'
            if session.exists():
                assert not json.loads(session.read_text())['torrents']
        elif mode.startswith('inspect'):
            result.check_returncode()
            resolved = json.loads(result.stdout)
            assert resolved['metadata']['id'] == v2 and not resolved['snapshot']['torrents']
            metainfo = bdecode(bytes(resolved['metadata']['torrentBytes']))
            assert metainfo[b'piece layers'] == expected_layers
            assert not any(p.is_file() for p in downloads.rglob('*'))
            assert 'legacy payload Merkle layers authenticated' in diagnostics
        else:
            result.check_returncode()
            item = json.loads(result.stdout)['torrents'][0]
            assert item['id'] == v2 and item['downloadedBytes'] == sum(len(data) for _, data in files)
            for path, data in files:
                assert (downloads / path).read_bytes() == data
            if mode == 'partial':
                assert all(seed.requests for seed in seeds), 'both partial peers must contribute'
            if mode in ('corrupt', 'padding'):
                assert first.corrupted.is_set() and second.requests
                assert 'legacy layer peer failed' in diagnostics
                assert ('nonzero legacy layer padding' if mode == 'padding' else 'legacy payload does not match its SHA-1') in diagnostics
        if identity == 'dual' and mode != 'inconsistent':
            assert f'wire_hash={v1} wire_v2=false' in diagnostics, 'legacy discovery identity was not preserved'
        assert any(seed.metadata_requests for seed in seeds)
        for index, seed in enumerate(seeds):
            write_json(work / f'peer-{index}.json', dict(available=sorted(seed.available), requests=seed.requests, metadata=seed.metadata_requests, declinedV2=seed.declined_v2, corrupted=seed.corrupted.is_set()))
        announces = [json.loads(line) for line in (work / 'tracker.jsonl').read_text().splitlines()]
        assert any(e['infoHash'] == v2[:40] and not e['returnedPorts'] for e in announces), 'v2 proof discovery must have no provider'
        record = dict(mode=mode, identity=identity, result='PASS', bytes=sum(len(data) for _, data in files), infoHashV1=v1, infoHashV2=v2, evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record)
        return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['complete', 'partial', 'corrupt', 'padding', 'private', 'inconsistent', 'inspect-only', 'inspect-unpadded', 'all'], default='all')
    parser.add_argument('--identity', choices=['btih', 'dual'], default='btih')
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = []
    try:
        for mode in ['complete', 'partial', 'corrupt', 'padding', 'private', 'inconsistent', 'inspect-only', 'inspect-unpadded']:
            if args.mode in ('all', mode): results.append(run_case(mode, args.identity))
    finally:
        write_json(ROOT / f'.build/v2-legacy-layers-{args.identity}-{args.mode}-result.json', results)
        if args.identity == 'btih':
            write_json(ROOT / f'.build/v2-legacy-layers-{args.mode}-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
