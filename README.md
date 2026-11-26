# Deadlock live match collector

This Windows x64 console app sends your live scoreboard to the Deutsche Deadlock Community Twitch overlay. Run it on the PC where you play Deadlock. You need a connection key from the overlay builder. [Windows ZIP and checksum](https://github.com/EarlySalty/deadlock-live-match-collector/releases) · [Setup instructions in German](packaging/Anleitung.txt)

## Beim ersten Start unter Windows

Windows kann „Der Computer wurde durch Windows geschützt“ anzeigen. Der Collector hat keine Windows-Codesignatur. Bei selten heruntergeladenen Programmen kann SmartScreen deshalb vor einer unbekannten App warnen. Vergleiche vor dem Start die SHA-256-Prüfsumme deiner ZIP mit der Datei `SHA256SUMS` aus demselben Release. Wenn die Werte übereinstimmen und du der Quelle vertraust, kannst du den Start bestätigen, sofern Windows diese Möglichkeit anbietet. [So bewertet Microsoft die App-Reputation](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)

Nennt dein Virenscanner eine konkrete Bedrohung oder stellt die EXE unter Quarantäne, lass sie dort. Melde den Erkennungsnamen, die Version des Collectors und die SHA-256-Prüfsumme der EXE im [Issue-Tracker](https://github.com/EarlySalty/deadlock-live-match-collector/issues). Dein Verbindungsschlüssel gehört nur in das Eingabefeld des Collectors.

## How the overlay gets its data

The Windows part lives in [src/main.rs](src/main.rs) and [src/telemetry.rs](src/telemetry.rs). The main loop waits for Deadlock, attaches through the reader and requests a live snapshot. The `prepare` function decides whether anything may be sent. The `summary` function checks the roster, totals each team’s souls and selects the local player’s K/D/A. The `collect` function in the telemetry module builds the detail record field by field. The uploader sends the result to the Twitch service. The service handles channel pairing, expires old snapshots and supplies the OBS overlays.

Every range the service accepts is defined once in [src/limits.rs](src/limits.rs). The tests pass each payload through [src/contract.rs](src/contract.rs), a copy of the service’s upload check taken from the revision named at the top of that file. The copy is compiled for tests only and is not part of the executable.

The game-specific process reader and hero catalogue are Rust dependencies pinned to a specific revision in [Cargo.toml](Cargo.toml). You can inspect the game-reading implementation in [deadlock-reader](https://github.com/deadlock-plus/deadlock-rs/tree/8b408b9103a022e7bac620bfa4916778b8fdfaa1/crates/deadlock-reader).

## What the collector reads and sends

The collector gets its values by reading the memory of the Deadlock process running on your PC. It only reads. It does not write to the game, inject code or hook functions. Valve decides how it treats external readers, and neither this project nor the reader library can promise how that policy applies. Decide for yourself whether you want to run it on your account.

Every three seconds the collector asks the read-only deadlock-rs library for a snapshot of the running game. It checks that you are playing, identifies your hero and team, and sums the players’ net worth for each side. [Collector source](src/main.rs) · [Pinned reader source](https://github.com/deadlock-plus/deadlock-rs/tree/8b408b9103a022e7bac620bfa4916778b8fdfaa1)

The collector sends nothing at all in these cases, and the console window names the reason once it has held for a few samples.

- The reader reports schema drift, which means the game’s memory layout no longer matches this build. Any drift entry stops every upload, inside and outside a match.
- You are not playing a live match. That covers the menu, spectating and the time after a match has ended.
- The game marks the match `dont_upload_stats`.
- The summary cannot be built. This happens when the match ID or your hero is missing, when the number of players differs from the number the game expects, when a player has no net worth or is on neither of the two playing teams, or when a summary value lies outside the range the service accepts.

The collector sends one HTTPS POST to `https://deutsche-deadlock-community.de/twitch/api/v2/live-match` for each usable snapshot. The body always starts with these summary fields.

```json
{"match_id":"123","hero":"Infernus","game_seconds":42,"kills":2,"deaths":3,"assists":4,"team_souls":500,"enemy_souls":600,"game_mode":"normal","paused":false,"midboss":{"present":false,"next_spawn_in_seconds":90,"source":"game"}}
```

`game_mode`, `paused` and `midboss` are optional and are left out when unknown. The collector sends `midboss` only in the standard mode and only when the game itself reports the Mid-Boss state. `next_spawn_in_seconds` is left out while the Mid-Boss is on the map and when the game’s value lies outside the accepted range. The values above are an example, not a live measurement.

The body also carries a `details` object with `schema_version` 1 when all of the following hold: the game mode is known, your team is one of the two playing teams, the scoreboard has between 1 and 12 players, and exactly one of them is you. Otherwise the summary is sent without `details`. This shortened example shows every section and every field name.

```json
{
  "schema_version": 1,
  "capabilities": ["scoreboard", "teams", "own_loadout", "structures", "objectives", "timer_provenance"],
  "scoreboard": [
    {"side": "own", "is_local": true, "slot": 1, "hero_id": 1, "level": 12, "souls": 9500, "kills": 2, "deaths": 3, "assists": 4, "last_hits": 80, "denies": 6, "hero_damage": 7000, "objective_damage": 1500, "healing": 300, "alive": true},
    {"side": "enemy", "is_local": false, "slot": 7, "hero_id": 2, "level": 11, "souls": 9100, "kills": 3, "deaths": 2, "assists": 1, "last_hits": null, "denies": null, "hero_damage": null, "objective_damage": null, "healing": null, "alive": true}
  ],
  "teams": [
    {"side": "own", "souls": 52000, "ability_points": 60, "kills": 14, "deaths": 12, "assists": 30, "last_hits": 410, "denies": 25, "hero_damage": 41000, "objective_damage": 9000, "healing": 2500, "rejuvenators": 0, "structures_alive": 9, "complete": true},
    {"side": "enemy", "souls": 50500, "ability_points": null, "kills": 12, "deaths": 14, "assists": 22, "last_hits": null, "denies": null, "hero_damage": null, "objective_damage": null, "healing": null, "rejuvenators": 0, "structures_alive": 8, "complete": true}
  ],
  "own_loadout": {"level": 12, "health": 1450, "max_health": 1800, "souls": 9500, "ability_points": 10, "creep_souls": 5200, "secured_souls": 900, "denied_souls": 300, "neutral_souls": 1100, "farm_baseline": 8000, "kill_streak": 1, "items": [1234, 5678], "ability_upgrades": [{"item_id": 4321, "unlocked": true, "points": 2}]},
  "structures": [{"side": "enemy", "kind": "walker", "lane": 1, "state": "standing"}],
  "objectives": {
    "midboss": {"present": false, "count": 0, "source": "game", "next_spawn_in_seconds": 90, "estimated_spawn_in_seconds": null},
    "urn": null,
    "rift": null,
    "bridge_buffs": null
  }
}
```

The detail record follows these rules.

- The collector builds it from a fixed list of fields. It never serialises the reader’s snapshot as a whole.
- It contains no Steam IDs, player names, positions, process addresses or enemy ability cooldowns.
- Last hits, denies, damage and healing are sent for your own row and your own team only. Items, ability upgrades and health are sent for your own hero only.
- A single value the reader cannot supply, or one outside the range the service accepts, is sent as `null`. The same applies to a player’s `hero_id` of 0 and to a `slot` or `lane` outside the accepted range.
- `items` and `ability_upgrades` are sent whole or as `null`, never in part. Empty slots (ID 0) are not entries and are left out first. If the list is then longer than the service accepts, or an ability upgrade has more points than the service accepts, the whole list is `null`.
- A team row is left out when the team has no players, when it is not one of the two playing teams or when its souls, kills, deaths or assists exceed the accepted range. `teams` is an empty list when more than two rows remain or both rows would describe the same side.
- `structures` lists guardians, walkers, base guardians, patrons, shrines and the Mid-Boss. Other objects such as lane troopers and jungle camps are never sent. The list is empty when it would hold more rows than the service accepts.
- `side` is `own` or `enemy` for the two playing teams only. A structure or player that belongs to neither of them has `side` `unknown`.
- `next_spawn_in_seconds` is filled only for a timer with `source` `game`, which comes directly from the game. `estimated_spawn_in_seconds` is filled only for the bridge buffs and only with `source` `schedule`, which is an estimate. Neither is filled while the objective is on the map or when the time lies outside the accepted range. In other game modes all four objectives are `null`.

The service is meant to show the detail record only to the signed-in streamer of the channel. The public overlay works from the summary fields.

The connection key travels in the Authorization header. The interactive prompt hides it as you type, and the collector keeps it in memory until you close the program. Press Ctrl+C to stop sending updates. Generate a new key in the overlay builder to invalidate the previous one. Never put the key into an OBS URL.

The collector ends on its own only when the service answers HTTP 401, which means the key was rejected. After any other failed upload it reports the failure and tries again with the next sample. It does not follow redirects and waits at most five seconds for an answer.

## Verify a download

The release includes `clamav-report.txt` with the signature database version, SHA-256 of the extracted executable and ClamAV result for that file.

Download the ZIP and `SHA256SUMS` from the same release. In the commands below, replace `VERSION` with the version in the file name of your download. In PowerShell, run the following command and compare the result with the checksum in that file.

```powershell
Get-FileHash .\live-match-collector-VERSION-win64.zip -Algorithm SHA256
```

To verify the GitHub build attestation, use a recent version of the GitHub CLI.

```powershell
gh attestation verify .\live-match-collector-VERSION-win64.zip --repo EarlySalty/deadlock-live-match-collector
```

## Release and deployment checklist (maintainers)

The collector and the Twitch service are released separately. Before publishing a new collector release, verify that the deployed Twitch service supports `POST /twitch/api/v2/live-match` and the versioned details contract in `src/contract.rs`. The collector's tests use an offline copy of that contract and cannot establish deployment compatibility.

1. Review the service changes against its current `main`, merge and deploy them first. Confirm that an unauthenticated POST to the route returns 401 (not 404).
2. With a dedicated test channel and a valid connection key, verify a live match upload returns 204, the private dashboard receives the details, and the public overlay receives only the summary. Also verify that an invalidated key returns 401. Never include connection keys in logs or review evidence.
3. Review and merge the collector after its branch/PR CI succeeds. Verify the packaged Windows executable on a Windows x64 machine, including startup, key entry, match detection, schema drift handling and shutdown.
4. After these gates pass, tag the merged commit with `v` followed by the version in `Cargo.toml` and push the tag. The release workflow requires the tagged commit to be on `main`, builds and scans the ZIP, attests it and publishes the release. Verify the published checksum and attestation before distributing it.

## Build from source

Check out the release tag you want to inspect. On Windows x64, use a stable Rust toolchain with the MSVC build tools and run the following commands.

```powershell
cargo test --locked
cargo build --locked --release
```

With Cargo’s default target directory, the executable is written to `target/release/live-match-collector.exe`. The release workflow cross-compiles the Windows GNU target on Ubuntu. The packaging script needs rustup with the `x86_64-pc-windows-gnu` target, the Ubuntu MinGW-w64 packages, zip and jq. It removes the ZIP, checksum file and scan report of an earlier run from `dist/`, puts the executable, setup instructions and dependency notices into one ZIP there, writes `SHA256SUMS` and prints the path of the ZIP. It stops if the executable would need a MinGW runtime DLL. The scan script takes that path, runs ClamAV on the packaged executable and writes the report next to the ZIP. A second workflow runs the same build, packaging and scan on every branch push without publishing anything. [Release workflow](.github/workflows/release.yml) · [Build check](.github/workflows/ci.yml) · [Packaging script](packaging/package-win64.sh) · [Scan script](packaging/scan-win64.sh)
