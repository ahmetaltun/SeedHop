#!/usr/bin/env python3
"""Causal legacy-layer interest, choke recovery and useful-progress deadlines."""
import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import select
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse
from interop import ROOT, LAB, Tracker, bencode, write_json
from v2_validation import fixture, digest
from v2_hardening import bdecode
from v2_hybrid_interop import PeerServer, V1Seed, read, frame, send, wire_content


class LayerPeer(PeerServer):
    def __init__(self, info, content, observer, allowed, release_after_choke=None):
        self.info, self.content = bencode(info), content
        self.identity = hashlib.sha1(self.info).digest()
        self.observer, self.allowed = observer, allowed
        self.release_after_choke = release_after_choke
        self.states, self.payload, self.metadata = [], [], []
        self.classified = threading.Event()
        self.completed = threading.Event()
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68)
        if handshake[:20] != b'\x13BitTorrent protocol' or handshake[28:48] != self.identity:
            return
        reserved = bytes([0, 0, 0, 0, 0, 16, 0, 0])
        connection.sendall(handshake[:20] + reserved + self.identity + b'-PyLay-1234567890123')
        send(connection, b'\x05' + (b'\x80' if self.observer else b'\x20' if self.release_after_choke else b'\x40'))
        if not self.observer:
            send(connection, b'\x01')
        send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': len(self.info)}))
        metadata_id, dictionary, phase = None, False, 0
        started = time.monotonic()
        premature = []
        published_release = False
        while True:
            if self.release_after_choke and self.release_after_choke.is_set() and not published_release:
                send(connection, b'\x04' + struct.pack('!I', 1))
                published_release = True
            ready, _, _ = select.select([connection], [], [], .05)
            if not ready:
                if self.observer and not dictionary and phase == 0 and time.monotonic() - started > 1:
                    assert not premature, f'interested before any useful layer piece: {premature}'
                    self.classified.set()
                    phase = 1
                    send(connection, b'\x04' + struct.pack('!I', 1))
                continue
            message = frame(connection)
            if message[:2] == b'\x14\x00':
                extensions = bdecode(message[2:]).get(b'm', {})
                metadata_id = extensions.get(b'ut_metadata', metadata_id)
            elif message[:2] == b'\x14\x01':
                request = bdecode(message[2:])
                assert request[b'msg_type'] == 0 and metadata_id
                dictionary = True
                index = request[b'piece']; self.metadata.append(index)
                block = self.info[index * 16384:(index + 1) * 16384]; assert block
                send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': 1, b'piece': index, b'total_size': len(self.info)}) + block)
            elif message[:1] == b'\x02' and self.observer:
                if phase == 0:
                    premature.append('interested')
                else:
                    self.states.append('interested')
                    if phase == 1:
                        self.allowed.set()
                    elif phase == 2:
                        phase = 3
                        send(connection, b'\x01')
                    else:
                        raise AssertionError(f'duplicate interest in phase {phase}')
            elif message[:1] == b'\x03' and self.observer:
                assert phase == 1, f'unexpected not-interested in phase {phase}'
                self.states.append('not-interested')
                phase = 2
                send(connection, b'\x04' + struct.pack('!I', 2))
            elif message[:1] == b'\x06':
                index, begin, length = struct.unpack('!III', message[1:])
                assert index in ([2] if self.observer else [1, 2] if self.release_after_choke else [1])
                if self.observer:
                    assert phase == 3, 'observer requested while choked'
                elif self.release_after_choke:
                    assert index != 1 or published_release, 'released piece was requested before the choke connection closed'
                else:
                    assert self.allowed.wait(8), 'source ran before observer became interested while choked'
                offset = index * 32768 + begin
                block = self.content[offset:offset + length]
                assert len(block) == length and 0 < length <= 16384
                self.payload.append(dict(index=index, begin=begin, length=length))
                send(connection, b'\x07' + struct.pack('!II', index, begin) + block)
                if self.observer and begin + length == 32768:
                    self.completed.set()

    def close(self):
        self.allowed.set()
        super().close()


