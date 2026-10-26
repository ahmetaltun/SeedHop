#!/usr/bin/env python3
"""Forced-uTP transfers through a lossy, reordering UDP relay with a blackout.

Uses isolated loopback qBittorrent profiles. The relay owns a UDP port only, so
neither peer can use TCP through it. Packet diagnostics and exact bytes are kept.
"""
import argparse
import contextlib
import hashlib
import heapq
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

from interop import ROOT, LAB, QBIT, QBittorrent, Tracker, bencode, stop, write_json
from v2_validation import fixture, digest


class ImpairedRelay:
    def __init__(self, work, target, legacy_hash=None):
        self.work, self.target, self.client = work, target, None
        self.legacy_hash, self.handshakes = legacy_hash, []
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(('127.0.0.1', 0))
        self.socket.settimeout(.003)
        self.port = self.socket.getsockname()[1]
        self.stopping = threading.Event()
        self.closed = False
        self.events, self.errors, self.pending, self.sequences = [], [], [], set()
        self.received = self.data = self.blackout_until = 0
        self.blackout_started = False
        self.ordinals, self.highest = {}, {}
        self.stats = dict(forwarded=0, dropped=0, delayed=0, reordered=0,
                          blackoutDropped=0, forwardedAfterBlackout=0, repeatedDataSequences=0)
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def forward(self, packet, destination, direction, ordinal):
        self.socket.sendto(packet, destination)
        self.stats['forwarded'] += 1
        if ordinal < self.highest.get(direction, 0):
            self.stats['reordered'] += 1
        self.highest[direction] = max(ordinal, self.highest.get(direction, 0))
        if self.blackout_started and time.monotonic() >= self.blackout_until:
            self.stats['forwardedAfterBlackout'] += 1

    def run(self):
        try:
            while not self.stopping.is_set():
                now = time.monotonic()
                while self.pending and self.pending[0][0] <= now:
                    _, _, packet, destination, direction, ordinal = heapq.heappop(self.pending)
                    if now < self.blackout_until:
                        self.stats['blackoutDropped'] += 1
                    else:
                        self.forward(packet, destination, direction, ordinal)
                try:
                    packet, address = self.socket.recvfrom(65536)
                except socket.timeout:
                    continue
                assert len(packet) >= 20 and packet[0] & 15 == 1, 'not a uTP v1 packet'
                direction = 'fromTarget' if address == self.target else 'toTarget'
                if direction == 'toTarget':
                    self.client = address
                destination = self.client if direction == 'fromTarget' else self.target
                assert destination is not None
                self.received += 1
                ordinal = self.ordinals.get(direction, 0) + 1
                self.ordinals[direction] = ordinal
                kind, connection, sequence = packet[0] >> 4, struct.unpack('!H', packet[2:4])[0], struct.unpack('!H', packet[16:18])[0]
                if kind == 0 and self.legacy_hash is not None:
                    extension, offset = packet[1], 20
                    while extension:
                        extension, length = packet[offset:offset + 2]
                        offset += 2 + length
                        assert offset <= len(packet), 'malformed uTP extension'
                    if packet[offset:offset + 20] == b'\x13BitTorrent protocol':
                        assert len(packet) - offset >= 68, 'fixture requires a complete handshake packet'
                        assert packet[offset + 28:offset + 48] == self.legacy_hash, 'v2 identity used on forced v1 wire'
                        modified = bytearray(packet)
                        modified[offset + 27] &= ~0x10
                        packet = bytes(modified)
                        self.handshakes.append(dict(direction=direction, infoHash=self.legacy_hash.hex(), v2UpgradeBit=False))
                event = dict(direction=direction, kind=kind, connection=connection,
                             sequence=sequence, length=len(packet), action='forward')
                if kind == 0:
                    self.data += 1
                    key = (direction, connection, sequence)
                    if key in self.sequences:
                        self.stats['repeatedDataSequences'] += 1
                    self.sequences.add(key)
                    if self.data >= 80 and not self.blackout_started:
                        self.blackout_started = True
                        self.blackout_until = time.monotonic() + .35
                if time.monotonic() < self.blackout_until:
                    self.stats['blackoutDropped'] += 1
                    event['action'] = 'blackout'
                elif kind == 0 and self.data % 37 == 0:
                    self.stats['dropped'] += 1
                    event['action'] = 'drop'
                elif kind == 0 and self.data % 11 == 0:
                    self.stats['delayed'] += 1
                    event['action'] = 'delay'
                    heapq.heappush(self.pending, (time.monotonic() + .025, self.received,
                                                 packet, destination, direction, ordinal))
                else:
                    self.forward(packet, destination, direction, ordinal)
                self.events.append(event)
        except Exception as error:
            self.errors.append(repr(error))

    def close(self):
        if self.closed:
            return
        self.closed = True
        self.stopping.set()
        self.thread.join(timeout=3)
        self.socket.close()
        write_json(self.work / 'relay-packets.json', self.events)
        write_json(self.work / 'relay-stats.json', self.stats)
        write_json(self.work / 'wire-handshakes.json', self.handshakes)
        assert not self.thread.is_alive() and not self.errors, self.errors


