# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Updater tests use disposable locally compiled/signed bundles, never user apps."""
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import app_bundle as b


class UpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.shared = tempfile.TemporaryDirectory(prefix="libretype-update-tests-")
        cls.base = Path(cls.shared.name)
        source = cls.base / "fixture.c"
        source.write_text('#include <unistd.h>\nint main(void) { for (;;) pause(); }\n')
        cls.binary = cls.base / "fixture"
        subprocess.run(["/usr/bin/clang", str(source), "-o", str(cls.binary)], check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.shared.cleanup()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=self.base, prefix="paths with spaces é-")
        self.root = Path(self.temp.name)
        self.old = self.bundle("installed", "1")
        self.new = self.bundle("release", "2")
        self.old_digest = b.inventory_digest(self.old)

    def tearDown(self):
        self.temp.cleanup()

    def bundle(self, folder, number):
        app = self.root / folder / b.APP_NAME
        (app / "Contents/MacOS").mkdir(parents=True)
        shutil.copy2(self.binary, app / "Contents/MacOS" / b.EXECUTABLE)
        info = plistlib.loads((b.ROOT / "packaging/Info.plist").read_bytes())
        info["CFBundleVersion"] = number
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        b.run([b.CODESIGN, "--force", "--sign", "-", "--identifier", b.APP_ID, app])
        return app

    def test_same_artifact_is_a_noop_and_matches_its_requirement(self):
        copy = self.root / "same" / b.APP_NAME
        copy.parent.mkdir()
        shutil.copytree(self.old, copy)
        report = b.inspect_update(copy, self.old)
        self.assertTrue(report["same_signing_identity"])
        self.assertFalse(report["accessibility_renewal_may_be_needed"])
        with patch.object(b, "stop_app", side_effect=AssertionError("must not stop identical app")):
            self.assertEqual(b.install(copy, self.old)["result"], "already installed")

    def test_changed_adhoc_build_requires_explicit_migration(self):
        report = b.inspect_update(self.new, self.old)
        self.assertFalse(report["same_signing_identity"])
        with self.assertRaises(b.UpdateError):
            b.install(self.new, self.old)
        self.assertEqual(b.inventory_digest(self.old), self.old_digest)

    def test_atomic_update_retains_a_verified_backup(self):
        report = b.install(self.new, self.old, allow_identity_change=True)
        self.assertEqual(b.verify(self.old)["build"], "2")
        backup = Path(report["backup"])
        self.assertEqual(b.inventory_digest(backup), self.old_digest)
        self.assertEqual(b.verify(backup)["build"], "1")
        self.assertEqual(b.inventory_digest(self.new), b.inventory_digest(self.old))

    def test_failed_post_swap_validation_restores_previous_app(self):
        real_verify = b.verify
        def fail_new_destination(path):
            result = real_verify(path)
            if path == self.old and result["build"] == "2":
                raise b.UpdateError("injected final verification failure")
            return result
        with patch.object(b, "verify", side_effect=fail_new_destination):
            with self.assertRaisesRegex(b.UpdateError, "injected"):
                b.install(self.new, self.old, allow_identity_change=True)
        self.assertEqual(b.inventory_digest(self.old), self.old_digest)
        self.assertEqual(b.verify(self.old)["build"], "1")

    def test_invalid_signature_and_symlink_are_rejected_before_replacement(self):
        info = self.new / "Contents/Info.plist"
        data = plistlib.loads(info.read_bytes())
        data["CFBundleVersion"] = "3"
        info.write_bytes(plistlib.dumps(data))
        with self.assertRaises(b.UpdateError):
            b.install(self.new, self.old, allow_identity_change=True)
        self.assertEqual(b.inventory_digest(self.old), self.old_digest)
        link = self.root / "alias.app"
        link.symlink_to(self.old, target_is_directory=True)
        with self.assertRaises(b.UpdateError):
            b.verify(link)

    def test_certificate_downgrade_is_rejected_even_with_identity_override(self):
        report = {"installed": {"signing": "certificate"}, "new": {"signing": "ad-hoc"}, "same_signing_identity": False}
        with self.assertRaisesRegex(b.UpdateError, "certificate-signed"):
            b.authorize_update(report, True)
        with patch.object(b, "run") as command:
            with self.assertRaises(b.UpdateError):
                b.build("not a fingerprint", None)
            command.assert_not_called()
        with patch.object(b, "run", return_value=subprocess.CompletedProcess([], 0, "0 valid identities found", "")):
            with self.assertRaisesRegex(b.UpdateError, "no ad-hoc fallback"):
                b.build("f" * 40, None)

    def test_only_the_exact_installed_executable_is_stopped(self):
        target = subprocess.Popen([str(self.old / "Contents/MacOS" / b.EXECUTABLE)])
        unrelated = subprocess.Popen([str(self.binary)])
        try:
            with self.assertRaisesRegex(b.UpdateError, "running"):
                b.install(self.new, self.old, allow_identity_change=True)
            self.assertIsNone(target.poll())
            b.install(self.new, self.old, allow_identity_change=True, quit_app=True)
            target.wait(timeout=2)
            self.assertIsNone(unrelated.poll())
        finally:
            for proc in (target, unrelated):
                if proc.poll() is None:
                    proc.terminate()
                proc.wait(timeout=2)

    def test_concurrent_update_is_rejected_and_first_install_needs_no_migration(self):
        with b.exclusive_lock(self.old.parent, ".libretype-install.lock"):
            with self.assertRaisesRegex(b.UpdateError, "Another"):
                b.install(self.new, self.old, allow_identity_change=True)
        destination = self.root / "first install" / b.APP_NAME
        result = b.install(self.new, destination)
        self.assertIsNone(result["backup"])
        self.assertEqual(b.verify(destination)["build"], "2")


if __name__ == "__main__":
    unittest.main()