def run_case(identity):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'legacy-interest-{identity}-', dir=ROOT / '.build'))
    print('RUN', work, flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work); cleanup.callback(tracker.close)
        torrent, _, info = fixture(work, True, tracker.url, files=[('a', b'7'), ('b', bytes(i % 251 for i in range(65536)))])
        expected_layers = bdecode(torrent.read_bytes())[b'piece layers']
        content = wire_content(info, [('a', b'7'), ('b', bytes(i % 251 for i in range(65536)))])
        allowed = threading.Event()
        observer = LayerPeer(info, content, True, allowed); cleanup.callback(observer.close)
        source = LayerPeer(info, content, False, allowed); cleanup.callback(source.close)
        v1, v2 = hashlib.sha1(bencode(info)).hexdigest(), digest(bencode(info)).hex()
        tracker.info_hash, tracker.accepted_hashes = v2[:40], {v1}
        tracker.peers_by_hash = {v1: [observer.port, source.port]}
        parameters = [('xt', 'urn:btih:' + v1), ('tr', tracker.url)]
        if identity == 'dual': parameters.insert(1, ('xt', 'urn:btmh:1220' + v2))
        downloads = work / 'downloads'; downloads.mkdir()
        with (work / 'seedhop.log').open('w') as log:
            result = subprocess.run([str(LAB), 'inspect-magnet', 'magnet:?' + urllib.parse.urlencode(parameters), str(work / 'state'), str(downloads)],
                                    stdout=subprocess.PIPE, stderr=log, text=True, timeout=35, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION='disabled'))
            log.write(result.stdout)
        result.check_returncode()
        assert observer.classified.is_set() and observer.completed.is_set()
        assert observer.states == ['interested', 'not-interested', 'interested'], observer.states
        assert {p['index'] for p in source.payload} == {1}
        assert {p['index'] for p in observer.payload} == {2}
        resolved = json.loads(result.stdout)
        assert resolved['metadata']['id'] == v2 and not resolved['snapshot']['torrents']
        assert bdecode(bytes(resolved['metadata']['torrentBytes']))[b'piece layers'] == expected_layers
        assert not any(path.is_file() for path in downloads.rglob('*'))
        record = dict(mode='interest', identity=identity, result='PASS', states=observer.states, sourceRequests=source.payload, observerRequests=observer.payload, evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record)
        return record



class ChokingSeed(V1Seed):
    def __init__(self, info, content):
        self.current = threading.local()
        self.closed_after_choke = threading.Event()
        self.choked_requests = []
        self.closed_count = 0
        super().__init__(info, content, metadata=True, available=[1], corrupt=self.interrupt)

    def interrupt(self, index, begin, data):
        assert not self.current.choked, 'client requested more payload after being choked'
        self.current.choked = True
        self.choked_requests.append(dict(index=index, begin=begin, length=len(data)))
        send(self.current.connection, b'\x00')
        # This block arrives after implicit cancellation and must not complete
        # a claim or be carried into the healthy provider's replacement piece.
        return bytes([data[0] ^ 1]) + data[1:]

    def serve(self, connection):
        self.current.connection, self.current.choked = connection, False
        try:
            super().serve(connection)
        finally:
            if self.current.choked:
                with self.lock:
                    self.closed_count += 1
                self.closed_after_choke.set()


