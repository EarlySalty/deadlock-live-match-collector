# Live match API v3 / upload schema 3

Internal function-test contract. This version restores the original live scoreboard, summary and detailed game values. It is not a deployed or published release. The [JSON Schema](live-match-api-v3.schema.json) and [synthetic vectors](live-match-api-v3-vectors.json) are the canonical wire artifacts.

## Transport and envelope

HTTPS POST /twitch/api/v3/live-match, Content-Type application/json, Authorization Bearer channel key (dlm_ followed by exactly 64 ASCII hexadecimal characters, case preserved). No credentials in URLs, no redirects, no v2 fallback. At most 32768 bytes of the complete encoded UTF-8 JSON, including escaping and whitespace. Root and nested objects reject additional fields. Upload schema 2 is incompatible and rejected (400); the old v2 upload path returns 410.

| Property | Contract |
| --- | --- |
| schema_version | Required integer, exactly 3 |
| writer_id | Required canonical lowercase hyphenated random UUIDv4 per process |
| sequence | Required integer 1–9007199254740991; increases per attempted upload |
| match_epoch | Required integer in the same range; increases only for a genuinely new, previously unseen match |
| hero | Required trimmed local hero, 1–80 UTF-8 bytes, no Unicode control characters |
| game_seconds | Required integer 0–86400; observed playing time |
| kills, deaths, assists | Optional nullable integer 0–1000 |
| team_souls, enemy_souls | Optional nullable integer 0–1000000000 |
| game_mode | Optional nullable normal or street_brawl |
| paused | Optional nullable boolean |
| midboss | Optional nullable game-observed normal-mode state; present boolean, source game, optional nullable countdown 0–7200; no countdown while present |
| details | Optional nullable full details object, schema_version=1 |

Keep real match IDs volatile and local. Unavailable statistics are omitted or null, never invented zeros. Skip ineligible, spectator, drifted or uncertain local-player/clock samples. Do not send names, Steam/account IDs, raw match IDs, addresses, raw memory, positions, hidden enemy statistics or ability cooldowns. A successful read, local sequence or wall clock is not a game heartbeat.

## Detail sections

Details schema 1 requires all five structural sections and the six distinct capability labels (scoreboard, teams, own_loadout, structures, objectives, timer_provenance). See the machine schema for exact field keys, types and bounds. Nullable members may be omitted. Only the listed game-visible values are in scope.

| Section | Content |
| --- | --- |
| scoreboard | 1–12 rows; own/enemy side, exactly one local own-side row, optional slot 1–12, hero ID, level, souls, K/D/A, alive; last hits, denies, damage and healing only for local row |
| teams | 0–2 distinct own/enemy rows; souls, K/D/A, rejuvenators, structures alive, completeness flag; ability points, last hits, denies, damage and healing only for own team |
| own_loadout | Local level, health/max health, souls/ability points, creep/secured/denied/neutral souls, farm baseline, kill streak; up to 64 item IDs and 16 ability upgrades (item_id, unlocked, points 0–5) |
| structures | Up to 48 visible structure rows, own/enemy/unknown side, guardian/walker/base_guardian/patron/shrine/midboss, optional lane 0–8, standing/destroyed/unknown |
| objectives | Nullable midboss, urn, rift, bridge_buffs; source game/schedule/unknown, optional presence, count 0–128, direct or estimated countdown 0–7200 |

Direct timer requires source=game; estimate requires source=schedule and is allowed only for bridge_buffs; neither when present=true. Unknown source has no timer. Top-level Mid-Boss and nonnull objective rows require normal mode. No guessed urn/rift/Mid-Boss timing. IDs are positive u32, general stats max 1e9, level max 1000. Signed health -1–1000000, max health 0–1000000. A team with incomplete observations may have null totals and complete=false.

JSON Schema does not cover transport, HTTP status classification, encoded byte size, Unicode byte-length/trim/control rules, or authenticated ordering. The local Rust test oracle checks closed structure, semantic constraints and these byte/text rules. It is not server implementation or game-source proof.

## Ordering, freshness and transient storage

A key is bound to the first accepted writer. Reject nonincreasing sequence, falling epoch/time or a competing writer. A new genuine epoch starts a match. In the same epoch, only strictly advancing observed unpaused clock refreshes. Pause-only toggles and identical samples do not extend freshness; advancing values while paused do not advance the clock. A long unchanged pause/frozen reader becomes stale.

The latest payload lives only in RAM. Display stale after 12 seconds; delete expired snapshot after 60 seconds, on rotation/revocation, and lose it on process restart. No telemetry payload in DB, files, backups, analytics or logs. Persist only minimal key/ordering/rate metadata, not game values; a separate minimal volatile clock watermark permits legitimate resume at a higher observed unpaused clock with the same writer and higher sequence. If that watermark is unavailable after expiry/restart, the server returns 409 with Live-Match-Conflict: epoch-expired only for the valid writer/sequence/current epoch. The client stops sending that epoch and waits for a genuinely new match without rotating the key. Other conflicts remain fatal. Do not fake a match transition, take over another writer or replay buffered observations. Unknown delayed packets with progress cannot be distinguished from fresh observations solely by ordering.

## HTTP

| Status | Meaning |
| --- | --- |
| 204 | Accepted or unchanged sample acknowledged; not proof of display or source freshness |
| 400 | Malformed JSON or invalid semantic values, including old upload schema |
| 401 | Missing/malformed/unknown/revoked/inactive key |
| 408 | Body read timeout |
| 409 | Writer/sequence/epoch/clock conflict; epoch-expired header as above |
| 413 | Encoded body exceeds 32768 bytes |
| 415 | Unsupported/missing content type after authentication |
| 422 | Invalid structure, types or unknown fields |
| 429 | Rate limit; Retry-After: 1 |
| 503 | Unavailable |

Authenticate before parsing; retain global ingress and per-key rate protection. HEAD on upload path validates active key without mutation (204/401/429/503), not writer binding or freshness. POST/HEAD empty responses, no-store. On temporary failures use bounded backoff and a newly read observation; do not resend an unacknowledged packet.

## Consumer and privacy boundary

Public GET /twitch/api/v2/public/live-match?streamer=... remains the existing allowlist: status, hero, game_seconds, paused, midboss and received_at; no full detail publication. Anyone with channel name can query this projection, which may be correlated with public game/stream data. Authenticated own-channel GET /twitch/api/v3/streamer/live-match-details provides full detail data only to an authorized partner session. Authorized in-process bot consumer reads channel-scoped snapshots. Private status GET /twitch/api/v3/streamer/live-match-status and existing own-channel key management remain. Verify actual router, consumer, no-store, scope, rotation, expiry, restart and nonpersisted payloads in an isolated backend test.

## Test evidence required

All shared cases are synthetic, independently tested with empty writer state. The compact minimal body is 136 bytes, full original-field fixture 4412 bytes and maximal-shape fixture 10084 bytes (not a theoretical maximum or actual match capture). Pad valid JSON to 32767/32768/32769 encoded bytes to test the exact cap. Compare schema and vector fingerprints between client and backend; exercise actual client serialization, router and bot consumer together, plus Windows x64 test executable and checksum. This local contract is not evidence that the backend is already deployed, or that use in a real game is authorized.
