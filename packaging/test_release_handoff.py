import datetime as dt
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release_handoff as handoff
from release_policy import allowed
from verify_draft import verify_draft


class PolicyTests(unittest.TestCase):
    def test_only_exact_approved_preview(self):
        env = dict(GITHUB_REF="refs/heads/main", GITHUB_EVENT_NAME="workflow_dispatch",
                   RELEASE_MODE="prerelease", RELEASE_TAG="v0.3.0",
                   GITHUB_SHA="a"*40, RELEASE_PREVIEW_SHA="a"*40,
                   RELEASE_HANDOFF_APPROVED="true")
        self.assertTrue(allowed(env))
        for key, bad in (("GITHUB_REF", "refs/heads/feature"),
                         ("GITHUB_EVENT_NAME", "push"), ("RELEASE_MODE", "stable"),
                         ("RELEASE_TAG", "v0.3.1"), ("GITHUB_SHA", "b"*40),
                         ("RELEASE_PREVIEW_SHA", ""), ("RELEASE_HANDOFF_APPROVED", "false")):
            with self.subTest(key=key):
                self.assertFalse(allowed(dict(env, **{key: bad})))
        for key in env:
            self.assertFalse(allowed({k:v for k,v in env.items() if k != key}))


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.meta = dict(sha="a"*40, run="1", attempt="1", tag="v0.3.0", stage="build")
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        for n in handoff.names(self.meta):
            (self.dist / n).write_bytes(n.encode())

    def test_exact_roundtrip_and_scope(self):
        blob = handoff.pack(self.dist, self.meta)
        target = self.root / "out"
        handoff.unpack(blob, target, self.meta)
        for n in handoff.names(self.meta):
            self.assertEqual((target / n).read_bytes(), (self.dist / n).read_bytes())
        for change in (dict(sha="b"*40), dict(run="2"), dict(attempt="2"),
                       dict(tag="v0.3.1"), dict(stage="scan")):
            with self.assertRaises(ValueError):
                handoff.unpack(blob, self.root / "bad", dict(self.meta, **change))
        with self.assertRaises(FileExistsError):
            handoff.unpack(blob, target, self.meta)

    def test_reject_symlink_and_missing_report(self):
        path = self.dist / "SHA256SUMS"
        path.unlink()
        path.symlink_to(self.dist / "SHA256SUMS.exe")
        with self.assertRaises(ValueError):
            handoff.pack(self.dist, self.meta)
        path.unlink()
        path.write_text("example")
        with self.assertRaises(ValueError):
            handoff.pack(self.dist, dict(self.meta, stage="scan"))

    def test_reject_extra_path_and_changed_member(self):
        original = handoff.pack(self.dist, self.meta)
        for extra in ("../evil", "SHA256SUMS"):
            out = io.BytesIO()
            with tarfile.open(fileobj=io.BytesIO(original)) as old, tarfile.open(fileobj=out, mode="w") as new:
                for m in old:
                    new.addfile(m, old.extractfile(m))
                member = tarfile.TarInfo(extra)
                member.size = 1
                new.addfile(member, io.BytesIO(b"x"))
            with self.assertRaises(ValueError):
                handoff.unpack(out.getvalue(), self.root / "bad", self.meta)
        out = io.BytesIO()
        with tarfile.open(fileobj=io.BytesIO(original)) as old, tarfile.open(fileobj=out, mode="w") as new:
            for m in old:
                data = old.extractfile(m).read()
                if m.name == "SHA256SUMS":
                    data = b"x" * len(data)
                new.addfile(m, io.BytesIO(data))
        with self.assertRaises(ValueError):
            handoff.unpack(out.getvalue(), self.root / "bad", self.meta)

    def test_immutable_storage_receipt(self):
        sha = "a"*64
        head = dict(VersionId="version1", ChecksumSHA256=handoff.checksum(sha),
                    ContentLength=100, ObjectLockMode="COMPLIANCE",
                    ServerSideEncryption="AES256",
                    ObjectLockRetainUntilDate=(dt.datetime.now(dt.timezone.utc)+dt.timedelta(days=1)).isoformat())
        handoff.check_head(head, "version1", sha, 100)
        for field, bad in (("VersionId", "other"), ("ChecksumSHA256", "wrong"),
                           ("ContentLength", 99), ("ObjectLockMode", "GOVERNANCE"),
                           ("ServerSideEncryption", ""), ("ObjectLockRetainUntilDate", "2000-01-01T00:00:00+00:00")):
            with self.subTest(field=field), self.assertRaises(ValueError):
                handoff.check_head(dict(head, **{field: bad}), "version1", sha, 100)
        self.assertFalse(handoff.valid_version("null"))
        self.assertFalse(handoff.valid_version("a\nb"))

    def test_published_requires_same_asset_digest(self):
        release = dict(id=12, tag_name="v0.3.0", draft=False, prerelease=True)
        asset = dict(id=1, name="exe", state="uploaded", size=1, digest="sha256:abc")
        expected = {"exe": (1, "sha256:abc")}
        self.assertEqual(verify_draft(release, [asset], "v0.3.0", expected, True, draft=False), 12)
        with self.assertRaises(ValueError):
            verify_draft(release, [asset], "v0.3.0", expected, True)
        with self.assertRaises(ValueError):
            verify_draft(release, [dict(asset, digest="sha256:def")], "v0.3.0", expected, True, draft=False)


if __name__ == "__main__":
    unittest.main()
