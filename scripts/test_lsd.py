#!/usr/bin/env python3
"""Run production LSD unit tests without rqbit's unrelated desktop workspace."""
import json
import os
import pathlib
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--offline', '--format-version', '1'], cwd=ROOT))
    packages = {package['id']: package for package in metadata['packages']}
    package = next(package for package in packages.values() if package['name'] == 'librqbit-lsd')
    node = next(node for node in metadata['resolve']['nodes'] if node['id'] == package['id'])
    resolved = {dependency['name'].replace('_', '-'): packages[dependency['pkg']]['version'] for dependency in node['deps']}
    lines = ['[workspace]', '[package]', 'name = "librqbit-lsd"', f'version = {json.dumps(package["version"])}', 'edition = "2024"', 'publish = false', '[lib]',
             f'path = {json.dumps(str(pathlib.Path(package["manifest_path"]).parent / "src/lib.rs"))}']
    for kind, section in [(None, 'dependencies'), ('dev', 'dev-dependencies')]:
        lines.append(f'[{section}]')
        for dependency in package['dependencies']:
            if dependency['kind'] != kind:
                continue
            name = dependency['rename'] or dependency['name']
            settings = []
            if dependency.get('path'):
                settings.append(f'path = {json.dumps(dependency["path"])}')
            else:
                version = resolved.get(name.replace('_', '-'))
                settings.append(f'version = {json.dumps("=" + version if version else dependency["req"])}')
            if dependency['rename']:
                settings.append(f'package = {json.dumps(dependency["name"])}')
            settings += [f'default-features = {str(dependency["uses_default_features"]).lower()}', f'features = {json.dumps(dependency["features"])}']
            lines.append(f'{json.dumps(name)} = {{ {", ".join(settings)} }}')
    work = ROOT / '.build/lsd-unit-gate'
    work.mkdir(parents=True, exist_ok=True)
    manifest = work / 'Cargo.toml'
    manifest.write_text('\n'.join(lines) + '\n')
    shutil.copyfile(ROOT / 'Cargo.lock', work / 'Cargo.lock')
    print('Testing production LSD source through an isolated manifest; root dependency versions and cache reused.', flush=True)
    result = subprocess.run(['cargo', 'test', '--manifest-path', str(manifest), '--offline', '--lib', '--', '--test-threads=1'],
                            cwd=ROOT, env=dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target')))
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
