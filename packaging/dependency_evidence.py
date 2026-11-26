"""Inspect the full locked Cargo graph (all platforms and dependency kinds)."""
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

FORBIDDEN = re.compile(r"deadlock[-_]?plus|deadlock[-_]?(rs|reader|core|data|memory)|source2(?:[-_]rs)?", re.I)

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def inventory(metadata):
    packages = metadata["packages"]
    nodes = metadata["resolve"]["nodes"]
    by_id = {p["id"]: p for p in packages}
    if not packages or len(by_id) != len(packages) or set(by_id) != {n["id"] for n in nodes}:
        raise ValueError("incomplete Cargo graph")
    refs = {p["id"]: f"{p['name']}@{p['version']}#{hashlib.sha256((p.get('source') or 'workspace').encode()).hexdigest()[:16]}" for p in packages}
    if len(set(refs.values())) != len(refs):
        raise ValueError("ambiguous package identifiers")
    components, dependencies, blocked = [], [], []
    for p in packages:
        source = p.get("source")
        ref = refs[p["id"]]
        if FORBIDDEN.search(p["name"]) or FORBIDDEN.search(source or ""):
            blocked.append(ref)
        component = {"type": "library", "bom-ref": ref, "name": p["name"], "version": p["version"]}
        if source and source.startswith("registry+https://github.com/rust-lang/crates.io-index"):
            component["purl"] = f"pkg:cargo/{p['name']}@{p['version']}"
        components.append(component)
    for n in nodes:
        dependencies.append({"ref": refs[n["id"]], "dependsOn": sorted({refs[d] for d in n["dependencies"]})})
    return {"bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
            "components": sorted(components, key=lambda c: c["bom-ref"]),
            "dependencies": sorted(dependencies, key=lambda d: d["ref"])}, sorted(blocked)

def locked_blocked(lock):
    return sorted({f"{p['name']}@{p['version']}" for p in lock["package"]
                   if FORBIDDEN.search(p["name"]) or FORBIDDEN.search(p.get("source", ""))})

def main():
    out = Path(sys.argv[1])
    if "--verify" in sys.argv[2:]:
        evidence = json.loads((out / "dependency-gate.json").read_text())
        return int(evidence != {"source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                                "lock_sha256": digest("Cargo.lock"), "sbom_sha256": digest(out / "cargo-sbom.cdx.json"),
                                "blocked": [], "exit_code": 0})
    out.mkdir(parents=True, exist_ok=True)
    command = ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"]
    if "--offline" in sys.argv[2:]:
        command.append("--offline")
    lock = digest("Cargo.lock")
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    blocked = locked_blocked(tomllib.loads(Path("Cargo.lock").read_text()))
    try:
        data = json.loads(subprocess.check_output(command))
        sbom, graph_blocked = inventory(data)
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        (out / "dependency-gate.json").write_text(json.dumps({"source": source, "lock_sha256": lock,
            "blocked": blocked, "exit_code": 1, "status": "metadata unavailable or invalid"}, indent=2) + "\n")
        return 1
    blocked = sorted(set(graph_blocked) | set(blocked))
    (out / "cargo-sbom.cdx.json").write_text(json.dumps(sbom, indent=2) + "\n")
    evidence = {"source": source,
                "lock_sha256": lock, "sbom_sha256": digest(out / "cargo-sbom.cdx.json"),
                "blocked": blocked, "exit_code": int(bool(blocked))}
    (out / "dependency-gate.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return evidence["exit_code"]

if __name__ == "__main__":
    sys.exit(main())