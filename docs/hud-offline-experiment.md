# Offline HUD/OCR experiment (package 12)

Status: **experimental, not an authorized live provider**. No runtime gate change.

## Implementation and reproduction

`src/hud_source.rs` is included from `src/lib.rs` only under `cfg(test)`, not even in an internal-test executable. It implements the existing GameSource trait for a single explicitly selected crop set. Each crop is a grayscale raster of a single field, not a complete screenshot. The test recognizer sends PGM bytes via stdin to local Tesseract 5.3.4, reads TSV from stdout, requires a single line and confidence >=85 for every word. This threshold is a development choice, not a measured error bound. No shell, no raw text logging, no file persistence by the adapter, no capture, no network, no automatic inputs. Crop count, dimensions, row indexes, duplicates and field limits are checked. EOF/error consumes the sample. Unknown or ambiguous values remain None; zero is allowed only when explicitly recognized.

Reproduce from repository root with Rust toolchain pinned by the repo. Normal regression suite needs no OCR installation:

```sh
cargo test --locked --offline -- --test-threads=1
cargo test --locked --offline --features internal-test -- --test-threads=1
cargo check --locked --offline --target x86_64-pc-windows-gnu --all-targets --all-features
cargo test --locked --offline --lib hud_real_ocr_pixels -- --ignored --test-threads=1
```

The last test requires locally installed Tesseract (eng data), ImageMagick convert and DejaVu Sans. It renders 21 artificial text crops and runs actual OCR over the pixel buffers. No game art or game recording is bundled. It is not a test of the current Deadlock HUD. No native Windows OCR test. Tests use local executable lookup and a blocking subprocess without an explicit timeout; these are development-only choices, not suitable for a continuous production capture pipeline. A recording experiment would require selecting real crop coordinates and constructing Crop inputs in the test module; no general recording loader or calibrated HUD detector is delivered.

## Field coverage

| Field | Offline synthetic image recognition | Real Deadlock observation |
|---|---|---|
| Hero name | Text crop, Infernus | Not measured; no validated hero ID/catalog |
| Mode | Exact English Street Brawl/Normal labels (Street Brawl pixels tested) | Not measured; actual label/placement unknown |
| Clock | MM:SS, 12:34 -> 754 | Not measured; actual timer and format unknown |
| Pause | Explicit Paused/Playing (Paused pixels tested) | Not measured; absence never implies not paused |
| K/D/A, Souls, level, alive | Numeric and explicit label crops | Not measured; scoreboard crop semantics/layout unresolved |
| Relative team/local role | Exact Own/Enemy and Local/Other labels | Not measured; no genuine role/side detector |
| Own health/max/AP | Numeric crops, accepted only with unique local Own row | Not measured |
| Team totals/full roster | No claim; no aggregate computed from partial rows | Missing |
| Items / ability upgrades | None; no name/icon-to-ID mapping | Missing |
| Objectives | Explicit presence/count/clock crops; Urn and bridge count tested; no timer source assertion | Not measured; no actual HUD mapping |
| Structures and other extended metrics | Not implemented | Missing |
| Stable match token, eligibility, context, roster size, game slots, IDs | None/Unknown | Missing; cannot satisfy core contract |

All actual image test values are synthetic. No test result is a real game measurement. The source always uses Unknown provenance/context/eligibility and cannot be used by prepare(); no upload can occur even if OCR recognizes all selected test values. Production BlockedSource and the wire contract are unchanged. No authorization is inferred from OCR, confidence, or a manually supplied crop.

## Source decision

Prior task reports 03/07 and interface 02 remain the basis. Relevant public documentation:
- Valve Steamworks Web API reference: https://partner.steamgames.com/doc/webapi — prior reports found no documented own-player Deadlock live feed.
- Overwolf Deadlock Game Events: https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/deadlock/ — documented event feed; current guidelines https://dev.overwolf.com/ow-electron/getting-started/guidelines-and-requirements/ exclude telemetry/bridge apps. Not used.
- Microsoft screen capture: https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture — standard consent-based candidate for a future source, not a game-specific permission.
- Valve agreement: https://store.steampowered.com/agreement/?l=english — no inference of explicit permission for this application from generic terms.
- Community spectator/demo feeds: see report 07; delay and perspective/permission remain incompatible with the current contract.

A passive recording experiment is technically possible, demonstrated here without touching a game. Real authorized recordings, game build/layout/locale/role validation, stable core identity and completeness, permission for live collection/relay, and a real Windows test remain necessary. No protected-process access, game hooks, private protocols, hidden data, anti-cheat workarounds or Deadlock Plus dependencies. Do not replace BlockedSource until the actual source gate is met.
