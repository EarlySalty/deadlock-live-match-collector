// Standalone regression target: no runtime reader or external game-type dependencies.
#![allow(dead_code)]
#[path = "../src/contract.rs"]
mod contract;
#[path = "../src/game_model.rs"]
mod game_model;
#[path = "../src/game_source.rs"]
mod game_source;
#[path = "../src/limits.rs"]
mod limits;
#[path = "../src/telemetry.rs"]
mod telemetry;

use game_model::*;
use game_source::{GameSource, SyntheticSource};
use serde_json::{json, Value};

fn encode_body<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, ()> {
    let bytes = serde_json::to_vec(value).map_err(|_| ())?;
    if bytes.len() > limits::BODY_BYTES {
        return Err(());
    }
    Ok(bytes)
}

fn snapshot() -> GameSnapshot {
    GameSnapshot {
        match_token: Some(123456789),
        context: MatchContext::LivePlaying,
        eligibility: Eligibility::Eligible,
        provenance: Provenance::Synthetic,
        local_hero_name: Some("Infernus".into()),
        expected_player_count: Some(2),
        game_mode: Some(GameMode::Normal),
        playing_seconds: Some(42.8),
        paused: Some(false),
        players: vec![
            PlayerSample {
                team: Some(Team::Own),
                is_local: Some(true),
                hero_id: Some(1),
                ..Default::default()
            },
            PlayerSample {
                team: Some(Team::Enemy),
                hero_id: Some(2),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn wire(s: &GameSnapshot) -> Value {
    let sample = telemetry::observation(s).expect("valid synthetic sample");
    let payload = telemetry::Payload::new(
        uuid::Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        limits::MAX_COUNTER,
        limits::MAX_COUNTER,
        sample,
    );
    let bytes = encode_body(&payload).unwrap();
    assert_eq!(contract::body_status(&bytes), 204);
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn all_visible_fields_and_limits_survive_serialization() {
    let mut s = snapshot();
    s.expected_player_count = Some(12);
    while s.players.len() < 12 {
        s.players.push(s.players[1].clone());
    }
    for (i, p) in s.players.iter_mut().enumerate() {
        p.team = Some(if i < 6 { Team::Own } else { Team::Enemy });
        p.slot = Some(i as u32 + 1);
        p.hero_id = Some(u32::MAX);
        p.level = Some(1000);
        p.souls = Some(limits::SOULS / 6);
        p.kills = Some(1000);
        p.deaths = Some(999);
        p.assists = Some(998);
        p.last_hits = Some(17);
        p.denies = Some(18);
        p.hero_damage = Some(19);
        p.objective_damage = Some(20);
        p.healing = Some(21);
        p.alive = Some(true);
        p.items = Some(vec![u32::MAX; 64]);
    }
    let p = &mut s.players[0];
    p.health = Some(-1);
    p.max_health = Some(1_000_000);
    p.ability_points = Some(4);
    p.creep_souls = Some(5);
    p.secured_souls = Some(6);
    p.denied_souls = Some(7);
    p.neutral_souls = Some(8);
    p.farm_baseline = Some(9);
    p.kill_streak = Some(10);
    p.ability_upgrades = Some(
        (0..16)
            .map(|i| AbilityUpgrade {
                item_id: u32::MAX - i,
                unlocked: true,
                points: 5,
            })
            .collect(),
    );
    s.teams = [Team::Own, Team::Enemy]
        .into_iter()
        .map(|team| TeamSample {
            team,
            complete: true,
            ability_points: Some(11),
            last_hits: Some(12),
            denies: Some(13),
            hero_damage: Some(14),
            objective_damage: Some(15),
            healing: Some(16),
            rejuvenators: Some(1000),
            structures_alive: Some(128),
        })
        .collect();
    s.structures = (0..48)
        .map(|_| StructureSample {
            team: Some(Team::Own),
            kind: StructureKind::Guardian,
            lane: Some(8),
            state: StructureState::Standing,
        })
        .collect();
    let direct = ObjectiveTimer {
        present: Some(false),
        count: Some(128),
        next_spawn_in: Some(7200.0),
        source: TimerSource::Game,
    };
    s.timers = ObjectiveTimers {
        midboss: Some(direct.clone()),
        urn: Some(direct.clone()),
        rift: Some(direct.clone()),
        bridge_buffs: Some(ObjectiveTimer {
            source: TimerSource::Schedule,
            ..direct
        }),
    };
    let body = wire(&s);
    assert_eq!(body["schema_version"], 3);
    assert_eq!(body["hero"], "Infernus");
    assert_eq!(body["game_seconds"], 42);
    assert_eq!(body["kills"], 1000);
    assert_eq!(body["deaths"], 999);
    assert_eq!(body["assists"], 998);
    assert_eq!(body["paused"], false);
    assert_eq!(body["team_souls"], 999999996);
    assert_eq!(body["enemy_souls"], 999999996);
    assert_eq!(body["midboss"]["next_spawn_in_seconds"], 7200);
    let d = &body["details"];
    assert_eq!(d["scoreboard"].as_array().unwrap().len(), 12);
    assert_eq!(d["structures"].as_array().unwrap().len(), 48);
    assert_eq!(
        d["structures"][0],
        json!({"side":"own","kind":"guardian","lane":8,"state":"standing"})
    );
    assert_eq!(d["teams"].as_array().unwrap().len(), 2);
    for (k, v) in [
        ("last_hits", 17),
        ("denies", 18),
        ("hero_damage", 19),
        ("objective_damage", 20),
        ("healing", 21),
    ] {
        assert_eq!(d["scoreboard"][0][k], v);
        for row in &d["scoreboard"].as_array().unwrap()[1..] {
            assert!(row[k].is_null());
        }
    }
    for (k, v) in [
        ("ability_points", 11),
        ("last_hits", 12),
        ("denies", 13),
        ("hero_damage", 14),
        ("objective_damage", 15),
        ("healing", 16),
    ] {
        assert_eq!(d["teams"][0][k], v);
        assert!(d["teams"][1][k].is_null());
    }
    assert_eq!(d["teams"][0]["kills"], 6000);
    assert_eq!(d["teams"][0]["rejuvenators"], 1000);
    assert_eq!(d["teams"][0]["structures_alive"], 128);
    let loadout = &d["own_loadout"];
    for (k, v) in [
        ("level", 1000),
        ("health", -1),
        ("max_health", 1000000),
        ("ability_points", 4),
        ("creep_souls", 5),
        ("secured_souls", 6),
        ("denied_souls", 7),
        ("neutral_souls", 8),
        ("farm_baseline", 9),
        ("kill_streak", 10),
    ] {
        assert_eq!(loadout[k], v);
    }
    assert_eq!(loadout["items"].as_array().unwrap().len(), 64);
    assert_eq!(loadout["ability_upgrades"].as_array().unwrap().len(), 16);
    for k in ["midboss", "urn", "rift"] {
        assert_eq!(d["objectives"][k]["next_spawn_in_seconds"], 7200);
    }
    assert_eq!(
        d["objectives"]["bridge_buffs"]["estimated_spawn_in_seconds"],
        7200
    );
    let serialized = body.to_string();
    for k in [
        "match_token",
        "provenance",
        "context",
        "eligibility",
        "steam_id",
        "position",
        "match_id",
    ] {
        assert!(!serialized.contains(&format!("\"{k}\"")), "{k}");
    }
    assert!(!serialized.contains("123456789"));
    eprintln!("Model maximal shape: {} bytes", serialized.len());
}

#[test]
fn missing_or_invalid_core_data_blocks_observation() {
    let base = snapshot();
    let mut cases = Vec::new();
    macro_rules! invalid { ($name:ident, $($body:tt)*) => {{ let mut $name = base.clone(); $($body)* cases.push($name); }}; }
    invalid!(s, s.provenance = Provenance::Unknown;);
    invalid!(s, s.context = MatchContext::Other;);
    invalid!(s, s.eligibility = Eligibility::Excluded;);
    invalid!(s, s.eligibility = Eligibility::Unknown;);
    invalid!(s, s.match_token = Some(0););
    invalid!(s, s.match_token = None;);
    invalid!(s, s.players[0].is_local = None;);
    invalid!(s, s.players[1].is_local = Some(true););
    invalid!(s, s.players[0].is_spectator = true;);
    invalid!(s, s.players[0].team = Some(Team::Enemy););
    invalid!(s, s.players[1].team = Some(Team::Unknown););
    invalid!(s, s.players[1].team = None;);
    invalid!(s, s.players[1].team = Some(Team::Own););
    invalid!(s, s.players[1].is_spectator = true;);
    invalid!(s, s.expected_player_count = None;);
    invalid!(s, s.expected_player_count = Some(12););
    invalid!(s, s.players[0].hero_id = None;);
    invalid!(s, s.players[0].hero_id = Some(0););
    invalid!(s, s.local_hero_name = None;);
    invalid!(s, s.local_hero_name = Some("ü".repeat(41)););
    invalid!(s, s.local_hero_name = Some("a\u{0007}b".into()););
    for clock in [
        None,
        Some(f32::NAN),
        Some(f32::INFINITY),
        Some(-0.5),
        Some(86400.5),
    ] {
        invalid!(s, s.playing_seconds = clock;);
    }
    for (i, s) in cases.iter().enumerate() {
        assert!(telemetry::observation(s).is_none(), "invalid case {i}");
    }
}

#[test]
fn invalid_optional_values_and_unknown_are_not_invented() {
    let mut s = snapshot();
    s.players[0].kills = Some(1001);
    s.players[0].souls = Some(u32::MAX);
    s.players[0].level = Some(1001);
    s.players[0].health = Some(-2);
    s.players[0].items = Some(vec![1; 65]);
    s.players[0].ability_upgrades = Some(vec![AbilityUpgrade {
        item_id: 1,
        unlocked: true,
        points: 6,
    }]);
    s.teams.push(TeamSample {
        team: Team::Own,
        complete: true,
        ..Default::default()
    });
    s.timers.midboss = Some(ObjectiveTimer {
        present: Some(true),
        count: Some(129),
        source: TimerSource::Game,
        next_spawn_in: Some(2.0),
    });
    s.timers.urn = Some(ObjectiveTimer {
        source: TimerSource::Schedule,
        next_spawn_in: Some(2.0),
        ..Default::default()
    });
    let b = wire(&s);
    assert!(b["kills"].is_null());
    assert!(b["team_souls"].is_null());
    assert!(b["enemy_souls"].is_null());
    assert!(b["midboss"]["next_spawn_in_seconds"].is_null());
    let d = &b["details"];
    for k in ["level", "health", "items", "ability_upgrades", "souls"] {
        assert!(d["own_loadout"][k].is_null());
    }
    assert!(d["teams"][0]["ability_points"].is_null());
    assert!(d["teams"][0]["souls"].is_null());
    assert!(d["objectives"]["midboss"]["count"].is_null());
    assert!(d["objectives"]["midboss"]["next_spawn_in_seconds"].is_null());
    assert!(d["objectives"]["urn"]["estimated_spawn_in_seconds"].is_null());
    s.game_mode = Some(GameMode::StreetBrawl);
    let b = wire(&s);
    assert!(b["midboss"].is_null());
    assert!(b["details"]["objectives"]["midboss"].is_null());
    s.game_mode = None;
    assert!(wire(&s)["details"].is_null());
}

#[test]
fn source_poll_uses_observed_clock_and_never_creates_progress() {
    let s = snapshot();
    let mut paused = s.clone();
    paused.paused = Some(true);
    let mut source = SyntheticSource::new(vec![Ok(Some(s.clone())), Ok(Some(paused)), Ok(Some(s))]);
    for expected_pause in [false, true, false] {
        let b = wire(&source.poll().unwrap().unwrap());
        assert_eq!(b["game_seconds"], 42);
        assert_eq!(b["paused"], expected_pause);
    }
    assert_eq!(source.poll().unwrap(), None);
}
