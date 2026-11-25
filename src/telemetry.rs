use deadlock_reader::snapshot::{
    GameMode, LiveSnapshot, ObjectiveKind, PlayerRow, StructureState, TeamStats,
};
use deadlock_reader::timers::{ObjectiveTimer, TimerSource};
use serde::Serialize;

const MAX_STAT: u32 = 1_000_000_000;
const MAX_LEVEL: u32 = 1000;
const MAX_HEALTH: i32 = 1_000_000;
const MAX_REJUVENATORS: u32 = 1000;
const MAX_STRUCTURES: u32 = 128;
const MAX_TIMER_SECONDS: f32 = 7200.0;

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
    pub side: &'static str,
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
    pub side: &'static str,
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
    pub side: &'static str,
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

fn objective(timer: Option<&ObjectiveTimer>) -> Option<ObjectiveRow> {
    let timer = timer?;
    let (source, direct, estimate) = match timer.source {
        TimerSource::Game => ("game", true, false),
        TimerSource::Schedule => ("schedule", false, true),
        TimerSource::Unknown => ("unknown", false, false),
    };
    let seconds = timer
        .next_spawn_in
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= MAX_TIMER_SECONDS)
        .map(|n| n.ceil() as u32);
    let seconds = if timer.present == Some(true) {
        None
    } else {
        seconds
    };
    Some(ObjectiveRow {
        present: timer.present,
        count: timer.count.filter(|n| *n <= 128),
        source,
        next_spawn_in_seconds: if direct { seconds } else { None },
        estimated_spawn_in_seconds: if estimate { seconds } else { None },
    })
}

fn stat(value: Option<u32>) -> Option<u32> {
    value.filter(|n| *n <= MAX_STAT)
}

fn level(value: Option<u32>) -> Option<u32> {
    value.filter(|n| *n <= MAX_LEVEL)
}

fn side(team: Option<deadlock_core::Team>, local: deadlock_core::Team) -> &'static str {
    match team {
        Some(team) if team == local => "own",
        Some(_) => "enemy",
        None => "unknown",
    }
}

fn scoreboard(row: &PlayerRow, local: deadlock_core::Team) -> ScoreboardRow {
    let is_local = row.is_local == Some(true);
    let own = |value: Option<u32>| stat(value).filter(|_| is_local);
    ScoreboardRow {
        side: side(row.team, local),
        is_local,
        slot: row.slot.filter(|slot| (1..=12).contains(slot)),
        hero_id: row.hero_id.map(|id| id.0).filter(|id| *id > 0),
        level: level(row.level),
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

fn team(stats: &TeamStats, local: deadlock_core::Team) -> Option<TeamRow> {
    let totals = [stats.souls, stats.kills, stats.deaths, stats.assists];
    if totals.iter().any(|n| *n > MAX_STAT) {
        return None;
    }
    let own = |value: u32| stat(Some(value)).filter(|_| stats.team == local);
    Some(TeamRow {
        side: side(Some(stats.team), local),
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
        rejuvenators: stats.rejuvenators.filter(|n| *n <= MAX_REJUVENATORS),
        structures_alive: stats
            .structures_known
            .then_some(stats.structures_alive)
            .filter(|n| *n <= MAX_STRUCTURES),
        complete: stats.complete,
    })
}

fn teams(s: &LiveSnapshot, local: deadlock_core::Team) -> Vec<TeamRow> {
    let rows = s
        .teams
        .iter()
        .filter(|stats| stats.players > 0)
        .take(2)
        .filter_map(|stats| team(stats, local))
        .collect::<Vec<_>>();
    if rows.len() == 2 && rows[0].side == rows[1].side {
        return Vec::new();
    }
    rows
}

fn structures(s: &LiveSnapshot, local: deadlock_core::Team) -> Vec<StructureRow> {
    s.objectives
        .iter()
        .filter(|o| {
            matches!(
                o.kind,
                ObjectiveKind::Guardian
                    | ObjectiveKind::Walker
                    | ObjectiveKind::BaseGuardian
                    | ObjectiveKind::Patron
                    | ObjectiveKind::Shrine
                    | ObjectiveKind::Midboss
            )
        })
        .take(48)
        .map(|o| StructureRow {
            side: side(o.team, local),
            kind: o.kind.label(),
            lane: o.lane.filter(|n| (0..=8).contains(n)),
            state: match o.state {
                StructureState::Standing => "standing",
                StructureState::Destroyed => "destroyed",
                StructureState::Unknown => "unknown",
            },
        })
        .collect()
}

fn objectives(s: &LiveSnapshot) -> ObjectiveRows {
    let normal = s.game_mode == Some(GameMode::Normal);
    let row = |timer: &Option<ObjectiveTimer>| normal.then(|| objective(timer.as_ref())).flatten();
    ObjectiveRows {
        midboss: row(&s.timers.midboss),
        urn: row(&s.timers.urn),
        rift: row(&s.timers.rift),
        bridge_buffs: row(&s.timers.bridge_buffs),
    }
}

fn local_loadout(local: &PlayerRow) -> OwnLoadout {
    let items = local
        .items
        .as_ref()
        .filter(|v| v.len() <= 64 && v.iter().all(|id| *id > 0))
        .cloned();
    let ability_upgrades = local
        .ability_upgrades
        .as_ref()
        .filter(|v| v.len() <= 16 && v.iter().all(|a| a.item_id > 0 && a.points <= 5))
        .map(|v| {
            v.iter()
                .map(|a| AbilityRow {
                    item_id: a.item_id,
                    unlocked: a.unlocked,
                    points: a.points,
                })
                .collect()
        });
    OwnLoadout {
        level: level(local.level),
        health: local.health.filter(|n| (-1..=MAX_HEALTH).contains(n)),
        max_health: local.max_health.filter(|n| (0..=MAX_HEALTH).contains(n)),
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
    let local_team = local.team?;
    let rows = s
        .scoreboard()
        .filter(|row| !row.is_spectator)
        .collect::<Vec<_>>();
    let locals = rows.iter().filter(|r| r.is_local == Some(true)).count();
    if rows.len() > 12 || locals != 1 {
        return None;
    }
    Some(MatchDetails {
        schema_version: 1,
        capabilities: &[
            "scoreboard",
            "teams",
            "own_loadout",
            "structures",
            "objectives",
            "timer_provenance",
        ],
        scoreboard: rows.iter().map(|r| scoreboard(r, local_team)).collect(),
        teams: teams(s, local_team),
        own_loadout: local_loadout(local),
        structures: structures(s, local_team),
        objectives: objectives(s),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimated_timer_cannot_be_reported_as_game_timer() {
        let source = ObjectiveTimer {
            present: Some(false),
            count: Some(0),
            next_spawn_in: Some(45.1),
            source: TimerSource::Schedule,
        };
        let estimate = objective(Some(&source)).unwrap();
        assert_eq!(estimate.estimated_spawn_in_seconds, Some(46));
        assert_eq!(estimate.next_spawn_in_seconds, None);
        let unknown = objective(Some(&ObjectiveTimer {
            source: TimerSource::Unknown,
            ..source
        }))
        .unwrap();
        assert!(unknown.next_spawn_in_seconds.is_none());
        assert!(unknown.estimated_spawn_in_seconds.is_none());
    }
    #[test]
    fn no_private_identifiers_in_scoreboard_contract() {
        let row = ScoreboardRow {
            side: "own",
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
