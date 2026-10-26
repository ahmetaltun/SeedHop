#!/usr/bin/env python3
"""Hybrid transfers with an independent BEP 3-only peer and qBittorrent v2 peer.

The tracker advertises different endpoints under the SHA-1 and SHA-256 identities.
Mixed downloads hide piece zero from qBittorrent, requiring both swarms to supply
payload. The BEP 3 peer never sends an upgrade bit or handles v2 hash messages.
"""
import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import select
import socket
import struct
import subprocess
import tempfile
import threading
import urllib.parse

from interop import ROOT, LAB, QBIT, QBittorrent, Tracker, bencode, stop, write_json
from v2_validation import fixture, digest
from v2_hardening import bdecode


def read(connection, length):
    data = bytearray()
    while len(data) < length:
        chunk = connection.recv(length - len(data))
        if not chunk:
            raise EOFError('peer closed')
        data.extend(chunk)
    return bytes(data)


def frame(connection):
    length, = struct.unpack('!I', read(connection, 4))
    assert length <= 1024 * 1024, 'oversized peer frame'
    return read(connection, length)


def send(connection, payload):
    connection.sendall(struct.pack('!I', len(payload)) + payload)


def wire_content(info, files):
    data = dict(files)
    content = b''.join(bytes(item[b'length']) if b'p' in item.get(b'attr', b'') else
                       data[b'/'.join(item[b'path']).decode()] for item in info[b'files'])
    hashes = b''.join(hashlib.sha1(content[i:i + info[b'piece length']]).digest()
                      for i in range(0, len(content), info[b'piece length']))
    assert hashes == info[b'pieces'], 'independent v1 fixture verification failed'
    return content


class PeerServer:
    def __init__(self):
        self.socket = socket.socket()
        self.socket.bind(('127.0.0.1', 0))
        self.port = self.socket.getsockname()[1]
        self.socket.listen()
        self.socket.settimeout(.2)
        self.stopping = threading.Event()
        self.connections, self.workers, self.errors, self.requests = [], [], [], []
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stopping.is_set():
            try:
                connection, _ = self.socket.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.connections.append(connection)
            connection.settimeout(15)
            worker = threading.Thread(target=self.guarded, args=(connection,), daemon=True)
            self.workers.append(worker)
            worker.start()

    def guarded(self, connection):
        try:
            with connection:
                self.serve(connection)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except Exception as error:
            if not self.stopping.is_set():
                with self.lock:
                    self.errors.append(repr(error))

    def close(self):
        self.stopping.set()
        self.socket.close()
        for connection in self.connections:
            with contextlib.suppress(OSError):
                connection.shutdown(socket.SHUT_RDWR)
        self.thread.join(timeout=2)
        for worker in self.workers:
            worker.join(timeout=2)
        assert not self.errors, self.errors


