import unittest
from unittest.mock import patch
from release_policy import allowed
from release_checks import check_main
from verify_draft import verify_draft


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.sha = "a" * 40
        self.repo = "EarlySalty/deadlock-live-match-collector"
        self.env = dict(GITHUB_REPOSITORY=self.repo, GITHUB_REF="refs/heads/main",
                        GITHUB_EVENT_NAME="workflow_run", RELEASE_MODE="prerelease",
                        RELEASE_TAG="v0.3.1", RELEASE_PREVIEW_TAG="v0.3.1", GITHUB_SHA=self.sha, RELEASE_PREVIEW_ENABLED="true")
        self.event = dict(action="completed", workflow_run=dict(event="push", conclusion="success",
                          head_branch="main", head_sha=self.sha, head_repository={"full_name": self.repo},
                          path=".github/workflows/ci.yml"))

    def test_exact_preview_only(self):
        self.assertTrue(allowed(self.env, self.event))
        for k, v in (("GITHUB_REPOSITORY", "fork/repo"), ("GITHUB_REF", "refs/heads/feature"),
                     ("GITHUB_EVENT_NAME", "push"), ("RELEASE_MODE", "stable"),
                     ("RELEASE_TAG", "v0.3.2"), ("RELEASE_PREVIEW_TAG", "v0.3.0"), ("GITHUB_SHA", "b"*40), ("RELEASE_PREVIEW_ENABLED", "false")):
            with self.subTest(key=k):
                self.assertFalse(allowed(dict(self.env, **{k:v}), self.event))
        for key in self.env:
            self.assertFalse(allowed({k:v for k,v in self.env.items() if k != key}, self.event))

    def test_untrusted_ci_event_rejected(self):
        for k, v in (("event", "pull_request"), ("conclusion", "failure"),
                     ("head_branch", "feature"), ("head_sha", "b"*40),
                     ("head_repository", {"full_name": "fork/repo"}), ("path", "other.yml")):
            with self.subTest(key=k):
                self.assertFalse(allowed(self.env, dict(self.event, workflow_run=dict(self.event["workflow_run"], **{k:v}))))
        self.assertFalse(allowed(self.env, dict(self.event, action="requested")))

    def test_manual_retry_main_only(self):
        env = dict(self.env, GITHUB_EVENT_NAME="workflow_dispatch")
        self.assertTrue(allowed(env, {"ref":"refs/heads/main"}))
        self.assertFalse(allowed(env, {"ref":"refs/heads/feature"}))

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
