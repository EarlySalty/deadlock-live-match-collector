import hashlib
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import urllib.error
import virustotal_report as vt

class ReportTests(unittest.TestCase):
    def test_no_key_link_and_no_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            blob = b"synthetic"
            sha = hashlib.sha256(blob).hexdigest()
            (root / "live-match-collector.exe").write_bytes(blob)
            (root / "SHA256SUMS.exe").write_text(sha + "  live-match-collector.exe\n")
            with patch.dict(os.environ, {"VIRUSTOTAL_API_KEY": ""}), patch.object(vt.sys, "argv", ["script", tmp]), patch.object(vt, "request") as req:
                vt.main()
                req.assert_not_called()
                report = (root / "virustotal-report.md").read_text()
                self.assertIn(sha, report)
                self.assertIn("not submitted", report)
                (root / "SHA256SUMS.exe").write_text("wrong")
                with self.assertRaises(ValueError):
                    vt.main()

    def test_existing_report_and_hash_mismatch(self):
        sha = "a" * 64
        stats = dict(malicious=0, suspicious=0, undetected=60, harmless=1)
        with patch.object(vt, "request", return_value={"id": sha, "attributes": {"last_analysis_stats": stats}}) as req:
            self.assertEqual(vt.scan("key", b"exe", sha), ("report retrieved", stats))
            req.assert_called_once_with("key", "/files/" + sha)
        with patch.object(vt, "request", return_value={"id": "b" * 64, "attributes": {}}):
            with self.assertRaises(ValueError):
                vt.scan("key", b"exe", sha)

    def test_submit_and_poll(self):
        sha = "a" * 64
        stats = dict(malicious=0, suspicious=0, undetected=60, harmless=1)
        missing = urllib.error.HTTPError("https://www.virustotal.com", 404, "not found", None, None)
        responses = [missing, {"id": "analysis123"}, {"attributes": {"status": "completed"}}, {"id": sha, "attributes": {"last_analysis_stats": stats}}]
        with patch.object(vt, "request", side_effect=responses) as req, patch.object(vt.time, "sleep"):
            self.assertEqual(vt.scan("key", b"synthetic exe", sha), ("report retrieved", stats))
            self.assertEqual(req.call_args_list[1].args[1], "/files")
            self.assertIn(b"synthetic exe", req.call_args_list[1].args[2])

if __name__ == "__main__":
    unittest.main()
