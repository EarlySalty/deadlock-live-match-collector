#[cfg(test)]
mod contract;
mod limits;
mod telemetry;

use std::io::{self, IsTerminal};
use std::mem::discriminant;
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

use telemetry::Side;

const ENDPOINT: &str = "https://deutsche-deadlock-community.de/twitch/api/v2/live-match";
const PERIOD: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const KEY_REJECTED: &str = "Verbindungsschlüssel abgelehnt. Erstelle im Overlay-Baukasten einen neuen Schlüssel und starte das Programm neu.";

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
    Some(MidbossTelemetry {
        present,
        next_spawn_in_seconds: limits::timer_seconds(timer.next_spawn_in).filter(|_| !present),
        source: "game",
    })
}

fn hero_name(heroes: &HeroCatalog, id: HeroId) -> String {
    heroes
        .hero_name(id)
        .and_then(limits::hero_name)
        .map(str::to_owned)
        .unwrap_or_else(|| id.0.to_string())
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Waiting,
    Blocked(String),
    ReadFailed,
    Outdated,
    Excluded,
    Unreadable,
    Sending,
    Rejected(u16),
    Unreachable,
}

impl Status {
    fn patience(&self) -> u8 {
        match self {
            Status::Sending => 1,
            Status::Rejected(_) | Status::Unreachable | Status::Blocked(_) => 3,
            _ => 5,
        }
    }

    fn failing(&self) -> bool {
        matches!(self, Status::Rejected(_) | Status::Unreachable)
    }

    fn continues(&self, earlier: &Status) -> bool {
        discriminant(self) == discriminant(earlier) || (self.failing() && earlier.failing())
    }

    fn message(&self) -> String {
        match self {
            Status::Waiting => "Warte auf ein laufendes Deadlock-Match.".to_owned(),
            Status::Blocked(reason) => format!("Deadlock konnte nicht geöffnet werden: {reason}. Bitte Spiel und Zugriffsrechte prüfen."),
            Status::ReadFailed => "Deadlock läuft, aber die Spieldaten lassen sich gerade nicht lesen. Es wird weiter versucht. Bleibt das so, prüfe, ob es eine neuere Version des Collectors gibt.".to_owned(),
            Status::Outdated => "Deadlock hat sich seit dieser Programmversion verändert. Es wird nichts übertragen. Prüfe, ob es eine neuere Version des Collectors gibt.".to_owned(),
            Status::Excluded => "Für dieses Match lädt Deadlock selbst keine Statistiken hoch. Der Collector sendet deshalb nichts.".to_owned(),
            Status::Unreadable => "Match erkannt, aber die Spieldaten sind nicht vollständig lesbar. Solange wird nichts übertragen.".to_owned(),
            Status::Sending => "Match erkannt. Die Live-Daten werden übertragen.".to_owned(),
            Status::Rejected(code) => format!("Der Server hat die Übertragung abgelehnt (HTTP {code}). Es wird weiter versucht."),
            Status::Unreachable => "Der Server ist nicht erreichbar. Es wird weiter versucht.".to_owned(),
        }
    }
}

struct Announcer {
    shown: Status,
    candidate: Status,
    seen: u8,
}

impl Announcer {
    fn new() -> Self {
        Announcer {
            shown: Status::Waiting,
            candidate: Status::Waiting,
            seen: 0,
        }
    }

    fn observe(&mut self, status: Status) -> Option<String> {
        if status == self.shown {
            self.seen = 0;
            return None;
        }
        self.seen = if self.seen > 0 && status.continues(&self.candidate) {
            self.seen.saturating_add(1)
        } else {
            1
        };
        if self.seen < status.patience() {
            self.candidate = status;
            return None;
        }
        self.seen = 0;
        self.shown = status;
        Some(self.shown.message())
    }
}

fn in_match(s: &LiveSnapshot) -> bool {
    s.context == Context::Match
        && s.perspective == Perspective::Playing
        && s.game_state.is_some_and(|state| state.is_live())
}

fn summary(s: &LiveSnapshot, heroes: &HeroCatalog) -> Option<Payload> {
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
        let total = match Side::of(row.team, team) {
            Side::Own => &mut own,
            Side::Enemy => &mut enemy,
            Side::Unknown => return None,
        };
        *total = Some(total.unwrap_or(0).checked_add(row.net_worth?)?);
    }
    if players != expected {
        return None;
    }
    let hero_id = player.hero_id.filter(|id| id.0 > 0)?;
    let game_mode = match s.game_mode {
        Some(GameMode::Normal) => Some("normal"),
        Some(GameMode::StreetBrawl) => Some("street_brawl"),
        _ => None,
    };
    Some(Payload {
        match_id: id.to_string(),
        hero: hero_name(heroes, hero_id),
        game_seconds: limits::clock_seconds(s.clock.playing_seconds())?,
        kills: limits::within(player.kills, limits::SUMMARY_SCORE)?,
        deaths: limits::within(player.deaths, limits::SUMMARY_SCORE)?,
        assists: limits::within(player.assists, limits::SUMMARY_SCORE)?,
        team_souls: limits::within(own, limits::SOULS)?,
        enemy_souls: limits::within(enemy, limits::SOULS)?,
        game_mode,
        paused: s.paused,
        midboss: midboss(s),
        details: game_mode
            .is_some()
            .then(|| telemetry::collect(s, player))
            .flatten(),
    })
}

