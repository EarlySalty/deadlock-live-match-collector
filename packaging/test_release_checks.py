import hashlib
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import zipfile

from release_checks import SUMMARY, preflight, successful_job, successful_run, require_tag

SHA = "a" * 40
OTHER = "b" * 40
REPO = "owner/repository"
ROOT = Path(__file__).resolve().parent.parent


class PreflightTests(unittest.TestCase):
    def setUp(self):
        self.requests = []
        self.status = 401
        self.delay = 0
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                outer.requests.append((self.path, dict(self.headers), json.loads(body)))
                time.sleep(outer.delay)
                self.send_response(outer.status)
                if outer.status in (301, 302, 307, 308):
                    self.send_header("Location", "/redirected")
                self.end_headers()

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/upload"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def test_valid_summary_without_key(self):
        preflight(self.url)
        self.assertEqual(len(self.requests), 1)
        path, headers, body = self.requests[0]
        self.assertEqual(path, "/upload")
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertNotIn("Authorization", headers)
        self.assertEqual(body, SUMMARY)
        self.assertEqual(set(body), {"match_id", "hero", "game_seconds", "kills", "deaths", "assists", "team_souls", "enemy_souls"})
        self.assertIsInstance(body["match_id"], str)
        self.assertTrue(0 < int(body["match_id"]) < 2**64)
        self.assertTrue(0 < len(body["hero"].encode()) <= 80)
        self.assertTrue(0 <= body["game_seconds"] <= 86400)
        for field in ("kills", "deaths", "assists"):
            self.assertTrue(type(body[field]) is int and 0 <= body[field] <= 1000)
        for field in ("team_souls", "enemy_souls"):
            self.assertTrue(type(body[field]) is int and 0 <= body[field] <= 10**9)

    def test_every_other_status_fails_without_redirect(self):
        for status in (200, 204, 301, 302, 307, 308, 400, 403, 404, 415, 422, 429, 500, 503):
            with self.subTest(status=status):
                self.status = status
                self.requests.clear()
                with self.assertRaises(ValueError):
                    preflight(self.url)
                self.assertEqual([r[0] for r in self.requests], ["/upload"])

    def test_timeout(self):
        self.delay = 1
        with self.assertRaises(ValueError):
            preflight(self.url, timeout=0.1)

    def test_network_failure(self):
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            with self.assertRaises(ValueError):
                preflight(f"http://127.0.0.1:{sock.getsockname()[1]}/upload", timeout=1)


