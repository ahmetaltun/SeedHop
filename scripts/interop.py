#!/usr/bin/env python3
"""Local-only qBittorrent interoperability. Never touches the user's qBittorrent profile."""
import hashlib
import base64
import http.server
import json
import os
import pathlib
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
QBIT = pathlib.Path("/Applications/qbittorrent.app/Contents/MacOS/qbittorrent")

def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1",0))
        return s.getsockname()[1]

def main():
    if not QBIT.exists(): raise SystemExit("qBittorrent is required at /Applications/qbittorrent.app")
    subprocess.run(["cargo","build","--locked","--offline","-p","seedhop-core","--bin","seedhop-lab"],cwd=ROOT,check=True)
    lab = ROOT/"target/debug/seedhop-lab"
    (ROOT/".build").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="interop-",dir=ROOT/".build",delete=False) as temporary:
        work = pathlib.Path(temporary)
        seed = work/"seed"; downloads = work/"downloads"; downloads.mkdir()
        subprocess.run([lab,"fixture",seed],check=True)
        web,peer,tracker = port(),port(),port()
        advertised_peer=0
        class Tracker(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                with open(work/"tracker.log","a") as tracklog: tracklog.write(self.headers.get("User-Agent","")+" "+self.path+"\n")
                compact = socket.inet_aton("127.0.0.1") + struct.pack("!H",advertised_peer) if advertised_peer and "qBittorrent" in self.headers.get("User-Agent","") else b""
                body = b"d8:intervali2e5:peers" + str(len(compact)).encode() + b":" + compact + b"e"
                self.send_response(200); self.end_headers(); self.wfile.write(body)
            def log_message(self,*args): pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1",tracker),Tracker)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        announce = f"http://127.0.0.1:{tracker}/announce".encode()
        original = (seed/"fixture.torrent").read_bytes()
        torrent = seed/"tracked.torrent"
        torrent.write_bytes(b"d8:announce"+str(len(announce)).encode()+b":"+announce+original[1:])
        profile = work/"profile"; conf = profile/"qBittorrent/config"; conf.mkdir(parents=True)
        salt=os.urandom(16)
        password_hash=base64.b64encode(salt).decode()+":"+base64.b64encode(hashlib.pbkdf2_hmac("sha512",b"seedhop-local-test",salt,100000)).decode()
        (conf/"qBittorrent.ini").write_text(f"""[LegalNotice]
Accepted=true
[Preferences]
WebUI\\Enabled=true
WebUI\\Address=127.0.0.1
WebUI\\Port={web}
WebUI\\Username=seedhop-test
WebUI\\Password_PBKDF2=@ByteArray({password_hash})
WebUI\\LocalHostAuth=false
WebUI\\CSRFProtection=false
WebUI\\HostHeaderValidation=false
Connection\\PortRangeMin={peer}
Connection\\UPnP=false
Connection\\Interface=
Connection\\InterfaceName=
Connection\\InterfaceAddress=
Connection\\GlobalDLLimitAlt=0
Connection\\GlobalUPLimitAlt=0
Downloads\\SavePath={downloads}/
[BitTorrent]
Session\\DHTEnabled=false
Session\\UPnPEnabled=false
Session\\LSDEnabled=false
Session\\PeXEnabled=false
Session\\Port={peer}
Session\\GlobalMaxInactiveSeedingMinutes=-1
Session\\Interface=
Session\\InterfaceAddress=
""")
        log = open(work/"qbit.log","w+")
        env = dict(os.environ,QT_QPA_PLATFORM="cocoa")
        process = subprocess.Popen([QBIT,"--confirm-legal-notice","--no-splash",f"--profile={profile}",f"--webui-port={web}",f"--torrenting-port={peer}"],env=env,stdout=log,stderr=log)
        base = f"http://127.0.0.1:{web}/api/v2"
        def api(path,values=None):
            data = urllib.parse.urlencode(values).encode() if values is not None else None
            with urllib.request.urlopen(base+path,data=data,timeout=3) as response: return response.read()
        engine_process=None
        engine_log=open(work/"seedhop.log","w")
        try:
            deadline = time.monotonic()+20
            while True:
                try: version = api("/app/version").decode(); break
                except Exception:
                    if process.poll() is not None or time.monotonic()>deadline:
                        log.seek(0); raise RuntimeError("Isolated qBittorrent did not start. Profile: "+str(profile)+"\n"+log.read()[-1500:])
                    time.sleep(0.2)
            api("/app/setPreferences",{"json":json.dumps({"encryption":2,"queueing_enabled":False,"max_ratio":-1,"max_seeding_time":-1,"max_inactive_seeding_time":-1,"dht":False,"pex":False,"lsd":False,"upnp":False,"resolve_peer_countries":False})})
            prefs=json.loads(api("/app/preferences"))
            (work/"peer-settings.json").write_text(json.dumps({key:prefs.get(key) for key in ["encryption","queueing_enabled","max_ratio","max_seeding_time","current_network_interface","current_interface_address"]},indent=2))
            engine_process=subprocess.Popen([lab,"seed",torrent,work/"seed-state",seed],env=dict(os.environ,SEEDHOP_DIAGNOSTICS="1"),stdout=subprocess.PIPE,stderr=engine_log,text=True)
            ready=engine_process.stdout.readline()
            if not ready: raise RuntimeError("SeedHop seed failed; see "+str(work/"seedhop.log"))
            advertised_peer=json.loads(ready)["listenPort"]
            boundary="seedhop-local-fixture"
            body=(f"--{boundary}\r\nContent-Disposition: form-data; name=\"savepath\"\r\n\r\n{downloads}/\r\n"
                  f"--{boundary}\r\nContent-Disposition: form-data; name=\"stopped\"\r\n\r\nfalse\r\n"
                  f"--{boundary}\r\nContent-Disposition: form-data; name=\"torrents\"; filename=\"fixture.torrent\"\r\nContent-Type: application/x-bittorrent\r\n\r\n").encode()+torrent.read_bytes()+f"\r\n--{boundary}--\r\n".encode()
            req=urllib.request.Request(base+"/torrents/add",data=body,headers={"Content-Type":f"multipart/form-data; boundary={boundary}"})
            with urllib.request.urlopen(req,timeout=5) as response:
                added=response.read().strip()
                if added.startswith(b"{"):
                    outcome=json.loads(added)
                    if outcome.get("failure_count",0) or outcome.get("success_count",0)!=1: raise RuntimeError("qBittorrent rejected fixture: "+repr(added[:300]))
                elif added not in (b"Ok.",b"Ok",b""): raise RuntimeError("qBittorrent rejected fixture: "+repr(added[:300]))
            deadline=time.monotonic()+30
            while True:
                items=json.loads(api("/torrents/info"))
                if items and items[0]["progress"]==1: break
                if time.monotonic()>deadline: raise RuntimeError(f"qBittorrent did not download fixture: {items}; logs at {work}")
                time.sleep(0.2)
            (work/"qbit-before.json").write_bytes(api("/torrents/info"))
            original_hash=hashlib.sha256((seed/"seedhop-fixture.bin").read_bytes()).hexdigest()
            downloaded_hash=hashlib.sha256((downloads/"seedhop-fixture.bin").read_bytes()).hexdigest()
            assert original_hash==downloaded_hash
            report={"peer":version,"direction":"SeedHop → qBittorrent","bytes":1048576,"sha256":downloaded_hash,"result":"PASS","network":"loopback peer traffic"}
            (ROOT/".build/interop-result.json").write_text(json.dumps(report,indent=2)+"\n")
            print(json.dumps(report,indent=2))
        finally:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
            if engine_process is not None:
                engine_process.terminate()
                try: engine_process.wait(timeout=10)
                except subprocess.TimeoutExpired: engine_process.kill(); engine_process.wait()
            server.shutdown(); log.close(); engine_log.close()

if __name__=="__main__": main()
