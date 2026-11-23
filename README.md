# Deadlock live match collector

This Windows x64 console app sends your live scoreboard to the Deutsche Deadlock Community Twitch overlay. Run it on the PC where you play Deadlock. You need a channel key from the overlay builder. [Windows ZIP and checksum](https://github.com/EarlySalty/deadlock-live-match-collector/releases) · [Setup instructions in German](packaging/Anleitung.txt)

## What the collector reads and sends

Every three seconds the collector asks the read-only deadlock-rs library for a snapshot of the running game. It checks that you are playing, identifies your hero and team, and sums the players’ net worth for each side. It skips spectator sessions and incomplete scoreboards. [Collector source](src/main.rs) · [Pinned reader source](https://github.com/deadlock-plus/deadlock-rs/tree/8b408b9103a022e7bac620bfa4916778b8fdfaa1)

The collector sends one HTTPS POST to `https://deutsche-deadlock-community.de/twitch/api/v2/live-match` for each usable snapshot. This example shows the entire JSON body.

```json
{"match_id":"123","hero":"Infernus","game_seconds":42,"kills":2,"deaths":3,"assists":4,"team_souls":500,"enemy_souls":600}
```

The channel key travels in the Authorization header. The interactive prompt hides it as you type, and the collector keeps it in memory until you close the program. The service uses these values for your public overlay. Press Ctrl+C to stop sending updates. Generate a new key in the builder to invalidate the previous one.

## Verify a download

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
