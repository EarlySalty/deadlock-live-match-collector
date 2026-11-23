//! Versioniertes, bereinigtes Live-Match-Datenpaket.
//! Keine Steam-IDs, Namen, Prozessadressen, Positionen oder gegnerischen Cooldowns.
use deadlock_reader::snapshot::{GameMode, LiveSnapshot, ObjectiveKind, PlayerRow, StructureState};
use deadlock_reader::timers::{ObjectiveTimer, TimerSource};
use serde::Serialize;

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
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 7200.0)
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

fn side(team: Option<deadlock_core::Team>, local: deadlock_core::Team) -> &'static str {
    match team {
        Some(team) if team == local => "own",
        Some(_) => "enemy",
        None => "unknown",
    }
}
fn scoreboard(row: &PlayerRow, local: deadlock_core::Team) -> ScoreboardRow {
    ScoreboardRow {
        side: side(row.team, local),
        is_local: row.is_local == Some(true),
        slot: row.slot.filter(|slot| (1..=12).contains(slot)),
        hero_id: row.hero_id.map(|id| id.0).filter(|id| *id > 0),
        level: row.level,
        souls: row.net_worth,
        kills: row.kills,
        deaths: row.deaths,
        assists: row.assists,
        // Nicht sichtbare Detailwerte anderer Spieler vermeiden.
        last_hits: row
            .is_local
            .and_then(|local| local.then_some(row.last_hits))
            .flatten(),
        denies: row
            .is_local
            .and_then(|local| local.then_some(row.denies))
            .flatten(),
        hero_damage: row
            .is_local
            .and_then(|local| local.then_some(row.hero_damage))
            .flatten(),
        objective_damage: row
            .is_local
            .and_then(|local| local.then_some(row.objective_damage))
            .flatten(),
        healing: row
            .is_local
            .and_then(|local| local.then_some(row.healing))
            .flatten(),
        alive: row.is_alive,
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
        level: local.level,
        health: local.health,
        max_health: local.max_health,
        souls: local.net_worth,
        ability_points: local.ability_points,
        creep_souls: local.creep_souls,
        secured_souls: local.secured_souls,
        denied_souls: local.denied_souls,
        neutral_souls: local.neutral_souls,
        farm_baseline: local.farm_baseline,
        kill_streak: local.kill_streak,
        items,
        ability_upgrades,
    }
}
pub fn collect(s: &LiveSnapshot, local: &PlayerRow) -> Option<MatchDetails> {
    let local_team = local.team?;
    let rows = s.scoreboard().collect::<Vec<_>>();
    if rows.is_empty() || rows.len() > 12 {
        return None;
    }
    let scoreboard = rows.iter().map(|r| scoreboard(r, local_team)).collect();
    let teams = s
        .teams
        .iter()
        .filter(|r| r.players > 0)
        .take(2)
        .map(|t| TeamRow {
            side: side(Some(t.team), local_team),
            souls: t.souls,
            ability_points: (t.team == local_team).then_some(t.ability_points),
            kills: t.kills,
            deaths: t.deaths,
            assists: t.assists,
            last_hits: (t.team == local_team).then_some(t.last_hits),
            denies: (t.team == local_team).then_some(t.denies),
            hero_damage: (t.team == local_team).then_some(t.hero_damage),
            objective_damage: (t.team == local_team).then_some(t.objective_damage),
            healing: (t.team == local_team).then_some(t.healing),
            rejuvenators: t.rejuvenators,
            structures_alive: t.structures_known.then_some(t.structures_alive),
            complete: t.complete,
        })
        .collect();
    let structures = s
        .objectives
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
            side: side(o.team, local_team),
            kind: o.kind.label(),
            lane: o.lane.filter(|n| (0..=8).contains(n)),
            state: match o.state {
                StructureState::Standing => "standing",
                StructureState::Destroyed => "destroyed",
                StructureState::Unknown => "unknown",
            },
        })
        .collect();
    // Objektiv-Timer gelten nur fuer den Standardmodus. In Brawl gibt es
    // andere Spielphasen; deren Werte sind hier bewusst unbekannt.
    let normal_objectives = s.game_mode == Some(GameMode::Normal);
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
        scoreboard,
        teams,
        own_loadout: local_loadout(local),
        structures,
        objectives: ObjectiveRows {
            midboss: normal_objectives
                .then(|| objective(s.timers.midboss.as_ref()))
                .flatten(),
            urn: normal_objectives
                .then(|| objective(s.timers.urn.as_ref()))
                .flatten(),
            rift: normal_objectives
                .then(|| objective(s.timers.rift.as_ref()))
                .flatten(),
            bridge_buffs: normal_objectives
                .then(|| objective(s.timers.bridge_buffs.as_ref()))
                .flatten(),
        },
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