fn prepare(snapshot: Option<&LiveSnapshot>, heroes: &HeroCatalog) -> Result<Payload, Status> {
    let Some(s) = snapshot else {
        return Err(Status::Waiting);
    };
    if !s.drift.is_empty() {
        return Err(Status::Outdated);
    }
    if !in_match(s) {
        return Err(Status::Waiting);
    }
    if s.dont_upload_stats == Some(true) {
        return Err(Status::Excluded);
    }
    summary(s, heroes).ok_or(Status::Unreadable)
}

fn attach_error(previous: Option<String>, attached: &Attached) -> Option<String> {
    match attached {
        Attached::Failed(reason) => Some(reason.clone()),
        Attached::Waiting => previous,
        _ => None,
    }
}

fn read(
    supervisor: &mut ReaderSupervisor,
    blocked: &mut Option<String>,
    heroes: &HeroCatalog,
) -> Result<Payload, Status> {
    let attached = supervisor.acquire();
    *blocked = attach_error(blocked.take(), &attached);
    let reader = match attached {
        Attached::Fresh(reader) | Attached::Held(reader) => reader,
        _ => return Err(blocked.clone().map_or(Status::Waiting, Status::Blocked)),
    };
    match reader.live_snapshot() {
        Ok(snapshot) => {
            supervisor.succeeded();
            prepare(snapshot.as_ref(), heroes)
        }
        Err(_) => {
            supervisor.failed();
            Err(Status::ReadFailed)
        }
    }
}

const UNREADABLE_KEY: &str = "Der Verbindungsschlüssel konnte nicht gelesen werden.";

fn checked_token(input: &str) -> Result<String, &'static str> {
    let token = input.trim();
    if token.is_empty() {
        return Err("Es wurde kein Verbindungsschlüssel eingegeben.");
    }
    if !token.chars().all(|c| c.is_ascii_graphic()) {
        return Err("Der Verbindungsschlüssel enthält ungültige Zeichen. Kopiere ihn vollständig aus dem Overlay-Baukasten.");
    }
    Ok(token.to_owned())
}

fn read_token() -> Result<String, &'static str> {
    if !io::stdin().is_terminal() {
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|_| UNREADABLE_KEY)?;
        return checked_token(&input);
    }
    loop {
        let input =
            rpassword::prompt_password("Verbindungsschlüssel: ").map_err(|_| UNREADABLE_KEY)?;
        match checked_token(&input) {
            Ok(token) => return Ok(token),
            Err(reason) => eprintln!("{reason}"),
        }
    }
}

fn leave(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    if io::stdin().is_terminal() {
        eprintln!("Drücke die Eingabetaste, um das Programm zu schließen.");
        let _ = io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code)
}

#[derive(Debug, PartialEq)]
struct KeyRejected;

fn outcome(code: Option<u16>) -> Result<Status, KeyRejected> {
    match code {
        Some(200..=299) => Ok(Status::Sending),
        Some(401) => Err(KeyRejected),
        Some(code) => Ok(Status::Rejected(code)),
        None => Ok(Status::Unreachable),
    }
}

fn upload(client: &Client, token: &str, data: &Payload) -> Result<Status, KeyRejected> {
    let sent = client.post(ENDPOINT).bearer_auth(token).json(data).send();
    outcome(sent.ok().map(|response| response.status().as_u16()))
}

