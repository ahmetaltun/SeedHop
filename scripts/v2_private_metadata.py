#!/usr/bin/env python3
"""Causal private-magnet teardown with withheld proofs and late KRPC responses.

All sources are independent loopback Python peers/trackers/DHT fixtures. Late DHT
responses introduce new nodes only after the public metadata peers have closed.
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
import time
import urllib.parse
from interop import ROOT, LAB, Tracker, bencode, stop, write_json
from v2_validation import fixture, digest
from v2_hardening import bdecode
from v2_hybrid_interop import PeerServer, read, frame, send, wire_content


class Sink(PeerServer):
    def __init__(self):
        self.accepted = self.closed = 0
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68)
        connection.sendall(handshake[:48] + b'-PySink-123456789012')
        with self.lock: self.accepted += 1
        try:
            while connection.recv(1024): pass
        finally:
            with self.lock: self.closed += 1


class PrivatePeer(PeerServer):
    def __init__(self, info, files):
        self.info = bencode(info)
        self.private = info.get(b'private') == 1
        self.extension_handshakes = []
        self.v2 = digest(self.info)[:20]
        self.v1 = hashlib.sha1(self.info).digest() if b'pieces' in info else None
        self.wire = wire_content(info, files) if self.v1 else None
        self.piece_length = info[b'piece length']
        self.pieces, self.trees = [], {}
        for _, data in files:
            self.pieces.extend(data[at:at + self.piece_length] for at in range(0, len(data), self.piece_length))
            if data:
                level = [digest(data[at:at + 16384]) for at in range(0, len(data), 16384)]
                level += [bytes(32)] * ((1 << (len(level) - 1).bit_length()) - len(level))
                tree = [level]
                while len(level) > 1:
                    level = [digest(level[at] + level[at + 1]) for at in range(0, len(level), 2)]
                    tree.append(level)
                self.trees[level[0]] = tree
        self.allow_dictionary, self.allow_proofs, self.proof_waiting = threading.Event(), threading.Event(), threading.Event()
        self.hash_requests = []
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68)
        identity = handshake[28:48]
        if handshake[:20] != b'\x13BitTorrent protocol' or identity not in (self.v1, self.v2): return
        connection.sendall(handshake[:48] + b'-PyPriv-123456789012')
        bits = bytearray((len(self.pieces) + 7) // 8)
        for index in range(len(self.pieces)): bits[index // 8] |= 128 >> (index % 8)
        send(connection, b'\x05' + bits)
        send(connection, b'\x01')
        send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': len(self.info)}))
        metadata_id = None
        while True:
            message = frame(connection)
            if message[:2] == b'\x14\x00':
                extensions = bdecode(message[2:]).get(b'm', {})
                metadata_id = extensions.get(b'ut_metadata', metadata_id)
                if self.private:
                    assert not extensions.get(b'ut_pex'), 'private peer advertised PEX'
                self.extension_handshakes.append(dict(metadataId=metadata_id, pexId=extensions.get(b'ut_pex')))

            elif message[:2] == b'\x14\x01':
                request = bdecode(message[2:]); assert request[b'msg_type'] == 0 and metadata_id
                assert self.allow_dictionary.wait(12), 'dictionary gate was not released'
                index = request[b'piece']; block = self.info[index * 16384:(index + 1) * 16384]; assert block
                send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': 1, b'piece': index, b'total_size': len(self.info)}) + block)
            elif message[:1] == b'\x15':
                root = message[1:33]; base, index, length, proof = struct.unpack('!IIII', message[33:]); tree = self.trees[root]
                self.hash_requests.append(dict(root=root.hex(), base=base, index=index, length=length, proof=proof))
                self.proof_waiting.set(); assert self.allow_proofs.wait(12), 'proof gate was not released'
                hashes = tree[base][index:index + length]; assert len(hashes) == length
                layer, node = base + length.bit_length() - 1, index // length
                while layer < base + proof + 1:
                    hashes.append(tree[layer][node ^ 1]); layer += 1; node //= 2
                send(connection, b'\x16' + message[1:] + b''.join(hashes))
            elif message[:1] == b'\x06':
                index, begin, length = struct.unpack('!III', message[1:])
                piece = self.pieces[index] if identity == self.v2 else self.wire[index * self.piece_length:(index + 1) * self.piece_length]
                assert 0 < length <= 16384 and begin + length <= len(piece)
                send(connection, b'\x07' + struct.pack('!II', index, begin) + piece[begin:begin + length])

    def close(self):
        self.allow_dictionary.set(); self.allow_proofs.set()
        super().close()


class LateDht:
    def __init__(self, work, hashes, peer):
        self.work, self.hashes, self.peer = work, set(hashes), peer
        self.events, self.errors, self.pending = [], [], []
        self.pending_hashes = set()
        self.released_replies = 0
        self.release, self.stopping = threading.Event(), threading.Event()
        self.bootstrap, self.probe = self.bind(), self.bind()
        self.held = {key: self.bind() for key in hashes}
        self.node_ids = {sock: key for key, sock in self.held.items()}
        self.sockets = [self.bootstrap, self.probe, *self.held.values()]
        self.port = self.bootstrap.getsockname()[1]
        self.thread = threading.Thread(target=self.run, daemon=True); self.thread.start()

    @staticmethod
    def bind():
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); sock.bind(('127.0.0.1', 0)); return sock

    @staticmethod
    def endpoint(sock):
        return socket.inet_aton('127.0.0.1') + struct.pack('!H', sock.getsockname()[1])

    def reply(self, sock, transaction, address, response):
        sock.sendto(bencode({b't': transaction, b'y': b'r', b'r': response}), address)

    def run(self):
        try:
            while not self.stopping.is_set():
                if self.release.is_set():
                    for sock, transaction, address, key in self.pending:
                        child = key[:-1] + bytes([key[-1] ^ 1])
                        self.reply(sock, transaction, address, {b'id': self.node_ids[sock], b'token': b'late-private-token', b'nodes': child + self.endpoint(self.probe)})
                        self.released_replies += 1
                    self.pending.clear()
                ready, _, _ = select.select(self.sockets, [], [], .02)
                for sock in ready:
                    packet, address = sock.recvfrom(65536); query = bdecode(packet)
                    if query.get(b'y') != b'q': continue
                    method, args = query[b'q'], query[b'a']; key = args.get(b'info_hash')
                    node = 'bootstrap' if sock is self.bootstrap else 'probe' if sock is self.probe else 'held'
                    self.events.append(dict(time=time.monotonic(), node=node, query=method.decode(), hash=key.hex() if key else None, afterRelease=self.release.is_set()))
                    base = digest(b'bootstrap-node')[:20]
                    response = {b'id': self.node_ids.get(sock, base)}
                    if method == b'get_peers' and key in self.hashes:
                        if node == 'bootstrap':
                            response.update({b'token': b'initial-token', b'values': [socket.inet_aton('127.0.0.1') + struct.pack('!H', self.peer)], b'nodes': key + self.endpoint(self.held[key])})
                        elif node == 'held' and not self.release.is_set():
                            self.pending.append((sock, query[b't'], address, key)); self.pending_hashes.add(key); continue
                        else:
                            response = {b'id': self.node_ids[sock] if node == 'held' else key[:-1] + bytes([key[-1] ^ 1]), b'token': b'after-private', b'nodes': b''}
                    elif method == b'find_node': response[b'nodes'] = base + self.endpoint(self.bootstrap) if node == 'bootstrap' else b''
                    self.reply(sock, query[b't'], address, response)
        except Exception as error:
            if not self.stopping.is_set(): self.errors.append(repr(error))

    def close(self):
        self.stopping.set(); self.thread.join(timeout=2)
        for sock in self.sockets: sock.close()
        write_json(self.work / 'dht-packets.json', self.events)
        assert not self.thread.is_alive() and not self.errors, self.errors


def wait_for(condition, message, timeout=8):
    deadline = time.monotonic() + timeout
    while not condition():
        if time.monotonic() >= deadline: raise AssertionError(message)
        time.sleep(.01)


def run_case(mode):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'private-metadata-{mode}-', dir=ROOT / '.build')); print(f'RUN {work}', flush=True)
    with contextlib.ExitStack() as cleanup:
        private_tracker, extra_tracker = Tracker(work), Tracker(work)
        cleanup.callback(private_tracker.close); cleanup.callback(extra_tracker.close)
        control = mode == 'public-control'
        hybrid = mode not in ('pure', 'public-control')
        _, files, info = fixture(work, hybrid, private_tracker.url, private=not control)
        source = PrivatePeer(info, files); cleanup.callback(source.close)
        sink = Sink(); cleanup.callback(sink.close)
        v2 = digest(bencode(info)); v1 = hashlib.sha1(bencode(info)).digest() if hybrid else None
        for tracker in [private_tracker, extra_tracker]:
            tracker.info_hash = v2[:20].hex()
            if hybrid: tracker.accepted_hashes.add(v1.hex())
        private_tracker.peer, extra_tracker.peer = source.port, sink.port
        hashes = [v1] if mode == 'btih' else [v2[:20]] + ([v1] if mode == 'dual' else [])
        dht = LateDht(work, hashes, source.port); cleanup.callback(dht.close)
        parameters = [('xt', 'urn:btih:' + v1.hex())] if mode == 'btih' else [('xt', 'urn:btmh:1220' + v2.hex())]
        if mode == 'dual': parameters.append(('xt', 'urn:btih:' + v1.hex()))
        parameters.append(('tr', private_tracker.url)); magnet = 'magnet:?' + urllib.parse.urlencode(parameters)
        downloads = work / 'downloads'; downloads.mkdir()
        with (work / 'seedhop.log').open('w') as log:
            engine = subprocess.Popen([str(LAB), 'probe-magnet-dht', magnet, str(downloads), f'127.0.0.1:{dht.port}', extra_tracker.url], stdout=log, stderr=log, text=True, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION='disabled'))
            cleanup.callback(stop, engine)
            wait_for(lambda: sink.accepted > 0 and dht.pending_hashes == set(hashes), 'public discovery did not reach sink and gated DHT nodes')
            source.allow_dictionary.set()
            wait_for(source.proof_waiting.is_set, 'Merkle proofs were not requested')
            if not control:
                wait_for(lambda: sink.closed == sink.accepted, 'unrelated public metadata connections did not close')
            time.sleep(.15)
            barrier = time.monotonic()
            accepted_at_barrier, closed_at_barrier = sink.accepted, sink.closed
            dht.release.set()
            time.sleep(.8)
            assert engine.poll() is None, 'proofs were deliberately withheld'
            descendants = [event for event in dht.events if event['node'] == 'probe' and event['query'] == 'get_peers']
            late_announces = [event for event in dht.events if event['node'] == 'held' and event['query'] == 'announce_peer' and event['afterRelease']]
            events = [json.loads(line) for line in (work / 'tracker.jsonl').read_text().splitlines()]
            extra_after_barrier = [event for event in events if event['time'] > barrier and event['returnedPort'] == sink.port and event['userAgent'].startswith('SeedHop/')]
            if control:
                assert descendants, 'public positive control did not follow the late DHT response'
                assert extra_after_barrier, 'public positive control did not keep its extra tracker'
            else:
                assert not descendants, 'late public response started descendant lookup'
                assert not late_announces, 'late response leaked a private announce'
                assert not extra_after_barrier, 'extra tracker kept announcing during private proof acquisition'
            source.allow_proofs.set()
            assert engine.wait(timeout=35) == 0
        for path, data in files: assert (downloads / path).read_bytes() == data
        diagnostics = (work / 'seedhop.log').read_text()
        assert ('private dictionary authenticated; public metadata discovery stopped' in diagnostics) != control
        record = dict(mode=mode, result='PASS', publicControl=control, infoHashV2=v2.hex(), bytes=sum(len(data) for _, data in files), unrelatedConnections=sink.accepted, closedConnections=sink.closed, acceptedBeforeProofRelease=accepted_at_barrier, closedBeforeProofRelease=closed_at_barrier, lateDhtReplies=dht.released_replies, gatedHashes=len(dht.pending_hashes), descendantQueries=len(descendants), lateAnnounces=len(late_announces), extraTrackerQueriesAfterBarrier=len(extra_after_barrier), barrierTime=barrier, hashRequests=source.hash_requests, extensionHandshakes=source.extension_handshakes, evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record); return record


def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--mode', choices=['public-control', 'pure', 'hybrid', 'dual', 'btih', 'all'], default='all'); args = parser.parse_args()
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = []
    try:
        for mode in ['public-control', 'pure', 'hybrid', 'dual', 'btih']:
            if args.mode in ('all', mode): results.append(run_case(mode))
    finally: write_json(ROOT / f'.build/v2-private-metadata-{args.mode}-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__': main()
