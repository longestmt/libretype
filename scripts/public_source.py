#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Inspect the public source manifest without creating or changing project Git history."""
from pathlib import Path
import argparse
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def public_files(root=ROOT):
    with tempfile.TemporaryDirectory(prefix='libretype-manifest-') as temporary:
        subprocess.run(['git', 'init', '-q', temporary], check=True)
        data = subprocess.check_output(['git', '--git-dir', str(Path(temporary) / '.git'), '--work-tree', str(root), 'ls-files', '--others', '--exclude-standard', '-z'], cwd=root)
    result = [Path(p.decode()) for p in data.split(b'\0') if p]
    for relative in result:
        path = root / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'Unsupported publication input: {relative}')
        path.read_text(encoding='utf-8')  # No binary files in the public source tree.
    return sorted(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--forbid-text', action='append', default=[])
    parser.add_argument('--list', action='store_true')
    args = parser.parse_args()
    files = public_files()
    required = ['LICENSE', 'COPYRIGHT', 'Cargo.toml', 'Cargo.lock', 'README.md', 'src/main.rs', 'src/lib.rs', 'scripts/build-app.sh', 'packaging/Info.plist', 'vendor/gemoji/emoji.json', 'vendor/objc/Cargo.toml', 'vendor/objc/src/lib.rs', 'vendor/objc/src/malloc_buf.rs', 'vendor/objc/LICENSE.txt', 'vendor/objc/COPYING-GPL-3.0', 'vendor/objc/SOURCE.md']
    for name in required:
        if Path(name) not in files:
            raise SystemExit('Missing public build input: ' + name)
    hits = []
    for relative in files:
        data = (ROOT / relative).read_text().casefold()
        for needle in args.forbid_text:
            if needle.casefold() in data or needle.casefold() in relative.as_posix().casefold():
                hits.append(str(relative))
    if hits:
        raise SystemExit('Forbidden text found in: ' + ', '.join(sorted(set(hits))))
    if args.list:
        print('\n'.join(p.as_posix() for p in files))
    print(f'Public manifest: {len(files)} UTF-8 source/documentation/notice files')


if __name__ == '__main__':
    main()
