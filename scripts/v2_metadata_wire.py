#!/usr/bin/env python3
"""Independent BEP 9 extensibility, size consistency and rejection recovery."""
import contextlib
import json
import os
import pathlib
import subprocess
import tempfile
import threading
import time
import urllib.parse
from interop import ROOT, LAB, Tracker, bencode, stop, write_json
from v2_validation import fixture, digest
from v2_hardening import bdecode
from v2_hybrid_interop import PeerServer, read, frame, send


class MetadataPeer(PeerServer):
    def __init__(self, info, mode, slow=False):
        self.info, self.mode, self.slow = bencode(info), mode, slow
        self.requests, self.rejects = [], []
        self.closed = threading.Event()
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68)
        assert handshake[28:48] == digest(self.info)[:20]
        connection.sendall(handshake[:48] + b'-PyMeta-123456789012')
        size = 0 if self.mode == 'zero' else len(self.info)
        send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': size}))
        metadata_id, pending = None, None
        try:
            while True:
                message = frame(connection)
                assert message[:1] not in (b'\x01', b'\x02'), 'metadata-only peer advertised payload unchoke/interest'
                if message[:2] == b'\x14\x00':
                    metadata_id = bdecode(message[2:])[b'm'][b'ut_metadata']
                elif message[:2] == b'\x14\x01':
                    request = bdecode(message[2:]); index = request[b'piece']
                    if request[b'msg_type'] == 2:
                        self.rejects.append(index)
                        if pending is not None: send(connection, pending); pending = None
                        continue
                    assert request[b'msg_type'] == 0 and metadata_id
                    self.requests.append(index)
                    if self.slow:
                        for _ in range(12):
                            send(connection, b'')
                            time.sleep(1)
                    if self.mode == 'reject':
                        send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': 2, b'piece': index}))
                        continue
                    block = self.info[index * 16384:(index + 1) * 16384]; assert block
                    if self.mode == 'healthy' and index == 0:
                        # Unknown IDs have no piece field and may have opaque
                        # trailing data; they must not close the connection.
                        for kind in [3, 4294967296, -1]:
                            send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': kind}) + b'future payload')
                        send(connection, bytes([20, metadata_id]) + bencode({b'msg_type': 0, b'piece': 7}))
                        # An extension update must not reset dictionary state.
                        send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': len(self.info)}))
                    total = len(self.info) + (self.mode == 'changed' and index == 0)
                    response = bytes([20, metadata_id]) + bencode({b'future': b'x' * 200, b'msg_type': 1, b'piece': index, b'total_size': total}) + block
                    if self.mode == 'healthy' and index == 1 and 7 not in self.rejects:
                        pending = response
                    else:
                        send(connection, response)
        finally:
            self.closed.set()


def run_case(mode):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'metadata-wire-{mode}-', dir=ROOT / '.build'))
    print('RUN', work, flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work); cleanup.callback(tracker.close)
        _, _, info = fixture(work, False, tracker.url)
        # Two metadata blocks, with no missing Merkle layers after inspection.
        # fixture has real layers, so use a small file whose root is sufficient.
        tree = info[b'file tree']
        # Generate one-file info directly from the existing short-file root.
        leaves = []
        def walk(node):
            if b'' in node: leaves.append(node[b''])
            else:
                for child in node.values(): walk(child)
        walk(tree)
        props = next(p for p in leaves if 0 < p[b'length'] <= info[b'piece length'])
        info = {b'file tree': {b'file': {b'': props}}, b'meta version': 2, b'name': b'wire', b'piece length': info[b'piece length'], b'future info': b'x' * 18000}
        expected = digest(bencode(info)).hex(); tracker.info_hash = expected[:40]
        healthy = MetadataPeer(info, 'healthy', slow=mode == 'progress'); cleanup.callback(healthy.close)
        bad = None if mode in ('healthy', 'progress') else MetadataPeer(info, mode)
        if bad: cleanup.callback(bad.close)
        tracker.peer = (bad or healthy).port
        source = 'magnet:?' + urllib.parse.urlencode([('xt', 'urn:btmh:1220' + expected), ('tr', tracker.url)])
        downloads = work / 'downloads'; downloads.mkdir()
        with (work / 'seedhop.log').open('w') as log:
            process = subprocess.Popen([str(LAB), 'inspect-magnet', source, str(work / 'state'), str(downloads)], stdout=subprocess.PIPE, stderr=log, text=True, env=dict(os.environ, SEEDHOP_DIAGNOSTICS='1'))
            cleanup.callback(lambda: stop(process))
            if bad:
                assert bad.closed.wait(3), f'{mode}: invalid metadata peer was not rejected promptly'
                tracker.peer = healthy.port
            output, _ = process.communicate(timeout=35 if mode == 'progress' else 20); log.write(output)
            assert process.returncode == 0, (work / 'seedhop.log').read_text()[-2000:]
        result = json.loads(output)
        assert result['metadata']['id'] == expected and not result['snapshot']['torrents']
        assert not any(p.is_file() for p in downloads.rglob('*'))
        assert healthy.requests == [0, 1], healthy.requests
        assert healthy.rejects == [7], healthy.rejects
        record = dict(mode=mode, result='PASS', healthyRequests=healthy.requests, metadataRejects=healthy.rejects, badRequests=bad.requests if bad else [], evidence=str(work.relative_to(ROOT)))
        write_json(work / 'result.json', record)
        return record


def main():
    subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab'], cwd=ROOT, check=True)
    results = []
    try:
        for mode in ['healthy', 'changed', 'zero', 'reject', 'progress']: results.append(run_case(mode))
    finally:
        write_json(ROOT / '.build/v2-metadata-wire-result.json', results)
    print(json.dumps(results, indent=2))


if __name__ == '__main__': main()
