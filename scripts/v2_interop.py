#!/usr/bin/env python3
"""Full v2 wire interoperability against isolated qBittorrent profiles."""
import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
import time
from interop import LAB, QBIT, ROOT, QBittorrent, Tracker, bencode, stop, write_json
from v2_validation import fixture, digest


def run_case(hybrid, direction, executable):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"v2-{direction}-{'hybrid' if hybrid else 'pure'}-", dir=ROOT / '.build'))
    print(f"RUN {direction}/{'hybrid' if hybrid else 'pure'} {work}", flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, hybrid, tracker.url)
        wire_hash = digest(bencode(info))[:20].hex()
        tracker.info_hash = wire_hash
        if hybrid:
            tracker.accepted_hashes.add(hashlib.sha1(bencode(info)).hexdigest())
        peer = QBittorrent(work, executable)
        cleanup.callback(peer.close)
        peer.start()
        try:
            if direction in ('download', 'magnet', 'magnet-v1', 'selective'):
                tracker.peer = peer.peer
                peer.add(torrent, work / 'seed')
                peer.complete(wire_hash)
                peer.ready_peer(wire_hash)
                downloads = work / 'downloads'
                downloads.mkdir()
                import urllib.parse
                source = str(torrent)
                if direction in ('magnet', 'magnet-v1'):
                    xt = 'urn:btih:' + hashlib.sha1(bencode(info)).hexdigest() if direction == 'magnet-v1' else 'urn:btmh:1220' + digest(bencode(info)).hex()
                    source = 'magnet:?' + urllib.parse.urlencode({'xt': xt, 'tr': tracker.url})
                command = 'download-magnet' if direction in ('magnet', 'magnet-v1') else 'download'
                arguments = [str(LAB), command, source, str(work / 'state'), str(downloads)]
                if direction == 'selective': arguments.append('1')
                completed = subprocess.run(arguments, capture_output=True, text=True, timeout=150, env=dict(os.environ, SEEDHOP_DIAGNOSTICS="1"))
                (work / 'seedhop.log').write_text(completed.stdout + completed.stderr)
                completed.check_returncode()
                for index, (path, data) in enumerate(files):
                    if direction == 'selective' and index != 1: continue
                    assert (downloads / path).read_bytes() == data, path
                if direction == 'selective':
                    snapshot = json.loads(completed.stdout)
                    assert all(file['downloaded'] == 0 for file in snapshot['torrents'][0]['files'] if file['index'] != 1)
            else:
                log = (work / 'seedhop.log').open('w+')
                cleanup.callback(log.close)
                engine = subprocess.Popen([str(LAB), 'seed', str(torrent), str(work / 'state'), str(work / 'seed/payload')], stdout=subprocess.PIPE, stderr=log, text=True, env=dict(os.environ, SEEDHOP_DIAGNOSTICS="1"))
                cleanup.callback(stop, engine)
                # Seed lab emits a snapshot once its on-disk SHA-256 recheck completes.
                import select
                ready, _, _ = select.select([engine.stdout], [], [], 15)
                if not ready:
                    raise RuntimeError('SeedHop did not finish its v2 recheck')
                line = engine.stdout.readline()
                if not line:
                    raise RuntimeError(f'SeedHop exited; see {work}')
                tracker.peer = json.loads(line)['listenPort']
                downloads = work / 'downloads'
                downloads.mkdir()
                peer.add(torrent, downloads)
                peer.complete(wire_hash)
                for path, data in files:
                    assert (downloads / 'payload' / path).read_bytes() == data, path
            result = {'format': 'hybrid' if hybrid else 'v2', 'direction': direction,
                      'bytes': len(files[1][1]) if direction == 'selective' else sum(len(data) for _, data in files), 'result': 'PASS',
                      'infoHashV2': digest(bencode(info)).hex(), 'qBittorrent': peer.version, 'libtorrent': peer.build.get('libtorrent')}
            write_json(work / 'result.json', result)
            return result
        finally:
            peer.diagnostics()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--direction', choices=['download', 'upload', 'magnet', 'magnet-v1', 'selective', 'both'], default='both')
    parser.add_argument('--format', choices=['pure', 'hybrid', 'both'], default='both')
    parser.add_argument('--qbit', type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = [run_case(hybrid, direction, args.qbit) for hybrid in [False,True] if args.format == 'both' or hybrid == (args.format == 'hybrid') for direction in ['download', 'upload', 'magnet', 'selective', 'magnet-v1'] if (args.direction == 'both' or direction == args.direction) and (direction != 'magnet-v1' or hybrid)]
    write_json(ROOT / '.build/v2-interop-result.json', results)
    print(json.dumps(results, indent=2))

if __name__ == '__main__':
    main()
