#!/usr/bin/env python3
"""Independent raw-name v2/hybrid transfers and uploads through sanitized storage."""
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
import urllib.parse
from interop import ROOT, LAB, Tracker, bencode, stop, write_json
from v2_hardening import bdecode
from v2_hybrid_interop import PeerServer, read, frame, send, v1_download


def fixture(hybrid, announce):
    names = [b'..', b'.', b'bad/name', b'bad\\name', b'bad\0name', b'\xff', b'\xc0\xae\xc0\xae',
             b'%FF', b'CON.txt', b'COM1', b'tail. ', b'A', b'a', b'%hfake', b'x' * 256, b'x' * 257,
             'é'.encode(), 'e\u0301'.encode(), 'ß'.encode(), b'ss', 'a\u200d'.encode(), '\u200d'.encode(),
             '.\u200d'.encode(), '🏠.txt'.encode(), '\ufeffhello'.encode(), b'hello']
    paths = [(name,) for name in names] + [(b'Dir', b'file'), (b'dir', b'FILE'), (b'..', b'escape')]
    # A raw path cannot describe both a file and a directory.
    paths.remove((b'..',))
    entries = [(path, bytes([i + 1]) * (123 + i)) for i, path in enumerate(sorted(paths))]
    tree, files, content = {}, [], bytearray()
    for path, data in entries:
        node = tree
        for part in path: node = node.setdefault(part, {})
        node[b''] = {b'length': len(data), b'pieces root': hashlib.sha256(data).digest()}
        if hybrid:
            files.append({b'length': len(data), b'path': list(path)})
            content.extend(data)
            padding = 32768 - len(data)
            files.append({b'attr': b'p', b'length': padding, b'path': [b'.pad', str(padding).encode()]})
            content.extend(bytes(padding))
    info = {b'file tree': tree, b'meta version': 2, b'name': b'filename-fixture', b'piece length': 32768}
    if hybrid:
        info[b'files'] = files
        info[b'pieces'] = b''.join(hashlib.sha1(content[i:i+32768]).digest() for i in range(0, len(content), 32768))
    torrent = bencode({b'announce': announce.encode(), b'info': info, b'piece layers': {}})
    return torrent, info, entries, bytes(content)


