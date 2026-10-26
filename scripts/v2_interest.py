#!/usr/bin/env python3
"""Causal outgoing interest transitions through independent choked partial peers."""
import contextlib
import hashlib
import json
import os
import pathlib
import struct
import subprocess
import tempfile
import threading
from interop import ROOT, LAB, Tracker, bencode, write_json
from v2_hybrid_interop import PeerServer, read, frame, send


class PartialPeer(PeerServer):
    def __init__(self, identity, data, observer, interested):
        self.identity,self.data,self.observer,self.interested=identity,data,observer,interested
        self.sequence=[];self.payload=[];self.unchoked=False
        super().__init__()

    def serve(self,connection):
        handshake=read(connection,68);assert handshake[28:48]==self.identity
        reserved=bytearray(handshake[20:28]);reserved[7]&=~0x10
        connection.sendall(handshake[:20]+reserved+self.identity+(b'-PyObs-1234567890123' if self.observer else b'-PySrc-1234567890123'))
        send(connection,b'\x05\x80')
        if not self.observer:send(connection,b'\x01');self.unchoked=True
        phase=0
        while True:
            message=frame(connection)
            if message[:1]==b'\x02':
                self.sequence.append('interested')
                if self.observer:
                    if phase==0:self.interested.set();phase=1
                    elif phase==2:send(connection,b'\x01');self.unchoked=True;phase=3
                    else:raise AssertionError(f'duplicate interest in phase {phase}')
            elif message[:1]==b'\x03':
                self.sequence.append('not-interested')
                if self.observer:
                    assert phase==1,'interest dropped without the other peer supplying its only piece'
                    assert not self.unchoked,'observer must remain choked during the change'
                    phase=2
                    send(connection,b'\x04'+struct.pack('!I',1));send(connection,b'\x04'+struct.pack('!I',2))
            elif message[:1]==b'\x06':
                index,begin,length=struct.unpack('!III',message[1:])
                assert self.unchoked and index in ([1,2] if self.observer else [0])
                if not self.observer:assert self.interested.wait(5),'observer did not express initial interest while choked'
                block=self.data[index][begin:begin+length];assert len(block)==length
                self.payload.append(dict(index=index,begin=begin,length=length))
                send(connection,b'\x07'+struct.pack('!II',index,begin)+block)


def run_case(mode):
    hybrid=mode!='pure';legacy=mode=='hybrid-v1'
    work=pathlib.Path(tempfile.mkdtemp(prefix=f'interest-{mode}-',dir=ROOT/'.build'))
    print('RUN',mode,work,flush=True)
    data=[bytes([7+i])*32768 for i in range(3)]
    tree={bytes([97+i]):{b'':{b'length':len(block),b'pieces root':hashlib.sha256(hashlib.sha256(block[:16384]).digest()+hashlib.sha256(block[16384:]).digest()).digest()}} for i,block in enumerate(data)}
    info={b'file tree':tree,b'meta version':2,b'name':b'interest',b'piece length':32768}
    if hybrid:
        info[b'files']=[{b'length':32768,b'path':[bytes([97+i])]} for i in range(3)]
        info[b'pieces']=b''.join(hashlib.sha1(block).digest() for block in data)
    v2=hashlib.sha256(bencode(info)).hexdigest();v1=hashlib.sha1(bencode(info)).hexdigest();wire=bytes.fromhex(v1 if legacy else v2[:40])
    with contextlib.ExitStack() as cleanup:
        tracker=Tracker(work);cleanup.callback(tracker.close)
        tracker.info_hash=v2[:40];tracker.accepted_hashes={v1} if hybrid else set()
        interested=threading.Event();observer=PartialPeer(wire,data,True,interested);source=PartialPeer(wire,data,False,interested)
        cleanup.callback(observer.close);cleanup.callback(source.close)
        tracker.peers_by_hash={wire.hex():[observer.port,source.port]}
        torrent=work/'fixture.torrent';torrent.write_bytes(bencode({b'announce':tracker.url.encode(),b'info':info,b'piece layers':{}}))
        (work/'downloads').mkdir()
        with (work/'seedhop.log').open('w') as log:
            result=subprocess.run([str(LAB),'download',str(torrent),str(work/'state'),str(work/'downloads')],stdout=subprocess.PIPE,stderr=log,text=True,timeout=35,env=dict(os.environ,SEEDHOP_DIAGNOSTICS='1'))
            log.write(result.stdout);assert result.returncode==0,(work/'seedhop.log').read_text()[-2500:]
        assert observer.sequence[:3]==['interested','not-interested','interested'],observer.sequence
        assert {r['index'] for r in observer.payload}=={1,2} and {r['index'] for r in source.payload}=={0}
        for i,block in enumerate(data):assert (work/'downloads'/chr(97+i)).read_bytes()==block
        record=dict(mode=mode,result='PASS',infoHashV2=v2,wireHash=wire.hex(),observerStates=observer.sequence,observerRequests=observer.payload,sourceRequests=source.payload,evidence=str(work.relative_to(ROOT)))
        write_json(work/'result.json',record);return record


def main():
    subprocess.run(['cargo','build','--locked','-p','seedhop-core','--bin','seedhop-lab'],cwd=ROOT,check=True)
    results=[]
    try:
        for mode in ['pure','hybrid-v2','hybrid-v1']:results.append(run_case(mode))
    finally:write_json(ROOT/'.build/v2-interest-result.json',results)
    print(json.dumps(results,indent=2))


if __name__=='__main__':main()
