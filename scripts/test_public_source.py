# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Publication boundaries and release gates using disposable source trees."""
from pathlib import Path
import json
import shutil
import tempfile
import unittest
from unittest.mock import patch

import public_source
import release_source


class PublicSourceTests(unittest.TestCase):
    def test_allowlist_keeps_build_inputs_and_rejects_unreviewed_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copyfile(public_source.ROOT / '.gitignore', root / '.gitignore')
            allowed = ['src/main.rs', 'scripts/build-app.sh', 'docs/guide.md', 'Cargo.lock', 'vendor/gemoji/emoji.json', 'third_party/notices/example/LICENSE', 'vendor/objc/Cargo.toml', 'vendor/objc/src/malloc_buf.rs', 'vendor/objc/LICENSE.txt', 'vendor/objc/COPYING-GPL-3.0', 'vendor/objc/SOURCE.md']
            excluded = ['research/raw.json', 'backups/src/old.rs', 'dist/App.app/binary', 'src/secret.key', 'src/.env.local', 'scripts/__pycache__/cached.pyc', 'unreviewed.txt', 'third_party/model.gguf', 'third_party/private.json', 'docs/capture.png', 'target/debug/app']
            for name in allowed + excluded:
                p = root / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text('synthetic fixture\n')
            files = {p.as_posix() for p in public_source.public_files(root)}
            self.assertTrue(set(allowed) <= files)
            self.assertTrue(set(excluded).isdisjoint(files))

    def test_public_source_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copyfile(public_source.ROOT / '.gitignore', root / '.gitignore')
            (root / 'src').mkdir()
            (root / 'hidden.txt').write_text('private fixture')
            (root / 'src/leak.rs').symlink_to(root / 'hidden.txt')
            with self.assertRaises(ValueError):
                public_source.public_files(root)

    def test_unresolved_notices_block_default_release_before_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'third_party/notices').mkdir(parents=True)
            (root / 'third_party/notices/inventory.json').write_text(json.dumps({'issues': ['missing upstream notice']}))
            with patch.object(release_source, 'ROOT', root), patch.object(release_source.subprocess, 'run'), patch.object(release_source, 'public_files', side_effect=AssertionError('must not export')):
                with self.assertRaisesRegex(ValueError, 'Public release blocked'):
                    release_source.create(root / 'release.tar.gz')
            self.assertFalse((root / 'release.tar.gz').exists())


if __name__ == '__main__':
    unittest.main()
