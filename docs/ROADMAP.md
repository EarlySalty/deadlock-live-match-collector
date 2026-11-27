# Development roadmap

## Desktop preview v0.3.0 (published, live runtime blocked)

The [public desktop preview](https://github.com/EarlySalty/deadlock-live-match-collector/releases/tag/v0.3.0) is available for isolated evaluation. Its release workflow and exact-file security gates passed. Live match collection and production deployment remain blocked.

- Collector-owned game model, source trait and API v3/schema 3 contract; no Deadlock Plus dependency.
- CLI exits before key input or network when a complete authorized source is unavailable.
- Automated checks: Rust default/all features, contract and packaging regressions, graph/SBOM, RustSec, Gitleaks, workflow lint, Windows x64 PE and two clean builds, ClamAV. See actual run evidence; configuration alone is not a passing result.
- Owner decision pending for licensing of own code. Public source visibility is not an open-source license; retain third-party notices.

## Required before real live operation

1. Establish permission and a complete visible-field matrix for a real independent source. Implement and test freshness, identity, field visibility and input bounds without fake values or hidden enemy information.
2. Test the adapter, Windows console/TLS/transport, real match transitions and actual backend/bot consumer in an isolated authorized environment; verify retention, replay, rotation and access boundaries.
3. Independently reproduce the build with documented/pinned native toolchain inputs; retain actual AV evidence and investigate historical findings. Evaluate legitimate signing. Keep the existing GitHub-hosted release gates and exact-file submission scope; no scanner bypass.
4. Obtain explicit production approval for source permission, distribution rights, privacy, backend deployment and stable release. The v0.3.0 desktop-preview exception is not production authorization.

No date or guarantee of an authorized source is asserted.
