use std::ops::RangeInclusive;

pub const BODY_BYTES: usize = 32_768;
pub const SCHEMA_VERSION: u8 = 3;
pub const MAX_COUNTER: u64 = 9_007_199_254_740_991;
pub const CLOCK_SECONDS: f32 = 86_400.0;
pub const HERO_NAME_BYTES: usize = 80;
pub const TIMER_SECONDS: f32 = 7200.0;

pub const SUMMARY_SCORE: u32 = 1000;
pub const SOULS: u32 = 1_000_000_000;
pub const STAT: u32 = 1_000_000_000;
pub const LEVEL: u32 = 1000;
pub const HEALTH: RangeInclusive<i32> = -1..=1_000_000;
pub const MAX_HEALTH: RangeInclusive<i32> = 0..=1_000_000;
pub const REJUVENATORS: u32 = 1000;
pub const STRUCTURES_ALIVE: u32 = 128;
pub const OBJECTIVE_COUNT: usize = 128;
pub const SCOREBOARD_ROWS: usize = 12;
pub const SLOTS: RangeInclusive<u32> = 1..=12;
pub const TEAM_ROWS: usize = 2;
pub const STRUCTURE_ROWS: usize = 48;
pub const LANES: RangeInclusive<i32> = 0..=8;
pub const ITEMS: usize = 64;
pub const ABILITY_UPGRADES: usize = 16;
pub const ABILITY_POINTS: u32 = 5;

pub fn within(value: Option<u32>, limit: u32) -> Option<u32> {
    value.filter(|n| *n <= limit)
}

pub fn timer_seconds(value: Option<f32>) -> Option<u32> {
    value
        .filter(|n| n.is_finite() && (0.0..=TIMER_SECONDS).contains(n))
        .map(|n| n.ceil() as u32)
}

pub fn clock_seconds(value: Option<f32>) -> Option<u32> {
    value
        .filter(|n| n.is_finite() && (0.0..=CLOCK_SECONDS).contains(n))
        .map(|n| n as u32)
}

pub fn hero_name(value: &str) -> Option<&str> {
    let name = value.trim();
    let fits = (1..=HERO_NAME_BYTES).contains(&name.len());
    (fits && !name.chars().any(char::is_control)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_is_rounded_up_and_dropped_outside_the_accepted_range() {
        assert_eq!(timer_seconds(Some(0.0)), Some(0));
        assert_eq!(timer_seconds(Some(41.2)), Some(42));
        assert_eq!(timer_seconds(Some(7200.0)), Some(7200));
        for invalid in [f32::NAN, f32::INFINITY, -0.5, 7200.5] {
            assert_eq!(timer_seconds(Some(invalid)), None);
        }
        assert_eq!(timer_seconds(None), None);
    }

    #[test]
    fn the_clock_is_rounded_down_and_dropped_outside_the_accepted_range() {
        assert_eq!(clock_seconds(Some(42.8)), Some(42));
        assert_eq!(clock_seconds(Some(86_400.0)), Some(86_400));
        for invalid in [f32::NAN, f32::INFINITY, -0.5, 86_401.0] {
            assert_eq!(clock_seconds(Some(invalid)), None);
        }
    }

    #[test]
    fn a_hero_name_is_trimmed_and_measured_in_bytes() {
        assert_eq!(hero_name("  Infernus "), Some("Infernus"));
        assert_eq!(hero_name(&"ü".repeat(40)).map(str::len), Some(80));
        for invalid in ["", "   ", "Infer\u{7}nus"] {
            assert_eq!(hero_name(invalid), None);
        }
        assert_eq!(hero_name(&"ü".repeat(41)), None);
        assert_eq!(hero_name(&"x".repeat(81)), None);
    }
}
