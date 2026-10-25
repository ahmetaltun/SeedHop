#!/usr/bin/env python3
"""Controlled v2 discovery against independent UDP/KRPC fixtures and qBittorrent.

No public DHT or normal qBittorrent profile is used. Retains packet logs and bytes.
"""
import argparse
import contextlib
import json
import os
import pathlib
import socket
import struct
import subprocess
import tempfile
import threading
import urllib.parse

from interop import ROOT, LAB, QBIT, QBittorrent, Tracker, bencode, write_json, stop
import select
from v2_validation import fixture, digest


def bdecode(data):
    def value(at):
        kind = data[at:at + 1]
        if kind == b'i':
            end = data.index(b'e', at)
            return int(data[at + 1:end]), end + 1
        if kind in (b'd', b'l'):
            items, at = [], at + 1
            while data[at:at + 1] != b'e':
                item, at = value(at)
                items.append(item)
            return (dict(zip(items[::2], items[1::2])) if kind == b'd' else items), at + 1
        colon = data.index(b':', at)
        end = colon + 1 + int(data[at:colon])
        return data[colon + 1:end], end
    decoded, end = value(0)
    assert end == len(data)
    return decoded


class DatagramFixture:
    def __init__(self, work, dht=False):
        self.work, self.dht = work, dht
        self.peer, self.info_hash = 0, ''
        self.events, self.errors = [], []
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(('127.0.0.1', 0))
        self.socket.settimeout(.2)
        self.port = self.socket.getsockname()[1]
        self.url = f'udp://127.0.0.1:{self.port}/announce'
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stopping.is_set():
            try:
                packet, address = self.socket.recvfrom(65536)
            except socket.timeout:
                continue
            try:
                response = self.krpc(packet) if self.dht else self.udp(packet)
                self.socket.sendto(response, address)
            except Exception as error:
                self.errors.append(repr(error))

    def udp(self, packet):
        connection, action, transaction = struct.unpack('!QII', packet[:16])
        if action == 0:
            assert connection == 0x41727101980
            return struct.pack('!IIQ', 0, transaction, 0x53454544484f50)
        assert action == 1 and connection == 0x53454544484f50 and len(packet) >= 98
        valid = packet[16:36].hex() == self.info_hash
        advertised = struct.unpack('!H', packet[96:98])[0]
        endpoint = self.peer if valid and advertised != self.peer else 0
        self.events.append({'query': 'announce', 'hash': packet[16:36].hex(), 'valid': valid,
                            'port': advertised, 'returnedPort': endpoint})
        if not valid:
            return struct.pack('!II', 3, transaction) + b'Unknown fixture'
        peers = socket.inet_aton('127.0.0.1') + struct.pack('!H', endpoint) if endpoint else b''
        return struct.pack('!IIIII', 1, transaction, 1, 0, 1) + peers

    def krpc(self, packet):
        query = bdecode(packet)
        method, args = query[b'q'], query[b'a']
        response = {b'id': digest(b'independent-node')[:20]}
        event = {'query': method.decode()}
        if method in (b'get_peers', b'announce_peer'):
            identity = args[b'info_hash'].hex()
            event.update(hash=identity, valid=identity == self.info_hash)
            response[b'token'] = b'fixture-token'
            if method == b'get_peers' and identity == self.info_hash:
                response[b'values'] = [socket.inet_aton('127.0.0.1') + struct.pack('!H', self.peer)]
        else:
            response[b'nodes'] = response[b'id'] + socket.inet_aton('127.0.0.1') + struct.pack('!H', self.port)
        self.events.append(event)
        return bencode({b't': query[b't'], b'y': b'r', b'r': response})

    def close(self):
        self.stopping.set()
        self.thread.join(timeout=3)
        self.socket.close()
        write_json(self.work / 'datagrams.json', self.events)
        assert not self.errors, self.errors


class DuplicateMetadataPeer:
    """Completes the info dictionary, then replays metadata while proofs are pending."""
    def __init__(self, info, tracker, fallback):
        self.info, self.tracker, self.fallback = bencode(info), tracker, fallback
        self.sent, self.errors = False, []
        self.socket = socket.socket()
        self.socket.bind(('127.0.0.1', 0))
        self.socket.listen()
        self.socket.settimeout(15)
        self.port = self.socket.getsockname()[1]
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    @staticmethod
    def read(connection, count):
        result = b''
        while len(result) < count:
            chunk = connection.recv(count - len(result))
            if not chunk: raise EOFError('peer closed')
            result += chunk
        return result

    @staticmethod
    def send(connection, frame):
        connection.sendall(struct.pack('!I', len(frame)) + frame)

    def run(self):
        try:
            connection, _ = self.socket.accept()
            with connection:
                connection.settimeout(10)
                handshake = self.read(connection, 68)
                connection.sendall(handshake[:48] + b'-SHattack-0123456789')
                self.send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 3}, b'metadata_size': len(self.info)}))
                metadata_id = None
                while True:
                    frame = self.read(connection, struct.unpack('!I', self.read(connection, 4))[0])
                    if frame[:2] == b'\x14\x00':
                        metadata_id = bdecode(frame[2:])[b'm'][b'ut_metadata']
                    elif frame[:2] == b'\x14\x03':
                        assert metadata_id is not None and len(self.info) < 16384
                        payload = bytes([20, metadata_id]) + bencode({b'msg_type': 1, b'piece': 0, b'total_size': len(self.info)}) + self.info
                        self.send(connection, payload)
                        self.send(connection, payload)
                        self.sent = True
                        self.tracker.peer = self.fallback
                        # The client must reject this peer while retaining the ability to retry.
                        while connection.recv(4096): pass
                        return
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            self.tracker.peer = self.fallback

    def close(self):
        self.thread.join(timeout=16)
        self.socket.close()
        assert not self.thread.is_alive(), 'metadata attacker did not stop'
        assert not self.errors, self.errors


