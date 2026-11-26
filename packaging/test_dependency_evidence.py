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
    def test_verify_recomputes_graph_not_just_local_hash(self):
        import hashlib
        import json
        import sys
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from dependency_evidence import main
        with tempfile.TemporaryDirectory() as td:
            out = Path(td)
            def command(args, **kwargs):
                if args[0] == "git":
                    return "a" * 40 + "\n"
                return json.dumps(graph()).encode()
            with patch("dependency_evidence.subprocess.check_output", side_effect=command):
                with patch.object(sys, "argv", ["dependency_evidence.py", td]):
                    self.assertEqual(main(), 0)
                with patch.object(sys, "argv", ["dependency_evidence.py", td, "--verify", "--offline"]):
                    self.assertEqual(main(), 0)
                    sbom = out / "cargo-sbom.cdx.json"
                    sbom.write_text("{}")
                    gate = out / "dependency-gate.json"
                    data = json.loads(gate.read_text())
                    data["sbom_sha256"] = hashlib.sha256(sbom.read_bytes()).hexdigest()
                    gate.write_text(json.dumps(data))
                    self.assertEqual(main(), 1)
                    self.assertEqual(sbom.read_text(), "{}")