def run_case(direction, hybrid, encrypted, executable, legacy_wire=False):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'utp-impaired-{direction}-{"legacy" if legacy_wire else "hybrid" if hybrid else "pure"}-{"rc4" if encrypted else "plain"}-', dir=ROOT / '.build'))
    print(f'RUN {work}', flush=True)
    payload = hashlib.shake_256(b'SeedHop uTP impairment fixture').digest(2 * 1024 * 1024 + 173)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work)
        cleanup.callback(tracker.close)
        torrent, files, info = fixture(work, hybrid, tracker.url,
                                      files=[('a.bin', payload), ('marker', b'utp fixture marker')])
        identity = digest(bencode(info))[:20].hex()
        tracker.info_hash = identity
        if hybrid:
            tracker.accepted_hashes.add(hashlib.sha1(bencode(info)).hexdigest())
        legacy_hash = hashlib.sha1(bencode(info)).digest() if legacy_wire else None
        if legacy_wire:
            tracker.peers_by_hash = {legacy_hash.hex(): 0}
        peer = QBittorrent(work, executable, protocol=2, encryption=1 if encrypted else 2)
        cleanup.callback(peer.close)
        peer.start()
        # The engine may reconnect after a lost stream. Give its existing
        # backoff time to run, while retaining a bounded fixture deadline.
        env = dict(os.environ, SEEDHOP_DIAGNOSTICS='1', SEEDHOP_PEER_ENCRYPTION='require' if encrypted else 'disabled', SEEDHOP_TRANSFER_TIMEOUT_SECS='120')
        try:
            downloads = work / 'downloads'
            downloads.mkdir()
            with (work / 'seedhop.log').open('w+') as log:
                if direction == 'download':
                    peer.add(torrent, work / 'seed')
                    peer.complete(identity)
                    relay = ImpairedRelay(work, ('127.0.0.1', peer.peer), legacy_hash)
                    cleanup.callback(relay.close)
                    tracker.suppress_ports.add(peer.peer)
                    tracker.peer = relay.port
                    if legacy_wire:
                        tracker.peers_by_hash[legacy_hash.hex()] = relay.port
                    completed = subprocess.run([str(LAB), 'download', str(torrent), str(work / 'state'), str(downloads)], stdout=subprocess.PIPE, stderr=log, text=True, timeout=135, env=env)
                    log.write(completed.stdout)
                    completed.check_returncode()
                    for path, data in files:
                        assert (downloads / path).read_bytes() == data
                else:
                    engine = subprocess.Popen([str(LAB), 'seed', str(torrent), str(work / 'state'), str(work / 'seed/payload')], stdout=subprocess.PIPE, stderr=log, text=True, env=env)
                    cleanup.callback(stop, engine)
                    ready, _, _ = select.select([engine.stdout], [], [], 15)
                    assert ready, 'SeedHop recheck did not complete'
                    line = engine.stdout.readline()
                    assert line, 'SeedHop exited before seeding'
                    engine_port = json.loads(line)['listenPort']
                    relay = ImpairedRelay(work, ('127.0.0.1', peer.peer if legacy_wire else engine_port), legacy_hash)
                    cleanup.callback(relay.close)
                    tracker.suppress_ports.add(peer.peer if legacy_wire else engine_port)
                    peer.add(torrent, downloads)
                    tracker.peer = relay.port
                    if legacy_wire:
                        tracker.peers_by_hash[legacy_hash.hex()] = relay.port
                    peer.complete(identity, timeout=120)
                    for path, data in files:
                        assert (downloads / 'payload' / path).read_bytes() == data
                log.flush()
            relay.close()
            assert relay.stats['dropped'] and relay.stats['reordered'], relay.stats
            assert relay.stats['blackoutDropped'] and relay.stats['forwardedAfterBlackout'], relay.stats
            assert relay.stats['repeatedDataSequences'], 'no data retransmission observed'
            if legacy_wire:
                assert {h['direction'] for h in relay.handshakes} == {'toTarget', 'fromTarget'}, relay.handshakes
            if encrypted:
                diagnostics = (work / 'seedhop.log').read_text()
                assert 'MSE ' in diagnostics and 'rc4=true' in diagnostics
            result = dict(direction=direction, format='hybrid' if hybrid else 'v2', encryption='required' if encrypted else 'off', transport='utp', bytes=sum(len(data) for _, data in files), largeFileSha256=digest(payload).hex(), result='PASS', relay=dict(relay.stats), qBittorrent=peer.version, libtorrent=peer.build.get('libtorrent'), evidence=str(work.relative_to(ROOT)))
            result['forcedLegacyWire'] = legacy_wire
            result['wireHandshakes'] = relay.handshakes
            write_json(work / 'result.json', result)
            return result
        finally:
            peer.diagnostics()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--direction', choices=['download', 'upload', 'both'], default='both')
    parser.add_argument('--format', choices=['pure', 'hybrid', 'both'], default='both')
    parser.add_argument('--encryption', choices=['off', 'required', 'both'], default='both')
    parser.add_argument('--wire', choices=['negotiated', 'v1'], default='negotiated')
    parser.add_argument('--qbit', type=pathlib.Path, default=QBIT)
    args = parser.parse_args()
    if args.wire == 'v1' and (args.format != 'hybrid' or args.encryption != 'off'):
        parser.error('--wire v1 requires --format hybrid --encryption off')
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = []
    try:
        for direction in ['download', 'upload']:
            for hybrid in [False, True]:
                for encrypted in [False, True]:
                    if args.direction not in ('both', direction): continue
                    if args.format != 'both' and hybrid != (args.format == 'hybrid'): continue
                    if args.encryption != 'both' and encrypted != (args.encryption == 'required'): continue
                    results.append(run_case(direction, hybrid, encrypted, args.qbit, args.wire == 'v1'))
    finally:
        suffix = '-legacy' if args.wire == 'v1' else ''
        write_json(ROOT / f'.build/v2-utp-impairment{suffix}-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
