# Deadlock live match collector

Windows x64 desktop/tray application intended to deliver complete live-game values to the Deutsche Deadlock Community Twitch bot. Current integration candidate: no authorized live source is available; the desktop client remains in standby and never uploads without an authorized source. The non-Windows development CLI still exits before credential input. The intended HTTPS transport is retained but inactive. The current source tree is independent of Deadlock Plus; no former reader, assets, or dependency are required. There is **no functioning live collector download** for this development state. Publicly readable source is not a grant of an open-source license; licensing of the collector code is pending an owner decision. Preserve third-party notices.

[German direct setup instructions](packaging/Anleitung-Direkt.txt) · [API v3 / upload schema 3](docs/contract/README.md) · [Security](SECURITY.md) · [Roadmap](ROADMAP.md)

## Live values

The former game-process reader and its asset parser have been removed. The independent model is intended to represent the local hero, playing time, game mode and pause; K/D/A, own/enemy team souls, visible scoreboard and team statistics, own items and ability upgrades, visible structures and objective status/timers. Unknown values remain null or absent. Only directly observed game timers are represented as direct; bridge-buff estimates are separate and explicitly labelled. No names, Steam IDs, raw match ID, raw memory, positions or new hidden enemy data are sent.

The v3 upload has schema_version=3, random per-process writer ID, monotonically increasing sequence and match epoch. Only an opaque source-provided volatile session token is used; real match IDs are not required. The complete encoded HTTPS JSON body is limited to 32768 bytes. Old upload schema 2 is rejected; no v2 fallback. See the [full synthetic payload](docs/contract/live-match-api-v3-vectors.json) and [field definitions](docs/contract/live-match-api-v3.schema.json).

The channel key (dlm_ plus 64 ASCII hexadecimal characters) is entered into a masked Windows settings field and stored in the current user's Windows Credential Manager; it is sent in the Authorization header only. The desktop client runs in the notification area, checks for Deadlock every two seconds when idle, and supports optional opt-in HKCU Run autostart, credential removal and explicit exit. Never include it in a URL or issue report. A key is bound to a writer; rotate for a new process if required, not merely because of a long pause. Temporary failures use bounded backoff with new observations. Select Beenden in the Windows settings or tray menu to stop sending; closing the window hides it. Autostart is disabled separately. A 204 response is an acknowledgment, not proof of fresh game data.

## Transient backend and access

The intended internal backend keeps only the latest game snapshot in RAM, never in a database, file, backup, analytics or log. Display becomes stale after 12 seconds and expired payloads are removed after 60 seconds; rotation/revocation and process restart remove the snapshot. Minimal key and ordering metadata are separate from game values. An advancing unpaused clock with the same valid writer and higher sequence can resume after a long pause if the volatile watermark is still available. If it has been lost, an explicit epoch-expired conflict makes the client wait for a real new match without rotating. No fabricated heartbeat, old-packet replay or automatic writer takeover.

Full values are restricted to the authorized bot consumer and the owning partner session. The public overlay's previous small projection is unchanged and can be queried by anyone who knows the Twitch channel name. Its URL is not a secret and values can be linked to public stream or game information. The actual nonpersisting router and bot-consumer behavior must be tested; these are internal candidate requirements, not production privacy guarantees.

## Internal function test

The following is the intended test plan after a source is authorized. Currently only synthetic model/transport tests and isolated backend tests are applicable; the Windows executable stays in standby without uploads; only the non-Windows CLI retains the source-blocked exit. No public release or production deployment is authorized.

1. Verify matching schema/vector hashes. Run the Collector Rust checks and backend collector/router/bot-consumer tests.
2. Test a full synthetic payload through the actual HTTP router and authorized bot consumer, check every intended game-value group, cross-channel denial, public allowlist and no telemetry persistence (including expiry, restart, rotation and concurrent requests).
3. Build a Windows x64 test executable and record its SHA-256. Copy it into the authorized test environment. Run the setup in [Anleitung.txt](packaging/Anleitung.txt).
4. Check Windows tray/settings, dummy-key Credential Manager roundtrip, duplicate-instance protection, optional autostart on/off and exit in an isolated test environment. Check actual game attach/detach, connection, drift/eligibility and pause/resume only after the actual rules and test environment are authorized. Verify the bot consumes the observed values, not synthetic ones masquerading as live measurements.

