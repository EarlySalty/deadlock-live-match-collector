use deadlock_core::Team;
use deadlock_reader::snapshot::{
    GameMode, LiveSnapshot, ObjectiveKind, PlayerRow, StructureState, TeamStats,
};
use deadlock_reader::timers::{ObjectiveTimer, TimerSource};
use serde::Serialize;

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
            Some(team) if team == local => Side::Own,
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
    pub souls: u32,
    pub ability_points: Option<u32>,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
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

fn scoreboard(row: &PlayerRow, local: Team) -> ScoreboardRow {
    let is_local = row.is_local == Some(true);
    let own = |value: Option<u32>| stat(value).filter(|_| is_local);
    ScoreboardRow {
        side: Side::of(row.team, local),
        is_local,
        slot: row.slot.filter(|slot| limits::SLOTS.contains(slot)),
        hero_id: row.hero_id.map(|id| id.0).filter(|id| *id > 0),
        level: limits::within(row.level, limits::LEVEL),
        souls: stat(row.net_worth),
        kills: stat(row.kills),
        deaths: stat(row.deaths),
        assists: stat(row.assists),
        last_hits: own(row.last_hits),
        denies: own(row.denies),
        hero_damage: own(row.hero_damage),
        objective_damage: own(row.objective_damage),
        healing: own(row.healing),
        alive: row.is_alive,
    }
}

fn team(stats: &TeamStats, local: Team) -> Option<TeamRow> {
    let side = Side::of(Some(stats.team), local);
    let scores = [stats.kills, stats.deaths, stats.assists];
    if side == Side::Unknown
        || stats.souls > limits::SOULS
        || scores.iter().any(|n| *n > limits::STAT)
    {
        return None;
    }
    let own = |value: u32| stat(Some(value)).filter(|_| side == Side::Own);
    Some(TeamRow {
        side,
        souls: stats.souls,
        ability_points: own(stats.ability_points),
        kills: stats.kills,
        deaths: stats.deaths,
        assists: stats.assists,
        last_hits: own(stats.last_hits),
        denies: own(stats.denies),
        hero_damage: own(stats.hero_damage),
        objective_damage: own(stats.objective_damage),
        healing: own(stats.healing),
        rejuvenators: limits::within(stats.rejuvenators, limits::REJUVENATORS),
        structures_alive: limits::within(
            stats.structures_known.then_some(stats.structures_alive),
            limits::STRUCTURES_ALIVE,
        ),
        complete: stats.complete,
    })
}

fn teams(s: &LiveSnapshot, local: Team) -> Vec<TeamRow> {
    let rows = s
        .teams
        .iter()
        .filter(|stats| stats.players > 0)
        .filter_map(|stats| team(stats, local))
        .collect::<Vec<_>>();
    let repeated = rows.len() == limits::TEAM_ROWS && rows[0].side == rows[1].side;
    if rows.len() > limits::TEAM_ROWS || repeated {
        return Vec::new();
    }
    rows
}

fn structure_kind(kind: ObjectiveKind) -> Option<&'static str> {
    matches!(
        kind,
        ObjectiveKind::Guardian
            | ObjectiveKind::Walker
            | ObjectiveKind::BaseGuardian
            | ObjectiveKind::Patron
            | ObjectiveKind::Shrine
            | ObjectiveKind::Midboss
    )
    .then(|| kind.label())
}

fn structures(s: &LiveSnapshot, local: Team) -> Vec<StructureRow> {
    let rows = s
        .objectives
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

fn objectives(s: &LiveSnapshot) -> ObjectiveRows {
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

fn local_loadout(local: &PlayerRow) -> OwnLoadout {
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
        souls: stat(local.net_worth),
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

pub fn collect(s: &LiveSnapshot, local: &PlayerRow) -> Option<MatchDetails> {
    let local_team = local.team.filter(|team| team.is_playing())?;
    let scoreboard = s
        .scoreboard()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut s = LiveSnapshot {
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
        assert_eq!(Side::of(Some(Team::AMBER), Team::AMBER), Side::Own);
        assert_eq!(Side::of(Some(Team::SAPPHIRE), Team::AMBER), Side::Enemy);
        assert_eq!(Side::of(Some(Team::AMBER), Team::SAPPHIRE), Side::Enemy);
        for other in [Team::UNASSIGNED, Team::SPECTATOR, Team::NEUTRAL, Team(9)] {
            assert_eq!(Side::of(Some(other), Team::AMBER), Side::Unknown);
        }
        assert_eq!(Side::of(None, Team::AMBER), Side::Unknown);
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
