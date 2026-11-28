import hashlib
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch
import zipfile

from release_assets import EVIDENCE, download_notes, verify_assets
from verify_draft import expected_assets


class ReleaseAssetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dist = Path(self.tmp.name)
        self.archive = self.dist / "live-match-collector-0.4.2-win64.zip"
        self.exe = self.dist / "live-match-collector.exe"
        self.exe.write_bytes(b"synthetic executable")
        self.notices = self.dist / "third-party-notices.zip"
        with zipfile.ZipFile(self.archive, "w") as package:
            package.writestr("live-match-collector.exe", self.exe.read_bytes())
            package.writestr("Lizenzen/dependency/LICENSE", b"notice text")
        with zipfile.ZipFile(self.notices, "w") as notices:
            notices.writestr("Lizenzen/dependency/LICENSE", b"notice text")
        for name in EVIDENCE:
            (self.dist / name).write_bytes(name.encode("ascii"))
        for name, path in (("SHA256SUMS", self.archive), ("SHA256SUMS.exe", self.exe)):
            (self.dist / name).write_text(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")

    def test_only_executable_is_public_asset(self):
        self.assertEqual(verify_assets(self.archive, self.dist, self.notices), [self.exe])
        with patch("verify_draft.verify_assets", side_effect=lambda archive, dist: verify_assets(archive, dist, self.notices)):
            expected = expected_assets(self.archive, self.dist)
        self.assertEqual(expected, {self.exe.name: (self.exe.stat().st_size,
                         "sha256:" + hashlib.sha256(self.exe.read_bytes()).hexdigest())})

    def test_missing_evidence_prevents_release(self):
        (self.dist / "clamav-report.txt").unlink()
        with self.assertRaises(ValueError):
            verify_assets(self.archive, self.dist, self.notices)

    def test_changed_notices_prevent_release(self):
        with zipfile.ZipFile(self.notices, "w") as notices:
            notices.writestr("Lizenzen/dependency/LICENSE", b"changed")
        with self.assertRaises(ValueError):
            verify_assets(self.archive, self.dist, self.notices)

    def test_changed_download_or_checksum_prevents_verification(self):
        for path in (self.exe, self.archive, self.dist / "SHA256SUMS.exe"):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                path.write_bytes(b"changed")
                with self.assertRaises(ValueError):
                    verify_assets(self.archive, self.dist, self.notices)
                path.write_bytes(original)

    def test_download_notes_are_versioned_and_link_evidence(self):
        args = (self.archive, "EarlySalty/deadlock-live-match-collector", "v0.4.2", "1234", "5678", self.dist, self.notices)
        notes = download_notes(*args)
        self.assertIn("/releases/download/v0.4.2/live-match-collector.exe", notes)
        self.assertIn(hashlib.sha256(self.exe.read_bytes()).hexdigest(), notes)
        self.assertIn("/actions/runs/1234/artifacts/5678", notes)
        self.assertIn("/blob/v0.4.2/packaging/third-party-notices.zip", notes)
        self.assertIn("/blob/v0.4.2/packaging/Anleitung-Direkt.md", notes)
        self.assertIn("90 days", notes)
        self.assertNotIn("/releases/download/v0.4.2/SHA256SUMS", notes)
        self.assertNotIn("/releases/download/v0.4.2/live-match-collector-0.4.2-win64.zip", notes)
        for invalid in ("", "0", "https://example.com"):
            with self.assertRaises(ValueError):
                download_notes(*args[:4], invalid, *args[5:])
