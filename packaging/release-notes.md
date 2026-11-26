## Deadlock Live Match Collector 0.2.1

Windows x64, direkte HTTPS-Verbindung zum Twitch-Bot über API v3 / Upload-Schema 3. Kein lokaler Proxy, keine Test-CA. Lade bevorzugt das ZIP mit Anleitung und Drittanbieterhinweisen herunter; die separat angebotene EXE ist bytegleich. Vergleiche die SHA-256-Prüfsummen. Der Quellcode, die Tests und die Build-Herkunft sind auf GitHub einsehbar; die Attestation lässt sich mit `gh attestation verify <Datei> --repo EarlySalty/deadlock-live-match-collector` prüfen.

1. Im eigenen Partner-Dashboard unter Windows-Tool für Live-Overlays den Verbindungsschlüssel erstellen.
2. EXE starten, Schlüssel verdeckt eingeben und bei zulässigem Spielzugriff Deadlock spielen.
3. Live-Match-Details im eigenen Dashboard prüfen; Strg+C beendet den Collector.

Der Collector übermittelt Held, Spielzeit, Spielmodus, Pause, K/D/A, Team-Souls, sichtbares Scoreboard und Teams, eigenen Loadout, Strukturen und Objective-Timer. Keine Spielernamen, Steam-IDs, Rohspeicher oder tatsächlichen Match-IDs. Der Backend-Kandidat verwendet kurzlebige RAM-Snapshots statt eines Matcharchivs. Die öffentliche Overlay-Projektion bleibt klein. Ein erfolgreicher Upload ist kein Beleg für Spielfrische.

Der Collector liest Spielprozessspeicher. Read-only garantiert keine Zulässigkeit nach Valve-, Anti-Cheat- oder Turnierregeln. Keine Schutzmechanismen umgehen oder deaktivieren. Windows kann wegen fehlender Codesignatur warnen; konkrete Virenscanner-Funde nicht ignorieren. Der veröffentlichte Quellcode ist zur Überprüfung einsehbar; die Drittanbieterhinweise im ZIP bleiben maßgeblich. Keine pauschale MIT-Lizenz für fremde Komponenten oder Spieldaten.