class V1Seed(PeerServer):
    def __init__(self, info, content, only_first=False, metadata=False, available=None, corrupt=False):
        self.info_hash = hashlib.sha1(bencode(info)).digest()
        self.metadata = bencode(info) if metadata else None
        self.private = info.get(b'private') == 1
        self.metadata_requests = []
        self.v2_hash = digest(bencode(info))[:20]
        self.declined_v2 = 0
        self.declined_encrypted = 0
        self.content, self.piece_length = content, info[b'piece length']
        self.count = len(info[b'pieces']) // 20
        self.only_first = only_first
        self.available = set(available) if available is not None else set(range(1 if only_first else self.count))
        self.corrupt = corrupt
        self.corrupted = threading.Event()
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68)
        if self.metadata and handshake[:20] != b'\x13BitTorrent protocol':
            self.declined_encrypted += 1
            return
        assert handshake[:20] == b'\x13BitTorrent protocol'
        if self.metadata and handshake[28:48] == self.v2_hash:
            self.declined_v2 += 1
            return
        assert handshake[28:48] == self.info_hash, 'wrong swarm identity sent to v1-only seed'
        reserved = bytearray(8)
        if self.metadata:
            reserved[5] = 0x10
        connection.sendall(handshake[:20] + reserved + self.info_hash + b'-V1test-123456789012')
        bits = bytearray((self.count + 7) // 8)
        for index in self.available:
            bits[index // 8] |= 128 >> (index % 8)
        send(connection, b'\x05' + bits)
        send(connection, b'\x01')
        if self.metadata:
            send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': len(self.metadata)}))
        metadata_id = None
        while True:
            message = frame(connection)
            if message[:2] == b'\x14\x00' and self.metadata:
                extensions = bdecode(message[2:]).get(b'm', {})
                metadata_id = extensions.get(b'ut_metadata', metadata_id)
                if self.private:
                    assert not extensions.get(b'ut_pex'), 'private peer advertised PEX'

            elif message[:2] == b'\x14\x01' and self.metadata:
                request = bdecode(message[2:])
                assert request[b'msg_type'] == 0 and metadata_id
                index = request[b'piece']
                data = self.metadata[index * 16384:(index + 1) * 16384]
                assert data
                self.metadata_requests.append(index)
                send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': 1, b'piece': index, b'total_size': len(self.metadata)}) + data)
            elif message[:1] == b'\x06':
                index, begin, length = struct.unpack('!III', message[1:])
                assert index in self.available
                size = min(self.piece_length, len(self.content) - index * self.piece_length)
                assert begin % 16384 == 0 and 0 < length <= 16384 and begin + length <= size
                offset = index * self.piece_length + begin
                with self.lock:
                    self.requests.append({'index': index, 'begin': begin, 'length': length})
                data = self.content[offset:offset + length]
                if self.corrupt:
                    damaged = self.corrupt(index, begin, data) if callable(self.corrupt) else bytes([data[0] ^ 1]) + data[1:]
                    if damaged != data:
                        self.corrupted.set()
                    data = damaged
                send(connection, b'\x07' + struct.pack('!II', index, begin) + data)
            else:
                assert not message or message[0] in (0, 1, 2, 3, 4, 5, 8), 'v2/extension frame sent to v1-only seed'


class V2AvailabilityRelay(PeerServer):
    def __init__(self, peer_port):
        self.peer_port = peer_port
        self.hash_requests = []
        super().__init__()

    def serve(self, connection):
        with socket.create_connection(('127.0.0.1', self.peer_port), timeout=5) as peer:
            peer.settimeout(15)
            handshake = read(connection, 68)
            peer.sendall(handshake)
            connection.sendall(read(peer, 68))

            def upstream():
                try:
                    while True:
                        message = frame(connection)
                        if message[:1] == b'\x06':
                            index, begin, length = struct.unpack('!III', message[1:])
                            assert index != 0, 'v2 peer must not supply piece zero'
                            with self.lock:
                                self.requests.append({'index': index, 'begin': begin, 'length': length})
                        if message[:1] == b'\x15':
                            base, index, length, proof = struct.unpack('!IIII', message[33:])
                            with self.lock:
                                self.hash_requests.append({'piecesRoot': message[1:33].hex(), 'baseLayer': base,
                                                           'index': index, 'length': length, 'proofLayers': proof})
                        send(peer, message)
                except (EOFError, OSError):
                    pass
                except Exception as error:
                    self.errors.append(repr(error))
                finally:
                    with contextlib.suppress(OSError):
                        peer.shutdown(socket.SHUT_RDWR)

            thread = threading.Thread(target=upstream, daemon=True)
            thread.start()
            try:
                while True:
                    message = frame(peer)
                    if message[:1] == b'\x05':
                        message = message[:1] + bytes([message[1] & 127]) + message[2:]
                    if message[:1] == b'\x04' and struct.unpack('!I', message[1:])[0] == 0:
                        continue
                    send(connection, message)
            finally:
                with contextlib.suppress(OSError):
                    connection.shutdown(socket.SHUT_RDWR)
                thread.join(timeout=2)


def v1_download(port, info, content):
    identity = hashlib.sha1(bencode(info)).digest()
    with socket.create_connection(('127.0.0.1', port), timeout=5) as connection:
        connection.settimeout(10)
        connection.sendall(b'\x13BitTorrent protocol' + bytes(8) + identity + b'-V1test-123456789012')
        assert read(connection, 68)[28:48] == identity, 'unsolicited v2 upgrade'
        while frame(connection)[:1] != b'\x01':
            pass
        send(connection, b'\x02')
        requests = 0
        for index, offset in enumerate(range(0, len(content), info[b'piece length'])):
            piece = content[offset:offset + info[b'piece length']]
            received = bytearray()
            for begin in range(0, len(piece), 16384):
                length = min(16384, len(piece) - begin)
                send(connection, b'\x06' + struct.pack('!III', index, begin, length))
                response = frame(connection)
                assert response[:9] == b'\x07' + struct.pack('!II', index, begin)
                assert response[9:] == piece[begin:begin + length]
                received.extend(response[9:])
                requests += 1
            assert hashlib.sha1(received).digest() == info[b'pieces'][index * 20:(index + 1) * 20]
        return requests


def run_case(direction, executable):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'hybrid-dual-{direction}-', dir=ROOT / '.build'))
    print(f'RUN {direction} {work}', flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, True, tracker.url)
        content = wire_content(info, files)
        v1_hash, v2_hash = hashlib.sha1(bencode(info)).hexdigest(), digest(bencode(info))[:20].hex()
        tracker.info_hash, tracker.accepted_hashes = v2_hash, {v1_hash}
        if direction == 'upload-v1':
            log = (work / 'seedhop.log').open('w')
            cleanup.callback(log.close)
            process = subprocess.Popen([str(LAB), 'seed', str(torrent), str(work / 'state'), str(work / 'seed/payload')], stdout=subprocess.PIPE, stderr=log, text=True, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
            cleanup.callback(stop, process)
            ready, _, _ = select.select([process.stdout], [], [], 15)
            assert ready, 'seed did not become ready'
            snapshot = json.loads(process.stdout.readline())
            requests = v1_download(snapshot['listenPort'], info, content)
            assert requests > 0
        else:
            legacy = V1Seed(info, content, only_first=direction != 'download-v1', metadata=direction in ('magnet-v1-mixed', 'magnet-dual-mixed'))
            cleanup.callback(legacy.close)
            v2_requests = []
            tracker.peers_by_hash = {v1_hash: legacy.port}
            if direction != 'download-v1':
                peer = QBittorrent(work, executable)
                cleanup.callback(peer.close)
                peer.start()
                peer.add(torrent, work / 'seed')
                peer.complete(v2_hash)
                peer.ready_peer(v2_hash)
                relay = V2AvailabilityRelay(peer.peer)
                cleanup.callback(relay.close)
                tracker.peers_by_hash[v2_hash] = relay.port
                v2_requests = relay.requests
            source, command = str(torrent), 'download'
            if direction in ('magnet-mixed', 'magnet-v1-mixed', 'magnet-dual-mixed'):
                command = 'download-magnet'
                identity = 'urn:btih:' + v1_hash if direction == 'magnet-v1-mixed' else 'urn:btmh:1220' + digest(bencode(info)).hex()
                parameters = [('xt', identity), ('tr', tracker.url)]
                if direction == 'magnet-dual-mixed': parameters.insert(0, ('xt', 'urn:btih:' + v1_hash))
                source = 'magnet:?' + urllib.parse.urlencode(parameters)
            (work / 'downloads').mkdir()
            with (work / 'seedhop.log').open('w') as log:
                completed = subprocess.run([str(LAB), command, source, str(work / 'state'), str(work / 'downloads')], stdout=subprocess.PIPE, stderr=log, text=True, timeout=150, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
                log.write(completed.stdout)
            completed.check_returncode()
            snapshot = json.loads(completed.stdout)
            item = snapshot['torrents'][0]
            assert item['id'] == digest(bencode(info)).hex()
            assert item['totalBytes'] == item['downloadedBytes'] == sum(len(data) for _, data in files)
            for path, data in files:
                assert (work / 'downloads' / path).read_bytes() == data, path
            assert legacy.requests, 'no payload received through the v1 swarm'
            if direction != 'download-v1':
                assert v2_requests, 'no payload received through the v2 swarm'
                assert all(r['index'] == 0 for r in legacy.requests)
            write_json(work / 'v1-requests.json', legacy.requests)
            write_json(work / 'v2-requests.json', v2_requests)
            if direction in ('magnet-mixed', 'magnet-v1-mixed', 'magnet-dual-mixed'):
                assert relay.hash_requests, 'v2 peer did not receive Merkle proof requests'
                write_json(work / 'v2-hash-requests.json', relay.hash_requests)
            if direction == 'magnet-v1-mixed':
                assert legacy.metadata_requests, 'v1-only provider did not supply metadata'
                write_json(work / 'v1-metadata-requests.json', {'pieces': legacy.metadata_requests, 'declinedV2Connections': legacy.declined_v2, 'declinedEncryptedConnections': legacy.declined_encrypted})
        announces = [json.loads(line) for line in (work / 'tracker.jsonl').read_text().splitlines()]
        announces = [event for event in announces if event['userAgent'].startswith('SeedHop/')]
        assert {v1_hash, v2_hash}.issubset({event['infoHash'] for event in announces}), 'both swarms must be announced'
        if direction != 'upload-v1':
            assert any(event['infoHash'] == v1_hash and event['left'] == len(content) for event in announces)
            assert any(event['infoHash'] == v2_hash and event['left'] == sum(len(data) for _, data in files) for event in announces)
        result = {'direction': direction, 'result': 'PASS', 'bytes': sum(len(data) for _, data in files), 'v1InfoHash': v1_hash, 'v2InfoHash': digest(bencode(info)).hex(), 'evidence': str(work.relative_to(ROOT))}
        write_json(work / 'result.json', result)
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--direction', choices=['download-v1', 'download-mixed', 'magnet-mixed', 'magnet-v1-mixed', 'magnet-dual-mixed', 'upload-v1', 'all'], default='all')
    parser.add_argument('--qbit', type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = [run_case(direction, args.qbit) for direction in ['download-v1', 'download-mixed', 'magnet-mixed', 'magnet-v1-mixed', 'magnet-dual-mixed', 'upload-v1'] if args.direction in ('all', direction)]
    write_json(ROOT / f'.build/v2-hybrid-{args.direction}-result.json', results)
    if args.direction == 'all':
        write_json(ROOT / '.build/v2-hybrid-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
