use std::io::{self, IsTerminal};
use std::thread;
use std::time::{Duration, Instant};

use deadlock_data::HeroCatalog;
use deadlock_reader::snapshot::{Context, LiveSnapshot, Perspective};
use deadlock_reader::supervise::{Attached, ReaderSupervisor};
use deadlock_reader::HeroNames;
use reqwest::blocking::Client;
use serde::Serialize;

const ENDPOINT: &str = "https://deutsche-deadlock-community.de/twitch/api/v2/live-match";
const PERIOD: Duration = Duration::from_secs(3);

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
}

fn payload(s: &LiveSnapshot, heroes: &HeroCatalog) -> Option<Payload> {
    if s.context != Context::Match
        || s.perspective != Perspective::Playing
        || !s.game_state?.is_live()
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
    let hero = heroes
        .hero_name(hero_id)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| hero_id.0.to_string());
    let seconds = s.clock.playing_seconds()?;
    if !seconds.is_finite() || seconds < 0.0 || seconds >= (u32::MAX as f32) {
        return None;
    }
    Some(Payload {
        match_id: id.to_string(),
        hero,
        game_seconds: seconds as u32,
        kills: player.kills?,
        deaths: player.deaths?,
        assists: player.assists?,
        team_souls: own?,
        enemy_souls: enemy?,
    })
}

fn read_token() -> io::Result<String> {
    let token = if io::stdin().is_terminal() {
        rpassword::prompt_password("Token: ")?
    } else {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        input
    };
    let token = token.trim().to_owned();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ungültiges Token",
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
            eprintln!("Das Token konnte nicht gelesen werden.");
            std::process::exit(1);
        }
    };
    let client = match Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
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
                    let error = match client.post(ENDPOINT).bearer_auth(&token).json(&data).send() {
                        Ok(response) if response.status().is_success() => None,
                        Ok(response) if response.status().as_u16() == 401 || response.status().as_u16() == 403 => {
                            eprintln!("Token abgelehnt. Bitte ein aktuelles Token erstellen oder erneut eingeben und das Programm neu starten.");
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
    use deadlock_core::{HeroId, Team};
    use deadlock_reader::snapshot::{GameState, MatchClock, PlayerRow};

    fn snapshot() -> LiveSnapshot {
        LiveSnapshot {
            match_id: Some(123),
            expected_player_count: Some(2),
            context: Context::Match,
            perspective: Perspective::Playing,
            game_state: Some(GameState::GameInProgress),
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
        assert_eq!(
            serde_json::to_value(data).unwrap(),
            serde_json::json!({"match_id":"123", "hero":"Infernus", "game_seconds":42, "kills":2, "deaths":3, "assists":4, "team_souls":500, "enemy_souls":600})
        );
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
