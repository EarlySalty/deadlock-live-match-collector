import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch
from release_policy import allowed, package_tag
from release_checks import check_main
from verify_draft import verify_draft


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.sha = "a" * 40
        self.repo = "EarlySalty/deadlock-live-match-collector"
        self.env = dict(GITHUB_REPOSITORY=self.repo, GITHUB_REF="refs/heads/main",
                        GITHUB_EVENT_NAME="workflow_run", RELEASE_MODE="prerelease",
                        RELEASE_TAG="v0.3.1", RELEASE_AUTO_PREVIEW="true", GITHUB_SHA=self.sha, RELEASE_PREVIEW_ENABLED="true")
        self.event = dict(action="completed", workflow_run=dict(event="push", conclusion="success",
                          head_branch="main", head_sha=self.sha, head_repository={"full_name": self.repo},
                          path=".github/workflows/ci.yml"))

    def test_exact_preview_only(self):
        self.assertTrue(allowed(self.env, self.event, "v0.3.1"))
        for k, v in (("GITHUB_REPOSITORY", "fork/repo"), ("GITHUB_REF", "refs/heads/feature"),
                     ("GITHUB_EVENT_NAME", "push"), ("RELEASE_MODE", "stable"),
                     ("RELEASE_TAG", "v0.3.2"), ("RELEASE_AUTO_PREVIEW", "false"), ("GITHUB_SHA", "b"*40), ("RELEASE_PREVIEW_ENABLED", "false")):
            with self.subTest(key=k):
                self.assertFalse(allowed(dict(self.env, **{k:v}), self.event, "v0.3.1"))
        for key in self.env:
            self.assertFalse(allowed({k:v for k,v in self.env.items() if k != key}, self.event, "v0.3.1"))

    def test_future_preview_matches_manifest(self):
        env = dict(self.env, RELEASE_TAG="v0.4.2")
        self.assertTrue(allowed(env, self.event, "v0.4.2"))
        self.assertFalse(allowed(env, self.event, "v0.3.1"))
        for tag in ("v0.04.2", "v0.4.2-rc.1", "v0.4.2\n", "v0.4"):
            self.assertFalse(allowed(dict(env, RELEASE_TAG=tag), self.event, tag))

    def test_package_tag_from_manifest(self):
        with TemporaryDirectory() as directory:
            path = Path(directory) / "Cargo.toml"
            for name, version, expected in (
                ("live-match-collector", "0.4.2", "v0.4.2"),
                ("other", "0.4.2", None),
                ("live-match-collector", "0.04.2", None),
                ("live-match-collector", "0.4.2-rc.1", None),
            ):
                path.write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n')
                if expected:
                    self.assertEqual(package_tag(path), expected)
                else:
                    with self.assertRaises(ValueError):
                        package_tag(path)

    def test_untrusted_ci_event_rejected(self):
        for k, v in (("event", "pull_request"), ("conclusion", "failure"),
                     ("head_branch", "feature"), ("head_sha", "b"*40),
                     ("head_repository", {"full_name": "fork/repo"}), ("path", "other.yml")):
            with self.subTest(key=k):
                self.assertFalse(allowed(self.env, dict(self.event, workflow_run=dict(self.event["workflow_run"], **{k:v})), "v0.3.1"))
        self.assertFalse(allowed(self.env, dict(self.event, action="requested"), "v0.3.1"))

    def test_manual_retry_main_only(self):
        env = dict(self.env, GITHUB_EVENT_NAME="workflow_dispatch")
        self.assertTrue(allowed(env, {"ref":"refs/heads/main"}, "v0.3.1"))
        self.assertFalse(allowed(env, {"ref":"refs/heads/feature"}, "v0.3.1"))

    def test_exact_remote_main_and_version(self):
        for head, remote, version, success in (
            (self.sha, self.sha+"\trefs/heads/main", "0.3.1", True),
            ("b"*40, self.sha+"\trefs/heads/main", "0.3.1", False),
            (self.sha, "b"*40+"\trefs/heads/main", "0.3.1", False),
            (self.sha, self.sha+"\trefs/heads/main", "0.3.0", False)):
            with self.subTest(success=success, version=version), patch.dict("os.environ", self.env), \
                    patch("release_checks.subprocess.check_output", side_effect=[head, remote]), \
                    patch("release_checks.command_json", return_value={"packages":[{"name":"live-match-collector","version":version}]}):
                if success:
                    check_main()
                else:
                    with self.assertRaises(ValueError):
                        check_main()

    def test_published_requires_same_asset_digest(self):
        release = dict(id=12, tag_name="v0.3.0", draft=False, prerelease=True)
        asset = dict(id=1, name="exe", state="uploaded", size=1, digest="sha256:abc")
        expected = {"exe": (1, "sha256:abc")}
        self.assertEqual(verify_draft(release, [asset], "v0.3.0", expected, True, draft=False), 12)
        with self.assertRaises(ValueError):
            verify_draft(release, [asset], "v0.3.0", expected, True)
        with self.assertRaises(ValueError):
            verify_draft(release, [dict(asset, digest="sha256:def")], "v0.3.0", expected, True, draft=False)
