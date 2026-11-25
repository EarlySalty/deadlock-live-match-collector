use serde::{Deserialize, Serialize};

pub const SOURCE: &str = "Deadlock-Twitch-Bot 77e25238e, rust/crates/tb-dashboard-api/src/handlers/live_match_collector.rs and live_match_details.rs";

const MAX_CLOCK: u32 = 86_400;
const MAX_SOULS: u32 = 1_000_000_000;
const MAX_STATS: u32 = 1_000_000_000;
const MAX_MIDBOSS: u32 = 7_200;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    match_id: String,
    hero: String,
    game_seconds: u32,
    kills: u32,
    deaths: u32,
    assists: u32,
    team_souls: u32,
    enemy_souls: u32,
    #[serde(default)]
    game_mode: Option<String>,
    #[serde(default)]
    paused: Option<bool>,
    #[serde(default)]
    midboss: Option<MidbossUpload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    details: Option<MatchDetails>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MidbossUpload {
    present: bool,
    #[serde(default)]
    next_spawn_in_seconds: Option<u32>,
    source: String,
}

impl Upload {
    pub fn valid(&self) -> bool {
        let id_ok = self.match_id.len() <= 20
            && self.match_id.parse::<u64>().is_ok_and(|v| v != 0)
            && self.match_id.bytes().all(|b| b.is_ascii_digit());
        let hero_ok = (1..=80).contains(&self.hero.len())
            && self.hero.trim() == self.hero
            && !self.hero.chars().any(char::is_control);
        let mode_ok = self
            .game_mode
            .as_deref()
            .is_none_or(|mode| mode == "normal" || mode == "street_brawl")
            && (self.midboss.is_none() || self.game_mode.as_deref() == Some("normal"));
        let midboss_ok = self.midboss.as_ref().is_none_or(|m| {
            m.source == "game"
                && (m.next_spawn_in_seconds.is_none() || !m.present)
                && m.next_spawn_in_seconds.is_none_or(|t| t <= MAX_MIDBOSS)
        });
        id_ok
            && hero_ok
            && self.game_seconds <= MAX_CLOCK
            && [self.kills, self.deaths, self.assists]
                .iter()
                .all(|&n| n <= 1000)
            && self.team_souls <= MAX_SOULS
            && self.enemy_souls <= MAX_SOULS
            && mode_ok
            && midboss_ok
            && self
                .details
                .as_ref()
                .is_none_or(|details| self.game_mode.is_some() && details.valid())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchDetails {
    pub schema_version: u8,
    pub capabilities: Vec<String>,
    pub scoreboard: Vec<ScoreboardRow>,
    pub teams: Vec<TeamRow>,
    pub own_loadout: OwnLoadout,
    pub structures: Vec<StructureRow>,
    pub objectives: ObjectiveRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Own,
    Enemy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityRow {
    pub item_id: u32,
    pub unlocked: bool,
    pub points: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureState {
    Standing,
    Destroyed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureKind {
    Guardian,
    Walker,
    BaseGuardian,
    Patron,
    Shrine,
    Midboss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureRow {
    pub side: Side,
    pub kind: StructureKind,
    pub lane: Option<i32>,
    pub state: StructureState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveRows {
    pub midboss: Option<ObjectiveRow>,
    pub urn: Option<ObjectiveRow>,
    pub rift: Option<ObjectiveRow>,
    pub bridge_buffs: Option<ObjectiveRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerSource {
    Game,
    Schedule,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveRow {
    pub present: Option<bool>,
    pub count: Option<usize>,
    pub source: TimerSource,
    pub next_spawn_in_seconds: Option<u32>,
    pub estimated_spawn_in_seconds: Option<u32>,
}

impl ObjectiveRow {
    fn valid(&self) -> bool {
        let observed = self.source == TimerSource::Game;
        let estimated = self.source == TimerSource::Schedule;
        (self.next_spawn_in_seconds.is_none() || observed)
            && (self.estimated_spawn_in_seconds.is_none() || estimated)
            && !(self.next_spawn_in_seconds.is_some() && self.estimated_spawn_in_seconds.is_some())
            && self.count.is_none_or(|v| v <= 128)
            && self.next_spawn_in_seconds.is_none_or(|v| v <= 7200)
            && self.estimated_spawn_in_seconds.is_none_or(|v| v <= 7200)
            && (self.present != Some(true)
                || (self.next_spawn_in_seconds.is_none()
                    && self.estimated_spawn_in_seconds.is_none()))
    }
}

impl MatchDetails {
    pub fn valid(&self) -> bool {
        const CAPS: [&str; 6] = [
            "scoreboard",
            "teams",
            "own_loadout",
            "structures",
            "objectives",
            "timer_provenance",
        ];
        if self.schema_version != 1
            || self
                .capabilities
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != CAPS
            || !(1..=12).contains(&self.scoreboard.len())
            || self.teams.len() > 2
            || self.structures.len() > 48
            || self.scoreboard.iter().filter(|p| p.is_local).count() != 1
        {
            return false;
        }
        if self.scoreboard.iter().any(|p| {
            p.is_local && p.side != Side::Own
                || (!p.is_local
                    && [
                        p.last_hits,
                        p.denies,
                        p.hero_damage,
                        p.objective_damage,
                        p.healing,
                    ]
                    .into_iter()
                    .any(|n| n.is_some()))
                || p.slot.is_some_and(|s| !(1..=12).contains(&s))
                || p.hero_id.is_some_and(|n| n == 0)
                || p.level.is_some_and(|n| n > 1000)
                || [
                    p.souls,
                    p.kills,
                    p.deaths,
                    p.assists,
                    p.last_hits,
                    p.denies,
                    p.hero_damage,
                    p.objective_damage,
                    p.healing,
                ]
                .into_iter()
                .flatten()
                .any(|n| n > MAX_STATS)
        }) {
            return false;
        }
        if self.teams.iter().any(|t| {
            t.side == Side::Unknown
                || t.souls > MAX_SOULS
                || [t.kills, t.deaths, t.assists]
                    .into_iter()
                    .any(|n| n > MAX_STATS)
                || [
                    t.ability_points,
                    t.last_hits,
                    t.denies,
                    t.hero_damage,
                    t.objective_damage,
                    t.healing,
                ]
                .into_iter()
                .flatten()
                .any(|n| n > MAX_STATS)
                || (t.side == Side::Enemy
                    && [
                        t.ability_points,
                        t.last_hits,
                        t.denies,
                        t.hero_damage,
                        t.objective_damage,
                        t.healing,
                    ]
                    .into_iter()
                    .any(|n| n.is_some()))
                || t.rejuvenators.is_some_and(|n| n > 1000)
                || t.structures_alive.is_some_and(|n| n > 128)
        }) {
            return false;
        }
        if self.teams.len() == 2 && self.teams[0].side == self.teams[1].side {
            return false;
        }
        let own = &self.own_loadout;
        if own.level.is_some_and(|n| n > 1000)
            || own.health.is_some_and(|n| !(-1..=1_000_000).contains(&n))
            || own
                .max_health
                .is_some_and(|n| !(0..=1_000_000).contains(&n))
            || [
                own.souls,
                own.ability_points,
                own.creep_souls,
                own.secured_souls,
                own.denied_souls,
                own.neutral_souls,
                own.farm_baseline,
                own.kill_streak,
            ]
            .into_iter()
            .flatten()
            .any(|n| n > MAX_STATS)
            || own
                .items
                .as_ref()
                .is_some_and(|v| v.len() > 64 || v.contains(&0))
            || own
                .ability_upgrades
                .as_ref()
                .is_some_and(|v| v.len() > 16 || v.iter().any(|a| a.item_id == 0 || a.points > 5))
        {
            return false;
        }
        if self
            .structures
            .iter()
            .any(|s| s.lane.is_some_and(|n| !(0..=8).contains(&n)))
        {
            return false;
        }
        [
            self.objectives.midboss.as_ref(),
            self.objectives.urn.as_ref(),
            self.objectives.rift.as_ref(),
            self.objectives.bridge_buffs.as_ref(),
        ]
        .into_iter()
        .flatten()
        .all(ObjectiveRow::valid)
    }
}