def run_choke_case(identity):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'legacy-choke-{identity}-', dir=ROOT / '.build'))
    print('RUN', work, flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work); cleanup.callback(tracker.close)
        files = [('a', b'7'), ('b', bytes(i % 251 for i in range(65536)))]
        torrent, _, info = fixture(work, True, tracker.url, files=files)
        expected_layers = bdecode(torrent.read_bytes())[b'piece layers']
        content = wire_content(info, files)
        choker = ChokingSeed(info, content); cleanup.callback(choker.close)
        source = LayerPeer(info, content, False, threading.Event(), choker.closed_after_choke); cleanup.callback(source.close)
        v1, v2 = hashlib.sha1(bencode(info)).hexdigest(), digest(bencode(info)).hex()
        tracker.info_hash, tracker.accepted_hashes = v2[:40], {v1}
        tracker.peers_by_hash = {v1: [choker.port, source.port]}
        parameters = [('xt', 'urn:btih:' + v1), ('tr', tracker.url)]
        if identity == 'dual': parameters.insert(1, ('xt', 'urn:btmh:1220' + v2))
        downloads = work / 'downloads'; downloads.mkdir()
        with (work / 'seedhop.log').open('w') as log:
            result = subprocess.run([str(LAB), 'inspect-magnet', 'magnet:?' + urllib.parse.urlencode(parameters), str(work / 'state'), str(downloads)],
                                    stdout=subprocess.PIPE, stderr=log, text=True, timeout=35, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION='disabled'))
            log.write(result.stdout)
        result.check_returncode()
        assert choker.choked_requests and choker.closed_count
        assert {request['index'] for request in source.payload} == {1, 2}
        assert any(request['index'] == 1 and request['begin'] == 0 for request in source.payload), 'released piece did not restart at its first block'
        resolved = json.loads(result.stdout)
        assert resolved['metadata']['id'] == v2 and not resolved['snapshot']['torrents']
        assert bdecode(bytes(resolved['metadata']['torrentBytes']))[b'piece layers'] == expected_layers
        assert not any(path.is_file() for path in downloads.rglob('*'))
        assert 'legacy layer peer choked its pending request' in (work / 'seedhop.log').read_text()
        record = dict(mode='choke', identity=identity, result='PASS', chokedRequests=choker.choked_requests, closedConnections=choker.closed_count, healthyRequests=source.payload, evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record)
        return record



class ProgressSeed(V1Seed):
    def __init__(self, info, content):
        self.current = threading.local()
        super().__init__(info, content, metadata=True, corrupt=self.delay)

    def serve(self, connection):
        self.current.connection = connection
        super().serve(connection)

    def delay(self, index, begin, data):
        if len(self.requests) <= 2:
            for _ in range(11):
                send(self.current.connection, b'')
                time.sleep(1)
        return data

def run_progress_case():
    work = pathlib.Path(tempfile.mkdtemp(prefix='legacy-progress-', dir=ROOT / '.build'))
    print('RUN', work, flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work); cleanup.callback(tracker.close)
        files = [('a', b'7'), ('b', bytes(i % 251 for i in range(32769)))]
        torrent, _, info = fixture(work, True, tracker.url, files=files)
        expected_layers = bdecode(torrent.read_bytes())[b'piece layers']
        source = ProgressSeed(info, wire_content(info, files))
        cleanup.callback(source.close)
        v1, v2 = hashlib.sha1(bencode(info)).hexdigest(), digest(bencode(info)).hex()
        tracker.info_hash, tracker.accepted_hashes = v2[:40], {v1}
        tracker.peers_by_hash = {v1: [source.port]}
        downloads = work / 'downloads'; downloads.mkdir()
        magnet = 'magnet:?' + urllib.parse.urlencode([('xt', 'urn:btih:' + v1), ('tr', tracker.url)])
        started = time.monotonic()
        with (work / 'seedhop.log').open('w') as log:
            result = subprocess.run([str(LAB), 'inspect-magnet', magnet, str(work / 'state'), str(downloads)],
                                    stdout=subprocess.PIPE, stderr=log, text=True, timeout=35, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION='disabled'))
            log.write(result.stdout)
        result.check_returncode()
        elapsed = time.monotonic() - started
        assert elapsed >= 22
        assert [(request['index'], request['begin']) for request in source.requests] == [(1, 0), (1, 16384), (2, 0), (2, 16384)], source.requests
        assert 'no payload progress for 20 seconds' not in (work / 'seedhop.log').read_text()
        resolved = json.loads(result.stdout)
        assert resolved['metadata']['id'] == v2 and not resolved['snapshot']['torrents']
        assert bdecode(bytes(resolved['metadata']['torrentBytes']))[b'piece layers'] == expected_layers
        assert not any(path.is_file() for path in downloads.rglob('*'))
        record = dict(mode='progress', identity='btih', result='PASS', elapsedSeconds=elapsed, requests=source.requests, evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record)
        return record

def main():
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['all', 'interest', 'choke', 'progress'], default='all')
    args = parser.parse_args()
    results = []
    try:
        for identity in ['btih', 'dual']:
            if args.mode in ('all', 'interest'): results.append(run_case(identity))
            if args.mode in ('all', 'choke'): results.append(run_choke_case(identity))
        if args.mode in ('all', 'progress'): results.append(run_progress_case())
    finally: write_json(ROOT / '.build/v2-legacy-interest-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
