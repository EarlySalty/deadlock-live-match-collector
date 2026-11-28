# Deadlock Live Match Collector v0.3.1 — Preview

This Windows x64 pre-release is intended for evaluating the desktop client. It is not a functional live-match integration.

## Bug fix

Windows sign-in autostart now stays in the notification area even when no connection key is saved. Manual launches still open the main window. No changes to live-data access or backend integration.

## What's included

- Native Windows desktop and notification-area interface with high-DPI support and Deutsche Deadlock Community branding.
- Per-user installation without administrator privileges, with manual upgrades and downgrade protection.
- Optional sign-in autostart, tray controls and a clear separation between game and connection status.
- Masked connection-key entry and storage in Windows Credential Manager.
- Update notifications with release notes and a link to the official GitHub release. Updates are never downloaded, installed or executed automatically.

## Getting started

1. Download `live-match-collector.exe` or the ZIP package with setup instructions and third-party notices.
2. Compare the SHA-256 of your download with the corresponding checksum file before opening it.
3. Follow the included setup instructions. This preview is for isolated evaluation only; do not use a real connection key.
4. Close the window to keep the application in the tray, or choose Exit to stop it. Autostart is optional.

To opt into public pre-release notifications, set `DEADLOCK_COLLECTOR_PUBLIC_TEST_CHANNEL=1` in the process environment before starting the collector. The default channel excludes pre-releases.

## Known limitations

- No authorized complete live-game data source is available. The client does not collect or upload live match values; Twitch bot integration is not available for production use.
- Native Windows end-to-end acceptance and production backend validation are pending.
- The executable is unsigned. Do not disable security software or ignore specific antivirus warnings.

The full ZIP includes third-party notices. SHA-256 checksums and the exact executable's security report accompany the release when publication is authorized.
