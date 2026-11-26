use crate::game_model::{
    Eligibility, GameMode, GameSnapshot, MatchContext, ObjectiveTimer, PlayerSample, Provenance,
    StructureKind, StructureState, Team, TeamSample, TimerSource,
};
use serde::Serialize;
use uuid::Uuid;

use crate::limits;

const CAPABILITIES: &[&str] = &[
    "scoreboard",
    "teams",
    "own_loadout",
    "structures",
    "objectives",
    "timer_provenance",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Own,
    Enemy,
    Unknown,
}

impl Side {
    pub fn of(team: Option<Team>, local: Team) -> Side {
        match team {
            Some(team) if team.is_playing() && team == local => Side::Own,
            Some(team) if local.opponent() == Some(team) => Side::Enemy,
            _ => Side::Unknown,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct MatchDetails {
    pub schema_version: u8,
    pub capabilities: &'static [&'static str],
    pub scoreboard: Vec<ScoreboardRow>,
    pub teams: Vec<TeamRow>,
    pub own_loadout: OwnLoadout,
    pub structures: Vec<StructureRow>,
    pub objectives: ObjectiveRows,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ScoreboardRow {
    pub side: Side,
    pub is_local: bool,
    pub slot: Option<u32>,
    pub hero_id: Option<u32>,
    pub level: Option<u32>,
    pub souls: Option<u32>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub last_hits: Option<u32>,
    pub denies: Option<u32>,
    pub hero_damage: Option<u32>,
    pub objective_damage: Option<u32>,
    pub healing: Option<u32>,
    pub alive: Option<bool>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TeamRow {
    pub side: Side,
    pub souls: Option<u32>,
    pub ability_points: Option<u32>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub last_hits: Option<u32>,
    pub denies: Option<u32>,
    pub hero_damage: Option<u32>,
    pub objective_damage: Option<u32>,
    pub healing: Option<u32>,
    pub rejuvenators: Option<u32>,
    pub structures_alive: Option<u32>,
    pub complete: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct OwnLoadout {
    pub level: Option<u32>,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub souls: Option<u32>,
    pub ability_points: Option<u32>,
    pub creep_souls: Option<u32>,
    pub secured_souls: Option<u32>,
    pub denied_souls: Option<u32>,
    pub neutral_souls: Option<u32>,
    pub farm_baseline: Option<u32>,
    pub kill_streak: Option<u32>,
    pub items: Option<Vec<u32>>,
    pub ability_upgrades: Option<Vec<AbilityRow>>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct AbilityRow {
    pub item_id: u32,
    pub unlocked: bool,
    pub points: u32,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StructureRow {
    pub side: Side,
    pub kind: &'static str,
    pub lane: Option<i32>,
    pub state: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ObjectiveRows {
    pub midboss: Option<ObjectiveRow>,
    pub urn: Option<ObjectiveRow>,
    pub rift: Option<ObjectiveRow>,
    pub bridge_buffs: Option<ObjectiveRow>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ObjectiveRow {
    pub present: Option<bool>,
    pub count: Option<usize>,
    pub source: &'static str,
    pub next_spawn_in_seconds: Option<u32>,
    pub estimated_spawn_in_seconds: Option<u32>,
}

fn objective(timer: Option<&ObjectiveTimer>, may_estimate: bool) -> Option<ObjectiveRow> {
    let timer = timer?;
    let seconds =
        limits::timer_seconds(timer.next_spawn_in).filter(|_| timer.present != Some(true));
    let observed = timer.source == TimerSource::Game;
    let estimated = may_estimate && timer.source == TimerSource::Schedule;
    Some(ObjectiveRow {
        present: timer.present,
        count: timer.count.filter(|n| *n <= limits::OBJECTIVE_COUNT),
        source: match timer.source {
            TimerSource::Game => "game",
            TimerSource::Schedule => "schedule",
            TimerSource::Unknown => "unknown",
        },
        next_spawn_in_seconds: seconds.filter(|_| observed),
        estimated_spawn_in_seconds: seconds.filter(|_| estimated),
    })
}

fn stat(value: Option<u32>) -> Option<u32> {
    limits::within(value, limits::STAT)
}

fn scoreboard(row: &PlayerSample, local: Team) -> ScoreboardRow {
    let is_local = row.is_local == Some(true);
    let own = |value: Option<u32>| stat(value).filter(|_| is_local);
    ScoreboardRow {
        side: Side::of(row.team, local),
        is_local,
        slot: row.slot.filter(|slot| limits::SLOTS.contains(slot)),
        hero_id: row.hero_id.filter(|id| *id > 0),
        level: limits::within(row.level, limits::LEVEL),
        souls: stat(row.souls),
        kills: stat(row.kills),
        deaths: stat(row.deaths),
        assists: stat(row.assists),
        last_hits: own(row.last_hits),
        denies: own(row.denies),
        hero_damage: own(row.hero_damage),
        objective_damage: own(row.objective_damage),
        healing: own(row.healing),
        alive: row.alive,
    }
}

fn sum_side(
    s: &GameSnapshot,
    team: Team,
    field: impl Fn(&PlayerSample) -> Option<u32>,
    limit: u32,
) -> Option<u32> {
    let mut rows = s
        .players
        .iter()
        .filter(|r| !r.is_spectator && r.team == Some(team))
        .peekable();
    rows.peek()?;
    let total = rows.try_fold(0u32, |sum, row| {
        sum.checked_add(limits::within(field(row), limit)?)
    })?;
    limits::within(Some(total), limit)
}

fn team(s: &GameSnapshot, stats: &TeamSample, local: Team) -> Option<TeamRow> {
    let side = Side::of(Some(stats.team), local);
    if side == Side::Unknown {
        return None;
    }
    let own = |value: Option<u32>| stat(value).filter(|_| side == Side::Own && stats.complete);
    Some(TeamRow {
        side,
        souls: sum_side(s, stats.team, |r| r.souls, limits::SOULS),
        ability_points: own(stats.ability_points),
        kills: sum_side(s, stats.team, |r| r.kills, limits::STAT),
        deaths: sum_side(s, stats.team, |r| r.deaths, limits::STAT),
        assists: sum_side(s, stats.team, |r| r.assists, limits::STAT),
        last_hits: own(stats.last_hits),
        denies: own(stats.denies),
        hero_damage: own(stats.hero_damage),
        objective_damage: own(stats.objective_damage),
        healing: own(stats.healing),
        rejuvenators: limits::within(stats.rejuvenators, limits::REJUVENATORS),
        structures_alive: limits::within(stats.structures_alive, limits::STRUCTURES_ALIVE),
        complete: stats.complete,
    })
}

fn teams(s: &GameSnapshot, local: Team) -> Vec<TeamRow> {
    let rows = s
        .teams
        .iter()
        .filter_map(|stats| team(s, stats, local))
        .collect::<Vec<_>>();
    let repeated = rows.len() == limits::TEAM_ROWS && rows[0].side == rows[1].side;
    if rows.len() > limits::TEAM_ROWS || repeated {
        return Vec::new();
    }
    rows
}

fn structure_kind(kind: StructureKind) -> Option<&'static str> {
    match kind {
        StructureKind::Guardian => Some("guardian"),
        StructureKind::Walker => Some("walker"),
        StructureKind::BaseGuardian => Some("base_guardian"),
        StructureKind::Patron => Some("patron"),
        StructureKind::Shrine => Some("shrine"),
        StructureKind::Midboss => Some("midboss"),
        StructureKind::Unknown => None,
    }
}

fn structures(s: &GameSnapshot, local: Team) -> Vec<StructureRow> {
    let rows = s
        .structures
        .iter()
        .filter_map(|o| {
            Some(StructureRow {
                kind: structure_kind(o.kind)?,
                side: Side::of(o.team, local),
                lane: o.lane.filter(|n| limits::LANES.contains(n)),
                state: match o.state {
                    StructureState::Standing => "standing",
                    StructureState::Destroyed => "destroyed",
                    StructureState::Unknown => "unknown",
                },
            })
        })
        .collect::<Vec<_>>();
    if rows.len() > limits::STRUCTURE_ROWS {
        return Vec::new();
    }
    rows
}

fn objectives(s: &GameSnapshot) -> ObjectiveRows {
    let normal = s.game_mode == Some(GameMode::Normal);
    let row = |timer: &Option<ObjectiveTimer>, may_estimate: bool| {
        normal
            .then(|| objective(timer.as_ref(), may_estimate))
            .flatten()
    };
    ObjectiveRows {
        midboss: row(&s.timers.midboss, false),
        urn: row(&s.timers.urn, false),
        rift: row(&s.timers.rift, false),
        bridge_buffs: row(&s.timers.bridge_buffs, true),
    }
}

fn local_loadout(local: &PlayerSample) -> OwnLoadout {
    let items = local
        .items
        .as_ref()
        .map(|v| v.iter().copied().filter(|id| *id > 0).collect::<Vec<_>>())
        .filter(|v| v.len() <= limits::ITEMS);
    let ability_upgrades = local
        .ability_upgrades
        .as_ref()
        .map(|v| {
            v.iter()
                .filter(|a| a.item_id > 0)
                .map(|a| AbilityRow {
                    item_id: a.item_id,
                    unlocked: a.unlocked,
                    points: a.points,
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| {
            v.len() <= limits::ABILITY_UPGRADES
                && v.iter().all(|a| a.points <= limits::ABILITY_POINTS)
        });
    OwnLoadout {
        level: limits::within(local.level, limits::LEVEL),
        health: local.health.filter(|n| limits::HEALTH.contains(n)),
        max_health: local.max_health.filter(|n| limits::MAX_HEALTH.contains(n)),
        souls: stat(local.souls),
        ability_points: stat(local.ability_points),
        creep_souls: stat(local.creep_souls),
        secured_souls: stat(local.secured_souls),
        denied_souls: stat(local.denied_souls),
        neutral_souls: stat(local.neutral_souls),
        farm_baseline: stat(local.farm_baseline),
        kill_streak: stat(local.kill_streak),
        items,
        ability_upgrades,
    }
}

pub fn collect(s: &GameSnapshot, local: &PlayerSample) -> Option<MatchDetails> {
    let local_team = local.team.filter(|team| *team == Team::Own)?;
    if !std::ptr::eq(local_player(s)?, local) || !complete_roster(s, local) {
        return None;
    }
    let scoreboard = s
        .players
        .iter()
        .filter(|row| !row.is_spectator)
        .map(|row| scoreboard(row, local_team))
        .collect::<Vec<_>>();
    let locals = scoreboard.iter().filter(|row| row.is_local).count();
    let misplaced = scoreboard
        .iter()
        .any(|row| row.is_local && row.side != Side::Own);
    if !(1..=limits::SCOREBOARD_ROWS).contains(&scoreboard.len()) || locals != 1 || misplaced {
        return None;
    }
    Some(MatchDetails {
        schema_version: 1,
        capabilities: CAPABILITIES,
        scoreboard,
        teams: teams(s, local_team),
        own_loadout: local_loadout(local),
        structures: structures(s, local_team),
        objectives: objectives(s),
    })
}

#[derive(Debug, Serialize, PartialEq)]
struct MidbossTelemetry {
    present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_spawn_in_seconds: Option<u32>,
    source: &'static str,
}

#[derive(Debug, PartialEq)]
pub struct Observation {
    hero: String,
    game_seconds: u32,
    kills: Option<u32>,
    deaths: Option<u32>,
    assists: Option<u32>,
    team_souls: Option<u32>,
    enemy_souls: Option<u32>,
    game_mode: Option<&'static str>,
    details: Option<MatchDetails>,
    paused: Option<bool>,
    midboss: Option<MidbossTelemetry>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Payload {
    schema_version: u8,
    writer_id: Uuid,
    sequence: u64,
    match_epoch: u64,
    hero: String,
    game_seconds: u32,
    kills: Option<u32>,
    deaths: Option<u32>,
    assists: Option<u32>,
    team_souls: Option<u32>,
    enemy_souls: Option<u32>,
    game_mode: Option<&'static str>,
    details: Option<MatchDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    midboss: Option<MidbossTelemetry>,
}

impl Payload {
    pub fn new(writer_id: Uuid, sequence: u64, match_epoch: u64, observation: Observation) -> Self {
        Self {
            schema_version: limits::SCHEMA_VERSION,
            writer_id,
            sequence,
            match_epoch,
            hero: observation.hero,
            game_seconds: observation.game_seconds,
            kills: observation.kills,
            deaths: observation.deaths,
            assists: observation.assists,
            team_souls: observation.team_souls,
            enemy_souls: observation.enemy_souls,
            game_mode: observation.game_mode,
            details: observation.details,
            paused: observation.paused,
            midboss: observation.midboss,
        }
    }
}

fn local_player(s: &GameSnapshot) -> Option<&PlayerSample> {
    let mut locals = s.players.iter().filter(|row| row.is_local == Some(true));
    let player = locals.next()?;
    (locals.next().is_none() && player.team == Some(Team::Own) && !player.is_spectator)
        .then_some(player)
}

fn complete_roster(s: &GameSnapshot, player: &PlayerSample) -> bool {
    let Some(team) = player.team else {
        return false;
    };
    let Some(opponent) = team.opponent() else {
        return false;
    };
    let mut count = 0;
    let mut own = false;
    let mut enemy = false;
    for row in s.players.iter().filter(|row| !row.is_spectator) {
        count += 1;
        match row.team {
            Some(side) if side == team => own = true,
            Some(side) if side == opponent => enemy = true,
            _ => return false,
        }
    }
    (1..=limits::SCOREBOARD_ROWS as u32).contains(&count)
        && s.expected_player_count == Some(count)
        && own
        && enemy
}

fn midboss(s: &GameSnapshot) -> Option<MidbossTelemetry> {
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

fn production_provenance(provenance: Provenance) -> bool {
    provenance == Provenance::Authorized
}

pub fn observation(s: &GameSnapshot) -> Option<Observation> {
    if !production_provenance(s.provenance)
        && !(cfg!(test) && s.provenance == Provenance::Synthetic)
    {
        return None;
    }
    if s.context != MatchContext::LivePlaying
        || s.eligibility != Eligibility::Eligible
        || !s.match_token.is_some_and(|token| token > 0)
    {
        return None;
    }
    let player = local_player(s)?;
    if !complete_roster(s, player) {
        return None;
    }
    player.hero_id.filter(|id| *id > 0)?;
    let hero = limits::hero_name(s.local_hero_name.as_deref()?)?.to_owned();
    let team = player.team?;
    let opponent = team.opponent()?;
    let game_mode = match s.game_mode {
        Some(GameMode::Normal) => Some("normal"),
        Some(GameMode::StreetBrawl) => Some("street_brawl"),
        _ => None,
    };
    Some(Observation {
        hero,
        kills: limits::within(player.kills, limits::SUMMARY_SCORE),
        deaths: limits::within(player.deaths, limits::SUMMARY_SCORE),
        assists: limits::within(player.assists, limits::SUMMARY_SCORE),
        team_souls: sum_side(s, team, |r| r.souls, limits::SOULS),
        enemy_souls: sum_side(s, opponent, |r| r.souls, limits::SOULS),
        game_mode,
        details: game_mode.and_then(|_| collect(s, player)),
        game_seconds: limits::clock_seconds(s.playing_seconds)?,
        paused: s.paused,
        midboss: midboss(s),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximal_wire_shape_fits_limit() {
        let observation = Observation {
            hero: "\"".repeat(limits::HERO_NAME_BYTES),
            game_seconds: limits::CLOCK_SECONDS as u32,
            kills: Some(limits::SUMMARY_SCORE),
            deaths: Some(limits::SUMMARY_SCORE),
            assists: Some(limits::SUMMARY_SCORE),
            team_souls: Some(limits::SOULS),
            enemy_souls: Some(limits::SOULS),
            game_mode: Some("normal"),
            details: None,
            paused: Some(false),
            midboss: Some(MidbossTelemetry {
                present: false,
                next_spawn_in_seconds: Some(limits::TIMER_SECONDS as u32),
                source: "game",
            }),
        };
        let packet = Payload::new(
            Uuid::new_v4(),
            limits::MAX_COUNTER,
            limits::MAX_COUNTER,
            observation,
        );
        let bytes = crate::encode_body(&packet).unwrap();
        assert!(bytes.len() <= limits::BODY_BYTES);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["hero"]
                .as_str()
                .unwrap()
                .len(),
            limits::HERO_NAME_BYTES
        );
        eprintln!("Extended summary wire shape: {} bytes", bytes.len());
    }
    fn timer(source: TimerSource) -> ObjectiveTimer {
        ObjectiveTimer {
            present: Some(false),
            count: Some(0),
            next_spawn_in: Some(45.1),
            source,
        }
    }

    #[test]
    fn estimated_timer_cannot_be_reported_as_game_timer() {
        let estimate = objective(Some(&timer(TimerSource::Schedule)), true).unwrap();
        assert_eq!(estimate.estimated_spawn_in_seconds, Some(46));
        assert_eq!(estimate.next_spawn_in_seconds, None);
        let unknown = objective(Some(&timer(TimerSource::Unknown)), true).unwrap();
        assert!(unknown.next_spawn_in_seconds.is_none());
        assert!(unknown.estimated_spawn_in_seconds.is_none());
    }

    #[test]
    fn only_bridge_buffs_may_carry_an_estimate() {
        let mut s = GameSnapshot {
            game_mode: Some(GameMode::Normal),
            ..Default::default()
        };
        s.timers.midboss = Some(timer(TimerSource::Schedule));
        s.timers.urn = Some(timer(TimerSource::Schedule));
        s.timers.rift = Some(timer(TimerSource::Schedule));
        s.timers.bridge_buffs = Some(timer(TimerSource::Schedule));
        let rows = objectives(&s);
        for row in [rows.midboss, rows.urn, rows.rift] {
            let row = row.unwrap();
            assert_eq!(row.source, "schedule");
            assert_eq!(row.estimated_spawn_in_seconds, None);
            assert_eq!(row.next_spawn_in_seconds, None);
        }
        let bridge = rows.bridge_buffs.unwrap();
        assert_eq!(bridge.estimated_spawn_in_seconds, Some(46));
        s.timers.urn = Some(timer(TimerSource::Game));
        assert_eq!(objectives(&s).urn.unwrap().next_spawn_in_seconds, Some(46));
    }

    #[test]
    fn a_side_is_own_or_enemy_only_for_the_two_playing_teams() {
        assert_eq!(Side::of(Some(Team::Own), Team::Own), Side::Own);
        assert_eq!(Side::of(Some(Team::Enemy), Team::Own), Side::Enemy);
        assert_eq!(Side::of(Some(Team::Own), Team::Enemy), Side::Enemy);
        assert_eq!(Side::of(Some(Team::Unknown), Team::Own), Side::Unknown);
        assert_eq!(Side::of(None, Team::Own), Side::Unknown);
    }

    #[test]
    fn no_private_identifiers_in_scoreboard_contract() {
        let row = ScoreboardRow {
            side: Side::Own,
            is_local: true,
            slot: Some(1),
            hero_id: Some(2),
            level: Some(4),
            souls: Some(1),
            kills: None,
            deaths: None,
            assists: None,
            last_hits: None,
            denies: None,
            hero_damage: None,
            objective_damage: None,
            healing: None,
            alive: None,
        };
        let data = serde_json::to_string(&row).unwrap();
        for field in [
            "steam_id",
            "controller",
            "pawn",
            "name",
            "position",
            "ultimate_cooldown",
        ] {
            assert!(!data.contains(field));
        }
    }
}
