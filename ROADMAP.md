# Development roadmap

## Independent development (current, blocked runtime)

- Collector-owned game model, source trait and API v3/schema 3 contract; no Deadlock Plus dependency.
- CLI exits before key input or network when a complete authorized source is unavailable.
- Automated checks: Rust default/all features, contract and packaging regressions, graph/SBOM, RustSec, Gitleaks, workflow lint, Windows x64 PE and two clean builds, ClamAV. See actual run evidence; configuration alone is not a passing result.
- Owner decision pending for licensing of own code. Public source visibility is not an open-source license; retain third-party notices.

## Required before real live operation

1. Establish permission and a complete visible-field matrix for a real independent source. Implement and test freshness, identity, field visibility and input bounds without fake values or hidden enemy information.
2. Test the adapter, Windows console/TLS/transport, real match transitions and actual backend/bot consumer in an isolated authorized environment; verify retention, replay, rotation and access boundaries.
3. Independently reproduce the build with documented/pinned native toolchain inputs; obtain actual AV evidence, resolve historical findings, approve signing/provenance and private digest-bound handoff. No scanner bypass or external submission without exact-hash permission.
4. Owner approves protected publisher, release rights, privacy and deployment separately. No public binary, tag or release while unconditional source/publisher gates remain.

No date or guarantee of an authorized source is asserted.