def run_case(mode, hybrid, executable):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'v2-hardening-{mode}-', dir=ROOT / '.build'))
    print(f'RUN {mode}/{"hybrid" if hybrid else "pure"} {work}', flush=True)
    ipv6, dht, private = mode.startswith('ipv6'), mode in ('dht', 'private-dht'), mode.startswith('private')
    with contextlib.ExitStack() as cleanup:
        tracker = DatagramFixture(work, dht) if dht or mode == 'udp' else Tracker(work, ipv6=ipv6)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, hybrid, None if dht else tracker.url, private=private)
        identity = digest(bencode(info))[:20].hex()
        tracker.info_hash = identity
        peer = QBittorrent(work, executable, ipv6=ipv6)
        cleanup.callback(peer.close)
        try:
            peer.start()
            if mode == 'ipv6-upload':
                log = (work / 'seedhop.log').open('w+')
                cleanup.callback(log.close)
                engine = subprocess.Popen([str(LAB), 'seed', str(torrent), str(work / 'state'), str(work / 'seed/payload')],
                                          stdout=subprocess.PIPE, stderr=log, text=True,
                                          env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
                cleanup.callback(stop, engine)
                ready, _, _ = select.select([engine.stdout], [], [], 15)
                assert ready, 'SeedHop did not verify the seed'
                tracker.peer = json.loads(engine.stdout.readline())['listenPort']
                downloads = work / 'downloads'
                downloads.mkdir()
                peer.add(torrent, downloads)
                peer.complete(identity)
                for path, data in files:
                    assert (downloads / 'payload' / path).read_bytes() == data, path
                result = {'mode': mode, 'format': 'hybrid' if hybrid else 'v2', 'result': 'PASS',
                          'infoHashV2': digest(bencode(info)).hex(), 'qBittorrent': peer.version,
                          'libtorrent': peer.build.get('libtorrent'), 'evidence': str(work.relative_to(ROOT))}
                write_json(work / 'result.json', result)
                return result
            peer.add(torrent, work / 'seed')
            peer.complete(identity)
            if not ipv6:
                peer.ready_peer(identity)
            tracker.peer = peer.peer
            if mode == 'duplicate-metadata':
                attacker = DuplicateMetadataPeer(info, tracker, peer.peer)
                cleanup.callback(attacker.close)
                tracker.peer = attacker.port
            downloads = work / 'downloads'
            downloads.mkdir()
            args = [str(LAB), 'probe-dht', str(torrent), str(downloads), f'127.0.0.1:{tracker.port}'] if dht else [str(LAB), 'download', str(torrent), str(work / 'state'), str(downloads)]
            if mode == 'private-dht':
                args.append('private')
            if mode == 'duplicate-metadata':
                magnet = 'magnet:?' + urllib.parse.urlencode({'xt': 'urn:btmh:1220' + digest(bencode(info)).hex(), 'tr': tracker.url})
                args = [str(LAB), 'download-magnet', magnet, str(work / 'state'), str(downloads)]
            completed = subprocess.run(args, capture_output=True, text=True, timeout=65,
                                       env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
            (work / 'seedhop.log').write_text(completed.stdout + completed.stderr)
            completed.check_returncode()
            if mode == 'duplicate-metadata':
                assert attacker.sent, 'metadata replay was not exercised'
                assert 'metadata data received after info dictionary completed' in completed.stderr
                assert 'panicked' not in completed.stderr
            if mode == 'private-dht':
                assert any(e['query'] == 'find_node' for e in tracker.events), 'DHT bootstrap did not run'
                assert not any(e['query'] in ('get_peers', 'announce_peer') for e in tracker.events), tracker.events
            else:
                for path, data in files:
                    assert (downloads / path).read_bytes() == data, path
                if isinstance(tracker, DatagramFixture):
                    assert any(e.get('valid') and e.get('query') in ('announce', 'get_peers') for e in tracker.events)
            result = {'mode': mode, 'format': 'hybrid' if hybrid else 'v2', 'result': 'PASS',
                      'infoHashV2': digest(bencode(info)).hex(), 'qBittorrent': peer.version,
                      'libtorrent': peer.build.get('libtorrent'), 'evidence': str(work.relative_to(ROOT))}
            write_json(work / 'result.json', result)
            return result
        finally:
            peer.diagnostics()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['udp', 'ipv6', 'ipv6-upload', 'private', 'dht', 'private-dht', 'duplicate-metadata', 'all'], default='all')
    parser.add_argument('--format', choices=['pure', 'hybrid', 'both'], default='both')
    parser.add_argument('--qbit', type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = []
    try:
        for mode in ['udp', 'ipv6', 'ipv6-upload', 'private', 'dht', 'private-dht', 'duplicate-metadata']:
            if args.mode not in ('all', mode): continue
            for hybrid in [False, True]:
                if args.format != 'both' and hybrid != (args.format == 'hybrid'): continue
                results.append(run_case(mode, hybrid, args.qbit))
    finally:
        write_json(ROOT / '.build/v2-hardening-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
