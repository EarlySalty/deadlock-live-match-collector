import hashlib
import json
import os
import tempfile
import time
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch
import virustotal_report as vt

SHA = hashlib.sha256(b"synthetic").hexdigest()

def report(category="undetected", age=0, count=60):
    stats = dict.fromkeys(vt.CATEGORIES, 0)
    stats[category] = count
    return {"id": SHA, "type": "file", "attributes": {
        "sha256": SHA, "last_analysis_date": int(time.time()) - age,
        "last_analysis_stats": stats, "last_analysis_results": {
            f"Engine{i}": {"category": category, "result": "Test.Detection" if category in ("malicious", "suspicious") else None}
            for i in range(count)}}}

class ReportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "live-match-collector.exe").write_bytes(b"synthetic")
        (self.root / "SHA256SUMS.exe").write_text(SHA + "  live-match-collector.exe\n")

    def run_gate(self, response, key="synthetic-secret", upload="false"):
        with patch.dict(os.environ, {"VIRUSTOTAL_API_KEY": key, "VT_ALLOW_UPLOAD": upload}), patch.object(vt, "request", side_effect=response) as req, patch.object(vt.time, "sleep"):
            code = vt.main(self.root)
        evidence = (self.root / "virustotal-report.json").read_text()
        self.assertNotIn("synthetic-secret", evidence)
        self.assertEqual(json.loads(evidence)["exit_code"], code)
        return code, req

    def test_valid(self):
        code, req = self.run_gate([report()])
        self.assertEqual(code, 0)
        req.assert_called_once_with("synthetic-secret", "/files/" + SHA)
        self.assertEqual(vt.main(self.root, verify=True), 0)

    def test_no_key(self):
        code, req = self.run_gate([], key="")
        self.assertEqual(code, 1)
        req.assert_not_called()

    def test_negative_reports(self):
        mismatch = report()
        mismatch["id"] = "b" * 64
        wrong_stats = report()
        wrong_stats["attributes"]["last_analysis_stats"]["malicious"] = 1
        empty = report()
        empty["attributes"]["last_analysis_results"] = {}
        for bad in (report("malicious"), report("suspicious"), report(age=vt.MAX_AGE+10),
                    report(age=-100), report(count=59), mismatch, wrong_stats, empty, {}):
            with self.subTest(bad=bad):
                self.assertEqual(self.run_gate([bad])[0], 1)

    def test_api_failures_and_no_unauthorized_upload(self):
        for status in (401, 403, 404, 429, 500, 503):
            with self.subTest(status=status):
                error = urllib.error.HTTPError(vt.BASE, status, "secret raw error", None, None)
                code, req = self.run_gate([error])
                self.assertEqual(code, 1)
                self.assertEqual(req.call_count, 1)
        self.assertEqual(self.run_gate([OSError("secret raw error")])[0], 1)

    def test_pending_and_completed_submission(self):
        missing = urllib.error.HTTPError(vt.BASE, 404, "", None, None)
        queued = {"id": "analysis1", "attributes": {"status": "queued"}}
        completed = {"id": "analysis1", "attributes": {"status": "completed"}}
        self.assertEqual(self.run_gate([missing, {"id": "analysis1"}] + [queued]*8, upload="true")[0], 1)
        self.assertEqual(self.run_gate([missing, {"id": "analysis1"}, completed, report()], upload="true")[0], 0)

    def test_checksum_and_tampered_evidence(self):
        self.run_gate([report()])
        evidence = self.root / "virustotal-report.json"
        data = json.loads(evidence.read_text())
        data["engines"][0]["category"] = "malicious"
        evidence.write_text(json.dumps(data))
        with self.assertRaises(vt.GateError):
            vt.main(self.root, verify=True)
        (self.root / "SHA256SUMS.exe").write_text("wrong")
        self.assertEqual(self.run_gate([])[0], 1)

if __name__ == "__main__":
    unittest.main()
