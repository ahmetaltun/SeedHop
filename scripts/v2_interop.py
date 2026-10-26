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


def run_case(hybrid, direction, executable, transport='tcp', encryption='off', policy=None):
    policy = policy or ('require' if encryption == 'required' else 'allow')
    lab_env = dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION=policy)
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"v2-{transport}-{encryption}-{direction}-{'hybrid' if hybrid else 'pure'}-", dir=ROOT / '.build'))
    print(f"RUN {direction}/{'hybrid' if hybrid else 'pure'} {work}", flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, hybrid, tracker.url)
        wire_hash = digest(bencode(info))[:20].hex()
        tracker.info_hash = wire_hash
        if hybrid:
            tracker.accepted_hashes.add(hashlib.sha1(bencode(info)).hexdigest())
        peer = QBittorrent(work, executable, protocol=2 if transport == 'utp' else 1, encryption=1 if encryption == 'required' else 2)
        cleanup.callback(peer.close)
        peer.start()
        try:
            if direction in ('download', 'magnet', 'magnet-v1', 'magnet-dual', 'selective'):
                tracker.peer = peer.peer
                if direction == 'magnet-dual':
                    # Exercise metadata discovery when only the v2 swarm has
                    # an endpoint, even though both commitments are supplied.
                    tracker.peers_by_hash = {wire_hash: peer.peer}
                peer.add(torrent, work / 'seed')
                peer.complete(wire_hash)
                if transport == 'tcp' and encryption == 'off':
                    peer.ready_peer(wire_hash)
                downloads = work / 'downloads'
                downloads.mkdir()
                import urllib.parse
                source = str(torrent)
                if direction in ('magnet', 'magnet-v1', 'magnet-dual'):
                    xt = 'urn:btih:' + hashlib.sha1(bencode(info)).hexdigest() if direction == 'magnet-v1' else 'urn:btmh:1220' + digest(bencode(info)).hex()
                    parameters = [('xt', xt), ('tr', tracker.url)]
                    if direction == 'magnet-dual': parameters.insert(0, ('xt', 'urn:btih:' + hashlib.sha1(bencode(info)).hexdigest()))
                    source = 'magnet:?' + urllib.parse.urlencode(parameters)
                command = 'download-magnet' if direction in ('magnet', 'magnet-v1', 'magnet-dual') else 'download'
                arguments = [str(LAB), command, source, str(work / 'state'), str(downloads)]
                if direction == 'selective': arguments.append('1')
                with (work / 'seedhop.log').open('w') as log:
                    completed = subprocess.run(arguments, stdout=subprocess.PIPE, stderr=log, text=True, timeout=150, env=lab_env)
                    log.write(completed.stdout)
                completed.check_returncode()
                if direction == 'magnet-dual':
                    diagnostics = (work / 'seedhop.log').read_text()
                    assert f'wire_hash={wire_hash} wire_v2=true' in diagnostics
                    events = [json.loads(line) for line in (work / 'tracker.jsonl').read_text().splitlines()]
                    events = [event for event in events if event['userAgent'].startswith('SeedHop/')]
                    assert any(event['infoHash'] == hashlib.sha1(bencode(info)).hexdigest() for event in events)
                    assert all(not event['returnedPorts'] for event in events if event['infoHash'] == hashlib.sha1(bencode(info)).hexdigest())
                for index, (path, data) in enumerate(files):
                    if direction == 'selective' and index != 1: continue
                    assert (downloads / path).read_bytes() == data, path
                if direction == 'selective':
                    snapshot = json.loads(completed.stdout)
                    assert all(file['downloaded'] == 0 for file in snapshot['torrents'][0]['files'] if file['index'] != 1)
            else:
                log = (work / 'seedhop.log').open('w+')
                cleanup.callback(log.close)
                engine = subprocess.Popen([str(LAB), 'seed', str(torrent), str(work / 'state'), str(work / 'seed/payload')], stdout=subprocess.PIPE, stderr=log, text=True, env=lab_env)
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
            if encryption == 'required':
                diagnostics = (work / 'seedhop.log').read_text()
                assert 'MSE ' in diagnostics and 'rc4=true' in diagnostics, 'RC4 negotiation was not exercised'
            result = {'format': 'hybrid' if hybrid else 'v2', 'direction': direction, 'transport': transport, 'encryption': encryption, 'enginePolicy': policy,
                      'bytes': len(files[1][1]) if direction == 'selective' else sum(len(data) for _, data in files), 'result': 'PASS',
                      'infoHashV2': digest(bencode(info)).hex(), 'qBittorrent': peer.version, 'libtorrent': peer.build.get('libtorrent'),
                      'evidence': str(work.relative_to(ROOT))}
            write_json(work / 'result.json', result)
            return result
        finally:
            peer.diagnostics()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--direction', choices=['download', 'upload', 'magnet', 'magnet-v1', 'magnet-dual', 'selective', 'both'], default='both')
    parser.add_argument('--format', choices=['pure', 'hybrid', 'both'], default='both')
    parser.add_argument('--qbit', type=pathlib.Path, default=QBIT)
    parser.add_argument('--transport', choices=['tcp', 'utp'], default='tcp')
    parser.add_argument('--encryption', choices=['off', 'required'], default='off')
    parser.add_argument('--policy', choices=['disabled', 'allow', 'prefer', 'require'])
    args = parser.parse_args()
    if args.direction in ('magnet-v1', 'magnet-dual') and args.format == 'pure':
        parser.error('legacy or dual-hash magnet fixtures require --format hybrid or both')
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = [run_case(hybrid, direction, args.qbit, args.transport, args.encryption, args.policy) for hybrid in [False,True] if args.format == 'both' or hybrid == (args.format == 'hybrid') for direction in ['download', 'upload', 'magnet', 'selective', 'magnet-v1', 'magnet-dual'] if (args.direction == 'both' or direction == args.direction) and (direction not in ('magnet-v1', 'magnet-dual') or hybrid)]
    suffix = '-encrypted' if args.encryption == 'required' else ''
    policy = args.policy or ('require' if args.encryption == 'required' else 'allow')
    full_matrix = args.direction == 'both' and args.format == 'both'
    scope = '' if full_matrix else f'-{args.direction}-{args.format}'
    write_json(ROOT / f'.build/v2-interop-{args.transport}-{args.encryption}-{policy}{scope}-result.json', results)
    if args.policy is None and full_matrix:
        write_json(ROOT / f'.build/v2-interop-{args.transport}{suffix}-result.json', results)
    if args.transport == 'tcp' and args.encryption == 'off' and args.policy is None and full_matrix:
        write_json(ROOT / '.build/v2-interop-result.json', results)
    print(json.dumps(results, indent=2))

if __name__ == '__main__':
    main()
