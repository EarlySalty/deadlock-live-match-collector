mod telemetry;

use std::io::{self, IsTerminal};
use std::thread;
use std::time::{Duration, Instant};

use deadlock_core::HeroId;
use deadlock_data::HeroCatalog;
use deadlock_reader::snapshot::{Context, GameMode, LiveSnapshot, Perspective};
use deadlock_reader::supervise::{Attached, ReaderSupervisor};
use deadlock_reader::timers::TimerSource;
use deadlock_reader::HeroNames;
use reqwest::blocking::Client;
use serde::Serialize;

const ENDPOINT: &str = "https://deutsche-deadlock-community.de/twitch/api/v2/live-match";
const PERIOD: Duration = Duration::from_secs(3);
const MAX_CLOCK_SECONDS: f32 = 86_400.0;
const MAX_SCORE: u32 = 1000;
const MAX_SOULS: u32 = 1_000_000_000;
const MAX_HERO_NAME: usize = 80;
const MAX_MIDBOSS_SECONDS: f32 = 7200.0;

#[derive(Debug, Serialize, PartialEq)]
struct MidbossTelemetry {
    present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_spawn_in_seconds: Option<u32>,
    source: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
struct Payload {
    match_id: String,
    hero: String,
    game_seconds: u32,
    kills: u32,
    deaths: u32,
    assists: u32,
    team_souls: u32,
    enemy_souls: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    game_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    midboss: Option<MidbossTelemetry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<telemetry::MatchDetails>,
}

fn midboss(s: &LiveSnapshot) -> Option<MidbossTelemetry> {
    if s.game_mode != Some(GameMode::Normal) {
        return None;
    }
    let timer = s.timers.midboss.as_ref()?;
    if timer.source != TimerSource::Game {
        return None;
    }
    let present = timer.present?;
    let next_spawn_in_seconds = timer
        .next_spawn_in
        .filter(|n| !present && n.is_finite() && *n >= 0.0 && *n <= MAX_MIDBOSS_SECONDS)
        .map(|n| n.ceil() as u32);
    Some(MidbossTelemetry {
        present,
        next_spawn_in_seconds,
        source: "game",
    })
}

fn hero_name(heroes: &HeroCatalog, id: HeroId) -> String {
    heroes
        .hero_name(id)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= MAX_HERO_NAME)
        .filter(|name| !name.chars().any(char::is_control))
        .map(str::to_owned)
        .unwrap_or_else(|| id.0.to_string())
}

fn score(value: Option<u32>) -> Option<u32> {
    value.filter(|n| *n <= MAX_SCORE)
}

fn souls(value: Option<u32>) -> Option<u32> {
    value.filter(|n| *n <= MAX_SOULS)
}

fn payload(s: &LiveSnapshot, heroes: &HeroCatalog) -> Option<Payload> {
    if s.context != Context::Match
        || s.perspective != Perspective::Playing
        || !s.game_state?.is_live()
        || !s.drift.is_empty()
        || s.dont_upload_stats == Some(true)
    {
        return None;
    }
    let id = s.match_id.filter(|id| *id > 0)?;
    let player = s.local_player()?;
    if !player.is_playing() || player.is_spectator {
        return None;
    }
    let team = player.team?;
    let expected = s.expected_player_count?;
    let mut players = 0;
    let mut own = None::<u32>;
    let mut enemy = None::<u32>;
    for row in s.scoreboard().filter(|row| !row.is_spectator) {
        players += 1;
        let total = if row.team == Some(team) {
            &mut own
        } else {
            &mut enemy
        };
        *total = Some(total.unwrap_or(0).checked_add(row.net_worth?)?);
    }
    if players != expected {
        return None;
    }
    let hero_id = player.hero_id?;
    if hero_id.0 == 0 {
        return None;
    }
    let seconds = s.clock.playing_seconds()?;
    if !seconds.is_finite() || !(0.0..=MAX_CLOCK_SECONDS).contains(&seconds) {
        return None;
    }
    let game_mode = match s.game_mode {
        Some(GameMode::Normal) => Some("normal"),
        Some(GameMode::StreetBrawl) => Some("street_brawl"),
        _ => None,
    };
    Some(Payload {
        match_id: id.to_string(),
        hero: hero_name(heroes, hero_id),
        game_seconds: seconds as u32,
        kills: score(player.kills)?,
        deaths: score(player.deaths)?,
        assists: score(player.assists)?,
        team_souls: souls(own)?,
        enemy_souls: souls(enemy)?,
        game_mode,
        paused: s.paused,
        midboss: midboss(s),
        details: game_mode
            .is_some()
            .then(|| telemetry::collect(s, player))
            .flatten(),
    })
}

