# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Registration tests never touch the user's Chrome profile."""
import json
from pathlib import Path
import tempfile
import unittest
import install_docs_companion as installer

class RegistrationTests(unittest.TestCase):
    def test_registration_is_idempotent_and_never_replaces_a_different_host(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'hosts/org.libretype.docs.json'
            config={'name':'org.libretype.docs','path':'/synthetic/LibreTypeDocsBridge','allowed_origins':['chrome-extension://synthetic/']}
            self.assertEqual(installer.register(path,config),'registered')
            self.assertEqual(path.stat().st_mode&0o777,0o600)
            self.assertEqual(installer.register(path,config),'already registered')
            with self.assertRaisesRegex(ValueError,'different registration'):
                installer.register(path,{**config,'path':'/different/host'})
            self.assertEqual(json.loads(path.read_text()),config)
    def test_symlink_registration_is_rejected_without_touching_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);target=root/'untouched';target.write_text('synthetic unrelated data')
            link=root/'host.json';link.symlink_to(target)
            with self.assertRaisesRegex(ValueError,'symlink'):
                installer.register(link,{'name':'org.libretype.docs'})
            self.assertEqual(target.read_text(),'synthetic unrelated data')

if __name__=='__main__':
    unittest.main()
