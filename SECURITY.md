# Security and transparency

The Windows executable reads the local Deadlock game process and local installed game catalogues; it does not inject code or bypass protection. Review the code and applicable game rules before use. A read-only design and automated scans do not guarantee safety or permission.

## Data boundary

The client sends versioned game telemetry to https://deutsche-deadlock-community.de/twitch/api/v3/live-match using HTTPS, a channel-specific bearer key and a 32 KiB body limit. The intended wire fields are documented in docs/contract. No screenshots, player names, Steam IDs, raw process memory, actual match IDs, credentials in the JSON, or arbitrary local files are intended to be uploaded. The connection key is entered without echo and belongs only in the Authorization header. There is no automatic update mechanism. The backend is intended to retain only a short-lived RAM snapshot; its actual deployed behavior must be independently verified.

## Checks and verification

The public CI checks Rust formatting, Clippy, tests, Windows packaging, secret scanning, locked-dependency advisories and a ClamAV scan of the actual packaged EXE. Dependabot proposes dependency updates. The release workflow requires a matching protected version tag, successful CI for the exact main commit, and hosted-runner provenance for the ZIP and standalone EXE. Stable releases additionally require the HTTPS routing/authentication preflight. Pre-releases are not proof of production availability or real-game compatibility.

Download from GitHub Releases, verify SHA-256, and verify provenance with gh attestation verify <file> --repo EarlySalty/deadlock-live-match-collector. Preserve third-party notices. Do not disable endpoint protection. Report suspected vulnerabilities privately using GitHub's private vulnerability reporting if enabled; never include actual connection keys, live personal data or memory dumps in public issues.
