import hashlib
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import zipfile

from release_assets import EVIDENCE, build_assets, checksum_bytes, verify_assets
from verify_draft import expected_assets


class ReleaseAssetTests(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dist = Path(self.tmp.name)
        self.archive = self.dist / "live-match-collector-0.4.2-win64.zip"
        self.archive.write_bytes(b"synthetic package")
        (self.dist / "live-match-collector.exe").write_bytes(b"synthetic executable")
        for name in EVIDENCE:
            (self.dist / name).write_bytes(name.encode("ascii"))

    def test_exact_public_assets_and_checksums(self):
        files = build_assets(self.archive, self.dist)
        expected = {"live-match-collector-0.4.2-win64.zip", "live-match-collector.exe",
                    "security-evidence.zip", "SHA256SUMS"}
        self.assertEqual({p.name for p in files}, expected)
        self.assertEqual(set(expected_assets(self.archive, self.dist)), expected)
        sums = (self.dist / "public" / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(sums), 3)
        for line, path in zip(sums, files[:3]):
            self.assertEqual(line, f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")
        with zipfile.ZipFile(self.dist / "security-evidence.zip") as bundle:
            self.assertEqual(set(bundle.namelist()), set(EVIDENCE))
            for name in EVIDENCE:
                self.assertEqual(bundle.read(name), (self.dist / name).read_bytes())

    def test_missing_evidence_prevents_build(self):
        (self.dist / "clamav-report.txt").unlink()
        with self.assertRaises(ValueError):
            build_assets(self.archive, self.dist)

    def test_changed_evidence_prevents_remote_verification(self):
        build_assets(self.archive, self.dist)
        (self.dist / "virustotal-report.json").write_bytes(b"changed")
        with self.assertRaises(ValueError):
            expected_assets(self.archive, self.dist)

    def test_unexpected_bundle_file_even_with_updated_checksum(self):
        files = build_assets(self.archive, self.dist)
        with zipfile.ZipFile(files[2], "a") as bundle:
            bundle.writestr("unexpected.txt", b"unexpected")
        files[3].write_bytes(checksum_bytes(files[:3]))
        with self.assertRaises(ValueError):
            verify_assets(self.archive, self.dist)

    def test_changed_download_or_checksum_prevents_verification(self):
        files = build_assets(self.archive, self.dist)
        original = files[0].read_bytes()
        files[0].write_bytes(b"changed")
        with self.assertRaises(ValueError):
            verify_assets(self.archive, self.dist)
        files[0].write_bytes(original)
        files[3].write_bytes(b"wrong checksum")
        with self.assertRaises(ValueError):
            verify_assets(self.archive, self.dist)
