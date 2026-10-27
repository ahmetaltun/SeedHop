#!/usr/bin/env python3
"""Kill isolated engines at durable layout boundaries and verify restart recovery."""
import argparse
import hashlib
import json
import os
import pathlib
import selectors
import shutil
import subprocess
import tempfile
import time

from interop import ROOT, bencode
from v2_validation import fixture

LAB = ROOT / '.build/layout-lab'
STAGES = ('journal', 'created', 'copy', 'copied', 'saving', 'commit', 'quarantine', 'cleanup')
COMMITTED = {'commit', 'quarantine', 'cleanup'}


def command(*args):
    result = subprocess.run([str(LAB), *map(str, args)], text=True, capture_output=True, timeout=45)
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed: {result.stderr}\n{result.stdout}')
    return json.loads(result.stdout)


def kill_at(work, state, source, target, operation, stage):
    env = dict(os.environ, SEEDHOP_LAYOUT_CHECKPOINT=stage)
    with (work / 'interrupted-engine.log').open('w') as log:
        process = subprocess.Popen([str(LAB), 'layout-change', str(state), str(source), str(target), operation],
                                   stdout=subprocess.PIPE, stderr=log, text=True, env=env)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    if selector.select(0.1):
                        line = process.stdout.readline()
                        log.write(line)
                        if line.strip() == f'LAYOUT CHECKPOINT {stage}':
                            process.kill()
                            process.wait(timeout=10)
                            assert process.returncode == -9
                            return
                    if process.poll() is not None:
                        raise RuntimeError(f'Engine exited before {stage}; see {work}')
                raise TimeoutError(f'No {stage} checkpoint; see {work}')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            process.stdout.close()


def prepare(work, kind, intent, target_parent=None):
    work.mkdir(parents=True)
    state, source = work / 'state', work / 'source'
    target = (target_parent / work.name) if target_parent else work / 'destination'
    source.mkdir(); target.mkdir(parents=True)
    files = [('a.txt', b'a' * 32768), ('folder/payload.bin', bytes(i % 251 for i in range(2 * 1024 * 1024 + 17)))]
    if kind == 'v1':
        payload = b''.join(data for _, data in files)
        info = {b'name': b'payload', b'piece length': 32768,
                b'files': [{b'length': len(data), b'path': name.encode().split(b'/')} for name, data in files],
                b'pieces': b''.join(hashlib.sha1(payload[i:i + 32768]).digest() for i in range(0, len(payload), 32768))}
        torrent = work / 'fixture.torrent'
        torrent.write_bytes(bencode({b'info': info}))
    else:
        torrent, _, _ = fixture(work, kind == 'hybrid', files=files)
    preview = command('layout-inspect', torrent)
    for item, (_, data) in zip(preview['files'], files):
        path = source / item['name']; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
    before = command('layout-prepare', state, source, torrent, intent)
    transfer = next(item for item in before['torrents'] if item['name'] != 'queue-blocker')
    original = {item['name']: (source / item['name']).read_bytes() for item in transfer['files']}
    (work / 'before.json').write_text(json.dumps(before, indent=2))
    return state, source, target, transfer, original


def verify(work, kind, operation, stage, intent='paused', target_parent=None, legacy=False):
    state, source, target, before, original = prepare(work, kind, intent, target_parent)
    if legacy:
        session_path = state / 'session/session.json'
        session = json.loads(session_path.read_text())
        saved = next(item for item in session['torrents'].values() if item.get('info_hash_v2') == before['id'])
        saved.pop('file_name_mapping_version', None)
        saved['file_names'] = {}
        session_path.write_text(json.dumps(session))
    kill_at(work, state, source, target, operation, stage)
    snapshot = command('layout-status', state, source)
    (work / 'after.json').write_text(json.dumps(snapshot, indent=2))
    assert snapshot['storageRecovery'] is None, snapshot['storageRecovery']
    after = next(item for item in snapshot['torrents'] if item['id'] == before['id'])
    assert after['pausedIntent'] == before['pausedIntent']
    assert after['queuePosition'] == before['queuePosition']
    assert after['state'] == before['state'], (before['state'], after['state'])
    assert [item['selected'] for item in after['files']] == [item['selected'] for item in before['files']]
    assert [item['priority'] for item in after['files']] == [item['priority'] for item in before['files']]
    committed = stage in COMMITTED
    for index, item in enumerate(before['files']):
        changed = operation == 'relocate' or index == len(before['files']) - 1
        new_name = 'renamed.bin' if operation == 'rename' and changed else item['name']
        active_root = target if operation == 'relocate' and committed else source
        active_name = new_name if committed else item['name']
        assert (active_root / active_name).read_bytes() == original[item['name']]
        if changed:
            inactive = source / item['name'] if committed else ((target if operation == 'relocate' else source) / new_name)
            assert not inactive.exists(), inactive
    assert not (state / 'layout-transaction.json').exists()
    assert not list(source.rglob('.seedhop-*')) and not list(target.rglob('.seedhop-*'))
    again = command('layout-status', state, source)
    assert again['storageRecovery'] is None
    result = {'format': kind, 'operation': operation, 'stage': stage, 'intent': intent,
              'passed': True, 'legacyFilenameMap': legacy, 'sourceDevice': source.stat().st_dev, 'destinationDevice': target.stat().st_dev}
    if target_parent: assert result['sourceDevice'] != result['destinationDevice']
    print(f'PASS: {work.name}', flush=True)
    return result