fn read_token() -> io::Result<String> {
    let token = if io::stdin().is_terminal() {
        rpassword::prompt_password("Verbindungsschlüssel: ")?
    } else {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        input
    };
    let token = token.trim().to_owned();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ungültiger Verbindungsschlüssel",
        ));
    }
    Ok(token)
}

fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("Dieses Programm akzeptiert keine Befehlszeilenargumente.");
        std::process::exit(2);
    }
    let token = match read_token() {
        Ok(token) => token,
        Err(_) => {
            eprintln!("Der Verbindungsschlüssel konnte nicht gelesen werden.");
            std::process::exit(1);
        }
    };
    let client = match Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            eprintln!("Die sichere Verbindung konnte nicht eingerichtet werden.");
            std::process::exit(1);
        }
    };
    let heroes = HeroCatalog::bundled();
    let mut supervisor = ReaderSupervisor::new(PERIOD);
    eprintln!("Warte auf ein laufendes Deadlock-Match. Beenden mit Strg+C.");
    let mut last_transfer_error: Option<String> = None;
    loop {
        let started = Instant::now();
        let snapshot = match supervisor.acquire() {
            Attached::Fresh(reader) | Attached::Held(reader) => Some(reader.live_snapshot()),
            Attached::Failed(reason) => {
                eprintln!("Deadlock konnte nicht geöffnet werden: {reason}. Bitte Spiel und Zugriffsrechte prüfen.");
                None
            }
            _ => None,
        };
        match snapshot {
            Some(Ok(snapshot)) => {
                supervisor.succeeded();
                if let Some(data) = snapshot.as_ref().and_then(|s| payload(s, &heroes)) {
                    let budget = PERIOD.saturating_sub(started.elapsed());
                    if budget.is_zero() {
                        continue;
                    }
                    let error = match client
                        .post(ENDPOINT)
                        .timeout(budget)
                        .bearer_auth(&token)
                        .json(&data)
                        .send()
                    {
                        Ok(response) if response.status().is_success() => None,
                        Ok(response) if response.status().as_u16() == 401 || response.status().as_u16() == 403 => {
                            eprintln!("Verbindungsschlüssel abgelehnt. Erstelle im Overlay-Baukasten einen neuen Schlüssel und starte das Programm neu.");
                            std::process::exit(1);
                        }
                        Ok(response) => Some(format!("Übertragung abgelehnt (HTTP {}).", response.status().as_u16())),
                        Err(_) => Some("Übertragung fehlgeschlagen. Beim nächsten Messwert wird erneut versucht.".to_owned()),
                    };
                    if error != last_transfer_error {
                        if let Some(message) = &error {
                            eprintln!("{message}");
                        } else if last_transfer_error.is_some() {
                            eprintln!("Die Übertragung funktioniert wieder.");
                        }
                        last_transfer_error = error;
                    }
                }
            }
            Some(Err(_)) => {
                supervisor.failed();
            }
            None => {}
        }
        thread::sleep(PERIOD.saturating_sub(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::Team;
    use deadlock_reader::snapshot::{GameState, MatchClock, PlayerRow, TeamStats};
    use deadlock_reader::timers::ObjectiveTimer;

    fn snapshot() -> LiveSnapshot {
        LiveSnapshot {
            match_id: Some(123),
            expected_player_count: Some(2),
            context: Context::Match,
            perspective: Perspective::Playing,
            game_state: Some(GameState::GameInProgress),
            game_mode: Some(GameMode::Normal),
            clock: MatchClock {
                game_start_time: Some(10.0),
                now: Some(52.8),
                ..Default::default()
            },
            players: vec![
                PlayerRow {
                    is_local: Some(true),
                    team: Some(Team::AMBER),
                    hero_id: Some(HeroId(1)),
                    kills: Some(2),
                    deaths: Some(3),
                    assists: Some(4),
                    net_worth: Some(500),
                    ..Default::default()
                },
                PlayerRow {
                    team: Some(Team::SAPPHIRE),
                    net_worth: Some(600),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn emits_flat_local_payload() {
        let data = payload(&snapshot(), &HeroCatalog::bundled()).unwrap();
        let actual = serde_json::to_value(data).unwrap();
        for (key, value) in serde_json::json!({
            "match_id":"123", "hero":"Infernus", "game_seconds":42,
            "kills":2, "deaths":3, "assists":4,
            "team_souls":500, "enemy_souls":600, "game_mode":"normal"
        })
        .as_object()
        .unwrap()
        {
            assert_eq!(&actual[key], value, "field {key}");
        }
        assert_eq!(actual["details"]["schema_version"], 1);
        assert_eq!(actual["details"]["scoreboard"].as_array().unwrap().len(), 2);
        assert!(actual["details"]["own_loadout"].is_object());
        assert!(actual["details"]["objectives"].is_object());
        let serialized = actual.to_string();
        for hidden in [
            "steam_id",
            "controller",
            "pawn_handle",
            "position",
            "ultimate_cooldown",
            "raw_match_clock",
            "modifiers",
        ] {
            assert!(!serialized.contains(hidden), "leak: {hidden}");
        }
    }

    #[test]
    fn rich_snapshot_contains_only_safe_cross_player_values() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.players[0].hero_damage = Some(100);
        s.players[0].items = Some(vec![1234, 5678]);
        s.players[1].hero_damage = Some(999);
        s.players[1].last_hits = Some(300);
        s.teams = vec![
            TeamStats {
                team: Team::AMBER,
                players: 1,
                hero_damage: 100,
                ..Default::default()
            },
            TeamStats {
                team: Team::SAPPHIRE,
                players: 1,
                hero_damage: 999,
                ..Default::default()
            },
        ];
        let body = serde_json::to_value(payload(&s, &heroes).unwrap()).unwrap();
        assert_eq!(body["details"]["scoreboard"][0]["hero_damage"], 100);
        assert!(body["details"]["scoreboard"][1]["hero_damage"].is_null());
        assert!(body["details"]["scoreboard"][1]["last_hits"].is_null());
        assert_eq!(body["details"]["own_loadout"]["items"][0], 1234);
        assert_eq!(body["details"]["teams"][0]["hero_damage"], 100);
        assert!(body["details"]["teams"][1]["hero_damage"].is_null());
    }

    #[test]
    fn rejects_unusable_or_observed_snapshots() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.perspective = Perspective::Spectating;
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.players[0].is_local = Some(false);
        s.players[0].is_observed = true;
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.match_id = Some(0);
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.players[1].net_worth = None;
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.game_state = Some(GameState::PostGame);
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.players[0].kills = None;
        assert!(payload(&s, &heroes).is_none());
    }

    #[test]
    fn forwards_only_game_backed_midboss_timer_and_pause_state() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.paused = Some(true);
        s.timers.midboss = Some(ObjectiveTimer {
            present: Some(false),
            count: Some(0),
            next_spawn_in: Some(41.2),
            source: TimerSource::Game,
        });
        let result = payload(&s, &heroes).unwrap();
        assert_eq!(result.paused, Some(true));
        assert_eq!(
            result.midboss.as_ref().unwrap().next_spawn_in_seconds,
            Some(42)
        );
        assert_eq!(result.midboss.as_ref().unwrap().source, "game");
        s.timers.midboss.as_mut().unwrap().source = TimerSource::Schedule;
        assert_eq!(payload(&s, &heroes).unwrap().midboss, None);
        s.timers.midboss.as_mut().unwrap().source = TimerSource::Unknown;
        assert_eq!(payload(&s, &heroes).unwrap().midboss, None);
        s.timers.midboss.as_mut().unwrap().source = TimerSource::Game;
        s.timers.midboss.as_mut().unwrap().present = None;
        assert_eq!(payload(&s, &heroes).unwrap().midboss, None);
        s.timers.midboss.as_mut().unwrap().present = Some(false);
        s.game_mode = None;
        assert_eq!(payload(&s, &heroes).unwrap().midboss, None);
    }

    #[test]
    fn never_guesses_an_already_live_or_invalid_midboss_timer() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.timers.midboss = Some(ObjectiveTimer {
            present: Some(true),
            count: Some(1),
            next_spawn_in: Some(100.0),
            source: TimerSource::Game,
        });
        assert_eq!(
            payload(&s, &heroes)
                .unwrap()
                .midboss
                .unwrap()
                .next_spawn_in_seconds,
            None
        );
        s.timers.midboss.as_mut().unwrap().present = Some(false);
        for invalid in [f32::NAN, -1.0, f32::INFINITY, 100_000.0] {
            s.timers.midboss.as_mut().unwrap().next_spawn_in = Some(invalid);
            assert_eq!(
                payload(&s, &heroes)
                    .unwrap()
                    .midboss
                    .unwrap()
                    .next_spawn_in_seconds,
                None
            );
        }
        s.dont_upload_stats = Some(true);
        assert!(payload(&s, &heroes).is_none());
    }

    #[test]
    fn street_brawl_does_not_expose_normal_mode_objectives() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.game_mode = Some(GameMode::StreetBrawl);
        s.timers.midboss = Some(ObjectiveTimer {
            present: Some(false),
            count: Some(0),
            next_spawn_in: Some(10.0),
            source: TimerSource::Game,
        });
        let actual = payload(&s, &heroes).unwrap();
        assert_eq!(actual.game_mode, Some("street_brawl"));
        assert_eq!(actual.midboss, None);
        let details = actual.details.unwrap();
        assert!(details.objectives.midboss.is_none());
        assert_eq!(details.scoreboard.len(), 2);
    }

    #[test]
    fn stays_inside_the_limits_the_service_accepts() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.players[0].kills = Some(1001);
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.players[1].net_worth = Some(1_000_000_001);
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.clock.now = Some(90_000.0);
        assert!(payload(&s, &heroes).is_none());
        s = snapshot();
        s.players[0].level = Some(5000);
        s.players[0].health = Some(-7);
        s.players[0].max_health = Some(2_000_000);
        s.players[0].hero_damage = Some(u32::MAX);
        s.players[1].level = Some(5000);
        s.players[1].slot = Some(40);
        s.teams = vec![
            TeamStats {
                team: Team::AMBER,
                players: 1,
                rejuvenators: Some(5000),
                ..Default::default()
            },
            TeamStats {
                team: Team::SAPPHIRE,
                players: 1,
                kills: u32::MAX,
                ..Default::default()
            },
        ];
        let details = payload(&s, &heroes).unwrap().details.unwrap();
        assert_eq!(details.scoreboard[0].level, None);
        assert_eq!(details.scoreboard[0].hero_damage, None);
        assert_eq!(details.scoreboard[1].level, None);
        assert_eq!(details.scoreboard[1].slot, None);
        assert_eq!(details.own_loadout.level, None);
        assert_eq!(details.own_loadout.health, None);
        assert_eq!(details.own_loadout.max_health, None);
        assert_eq!(details.teams.len(), 1);
        assert_eq!(details.teams[0].rejuvenators, None);
        s.players[1].is_local = Some(true);
        assert!(payload(&s, &heroes).unwrap().details.is_none());
    }

    #[test]
    fn waits_for_the_expected_roster() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        for expected in [None, Some(0), Some(12), Some(u32::MAX)] {
            s.expected_player_count = expected;
            assert!(payload(&s, &heroes).is_none());
        }
    }

    #[test]
    fn sums_net_worth_and_allows_unknown_hero() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.players[0].hero_id = Some(HeroId(u32::MAX));
        s.players.push(PlayerRow {
            team: Some(Team::AMBER),
            net_worth: Some(50),
            healing: None,
            assists: None,
            ..Default::default()
        });
        s.expected_player_count = Some(3);
        let data = payload(&s, &heroes).unwrap();
        assert_eq!(data.hero, u32::MAX.to_string());
        assert_eq!(data.team_souls, 550);
        assert_eq!(data.enemy_souls, 600);
        s.players[2].net_worth = None;
        assert!(payload(&s, &heroes).is_none());
        s.players[2].net_worth = Some(u32::MAX);
        assert!(payload(&s, &heroes).is_none());
        s.players.truncate(1);
        assert!(payload(&s, &heroes).is_none());
    }
}
