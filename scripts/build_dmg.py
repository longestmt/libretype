#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Wrap an existing signed LibreType.app in a verified drag-to-install DMG.

Uses Python's standard library and macOS hdiutil. Does not rebuild or re-sign
the app, install it, change Finder preferences, or modify existing images.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

import app_bundle


def inventory(app):
    return {
        p.relative_to(app).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in app.rglob('*') if p.is_file()
    }


def build(app, output):
    app = app.resolve()
    output = output.absolute()
    if output.suffix.lower() != '.dmg':
        raise ValueError('Output must have a .dmg extension')
    if output.exists() or output.is_symlink():
        raise FileExistsError('Refusing to overwrite an existing disk image')
    info = app_bundle.verify(app)
    original = inventory(app)
    version = info['version']
    release = f'https://github.com/longestmt/libretype/releases/tag/v{version}'
    instructions = f'''Install LibreType {version}

1. Quit an existing LibreType before replacing its app.
2. Drag LibreType.app onto the Applications shortcut in this window.
3. Eject this disk image and open LibreType from Applications.

If you already keep LibreType in your home Applications folder (~/Applications),
replace that copy instead. Keep one installed copy. Do not run it from this image.
Finder replacement does not create the development installer's rollback backup.

LibreType needs a separately installed, running Ollama service and a local model.
Grant Accessibility to the installed copy for suggestions in other apps.
Replacing an ad-hoc build can require renewed Accessibility approval.
This app is ad-hoc signed and not notarized; macOS may block downloaded copies.

GNU GPL version 3 only. Copyright (c) 2026 LibreTypist contributors.
You may modify and redistribute under that license. NO WARRANTY, including
merchantability or fitness for a particular purpose. Third-party terms remain.
Full license/notices: LibreType menu > License and source > Open license files.

Matching complete source, DMG packaging source, checksums and installation notes:
{release}
'''
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.libretype-dmg-', dir=output.parent) as temporary:
        work = Path(temporary)
        stage = work / 'contents'
        stage.mkdir()
        shutil.copytree(app, stage / 'LibreType.app')
        (stage / 'Applications').symlink_to('/Applications', target_is_directory=True)
        (stage / 'Install LibreType.txt').write_text(instructions)
        candidate = work / output.name
        subprocess.run([
            '/usr/bin/hdiutil', 'create', '-volname', f'LibreType {version}',
            '-srcfolder', str(stage), '-fs', 'HFS+', '-format', 'UDZO',
            '-imagekey', 'zlib-level=9', '-nospotlight', '-o', str(candidate),
        ], check=True)
        subprocess.run(['/usr/bin/hdiutil', 'verify', str(candidate)], check=True)
        mount = work / 'mounted'
        mount.mkdir()
        attached = plistlib.loads(subprocess.check_output([
            '/usr/bin/hdiutil', 'attach', str(candidate), '-readonly', '-nobrowse',
            '-noautoopen', '-mountpoint', str(mount), '-plist',
        ]))
        device = next(e['dev-entry'] for e in attached['system-entities']
                      if e.get('mount-point') == str(mount))
        try:
            mounted_app = mount / 'LibreType.app'
            if inventory(mounted_app) != original:
                raise ValueError('Mounted app differs from source bundle')
            app_bundle.verify(mounted_app)
            if os.readlink(mount / 'Applications') != '/Applications':
                raise ValueError('Incorrect Applications shortcut')
            if (mount / 'Install LibreType.txt').read_text() != instructions:
                raise ValueError('Installation instructions changed')
            # Verify ordinary file-copy installation without changing the real app.
            copied_app = work / 'copy-check' / 'LibreType.app'
            shutil.copytree(mounted_app, copied_app)
            app_bundle.verify(copied_app)
            if inventory(copied_app) != original:
                raise ValueError('Copied app differs from source bundle')
        finally:
            subprocess.run(['/usr/bin/hdiutil', 'detach', device], check=True)
        if inventory(app) != original:
            raise ValueError('Source app changed during packaging')
        # Atomic no-overwrite publication; candidate and output share a filesystem.
        os.link(candidate, output)
    return {
        'dmg': str(output), 'version': version, 'build': info['build'],
        'app_signing': info['signing'], 'sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
        'verified_app_files': len(original), 'applications_shortcut': '/Applications',
        'read_only_mount_verified': True, 'copied_app_signature_verified': True,
        'source_app_unchanged': True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, default=app_bundle.ROOT / 'dist/LibreType.app')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(build(args.app, args.output), indent=2))


if __name__ == '__main__':
    main()
