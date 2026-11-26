//! Collector-owned observation DTOs, derived from the API v3 visible-data contract.
//! No serialization, persistent identifiers, process addresses or raw source records.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Team {
    Own,
    Enemy,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchContext {
    LivePlaying,
    Other,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    Excluded,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provenance {
    Authorized,
    Synthetic,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GameMode {
    Normal,
    StreetBrawl,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StructureKind {
    Guardian,
    Walker,
    BaseGuardian,
    Patron,
    Shrine,
    Midboss,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StructureState {
    Standing,
    Destroyed,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimerSource {
    Game,
    Schedule,
    #[default]
    Unknown,
}

impl Team {
    pub fn opponent(self) -> Option<Self> {
        match self {
            Self::Own => Some(Self::Enemy),
            Self::Enemy => Some(Self::Own),
            Self::Unknown => None,
        }
    }

    pub fn is_playing(self) -> bool {
        self.opponent().is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameSnapshot {
    pub match_token: Option<u64>,
    pub context: MatchContext,
    pub eligibility: Eligibility,
    pub provenance: Provenance,
    pub expected_player_count: Option<u32>,
    pub game_mode: Option<GameMode>,
    pub playing_seconds: Option<f32>,
    pub paused: Option<bool>,
    pub local_hero_name: Option<String>,
    pub players: Vec<PlayerSample>,
    pub teams: Vec<TeamSample>,
    pub structures: Vec<StructureSample>,
    pub timers: ObjectiveTimers,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSample {
    pub team: Option<Team>,
    pub is_local: Option<bool>,
    pub is_spectator: bool,
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
    pub ability_points: Option<u32>,
    pub creep_souls: Option<u32>,
    pub secured_souls: Option<u32>,
    pub denied_souls: Option<u32>,
    pub neutral_souls: Option<u32>,
    pub farm_baseline: Option<u32>,
    pub kill_streak: Option<u32>,
    pub alive: Option<bool>,
    pub health: Option<i32>,
    pub max_health: Option<i32>,
    pub items: Option<Vec<u32>>,
    pub ability_upgrades: Option<Vec<AbilityUpgrade>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AbilityUpgrade {
    pub item_id: u32,
    pub unlocked: bool,
    pub points: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamSample {
    pub team: Team,
    pub complete: bool,
    pub ability_points: Option<u32>,
    pub last_hits: Option<u32>,
    pub denies: Option<u32>,
    pub hero_damage: Option<u32>,
    pub objective_damage: Option<u32>,
    pub healing: Option<u32>,
    pub rejuvenators: Option<u32>,
    pub structures_alive: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructureSample {
    pub team: Option<Team>,
    pub kind: StructureKind,
    pub lane: Option<i32>,
    pub state: StructureState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectiveTimers {
    pub midboss: Option<ObjectiveTimer>,
    pub urn: Option<ObjectiveTimer>,
    pub rift: Option<ObjectiveTimer>,
    pub bridge_buffs: Option<ObjectiveTimer>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectiveTimer {
    pub present: Option<bool>,
    pub count: Option<usize>,
    pub next_spawn_in: Option<f32>,
    pub source: TimerSource,
}
