import copy
import unittest
from verify_draft import verify_draft


class DraftTests(unittest.TestCase):
    def test_exact_assets_and_mutations(self):
        tag = "v0.2.1"
        release = {"id": 42, "tag_name": tag, "draft": True, "prerelease": True}
        expected = {"x.zip": (12, "sha256:" + "a" * 64), "x.exe": (2, "sha256:" + "b" * 64)}
        assets = [{"id": i, "name": name, "size": size, "digest": digest, "state": "uploaded"}
                  for i, (name, (size, digest)) in enumerate(expected.items(), 1)]
        self.assertEqual(verify_draft(release, assets, tag, expected, True), 42)
        for field, value in [("digest", None), ("digest", "sha256:" + "c" * 64),
                             ("size", 99), ("state", "starter"), ("name", "other.exe"), ("id", 2)]:
            with self.subTest(field=field, value=value):
                modified = copy.deepcopy(assets)
                modified[0][field] = value
                with self.assertRaises(ValueError):
                    verify_draft(release, modified, tag, expected, True)
        for modified in [assets[:-1], assets + assets[:1], [assets[0], assets[0]]]:
            with self.assertRaises(ValueError):
                verify_draft(release, modified, tag, expected, True)
        for field, value in [("draft", False), ("tag_name", "v9.0.0"), ("prerelease", False), ("id", True)]:
            with self.assertRaises(ValueError):
                verify_draft(dict(release, **{field: value}), assets, tag, expected, True)
