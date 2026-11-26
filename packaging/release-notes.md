Neu in dieser Version: Der Collector kann zusätzlich zum bisherigen Scoreboard den beobachteten Pausenstatus, spielbestätigte Mid-Boss-Daten und versionierte Matchdetails für das private Dashboard übertragen. Nicht verfügbare oder ungültige Werte werden ausgelassen oder als `null` übertragen. Bei erkannten Änderungen am Speicherlayout stoppt die Übertragung.

Lade die ZIP für Windows x64 herunter. Die Datei `SHA256SUMS` enthält die Prüfsumme der ZIP. In `clamav-report.txt` findest du den Virenscan der darin enthaltenen EXE mit ihrer eigenen Prüfsumme und dem Stand der Virensignaturen.

Windows SmartScreen kann beim ersten Start vor einer unbekannten App warnen. Der Collector hat keine Windows-Codesignatur. Prüfe die ZIP vor dem Start. Nennt dein Virenscanner eine konkrete Bedrohung, lass die Datei gesperrt und melde den Fund im [Issue-Tracker](https://github.com/EarlySalty/deadlock-live-match-collector/issues).

Der Collector liest die Spielwerte aus dem Arbeitsspeicher des laufenden Deadlock-Prozesses. Er schreibt nichts in das Spiel. Wie Valve mit solchen Leseprogrammen umgeht, entscheidet allein Valve. Mehr dazu steht in der beiliegenden `Anleitung.txt`.