class Seed(PeerServer):
    def __init__(self, info, entries, wire, legacy_only):
        self.info, self.entries, self.wire, self.legacy_only = bencode(info), entries, wire, legacy_only
        self.v2 = hashlib.sha256(self.info).digest()[:20]
        self.v1 = hashlib.sha1(self.info).digest() if wire else None
        self.identities, self.metadata = [], []
        super().__init__()

    def serve(self, connection):
        handshake = read(connection, 68); identity = handshake[28:48]
        if identity not in (self.v1, self.v2) or (self.legacy_only and identity != self.v1): return
        reserved = bytearray(handshake[20:28]); reserved[7] &= ~0x10
        connection.sendall(handshake[:20] + reserved + identity + b'-PyName-123456789012')
        self.identities.append(identity.hex())
        bits = bytearray((len(self.entries) + 7) // 8)
        for index in range(len(self.entries)): bits[index // 8] |= 128 >> (index % 8)
        send(connection, b'\x05' + bits); send(connection, b'\x01')
        send(connection, b'\x14\x00' + bencode({b'm': {b'ut_metadata': 1}, b'metadata_size': len(self.info)}))
        metadata_id = None
        while True:
            message = frame(connection)
            if message[:2] == b'\x14\x00': metadata_id = bdecode(message[2:])[b'm'][b'ut_metadata']
            elif message[:2] == b'\x14\x01':
                request = bdecode(message[2:]); assert request[b'msg_type'] == 0 and metadata_id
                index = request[b'piece']; self.metadata.append(index)
                block = self.info[index*16384:(index+1)*16384]
                send(connection, bytes([20,metadata_id]) + bencode({b'msg_type':1,b'piece':index,b'total_size':len(self.info)}) + block)
            elif message[:1] == b'\x06':
                index, begin, length = struct.unpack('!III', message[1:]); self.requests.append((identity.hex(),index,begin,length))
                piece = self.entries[index][1] if identity == self.v2 else self.wire[index*32768:(index+1)*32768]
                assert 0 < length <= 16384 and begin + length <= len(piece)
                send(connection, b'\x07' + struct.pack('!II',index,begin) + piece[begin:begin+length])


def upload_v2(port, info, entries):
    identity = hashlib.sha256(bencode(info)).digest()[:20]
    with socket.create_connection(('127.0.0.1',port),timeout=5) as connection:
        connection.settimeout(10)
        connection.sendall(b'\x13BitTorrent protocol' + bytes(8) + identity + b'-PyRead-123456789012')
        assert read(connection,68)[28:48] == identity
        while frame(connection)[:1] != b'\x01': pass
        send(connection,b'\x02')
        for index,(_,data) in enumerate(entries):
            send(connection,b'\x06'+struct.pack('!III',index,0,len(data)))
            message = frame(connection)
            assert message == b'\x07'+struct.pack('!II',index,0)+data
    return len(entries)


def run_case(mode):
    hybrid = mode.startswith('hybrid'); legacy_only = mode.endswith(('btih','dual'))
    work = pathlib.Path(tempfile.mkdtemp(prefix=f'filenames-{mode}-',dir=ROOT/'.build'))
    print('RUN',mode,work,flush=True)
    with contextlib.ExitStack() as cleanup:
        tracker = Tracker(work); cleanup.callback(tracker.close)
        torrent,info,entries,wire = fixture(hybrid,tracker.url)
        v2 = hashlib.sha256(bencode(info)).hexdigest(); v1 = hashlib.sha1(bencode(info)).hexdigest()
        tracker.info_hash = v2[:40]; tracker.accepted_hashes = {v1} if hybrid else set()
        peer = Seed(info,entries,wire,legacy_only); cleanup.callback(peer.close)
        tracker.peers_by_hash = {v1:[peer.port]} if legacy_only else {v2[:40]:[peer.port]}
        torrent_path = work/'fixture.torrent'; torrent_path.write_bytes(torrent)
        source = str(torrent_path); command = 'download'
        if mode.endswith(('magnet','btih','dual')):
            parameters = [('xt','urn:btih:'+v1)] if legacy_only else [('xt','urn:btmh:1220'+v2)]
            if mode.endswith('dual'): parameters.append(('xt','urn:btmh:1220'+v2))
            source = 'magnet:?'+urllib.parse.urlencode(parameters+[('tr',tracker.url)]); command = 'download-magnet'
        downloads = work/'downloads'; downloads.mkdir()
        with (work/'seedhop.log').open('w') as log:
            result = subprocess.run([str(LAB),command,source,str(work/'state'),str(downloads)],stdout=subprocess.PIPE,stderr=log,text=True,timeout=60,env=dict(os.environ,SEEDHOP_DIAGNOSTICS='1'))
            log.write(result.stdout); assert result.returncode == 0,(work/'seedhop.log').read_text()[-2500:]
        item = json.loads(result.stdout)['torrents'][0]
        assert item['id'] == v2 and item['downloadedBytes'] == sum(len(data) for _,data in entries)
        paths, inodes, mappings = set(),set(),[]
        for file,(raw,data) in zip(item['files'],entries):
            name = file['name']; path = downloads/name
            assert not pathlib.PurePosixPath(name).is_absolute() and '..' not in pathlib.PurePosixPath(name).parts
            assert path.resolve().is_relative_to(downloads.resolve()) and path.read_bytes() == data
            paths.add(name); inodes.add(path.stat().st_ino)
            mappings.append(dict(raw=[part.hex() for part in raw],storage=name,bytes=len(data)))
        assert len(paths) == len(inodes) == len(entries)
        assert '🏠.txt' in paths, 'valid UTF-8 was reinterpreted through a legacy encoding'
        assert '%FF' in paths and '%25FF' in paths
        assert {str(p.relative_to(downloads)) for p in downloads.rglob('*') if p.is_file()} == paths
        uploads = {}
        if mode.endswith('file'):
            with (work/'upload.log').open('w') as log:
                process = subprocess.Popen([str(LAB),'seed',str(torrent_path),str(work/'upload-state'),str(downloads)],stdout=subprocess.PIPE,stderr=log,text=True)
                cleanup.callback(lambda:stop(process))
                assert select.select([process.stdout],[],[],12)[0],'seed did not verify sanitized paths'
                snapshot = json.loads(process.stdout.readline()); port = snapshot['listenPort']
                uploads['v2Requests'] = upload_v2(port,info,entries)
                if hybrid: uploads['v1Requests'] = v1_download(port,info,wire)
                stop(process)
        record = dict(mode=mode,result='PASS',bytes=item['totalBytes'],files=len(entries),infoHashV2=v2,metadataRequests=peer.metadata,wireIdentities=peer.identities,uploads=uploads,mappings=mappings,evidence=str(work.relative_to(ROOT)))
        write_json(work/'result.json',record); return record


def main():
    subprocess.run(['cargo','build','--locked','-p','seedhop-core','--bin','seedhop-lab'],cwd=ROOT,check=True)
    results=[]
    try:
        for mode in ['pure-file','pure-magnet','hybrid-file','hybrid-btih','hybrid-dual']: results.append(run_case(mode))
    finally: write_json(ROOT/'.build/v2-filenames-result.json',results)
    print(json.dumps(results,indent=2))


if __name__=='__main__': main()
