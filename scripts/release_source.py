#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Prepare corresponding source without publishing it or modifying Git history."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile

from public_source import ROOT, public_files


def create(output, allow_incomplete=False):
    subprocess.run(['python3', str(ROOT / 'scripts/license_inventory.py'), '--check'], check=True)
    inventory = json.loads((ROOT / 'third_party/notices/inventory.json').read_text())
    if inventory['issues'] and not allow_incomplete:
        raise ValueError('Public release blocked by unresolved notices: ' + '; '.join(inventory['issues']))
    with tempfile.TemporaryDirectory(prefix='libretype-source-') as temporary:
        stage = Path(temporary) / 'libretype'
        stage.mkdir()
        for relative in public_files():
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        vendored = stage / 'third_party/cargo-src'
        config = subprocess.check_output(['cargo', 'vendor', '--locked', '--versioned-dirs', str(vendored)], cwd=ROOT, text=True)
        # Cargo prints the supplied absolute directory. The archive is relocatable.
        config = config.replace(str(vendored), 'third_party/cargo-src')
        (stage / '.cargo').mkdir()
        (stage / '.cargo/config.toml').write_text(config)
        sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
        runtime = sysroot / 'lib/rustlib/src/rust'
        if not runtime.is_dir():
            raise ValueError('Install source for the exact Rust toolchain before making a complete-source archive')
        shutil.copytree(runtime, stage / 'third_party/rust-runtime-source', symlinks=False)
        manifest = {
            'application': 'LibreType', 'license': 'GPL-3.0-only',
            'rustc': inventory['rustc'], 'target': inventory['target'],
            'cargo_lock_sha256': inventory['cargo_lock_sha256'],
            'public_release_ready': not inventory['issues'],
            'unresolved_notices': inventory['issues'],
            'files': {},
        }
        for p in sorted(stage.rglob('*')):
            if p.is_file():
                manifest['files'][p.relative_to(stage).as_posix()] = hashlib.sha256(p.read_bytes()).hexdigest()
        (stage / 'RELEASE.json').write_text(json.dumps(manifest, indent=2) + '\n')
        output.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(output, 'w:gz') as archive:
            archive.add(stage, arcname='libretype')
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix + '.sha256').write_text(f'{digest}  {output.name}\n')
    return {'archive': str(output), 'sha256': digest, 'public_release_ready': not inventory['issues']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist/libretype-source.tar.gz')
    parser.add_argument('--allow-incomplete-notices', action='store_true', help='Developer validation only; marks archive NOT ready for public release')
    args = parser.parse_args()
    print(json.dumps(create(args.output, args.allow_incomplete_notices), indent=2))


if __name__ == '__main__':
    main()
