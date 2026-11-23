# Deadlock live match collector

This Windows x64 console app sends your live scoreboard to the Deutsche Deadlock Community Twitch overlay. Run it on the PC where you play Deadlock. You need a channel key from the overlay builder. [Windows ZIP and checksum](https://github.com/EarlySalty/deadlock-live-match-collector/releases) · [Setup instructions in German](packaging/Anleitung.txt)

## Beim ersten Start unter Windows

Windows kann „Der Computer wurde durch Windows geschützt“ anzeigen. Der Collector hat keine Windows-Codesignatur. Bei selten heruntergeladenen Programmen kann SmartScreen deshalb vor einer unbekannten App warnen. Vergleiche vor dem Start die SHA-256-Prüfsumme deiner ZIP mit der Datei `SHA256SUMS` aus demselben Release. Wenn die Werte übereinstimmen und du der Quelle vertraust, kannst du den Start bestätigen, sofern Windows diese Möglichkeit anbietet. [So bewertet Microsoft die App-Reputation](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)

Nennt dein Virenscanner eine konkrete Bedrohung oder stellt die EXE unter Quarantäne, lass sie dort. Melde den Erkennungsnamen, die Version des Collectors und die SHA-256-Prüfsumme der EXE im [Issue-Tracker](https://github.com/EarlySalty/deadlock-live-match-collector/issues). Dein Kanalschlüssel gehört nur in das Eingabefeld des Collectors.

## How the overlay gets its data

The Windows part lives in [src/main.rs](src/main.rs). The main loop waits for Deadlock, attaches through the reader and requests a live snapshot. The `payload` function checks the match state and roster, totals each team’s souls and selects the local player’s K/D/A. The uploader sends that snapshot to the Twitch service. The service handles channel pairing, expires old snapshots and supplies the rotating OBS card.

The game-specific process reader and hero catalogue are Rust dependencies pinned to a specific revision in [Cargo.toml](Cargo.toml). You can inspect the game-reading implementation in [deadlock-reader](https://github.com/deadlock-plus/deadlock-rs/tree/8b408b9103a022e7bac620bfa4916778b8fdfaa1/crates/deadlock-reader).

## What the collector reads and sends

**Neue Entwicklungsfunktion (noch nicht als Windows-Release veröffentlicht):**
Der Collector überträgt zusätzlich den Pausenstatus, falls dieser vom Reader
eindeutig vorliegt, und ausschließlich einen vom Spiel selbst gemeldeten
Mid-Boss-Zustand bzw. dessen nächsten Spawn. Die Herkunft wird mit
`source: "game"` markiert. Rift-, Urnen-, Jungle- und Bridge-Buff-Respawns
bleiben **unbekannt**, wenn die Client-Daten keine zuverlässige nächste
Spawnzeit liefern. Es gibt keinerlei Hochrechnung aus vorherigen Kills,
keine Gegnerpositionen, keine versteckten Fähigkeiten-Cooldowns und keine
automatische Weiterzählung im OBS-Browser.

Serverseitige Ergänzung im Twitch-Bot:
`POST /twitch/api/v2/live-match` (bestehender Pfad),
`GET /twitch/api/v2/public/live-match?streamer=...` (kurzlebige Anzeige),
`GET/POST/DELETE /twitch/api/v2/streamer/live-match-key` (eigener Kanal).
Der neue Schlüssel wird vom eingeloggten Streamer einmalig erzeugt und beim
Neuerstellen der alte widerrufen. **Nicht** in der OBS-URL verwenden.
Der Server muss mit der zugehörigen Datenbankmigration ausgerollt sein,
bevor der Collector mit diesem Branch verwendet werden kann.

Every three seconds the collector asks the read-only deadlock-rs library for a snapshot of the running game. It checks that you are playing, identifies your hero and team, and sums the players’ net worth for each side. It skips spectator sessions and incomplete scoreboards. [Collector source](src/main.rs) · [Pinned reader source](https://github.com/deadlock-plus/deadlock-rs/tree/8b408b9103a022e7bac620bfa4916778b8fdfaa1)

The collector sends one HTTPS POST to `https://deutsche-deadlock-community.de/twitch/api/v2/live-match` for each usable snapshot. This example shows the entire JSON body.

```json
{"match_id":"123","hero":"Infernus","game_seconds":42,"kills":2,"deaths":3,"assists":4,"team_souls":500,"enemy_souls":600,"game_mode":"normal","paused":false,"midboss":{"present":false,"next_spawn_in_seconds":90,"source":"game"}}
```

The optional `paused` and `midboss` properties are sent only when the pinned reader actually supplies them; the JSON above is an example, not a live measurement. If the game marks the match `dont_upload_stats` or the reader reports schema drift, the collector skips the snapshot.

The channel key travels in the Authorization header. The interactive prompt hides it as you type, and the collector keeps it in memory until you close the program. The service uses these values for your public overlay. Press Ctrl+C to stop sending updates. Generate a new key in the builder to invalidate the previous one.

## Verify a download
The release includes `clamav-report.txt` with the signature database version, SHA-256 of the extracted executable and ClamAV result for that file.

Download the ZIP and `SHA256SUMS` from the same release. In PowerShell, run the following command and compare the result with the checksum in that file.

```powershell
Get-FileHash .\live-match-collector-0.1.0-win64.zip -Algorithm SHA256
```

To verify the GitHub build attestation, use a recent version of the GitHub CLI.

```powershell
gh attestation verify .\live-match-collector-0.1.0-win64.zip --repo EarlySalty/deadlock-live-match-collector
```

## Build from source

Check out the release tag you want to inspect. On Windows x64, use a stable Rust toolchain with the MSVC build tools and run the following commands.

```powershell
cargo test --locked
cargo build --locked --release
```

With Cargo’s default target directory, the executable is written to `target/release/live-match-collector.exe`. The release workflow cross-compiles the Windows GNU target on Linux. Its packaging script puts the executable, setup instructions and dependency notices in `dist/`. [Release workflow](.github/workflows/release.yml) · [Packaging script](packaging/package-win64.sh)
