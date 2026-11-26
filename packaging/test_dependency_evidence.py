import unittest
from dependency_evidence import inventory, locked_blocked

def graph(name="serde", source="registry+https://github.com/rust-lang/crates.io-index"):
    return {"packages": [{"id": "root", "name": "collector", "version": "1", "source": None},
                         {"id": "dep", "name": name, "version": "1", "source": source}],
            "resolve": {"nodes": [{"id": "root", "dependencies": ["dep"]}, {"id": "dep", "dependencies": []}]}}

class GraphTests(unittest.TestCase):
    def test_clean(self):
        sbom, blocked = inventory(graph())
        self.assertEqual(blocked, [])
        self.assertEqual(len(sbom["components"]), 2)
        self.assertEqual(len(sbom["dependencies"]), 2)

    def test_names_and_origins(self):
        for name, source in (("deadlock-reader", None), ("deadlock-memory", None),
                             ("source2", None), ("renamed", "git+https://github.com/deadlock-plus/other#123"),
                             ("renamed", "git+https://github.com/other/source2-rs#123")):
            with self.subTest(name=name, source=source):
                self.assertEqual(len(inventory(graph(name, source))[1]), 1)

    def test_lock_only(self):
        self.assertEqual(locked_blocked({"package": [{"name": "source2", "version": "1"}]}), ["source2@1"])

    def test_incomplete(self):
        data = graph()
        data["resolve"]["nodes"].pop()
        with self.assertRaises(ValueError):
            inventory(data)