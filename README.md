<div align="center">

<img src="packaging/ddc-logo.png" alt="Deutsche Deadlock Community logo" width="112">

# Deadlock Live Match Collector

**A native Windows desktop companion for the Deutsche Deadlock Community Twitch integration.**

[![CI/CD](https://github.com/EarlySalty/deadlock-live-match-collector/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/EarlySalty/deadlock-live-match-collector/actions/workflows/ci.yml)
[![Published version (including previews)](https://img.shields.io/github/v/release/EarlySalty/deadlock-live-match-collector?include_prereleases&sort=semver&label=version&color=8957e5)](https://github.com/EarlySalty/deadlock-live-match-collector/releases)
[![Windows x64](https://img.shields.io/badge/Windows-x64-0078D4)](https://github.com/EarlySalty/deadlock-live-match-collector/releases)
[![Rust](https://img.shields.io/badge/built%20with-Rust-CE412B)](https://www.rust-lang.org/)

[**All releases & downloads**](https://github.com/EarlySalty/deadlock-live-match-collector/releases) · [Setup guide (DE)](packaging/Anleitung-Direkt.md) · [Roadmap](docs/ROADMAP.md) · [Security](SECURITY.md)

</div>

## Download

[**Browse all published releases, including pre-releases →**](https://github.com/EarlySalty/deadlock-live-match-collector/releases)

The version badge above shows the published version number and updates automatically, including pre-releases.

### v0.3.0 desktop preview

| Windows x64 | Description |
| --- | --- |
| [**Download EXE**](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/live-match-collector.exe) | Standalone desktop preview, per-user installation |
| [Download ZIP](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/live-match-collector-0.3.0-win64.zip) | Executable, setup guide and third-party notices |
| [Release notes](https://github.com/EarlySalty/deadlock-live-match-collector/releases/tag/v0.3.0) | v0.3.0 preview, limitations and all verification files |

> [!IMPORTANT]
> **v0.3.0 is a desktop preview, not a working live-match integration.** No authorized complete game data source is available. The client remains in standby and does not collect or upload live match values. Evaluate only in an isolated test environment; do not use a real connection key or play a live match with the preview.

## Features

| Available in the desktop preview | In development |
| --- | --- |
| Native Windows interface and notification-area controls | Authorized live game data source |
| Per-user installation without administrator rights | Live match telemetry |
| Masked connection-key input and Windows Credential Manager storage | Production backend and Twitch bot integration |
| Optional sign-in autostart | Real-game end-to-end acceptance |
| Update notifications with manual installation | |

## Getting started

1. Choose the desired published version from [Releases](https://github.com/EarlySalty/deadlock-live-match-collector/releases). The checksum links below refer specifically to v0.3.0.
2. Verify the executable's SHA-256 against [SHA256SUMS.exe](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/SHA256SUMS.exe). For the ZIP, use [SHA256SUMS](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/SHA256SUMS).
3. Follow the [German setup guide](packaging/Anleitung-Direkt.md) for isolated desktop evaluation.
4. Use the tray menu to reopen the window or exit. Autostart is optional.

Updates are notification-only: no automatic download, installation or execution. Public pre-release notifications are opt-in; see the release notes for the test-channel setting.

## Security & build integrity

[![v0.3.0 VirusTotal](https://img.shields.io/badge/v0.3.0%20VirusTotal-0%20detections%20%2F%2068-2EA043)](https://www.virustotal.com/gui/file/692d220e0159454569ff6ad1f9531fec7cf3d04a157cc32e626ff30defd730b6)
[![Release workflow](https://github.com/EarlySalty/deadlock-live-match-collector/actions/workflows/release.yml/badge.svg)](https://github.com/EarlySalty/deadlock-live-match-collector/actions/workflows/release.yml)

The published v0.3.0 executable passed the mandatory release gates: reproducible Windows builds, ClamAV, VirusTotal, RustSec, Gitleaks, dependency verification and GitHub build attestations.

The release's [VirusTotal report](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/virustotal-report.md) records **68 undetected, 0 malicious, 0 suspicious, 4 unsupported** for the exact published EXE. This is a historical result for v0.3.0, not a guarantee of safety or a continuously updated verdict. The executable is unsigned; never disable endpoint protection or ignore a specific warning.

See [security and privacy](SECURITY.md), [release verification](docs/release-security.md) and the [software bill of materials](https://github.com/EarlySalty/deadlock-live-match-collector/releases/download/v0.3.0/cargo-sbom.cdx.json).

## Development & documentation

The project is written in Rust. The source tree is independent of Deadlock Plus. The former game-process memory reader has been removed; an authorized source is still required before live operation.

| Resource | Contents |
| --- | --- |
| [Roadmap](docs/ROADMAP.md) | Remaining work and production prerequisites |
| [API contract](docs/contract/README.md) | API v3 / upload schema 3 and synthetic test vectors |
| [Development history](docs/archive/development-history.md) | Technical design, historical changes and validation plans |
| [CI/CD](https://github.com/EarlySalty/deadlock-live-match-collector/actions) | Build, tests and release evidence |

To run the local Rust checks with the pinned toolchain:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Community

[Deutsche Deadlock Community](https://deutsche-deadlock-community.de/) · [Issues](https://github.com/EarlySalty/deadlock-live-match-collector/issues) · [Discussions](https://github.com/EarlySalty/deadlock-live-match-collector/discussions)

For bug reports, include the version, Windows version and relevant non-sensitive details. Do not share connection keys, personal data or memory dumps.

## Licensing

The source is publicly visible; no open-source license has been granted for the collector's own code. Third-party notices accompany the ZIP. See [Security](SECURITY.md) for distribution and usage limits.
