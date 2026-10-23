#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Collect exact Cargo and Rust runtime notices; never invent missing notices."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def generate(destination):
    rust = output('rustc', '-Vv')
    target = next(line.split(': ', 1)[1] for line in rust.splitlines() if line.startswith('host:'))
    metadata = json.loads(output('cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', target))
    packages = {p['id']: p for p in metadata['packages']}
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    root = metadata['resolve']['root']
    roles = {}
    pending = [(root, 'runtime')]
    while pending:
        pid, role = pending.pop()
        if role in roles.setdefault(pid, set()):
            continue
        roles[pid].add(role)
        for dep in nodes[pid]['deps']:
            child = packages[dep['pkg']]
            for kind in dep['dep_kinds']:
                if kind['kind'] == 'dev':
                    continue
                build = role == 'build' or kind['kind'] == 'build' or any('proc-macro' in t['kind'] for t in child['targets'])
                pending.append((dep['pkg'], 'build' if build else 'runtime'))
    destination.mkdir(parents=True, exist_ok=True)
    inventory = {'target': target, 'rustc': rust, 'cargo_lock_sha256': sha(ROOT / 'Cargo.lock'), 'packages': [], 'issues': []}
    for pid in sorted(roles, key=lambda i: (packages[i]['name'], packages[i]['version'])):
        if pid == root:
            continue
        p = packages[pid]
        base = Path(p['manifest_path']).parent
        directory = destination / 'crates' / (p['name'] + '-' + p['version'])
        found = []
        for source in sorted(base.rglob('*')):
            name = source.name.upper()
            if not source.is_file() or not any(name.startswith(prefix) for prefix in ('LICENSE', 'LICENCE', 'COPYING', 'COPYRIGHT', 'NOTICE')):
                continue
            relative = source.relative_to(base)
            dest = directory / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, dest)
            found.append({'path': dest.relative_to(destination).as_posix(), 'sha256': sha(dest)})
        entry = {'name': p['name'], 'version': p['version'], 'license': p['license'], 'authors': p['authors'], 'repository': p.get('repository'), 'roles': sorted(roles[pid]), 'notices': found}
        if not found:
            inventory['issues'].append(f"{p['name']} {p['version']}: exact-version copyright/license text needs upstream clarification")
        inventory['packages'].append(entry)
    sysroot = Path(output('rustc', '--print', 'sysroot'))
    rust_dest = destination / 'rust-runtime'
    rust_dest.mkdir()
    runtime_files = ['LICENSE-MIT', 'LICENSE-APACHE', 'COPYRIGHT', 'share/doc/rustc/COPYRIGHT-library.html']
    inventory['rust_notices'] = []
    for relative in runtime_files:
        source = sysroot / relative
        if not source.is_file():
            inventory['issues'].append('Toolchain is missing ' + relative)
            continue
        dest = rust_dest / source.name
        shutil.copyfile(source, dest)
        inventory['rust_notices'].append({'path': dest.relative_to(destination).as_posix(), 'sha256': sha(dest)})
    licenses = sysroot / 'share/doc/rustc/licenses'
    if licenses.is_dir():
        shutil.copytree(licenses, rust_dest / 'licenses')
        for p in sorted((rust_dest / 'licenses').glob('*')):
            inventory['rust_notices'].append({'path': p.relative_to(destination).as_posix(), 'sha256': sha(p)})
    else:
        inventory['issues'].append('Rust standard-library subsidiary license texts are unavailable')
    (destination / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    return inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Check retained notices against this toolchain and lockfile')
    args = parser.parse_args()
    dest = ROOT / 'third_party' / 'notices'
    with tempfile.TemporaryDirectory(prefix='libretype-notices-') as temporary:
        staged = Path(temporary) / 'notices'
        result = generate(staged)
        if args.check:
            expected = {p.relative_to(staged).as_posix(): sha(p) for p in staged.rglob('*') if p.is_file()}
            actual = {p.relative_to(dest).as_posix(): sha(p) for p in dest.rglob('*') if p.is_file()}
            if expected != actual:
                raise SystemExit('Notice inventory is stale; run scripts/license_inventory.py')
        else:
            if dest.exists():
                shutil.rmtree(dest)
            shutil.copytree(staged, dest)
    print(json.dumps({'packages': len(result['packages']), 'issues': result['issues']}, indent=2))


if __name__ == '__main__':
    main()