A local unauthenticated routing/auth preflight for the matching endpoint uses the first synthetic vector; substitute the authorized HTTPS test base URL:
```sh
curl --disable --silent --show-error --output /dev/null --write-out '%{http_code}\n' \
  --connect-timeout 5 --max-time 10 --request POST \
  --header 'Content-Type: application/json' \
  --data-binary '{"schema_version":3,"writer_id":"123e4567-e89b-42d3-a456-426614174000","sequence":1,"match_epoch":1,"hero":"Infernus","game_seconds":42}' \
  https://deutsche-deadlock-community.de/twitch/api/v3/live-match
```
Do not run against production without permission. Expect exactly 401, which confirms routing/auth rejection only, not ingestion, schema or freshness. Use a dedicated authorized test key for authenticated end-to-end testing; do not log credentials.

## Windows safety and build

The program no longer reads game process memory. A complete authorized source is missing. No live game test or production use is permitted; do not bypass protection mechanisms.

Windows may show SmartScreen for an unsigned executable. Compare the provided SHA-256 with the executable or ZIP before starting; keep a file quarantined if antivirus reports a specific threat. Do not disable protection. To report a problem, use the version and file SHA-256, never the channel key.

Use the pinned Rust toolchain and Windows x64 build tools:
```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
cargo build --locked --release --target x86_64-pc-windows-gnu
Get-FileHash .\target\x86_64-pc-windows-gnu\release\live-match-collector.exe -Algorithm SHA256
```

Public distribution is a separate later step. Preserve third-party notices. Rights to distribute the executable/assets, actual privacy policy, real Windows/game and backend evidence, rule check, soak/outage/rollback, CI/scan/checksum/attestation, protected draft and final publication permission remain on that separate track. No tag, release or public push is necessary for the internal test.

## Development evidence

Linux CI runs independent Rust (default and all features), packaging/contract, locked dependency graph and CycloneDX SBOM, RustSec, Gitleaks, workflow lint, and Windows x64 build/PE/hash/reproducibility/ClamAV checks. `build` is an aggregate required check, not a publisher. CI uploads only selected text evidence, never EXE/ZIP. Two clean cross-builds use the pinned Rust toolchain, locked dependencies and deterministic linker configuration; actual equality and toolchain versions must be checked in the run evidence. A same-runner comparison is not an independently reproduced build. Hosted CI and native Windows behavior must be verified separately.

The earlier, different executable (SHA-256 `89e9ec4fda801a50f0fb8714a70b5dea69af533074482b5834527e56c261b161`) was reported to have four VirusTotal malicious classifications, zero suspicious and 66 undetected. [Historical hash report](https://www.virustotal.com/gui/file/89e9ec4fda801a50f0fb8714a70b5dea69af533074482b5834527e56c261b161). These findings are unresolved, not proven false positives or evidence about new bytes. No new VirusTotal result is claimed. Never disable protection, whitelist a finding simply to pass a gate, or send a binary to an external scanner without explicit permission for its hash.

### Windows-Oberfläche (0.2.2)

Die Nachtgold-Oberfläche nutzt native Win32-Steuerelemente für maskierte Eingabe und Tastaturnavigation, aber zeichnet Karten, Schaltflächen und Autostart-Umschalter selbst. Per-Monitor-DPI und ein eingebettetes eigenes Symbol benötigen keine zusätzlichen Laufzeitpakete. Die Statuskarten unterscheiden bestätigte HTTPS-Verbindung und lokalen Spielzustand. Dashboard und Repository werden ausschließlich über feste HTTPS-Adressen geöffnet. Schließen blendet das Fenster in den Infobereich aus; 'Collector beenden' stoppt den Hintergrundbetrieb. Windows-10/11-Anzeige, Skalierung und Tray müssen am realen Gerät abgenommen werden.

Version 0.2.3: gezielte optische Feinausrichtung. Schalter, Knopf und sichtbare Schrift teilen eine gemessene Mittelachse; Eingabefeld und Kopfzeile nutzen DPI-skalierte Innenabstände. Das bestehende Collector-Symbol bleibt erhalten.