class GateTests(unittest.TestCase):
    def setUp(self):
        self.run = {"id": 12, "run_number": 3, "run_attempt": 2, "head_sha": SHA,
                    "head_branch": "main", "event": "push", "head_repository": {"full_name": REPO},
                    "path": ".github/workflows/ci.yml", "status": "completed", "conclusion": "success"}
        self.job = {"id": 31, "name": "build", "run_id": 12, "head_sha": SHA,
                    "status": "completed", "conclusion": "success"}

    def test_success_exact_sha(self):
        run = successful_run({"total_count": 1, "workflow_runs": [self.run]}, SHA, REPO)
        self.assertEqual(successful_job({"total_count": 1, "jobs": [self.job]}, run), self.job)

    def test_missing_and_incomplete_runs(self):
        for response in ({}, {"total_count": 0, "workflow_runs": []},
                         {"total_count": 101, "workflow_runs": [self.run]}):
            with self.assertRaises(ValueError):
                successful_run(response, SHA, REPO)

    def test_wrong_identity_and_failed_runs(self):
        for field, value in (("head_sha", OTHER), ("head_branch", "feature"),
                             ("event", "pull_request"), ("head_repository", {"full_name": "fork/repo"}),
                             ("path", ".github/workflows/other.yml"), ("status", "in_progress"),
                             ("conclusion", "failure"), ("conclusion", "cancelled"), ("conclusion", "skipped")):
            with self.subTest(field=field, value=value):
                run = dict(self.run, **{field: value})
                with self.assertRaises(ValueError):
                    successful_run({"total_count": 1, "workflow_runs": [run]}, SHA, REPO)

    def test_newer_pending_run_blocks_older_success(self):
        newer = dict(self.run, run_number=4, status="queued", conclusion=None)
        with self.assertRaises(ValueError):
            successful_run({"total_count": 2, "workflow_runs": [self.run, newer]}, SHA, REPO)

    def test_job_must_be_successful_and_exact(self):
        for field, value in (("head_sha", OTHER), ("run_id", 13), ("run_attempt", 1),
                             ("status", "queued"), ("conclusion", "skipped"), ("name", "other")):
            with self.subTest(field=field):
                with self.assertRaises(ValueError):
                    successful_job({"total_count": 1, "jobs": [dict(self.job, **{field: value})]}, self.run)
        for jobs in ([], [self.job, self.job]):
            with self.assertRaises(ValueError):
                successful_job({"total_count": len(jobs), "jobs": jobs}, self.run)

    def test_lightweight_and_annotated_tags(self):
        require_tag("v0.2.0", SHA, "refs/heads/main", "0.2.0", f"{SHA}\trefs/tags/v0.2.0\n")
        require_tag("v0.2.0", SHA, "refs/heads/main", "0.2.0",
                    f"{OTHER}\trefs/tags/v0.2.0\n{SHA}\trefs/tags/v0.2.0^{{}}\n")

    def test_bad_tags(self):
        for tag, ref, version, refs in (
            ("v0.1.0", "refs/heads/main", "0.2.0", ""),
            ("v0.2.0", "refs/heads/feature", "0.2.0", f"{SHA}\trefs/tags/v0.2.0"),
            ("v0.2.0", "refs/heads/main", "0.2.0", ""),
            ("v0.2.0", "refs/heads/main", "0.2.0", f"{OTHER}\trefs/tags/v0.2.0"),
            ("v0.2.0", "refs/heads/main", "0.2.0", f"{SHA}\trefs/tags/v0.2.0\n{OTHER}\trefs/tags/v0.2.0^{{}}"),
            ("v0.2.0;bad", "refs/heads/main", "0.2.0", ""),
        ):
            with self.subTest(tag=tag, ref=ref, refs=refs):
                with self.assertRaises(ValueError):
                    require_tag(tag, SHA, ref, version, refs)


class DraftTests(unittest.TestCase):
    def test_draft_only_after_all_checks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "packaging").symlink_to(ROOT / "packaging", target_is_directory=True)
            (root / "dist").mkdir()
            (root / "bin").mkdir()
            archive = root / "dist" / "package.zip"
            exe = b"synthetic test executable"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("live-match-collector.exe", exe)
            (root / "dist" / "SHA256SUMS").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  package.zip\n")
            report = root / "dist" / "clamav-report.txt"
            report.write_text(f"exe_sha256={hashlib.sha256(exe).hexdigest()}\nInfected files: 0\n")
            (root / "dist" / "build-environment.txt").write_text("mock environment\n")
            log = root / "calls"
            for name in ("gh", "python3"):
                stub = root / "bin" / name
                stub.write_text('#!/bin/sh\nprintf "%s %s\\n" "${0##*/}" "$*" >> "$MOCK_LOG"\ncase "$*" in *"$MOCK_FAIL"*) [ -z "$MOCK_FAIL" ] || exit 1;; esac\nexit 0\n')
                stub.chmod(0o755)
            env = dict(os.environ, PATH=str(root / "bin") + ":" + os.environ["PATH"],
                       RELEASE_TAG="v0.2.0", GITHUB_SHA=SHA, GITHUB_REPOSITORY=REPO,
                       MOCK_LOG=str(log), MOCK_FAIL="")
            for fail in ("attestation verify", "release_checks.py tag", "release_checks.py ci",
                         "release_checks.py preflight", ""):
                with self.subTest(fail=fail):
                    log.write_text("")
                    result = subprocess.run(["bash", "packaging/create-draft.sh", str(archive)],
                                            cwd=root, env=dict(env, MOCK_FAIL=fail), capture_output=True)
                    calls = log.read_text()
                    if fail:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertNotIn("gh release create", calls)
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr.decode())
                        self.assertIn("gh release create v0.2.0 --draft --verify-tag", calls)
                        self.assertTrue(calls.splitlines()[-1].startswith("gh release create"))
            log.write_text("")
            report.write_text("exe_sha256=wrong\nInfected files: 0\n")
            result = subprocess.run(["bash", "packaging/create-draft.sh", str(archive)],
                                    cwd=root, env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(log.read_text(), "")


if __name__ == "__main__":
    unittest.main()