def retained_copy(work):
    state, source, target, before, original = prepare(work, 'v2', 'paused')
    kill_at(work, state, source, target, 'relocate', 'copied')
    first = before['files'][0]['name']
    (target / first).write_bytes(b'external edit')
    snapshot = command('layout-status', state, source)
    assert snapshot['storageRecovery']['activeLocation'] == 'original'
    assert snapshot['storageRecovery']['canKeepCopies']
    assert (target / first).read_bytes() == b'external edit'
    for name, data in original.items(): assert (source / name).read_bytes() == data
    kept = command('layout-resolve', state, source, 'keep')
    assert kept['storageRecovery'] is None
    assert (target / first).read_bytes() == b'external edit'
    assert not (state / 'layout-transaction.json').exists()
    print('PASS: edited-copy-retained-and-kept', flush=True)
    return {'case': 'edited-copy-retained-and-kept', 'passed': True}


def replaced_active_root(work):
    state, source, target, before, original = prepare(work, 'v2', 'paused')
    kill_at(work, state, source, target, 'relocate', 'copied')
    retained = work / 'original-folder-retained'
    source.rename(retained); source.mkdir()
    refused = subprocess.run([str(LAB), 'layout-status', str(state), str(source)], capture_output=True, text=True, timeout=30)
    assert refused.returncode != 0 and 'has been replaced' in refused.stderr
    assert not list(source.iterdir()), 'Restoration created replacement files'
    source.rmdir(); retained.rename(source)
    restored = command('layout-status', state, source)
    assert restored['storageRecovery'] is None
    for name, data in original.items(): assert (source / name).read_bytes() == data
    print('PASS: replaced-active-folder-refuses-writes-and-recovers', flush=True)
    return {'case': 'replaced-active-folder-refuses-writes-and-recovers', 'passed': True}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--cross-volume', action='store_true')
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--locked', '-p', 'seedhop-core', '--bin', 'seedhop-lab', '--features', 'layout-fault-injection'], cwd=ROOT, check=True)
        shutil.copy2(ROOT / 'target/debug/seedhop-lab', LAB)
    output = (args.output or pathlib.Path(tempfile.mkdtemp(prefix='layout-recovery-', dir=ROOT / '.build'))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    cases = []
    for kind in (('v1',) if args.quick else ('v1', 'v2', 'hybrid')):
        for operation in (('relocate',) if args.quick else ('relocate', 'rename')):
            for stage in STAGES:
                cases.append(verify(output / f'{kind}-{operation}-{stage}', kind, operation, stage))
    if not args.quick:
        for intent in ('waiting', 'queued'):
            for operation in ('relocate', 'rename'):
                for stage in ('copy', 'commit'):
                    cases.append(verify(output / f'v1-{intent}-{operation}-{stage}', 'v1', operation, stage, intent))
        cases.append(retained_copy(output / 'edited-copy'))
        cases.append(replaced_active_root(output / 'replaced-active-folder'))
        for kind, operation, stages in [('v2', 'relocate', ('journal', 'copy')), ('hybrid', 'rename', ('saving', 'commit'))]:
            for stage in stages:
                cases.append(verify(output / f'legacy-{kind}-{operation}-{stage}', kind, operation, stage, legacy=True))
    if args.cross_volume:
        image = output / 'cross-volume.sparseimage'
        mount = output / 'mounted-volume'; mount.mkdir()
        subprocess.run(['hdiutil', 'create', '-size', '64m', '-fs', 'HFS+', '-volname', 'SeedHopRecovery', '-type', 'SPARSE', str(image)], check=True, capture_output=True)
        attached = False
        try:
            subprocess.run(['hdiutil', 'attach', '-nobrowse', '-mountpoint', str(mount), str(image)], check=True, capture_output=True)
            attached = True
            for kind, stage in [('v2', 'copy'), ('hybrid', 'commit')]:
                cases.append(verify(output / f'cross-volume-{kind}-{stage}', kind, 'relocate', stage, target_parent=mount))
        finally:
            if attached: subprocess.run(['hdiutil', 'detach', str(mount)], check=True, capture_output=True)
            image.unlink(missing_ok=True)
    report = {'scope': 'Controlled SIGKILL layout recovery; not physical drive loss or power-loss certification',
              'fullMatrix': not args.quick, 'cases': cases, 'passed': len(cases), 'crossVolume': args.cross_volume}
    (output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'PASS: {len(cases)} recovery cases; results in {output}', flush=True)


if __name__ == '__main__':
    main()