fn main() {
    if std::env::args_os().len() != 1 {
        leave(
            "Dieses Programm akzeptiert keine Befehlszeilenargumente.",
            2,
        );
    }
    let token = match read_token() {
        Ok(token) => token,
        Err(reason) => leave(reason, 1),
    };
    let client = match Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .timeout(REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => leave(
            "Die sichere Verbindung konnte nicht eingerichtet werden.",
            1,
        ),
    };
    let heroes = HeroCatalog::bundled();
    let mut supervisor = ReaderSupervisor::new(PERIOD);
    let mut announcer = Announcer::new();
    let mut blocked = None;
    eprintln!("Warte auf ein laufendes Deadlock-Match. Beenden mit Strg+C.");
    loop {
        let started = Instant::now();
        let status = match read(&mut supervisor, &mut blocked, &heroes) {
            Ok(data) => match upload(&client, &token, &data) {
                Ok(status) => status,
                Err(KeyRejected) => leave(KEY_REJECTED, 1),
            },
            Err(status) => status,
        };
        if let Some(message) = announcer.observe(status) {
            eprintln!("{message}");
        }
        thread::sleep(PERIOD.saturating_sub(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadlock_core::Team;
    use deadlock_reader::snapshot::{
        AbilityUpgrade, GameState, MatchClock, Objective, ObjectiveKind, PlayerRow, StructureState,
        TeamStats,
    };
    use deadlock_reader::timers::ObjectiveTimer;
    use deadlock_reader::Drift;
    use serde_json::{json, Value};

    type Change = fn(&mut LiveSnapshot);

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

    fn timer(source: TimerSource, present: bool, seconds: f32) -> ObjectiveTimer {
        ObjectiveTimer {
            present: Some(present),
            count: Some(usize::from(present)),
            next_spawn_in: Some(seconds),
            source,
        }
    }

    fn structure(kind: ObjectiveKind, team: Team, lane: i32) -> Objective {
        Objective {
            kind,
            team_name: None,
            class: String::new(),
            address: 0,
            team: Some(team),
            health: None,
            max_health: None,
            lane: Some(lane),
            state: StructureState::Standing,
        }
    }

    fn rich() -> LiveSnapshot {
        let mut s = snapshot();
        s.players[0] = PlayerRow {
            slot: Some(1),
            level: Some(30),
            health: Some(900),
            max_health: Some(1200),
            last_hits: Some(100),
            denies: Some(5),
            hero_damage: Some(100),
            objective_damage: Some(50),
            healing: Some(20),
            is_alive: Some(true),
            ability_points: Some(3),
            creep_souls: Some(10),
            secured_souls: Some(11),
            denied_souls: Some(12),
            neutral_souls: Some(13),
            farm_baseline: Some(14),
            kill_streak: Some(2),
            items: Some(vec![1234, 0, 5678]),
            ability_upgrades: Some(vec![
                AbilityUpgrade {
                    item_id: 7,
                    raw: 0,
                    unlocked: true,
                    points: 3,
                },
                AbilityUpgrade::default(),
            ]),
            ..s.players[0].clone()
        };
        s.players[1] = PlayerRow {
            slot: Some(7),
            hero_id: Some(HeroId(2)),
            level: Some(28),
            kills: Some(1),
            deaths: Some(2),
            assists: Some(3),
            last_hits: Some(300),
            hero_damage: Some(999),
            is_alive: Some(false),
            ..s.players[1].clone()
        };
        s.teams = vec![
            TeamStats {
                team: Team::AMBER,
                players: 1,
                souls: 500,
                kills: 2,
                deaths: 3,
                assists: 4,
                ability_points: 3,
                last_hits: 100,
                denies: 5,
                hero_damage: 100,
                objective_damage: 50,
                healing: 20,
                rejuvenators: Some(1),
                structures_known: true,
                structures_alive: 9,
                complete: true,
                ..Default::default()
            },
            TeamStats {
                team: Team::SAPPHIRE,
                players: 1,
                souls: 600,
                hero_damage: 999,
                ..Default::default()
            },
        ];
        s.objectives = vec![
            structure(ObjectiveKind::Guardian, Team::AMBER, 1),
            structure(ObjectiveKind::Walker, Team::SAPPHIRE, 4),
            structure(ObjectiveKind::Midboss, Team::NEUTRAL, 0),
            structure(ObjectiveKind::LaneTrooper, Team::AMBER, 1),
        ];
        s.timers.midboss = Some(timer(TimerSource::Game, false, 41.2));
        s.timers.urn = Some(timer(TimerSource::Game, true, 5.0));
        s.timers.rift = Some(timer(TimerSource::Unknown, false, 5.0));
        s.timers.bridge_buffs = Some(timer(TimerSource::Schedule, false, 45.1));
        s
    }

    fn roster(s: &mut LiveSnapshot, rows: u32) {
        let teams = [Team::SAPPHIRE, Team::AMBER];
        s.players.extend((2..rows).map(|n| PlayerRow {
            team: Some(teams[n as usize % 2]),
            net_worth: Some(1),
            ..Default::default()
        }));
        s.expected_player_count = Some(rows);
    }

    fn payload(s: &LiveSnapshot, heroes: &HeroCatalog) -> Option<Payload> {
        prepare(Some(s), heroes).ok()
    }

    fn judged(body: Value) -> Result<(), String> {
        let upload = serde_json::from_value::<contract::Upload>(body)
            .map_err(|error| format!("shape: {error}"))?;
        if upload.valid() {
            Ok(())
        } else {
            Err("invalid_snapshot".to_owned())
        }
    }

    fn accepted(s: &LiveSnapshot) -> Value {
        let data = payload(s, &HeroCatalog::bundled()).expect("no payload");
        let body = serde_json::to_value(data).unwrap();
        assert_eq!(judged(body.clone()), Ok(()), "{}", contract::SOURCE);
        body
    }

    #[test]
    fn emits_flat_local_payload() {
        let actual = accepted(&snapshot());
        for (key, value) in json!({
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
        let body = accepted(&rich());
        let details = &body["details"];
        assert_eq!(details["scoreboard"][0]["hero_damage"], 100);
        assert_eq!(details["scoreboard"][0]["side"], "own");
        assert_eq!(details["scoreboard"][1]["side"], "enemy");
        assert!(details["scoreboard"][1]["hero_damage"].is_null());
        assert!(details["scoreboard"][1]["last_hits"].is_null());
        assert_eq!(details["own_loadout"]["items"], json!([1234, 5678]));
        assert_eq!(
            details["own_loadout"]["ability_upgrades"],
            json!([{"item_id": 7, "unlocked": true, "points": 3}])
        );
        assert_eq!(details["teams"][0]["hero_damage"], 100);
        assert_eq!(details["teams"][0]["structures_alive"], 9);
        assert!(details["teams"][1]["hero_damage"].is_null());
        assert!(details["teams"][1]["ability_points"].is_null());
        assert!(details["teams"][1]["structures_alive"].is_null());
        assert_eq!(details["structures"].as_array().unwrap().len(), 3);
        assert_eq!(
            details["structures"][2],
            json!({"side": "unknown", "kind": "midboss", "lane": 0, "state": "standing"})
        );
        let objectives = &details["objectives"];
        assert_eq!(objectives["midboss"]["next_spawn_in_seconds"], 42);
        assert_eq!(objectives["urn"]["present"], true);
        assert!(objectives["urn"]["next_spawn_in_seconds"].is_null());
        assert_eq!(objectives["rift"]["source"], "unknown");
        assert!(objectives["rift"]["next_spawn_in_seconds"].is_null());
        assert!(objectives["rift"]["estimated_spawn_in_seconds"].is_null());
        assert_eq!(objectives["bridge_buffs"]["estimated_spawn_in_seconds"], 46);
        assert!(objectives["bridge_buffs"]["next_spawn_in_seconds"].is_null());
        assert_eq!(body["midboss"]["next_spawn_in_seconds"], 42);
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
        s = snapshot();
        s.players[0].hero_id = Some(HeroId(0));
        assert!(payload(&s, &heroes).is_none());
    }

    #[test]
    fn a_row_outside_both_playing_teams_stops_the_summary() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        assert_eq!(Side::of(None, Team::AMBER), Side::Unknown);
        s.players[1].team = None;
        assert_eq!(s.scoreboard().count(), 1);
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Unreadable));
    }

    #[test]
    fn forwards_only_game_backed_midboss_timer_and_pause_state() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.paused = Some(true);
        s.timers.midboss = Some(timer(TimerSource::Game, false, 41.2));
        let body = accepted(&s);
        assert_eq!(body["paused"], true);
        assert_eq!(body["midboss"]["next_spawn_in_seconds"], 42);
        assert_eq!(body["midboss"]["source"], "game");
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
        assert!(accepted(&s)["details"].is_null());
    }

    #[test]
    fn never_guesses_an_already_live_or_invalid_midboss_timer() {
        let mut s = snapshot();
        s.timers.midboss = Some(timer(TimerSource::Game, true, 100.0));
        assert!(accepted(&s)["midboss"]["next_spawn_in_seconds"].is_null());
        s.timers.midboss.as_mut().unwrap().present = Some(false);
        for invalid in [f32::NAN, -1.0, f32::INFINITY, 7200.5, 100_000.0] {
            s.timers.midboss.as_mut().unwrap().next_spawn_in = Some(invalid);
            let body = accepted(&s);
            assert!(body["midboss"]["next_spawn_in_seconds"].is_null());
            assert!(body["details"]["objectives"]["midboss"]["next_spawn_in_seconds"].is_null());
        }
        s.timers.midboss.as_mut().unwrap().next_spawn_in = Some(7200.0);
        assert_eq!(accepted(&s)["midboss"]["next_spawn_in_seconds"], 7200);
    }

    #[test]
    fn street_brawl_does_not_expose_normal_mode_objectives() {
        let mut s = rich();
        s.game_mode = Some(GameMode::StreetBrawl);
        let body = accepted(&s);
        assert_eq!(body["game_mode"], "street_brawl");
        assert!(body["midboss"].is_null());
        for name in ["midboss", "urn", "rift", "bridge_buffs"] {
            assert!(body["details"]["objectives"][name].is_null(), "{name}");
        }
        assert_eq!(body["details"]["scoreboard"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn the_summary_is_sent_at_the_service_limits_and_withheld_beyond_them() {
        let heroes = HeroCatalog::bundled();
        let inside: [(Change, &str, Value); 4] = [
            (|s| s.players[0].kills = Some(1000), "/kills", json!(1000)),
            (|s| s.players[0].deaths = Some(1000), "/deaths", json!(1000)),
            (
                |s| s.players[1].net_worth = Some(1_000_000_000),
                "/enemy_souls",
                json!(1_000_000_000),
            ),
            (
                |s| s.clock.now = Some(86_410.0),
                "/game_seconds",
                json!(86_400),
            ),
        ];
        for (change, pointer, expected) in inside {
            let mut s = snapshot();
            change(&mut s);
            assert_eq!(accepted(&s).pointer(pointer), Some(&expected), "{pointer}");
        }
        let beyond: [Change; 6] = [
            |s| s.players[0].kills = Some(1001),
            |s| s.players[0].deaths = Some(1001),
            |s| s.players[0].assists = Some(1001),
            |s| s.players[0].net_worth = Some(1_000_000_001),
            |s| s.players[1].net_worth = Some(1_000_000_001),
            |s| s.clock.now = Some(86_411.0),
        ];
        for (index, change) in beyond.into_iter().enumerate() {
            let mut s = snapshot();
            change(&mut s);
            assert_eq!(
                prepare(Some(&s), &heroes),
                Err(Status::Unreadable),
                "case {index}"
            );
        }
        let mut body = accepted(&snapshot());
        let longest = format!(" {} ", "ü".repeat(40));
        body["hero"] = json!(limits::hero_name(&longest).unwrap());
        assert_eq!(judged(body), Ok(()));
    }

    #[test]
    fn details_keep_values_at_the_service_limits_and_null_values_beyond_them() {
        let cases: [(Change, &str, Value); 36] = [
            (
                |s| s.players[0].level = Some(1000),
                "/details/own_loadout/level",
                json!(1000),
            ),
            (
                |s| s.players[0].level = Some(1001),
                "/details/own_loadout/level",
                Value::Null,
            ),
            (
                |s| s.players[1].level = Some(1000),
                "/details/scoreboard/1/level",
                json!(1000),
            ),
            (
                |s| s.players[1].level = Some(1001),
                "/details/scoreboard/1/level",
                Value::Null,
            ),
            (
                |s| s.players[0].health = Some(-1),
                "/details/own_loadout/health",
                json!(-1),
            ),
            (
                |s| s.players[0].health = Some(-2),
                "/details/own_loadout/health",
                Value::Null,
            ),
            (
                |s| s.players[0].health = Some(1_000_000),
                "/details/own_loadout/health",
                json!(1_000_000),
            ),
            (
                |s| s.players[0].health = Some(1_000_001),
                "/details/own_loadout/health",
                Value::Null,
            ),
            (
                |s| s.players[0].max_health = Some(0),
                "/details/own_loadout/max_health",
                json!(0),
            ),
            (
                |s| s.players[0].max_health = Some(-1),
                "/details/own_loadout/max_health",
                Value::Null,
            ),
            (
                |s| s.players[0].max_health = Some(1_000_001),
                "/details/own_loadout/max_health",
                Value::Null,
            ),
            (
                |s| s.players[0].hero_damage = Some(1_000_000_000),
                "/details/scoreboard/0/hero_damage",
                json!(1_000_000_000),
            ),
            (
                |s| s.players[0].hero_damage = Some(1_000_000_001),
                "/details/scoreboard/0/hero_damage",
                Value::Null,
            ),
            (
                |s| s.players[0].kill_streak = Some(1_000_000_001),
                "/details/own_loadout/kill_streak",
                Value::Null,
            ),
            (
                |s| s.players[1].slot = Some(12),
                "/details/scoreboard/1/slot",
                json!(12),
            ),
            (
                |s| s.players[1].slot = Some(13),
                "/details/scoreboard/1/slot",
                Value::Null,
            ),
            (
                |s| s.players[1].slot = Some(0),
                "/details/scoreboard/1/slot",
                Value::Null,
            ),
            (
                |s| s.players[1].hero_id = Some(HeroId(0)),
                "/details/scoreboard/1/hero_id",
                Value::Null,
            ),
            (
                |s| s.players[1].hero_id = Some(HeroId(u32::MAX)),
                "/details/scoreboard/1/hero_id",
                json!(u32::MAX),
            ),
            (
                |s| s.teams[0].rejuvenators = Some(1000),
                "/details/teams/0/rejuvenators",
                json!(1000),
            ),
            (
                |s| s.teams[0].rejuvenators = Some(1001),
                "/details/teams/0/rejuvenators",
                Value::Null,
            ),
            (
                |s| s.teams[0].structures_alive = 128,
                "/details/teams/0/structures_alive",
                json!(128),
            ),
            (
                |s| s.teams[0].structures_alive = 129,
                "/details/teams/0/structures_alive",
                Value::Null,
            ),
            (
                |s| s.teams[0].hero_damage = 1_000_000_001,
                "/details/teams/0/hero_damage",
                Value::Null,
            ),
            (
                |s| s.timers.urn.as_mut().unwrap().count = Some(128),
                "/details/objectives/urn/count",
                json!(128),
            ),
            (
                |s| s.timers.urn.as_mut().unwrap().count = Some(129),
                "/details/objectives/urn/count",
                Value::Null,
            ),
            (
                |s| s.timers.bridge_buffs.as_mut().unwrap().next_spawn_in = Some(7200.0),
                "/details/objectives/bridge_buffs/estimated_spawn_in_seconds",
                json!(7200),
            ),
            (
                |s| s.timers.bridge_buffs.as_mut().unwrap().next_spawn_in = Some(7200.5),
                "/details/objectives/bridge_buffs/estimated_spawn_in_seconds",
                Value::Null,
            ),
            (
                |s| s.players[0].items = Some([vec![0; 9], vec![5; 64]].concat()),
                "/details/own_loadout/items",
                json!(vec![5; 64]),
            ),
            (
                |s| s.players[0].items = Some(vec![5; 65]),
                "/details/own_loadout/items",
                Value::Null,
            ),
            (
                |s| s.players[0].ability_upgrades.as_mut().unwrap()[0].points = 5,
                "/details/own_loadout/ability_upgrades/0/points",
                json!(5),
            ),
            (
                |s| s.players[0].ability_upgrades.as_mut().unwrap()[0].points = 6,
                "/details/own_loadout/ability_upgrades",
                Value::Null,
            ),
            (
                |s| {
                    let one = s.players[0].ability_upgrades.as_ref().unwrap()[0];
                    s.players[0].ability_upgrades = Some(vec![one; 17]);
                },
                "/details/own_loadout/ability_upgrades",
                Value::Null,
            ),
            (
                |s| s.objectives[0].lane = Some(8),
                "/details/structures/0/lane",
                json!(8),
            ),
            (
                |s| s.objectives[0].lane = Some(9),
                "/details/structures/0/lane",
                Value::Null,
            ),
            (
                |s| s.objectives[0].lane = Some(-1),
                "/details/structures/0/lane",
                Value::Null,
            ),
        ];
        for (index, (change, pointer, expected)) in cases.into_iter().enumerate() {
            let mut s = rich();
            change(&mut s);
            assert_eq!(
                accepted(&s).pointer(pointer),
                Some(&expected),
                "case {index} {pointer}"
            );
        }
    }

    #[test]
    fn rows_and_lists_beyond_the_service_limits_are_left_out_whole() {
        let rows =
            |body: &Value, pointer: &str| body.pointer(pointer).unwrap().as_array().unwrap().len();
        let mut s = rich();
        s.teams[1].kills = 1_000_000_001;
        let body = accepted(&s);
        assert_eq!(rows(&body, "/details/teams"), 1);
        assert_eq!(body["details"]["teams"][0]["side"], "own");
        s = rich();
        s.teams[1].souls = 1_000_000_001;
        assert_eq!(rows(&accepted(&s), "/details/teams"), 1);
        s = rich();
        s.teams[1].team = Team::AMBER;
        assert_eq!(rows(&accepted(&s), "/details/teams"), 0);
        s = rich();
        s.teams[1].team = Team::SPECTATOR;
        assert_eq!(rows(&accepted(&s), "/details/teams"), 1);
        s = rich();
        s.teams.insert(0, TeamStats::default());
        assert_eq!(rows(&accepted(&s), "/details/teams"), 2);
        s = rich();
        s.objectives = vec![structure(ObjectiveKind::Guardian, Team::AMBER, 1); 48];
        assert_eq!(rows(&accepted(&s), "/details/structures"), 48);
        s.objectives = vec![structure(ObjectiveKind::Guardian, Team::AMBER, 1); 49];
        assert_eq!(rows(&accepted(&s), "/details/structures"), 0);
        s = rich();
        roster(&mut s, 12);
        assert_eq!(rows(&accepted(&s), "/details/scoreboard"), 12);
        s = rich();
        roster(&mut s, 13);
        assert!(accepted(&s)["details"].is_null());
        s = rich();
        s.players[1].is_local = Some(true);
        assert!(accepted(&s)["details"].is_null());
    }

    #[test]
    fn the_ported_service_check_rejects_what_the_collector_withholds() {
        let body = accepted(&rich());
        let upgrade = json!({"item_id": 1, "unlocked": true, "points": 1});
        let structure = body["details"]["structures"][0].clone();
        let row = body["details"]["scoreboard"][1].clone();
        let team = body["details"]["teams"][1].clone();
        let withheld = [
            ("/kills", json!(1001)),
            ("/team_souls", json!(1_000_000_001u32)),
            ("/game_seconds", json!(86_401)),
            ("/hero", json!(" Infernus")),
            ("/hero", json!("")),
            ("/hero", json!("x".repeat(81))),
            ("/hero", json!("Infer\u{7}nus")),
            ("/match_id", json!("0")),
            ("/match_id", json!("12a")),
            ("/game_mode", json!("street_brawl")),
            ("/game_mode", json!("unknown")),
            ("/midboss/source", json!("schedule")),
            ("/midboss/next_spawn_in_seconds", json!(7201)),
            ("/details/schema_version", json!(2)),
            ("/details/capabilities", json!(["scoreboard"])),
            ("/details/own_loadout/level", json!(1001)),
            ("/details/own_loadout/health", json!(-2)),
            ("/details/own_loadout/health", json!(1_000_001)),
            ("/details/own_loadout/max_health", json!(-1)),
            ("/details/own_loadout/kill_streak", json!(1_000_000_001u32)),
            ("/details/own_loadout/items", json!([0])),
            ("/details/own_loadout/items", json!(vec![1; 65])),
            ("/details/own_loadout/ability_upgrades/0/points", json!(6)),
            ("/details/own_loadout/ability_upgrades/0/item_id", json!(0)),
            (
                "/details/own_loadout/ability_upgrades",
                json!(vec![upgrade; 17]),
            ),
            ("/details/scoreboard", json!(vec![row; 13])),
            ("/details/scoreboard/0/side", json!("enemy")),
            ("/details/scoreboard/0/slot", json!(13)),
            ("/details/scoreboard/0/level", json!(1001)),
            ("/details/scoreboard/1/is_local", json!(true)),
            ("/details/scoreboard/1/hero_id", json!(0)),
            ("/details/scoreboard/1/last_hits", json!(1)),
            ("/details/scoreboard/1/souls", json!(1_000_000_001u32)),
            ("/details/teams", json!(vec![team; 3])),
            ("/details/teams/0/side", json!("unknown")),
            ("/details/teams/1/side", json!("own")),
            ("/details/teams/1/hero_damage", json!(1)),
            ("/details/teams/0/rejuvenators", json!(1001)),
            ("/details/teams/0/structures_alive", json!(129)),
            ("/details/teams/0/kills", json!(1_000_000_001u32)),
            ("/details/structures", json!(vec![structure; 49])),
            ("/details/structures/0/lane", json!(9)),
            ("/details/structures/0/kind", json!("lane_trooper")),
            ("/details/objectives/urn/count", json!(129)),
            ("/details/objectives/urn/next_spawn_in_seconds", json!(5)),
            ("/details/objectives/rift/next_spawn_in_seconds", json!(5)),
            (
                "/details/objectives/rift/estimated_spawn_in_seconds",
                json!(5),
            ),
            (
                "/details/objectives/midboss/estimated_spawn_in_seconds",
                json!(5),
            ),
            (
                "/details/objectives/bridge_buffs/next_spawn_in_seconds",
                json!(5),
            ),
            (
                "/details/objectives/bridge_buffs/estimated_spawn_in_seconds",
                json!(7201),
            ),
        ];
        for (pointer, value) in withheld {
            let mut altered = body.clone();
            *altered.pointer_mut(pointer).expect(pointer) = value.clone();
            assert!(judged(altered).is_err(), "accepted {pointer} = {value}");
        }
        let mut altered = body.clone();
        altered["details"]["steam_id"] = json!(1);
        assert!(judged(altered).is_err());
        let mut altered = body.clone();
        altered["details"]["scoreboard"][1]["name"] = json!("x");
        assert!(judged(altered).is_err());
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
        roster(&mut s, 3);
        s.players[2].team = Some(Team::AMBER);
        s.players[2].net_worth = Some(50);
        let body = accepted(&s);
        assert_eq!(body["hero"], u32::MAX.to_string());
        assert_eq!(body["team_souls"], 550);
        assert_eq!(body["enemy_souls"], 600);
        s.players[2].net_worth = None;
        assert!(payload(&s, &heroes).is_none());
        s.players[2].net_worth = Some(u32::MAX);
        assert!(payload(&s, &heroes).is_none());
        s.players.truncate(1);
        assert!(payload(&s, &heroes).is_none());
    }

    #[test]
    fn accepts_only_keys_that_fit_an_http_header() {
        assert_eq!(checked_token("  dlm_abc123\r\n").unwrap(), "dlm_abc123");
        for invalid in ["", " \r\n", "dlm_abc 123", "dlm_schlüssel", "dlm_\u{7}abc"] {
            assert!(checked_token(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn names_the_reason_when_nothing_is_sent() {
        let heroes = HeroCatalog::bundled();
        assert_eq!(prepare(None, &heroes), Err(Status::Waiting));
        let mut s = snapshot();
        s.players[1].net_worth = None;
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Unreadable));
        s.dont_upload_stats = Some(true);
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Excluded));
        s.game_state = Some(GameState::PostGame);
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Waiting));
    }

    #[test]
    fn any_drift_stops_the_upload_inside_and_outside_a_match() {
        let heroes = HeroCatalog::bundled();
        let mut s = snapshot();
        s.drift = vec![Drift::StaleOffset {
            class: "C_CitadelGameRules".to_owned(),
            field: "m_bGamePaused".to_owned(),
        }];
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Outdated));
        s.game_state = Some(GameState::PostGame);
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Outdated));
        s.drift.clear();
        assert_eq!(prepare(Some(&s), &heroes), Err(Status::Waiting));
    }

    #[test]
    fn reports_a_status_once_and_only_when_it_holds() {
        let mut announcer = Announcer::new();
        assert_eq!(announcer.observe(Status::Waiting), None);
        assert!(announcer.observe(Status::Sending).is_some());
        assert_eq!(announcer.observe(Status::Sending), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert_eq!(announcer.observe(Status::Sending), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert!(announcer.observe(Status::Unreachable).is_some());
        assert!(announcer.observe(Status::Sending).is_some());
    }

    #[test]
    fn alternating_upload_failures_are_reported_once() {
        let mut announcer = Announcer::new();
        assert!(announcer.observe(Status::Sending).is_some());
        assert_eq!(announcer.observe(Status::Rejected(503)), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert_eq!(
            announcer.observe(Status::Rejected(503)),
            Some(Status::Rejected(503).message())
        );
        for _ in 0..4 {
            assert_eq!(announcer.observe(Status::Unreachable), None);
            assert_eq!(announcer.observe(Status::Rejected(503)), None);
        }
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert_eq!(announcer.observe(Status::Outdated), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert_eq!(announcer.observe(Status::Unreachable), None);
        assert!(announcer.observe(Status::Unreachable).is_some());
    }

    #[test]
    fn a_closing_game_is_not_reported_as_a_read_failure() {
        let mut announcer = Announcer::new();
        for _ in 0..3 {
            assert_eq!(announcer.observe(Status::ReadFailed), None);
        }
        assert_eq!(announcer.observe(Status::Waiting), None);
        for _ in 0..4 {
            assert_eq!(announcer.observe(Status::ReadFailed), None);
        }
        assert_eq!(
            announcer.observe(Status::ReadFailed),
            Some(Status::ReadFailed.message())
        );
    }

    #[test]
    fn an_attach_failure_is_kept_until_the_game_opens_or_is_gone() {
        let reason = "access denied".to_owned();
        let failed = Attached::Failed(reason.clone());
        assert_eq!(attach_error(None, &failed), Some(reason.clone()));
        assert_eq!(
            attach_error(Some(reason.clone()), &Attached::Waiting),
            Some(reason.clone())
        );
        assert_eq!(attach_error(None, &Attached::Waiting), None);
        assert_eq!(attach_error(Some(reason.clone()), &Attached::Absent), None);
        let mut announcer = Announcer::new();
        let blocked = Status::Blocked(reason.clone());
        assert_eq!(announcer.observe(blocked.clone()), None);
        assert_eq!(announcer.observe(blocked.clone()), None);
        let message = announcer.observe(blocked.clone()).unwrap();
        assert!(message.contains(&reason));
        assert_eq!(announcer.observe(blocked), None);
    }

    #[test]
    fn only_a_rejected_key_ends_the_program() {
        assert_eq!(outcome(Some(204)), Ok(Status::Sending));
        assert_eq!(outcome(Some(401)), Err(KeyRejected));
        for code in [301, 400, 403, 404, 503] {
            assert_eq!(outcome(Some(code)), Ok(Status::Rejected(code)));
        }
        assert_eq!(outcome(None), Ok(Status::Unreachable));
    }
}
