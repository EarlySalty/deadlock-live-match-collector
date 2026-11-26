//! Offline, test-only OCR experiment. No screen capture, live input, network or
//! authorization assertion. Crop coordinates/semantics must be independently
//! established on permitted recordings before any production integration.
use crate::game_model::*;
use crate::game_source::{GameSource, SourceError};
use crate::limits;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

const MAX_CROPS: usize = 128;
const MAX_PIXELS: usize = 1024 * 256;
const MAX_TEXT: usize = 80;
const MIN_CONFIDENCE: f32 = 85.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Column {
    Team,
    Local,
    Level,
    Souls,
    Kills,
    Deaths,
    Assists,
    Alive,
    Health,
    MaxHealth,
    AbilityPoints,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Objective {
    Midboss,
    Urn,
    Rift,
    BridgeBuffs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Clock,
    Mode,
    Pause,
    HeroName,
    Row(u8, Column),
    Presence(Objective),
    Count(Objective),
    Countdown(Objective),
}

struct Crop {
    field: Field,
    width: usize,
    height: usize,
    pixels: Vec<u8>, // grayscale; no file metadata or raw screenshot persisted
}

impl Crop {
    fn validate(&self) -> Result<(), SourceError> {
        if self.width == 0
            || self.width > 1024
            || self.height == 0
            || self.height > 256
            || self.width.checked_mul(self.height) != Some(self.pixels.len())
            || self.pixels.len() > MAX_PIXELS
        {
            return Err(SourceError::InvalidSample);
        }
        if let Field::Row(row, _) = self.field {
            if row == 0 || usize::from(row) > limits::SCOREBOARD_ROWS {
                return Err(SourceError::InvalidSample);
            }
        }
        Ok(())
    }
    fn pgm(&self) -> Vec<u8> {
        let mut bytes = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        bytes.extend_from_slice(&self.pixels);
        bytes
    }
}

trait Recognizer {
    fn recognize(&mut self, crop: &Crop) -> Result<Option<String>, SourceError>;
}

struct Tesseract;
impl Recognizer for Tesseract {
    fn recognize(&mut self, crop: &Crop) -> Result<Option<String>, SourceError> {
        crop.validate()?;
        let mut child = Command::new("tesseract")
            .args(["stdin", "stdout", "-l", "eng", "--psm", "7", "tsv"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| SourceError::Unavailable)?;
        let written = child
            .stdin
            .take()
            .ok_or(SourceError::Unavailable)
            .and_then(|mut input| {
                input
                    .write_all(&crop.pgm())
                    .map_err(|_| SourceError::Unavailable)
            });
        if written.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SourceError::Unavailable);
        }
        let output = child
            .wait_with_output()
            .map_err(|_| SourceError::Unavailable)?;
        if !output.status.success() {
            return Err(SourceError::Unavailable);
        }
        let tsv = std::str::from_utf8(&output.stdout).map_err(|_| SourceError::InvalidSample)?;
        Ok(parse_tsv(tsv))
    }
}

fn parse_tsv(tsv: &str) -> Option<String> {
    if tsv.len() > 16_384 {
        return None;
    }
    let mut words = Vec::new();
    let mut line = None;
    for row in tsv.lines().skip(1) {
        let cols: Vec<_> = row.splitn(12, '\t').collect();
        if cols.len() != 12 {
            return None;
        }
        if cols[0] != "5" {
            continue;
        }
        let confidence: f32 = cols[10].parse().ok()?;
        let word = cols[11].trim();
        if !confidence.is_finite()
            || !(MIN_CONFIDENCE..=100.0).contains(&confidence)
            || word.is_empty()
            || word.chars().any(char::is_control)
        {
            return None;
        }
        let key = (cols[1], cols[2], cols[3], cols[4]);
        if line.is_some_and(|old| old != key) {
            return None;
        }
        line = Some(key);
        words.push(word);
    }
    let text = words.join(" ");
    (!text.is_empty() && text.len() <= MAX_TEXT).then_some(text)
}

fn number(text: &str, max: u32) -> Option<u32> {
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|v| *v <= max)
}
fn clock(text: &str, max: u32) -> Option<f32> {
    let (minutes, seconds) = text.split_once(':')?;
    if seconds.len() != 2 {
        return None;
    }
    let minutes = number(minutes, max / 60)?;
    let seconds = number(seconds, 59)?;
    let total = minutes.checked_mul(60)?.checked_add(seconds)?;
    (total <= max).then_some(total as f32)
}
fn explicit(text: &str, yes: &str, no: &str) -> Option<bool> {
    if text == yes {
        Some(true)
    } else if text == no {
        Some(false)
    } else {
        None
    }
}

fn map_cells(cells: BTreeMap<Field, String>) -> GameSnapshot {
    let mut result = GameSnapshot::default(); // Unknown provenance, context and eligibility
    let mut rows = BTreeMap::<u8, PlayerSample>::new();
    for (field, text) in cells {
        let text = text.as_str();
        match field {
            Field::Clock => result.playing_seconds = clock(text, limits::CLOCK_SECONDS as u32),
            Field::Mode => {
                result.game_mode = match text {
                    "Normal" => Some(GameMode::Normal),
                    "Street Brawl" => Some(GameMode::StreetBrawl),
                    _ => None,
                }
            }
            Field::Pause => result.paused = explicit(text, "Paused", "Playing"),
            Field::HeroName => {
                result.local_hero_name = limits::hero_name(text)
                    .filter(|s| {
                        s.chars()
                            .all(|c| c.is_alphabetic() || c == ' ' || c == '-' || c == '\'')
                    })
                    .map(str::to_owned)
            }
            Field::Row(row, col) => {
                let player = rows.entry(row).or_default();
                match col {
                    Column::Team => {
                        player.team = match text {
                            "Own" => Some(Team::Own),
                            "Enemy" => Some(Team::Enemy),
                            _ => None,
                        }
                    }
                    Column::Local => player.is_local = explicit(text, "Local", "Other"),
                    Column::Level => player.level = number(text, limits::LEVEL),
                    Column::Souls => player.souls = number(text, limits::SOULS),
                    Column::Kills => player.kills = number(text, limits::SUMMARY_SCORE),
                    Column::Deaths => player.deaths = number(text, limits::SUMMARY_SCORE),
                    Column::Assists => player.assists = number(text, limits::SUMMARY_SCORE),
                    Column::Alive => player.alive = explicit(text, "Alive", "Dead"),
                    Column::Health => player.health = number(text, 1_000_000).map(|v| v as i32),
                    Column::MaxHealth => {
                        player.max_health = number(text, 1_000_000).map(|v| v as i32)
                    }
                    Column::AbilityPoints => player.ability_points = number(text, limits::STAT),
                }
            }
            Field::Presence(kind) | Field::Count(kind) | Field::Countdown(kind) => {
                let slot = match kind {
                    Objective::Midboss => &mut result.timers.midboss,
                    Objective::Urn => &mut result.timers.urn,
                    Objective::Rift => &mut result.timers.rift,
                    Objective::BridgeBuffs => &mut result.timers.bridge_buffs,
                };
                let timer = slot.get_or_insert_with(ObjectiveTimer::default);
                match field {
                    Field::Presence(_) => timer.present = explicit(text, "Present", "Absent"),
                    Field::Count(_) => {
                        timer.count =
                            number(text, limits::OBJECTIVE_COUNT as u32).map(|v| v as usize)
                    }
                    Field::Countdown(_) => {
                        timer.next_spawn_in = clock(text, limits::TIMER_SECONDS as u32)
                    }
                    _ => unreachable!(),
                }
                // Unknown until the real UI semantics are independently validated.
            }
        }
    }
    let unique_local = rows.values().filter(|p| p.is_local == Some(true)).count() == 1;
    for (_, mut row) in rows {
        if !unique_local || row.is_local != Some(true) || row.team != Some(Team::Own) {
            row.health = None;
            row.max_health = None;
            row.ability_points = None;
        }
        if row.health.zip(row.max_health).is_some_and(|(h, m)| h > m) {
            row.health = None;
            row.max_health = None;
        }
        // Row index is NOT a proven game slot. No hero IDs, item IDs, match ID or
        // full-roster claim can be derived from these crops.
        result.players.push(row);
    }
    for slot in [
        &mut result.timers.midboss,
        &mut result.timers.urn,
        &mut result.timers.rift,
        &mut result.timers.bridge_buffs,
    ] {
        if let Some(timer) = slot {
            if timer.present == Some(true) {
                timer.next_spawn_in = None;
            }
            if timer.present.is_none() && timer.count.is_none() && timer.next_spawn_in.is_none() {
                *slot = None;
            }
        }
    }
    result
}

/// Each explicit crop set is consumed once. No reuse after error/EOF.
struct OfflineHudSource<R> {
    frame: Option<Vec<Crop>>,
    recognizer: R,
}
impl<R: Recognizer> OfflineHudSource<R> {
    fn new(frame: Vec<Crop>, recognizer: R) -> Self {
        Self {
            frame: Some(frame),
            recognizer,
        }
    }
}
impl<R: Recognizer> GameSource for OfflineHudSource<R> {
    fn poll(&mut self) -> Result<Option<GameSnapshot>, SourceError> {
        let Some(frame) = self.frame.take() else {
            return Ok(None);
        };
        if frame.is_empty() {
            return Ok(None);
        }
        if frame.len() > MAX_CROPS {
            return Err(SourceError::InvalidSample);
        }
        let mut seen = std::collections::BTreeSet::new();
        for crop in &frame {
            crop.validate()?;
            if !seen.insert(crop.field) {
                return Err(SourceError::InvalidSample);
            }
        }
        let mut cells = BTreeMap::new();
        for crop in &frame {
            if let Some(text) = self.recognizer.recognize(crop)? {
                if text.len() <= MAX_TEXT && !text.chars().any(char::is_control) {
                    cells.insert(crop.field, text);
                }
            }
        }
        if cells.is_empty() {
            return Ok(None);
        }
        Ok(Some(map_cells(cells)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Column::*;
    use Field::*;

    fn mock_crop(field: Field) -> Crop {
        Crop {
            field,
            width: 1,
            height: 1,
            pixels: vec![0],
        }
    }
    struct Mock(Option<&'static str>);
    impl Recognizer for Mock {
        fn recognize(&mut self, _: &Crop) -> Result<Option<String>, SourceError> {
            Ok(self.0.map(str::to_owned))
        }
    }
    fn cells(entries: &[(Field, &str)]) -> GameSnapshot {
        map_cells(entries.iter().map(|(f, t)| (*f, t.to_string())).collect())
    }

    #[test]
    fn hud_unknowns_and_explicit_zero() {
        let s = cells(&[
            (Clock, "03:07"),
            (Mode, "Street Brawl"),
            (Pause, "Paused"),
            (HeroName, "Infernus"),
            (Row(1, Kills), "0"),
            (Row(1, Deaths), "O"),
            (Row(1, Souls), "12k"),
            (Row(1, Level), "1001"),
            (Row(1, Health), "500"),
            (Presence(Objective::Urn), "Absent"),
            (Countdown(Objective::Urn), "00:45"),
        ]);
        assert_eq!(s.playing_seconds, Some(187.0));
        assert_eq!(s.game_mode, Some(GameMode::StreetBrawl));
        assert_eq!(s.paused, Some(true));
        assert_eq!(s.local_hero_name.as_deref(), Some("Infernus"));
        assert_eq!(s.players[0].kills, Some(0));
        assert_eq!(s.players[0].deaths, None);
        assert_eq!(s.players[0].souls, None);
        assert_eq!(s.players[0].level, None);
        assert_eq!(s.players[0].health, None);
        assert_eq!(s.players[0].hero_id, None);
        assert_eq!(s.players[0].slot, None);
        assert_eq!(s.timers.urn.unwrap().next_spawn_in, Some(45.0));
        assert_eq!(s.match_token, None);
        assert_eq!(s.expected_player_count, None);
        assert_eq!(s.provenance, Provenance::Unknown);
        assert_eq!(s.eligibility, Eligibility::Unknown);
        assert_eq!(s.context, MatchContext::Unknown);
    }

    #[test]
    fn hud_parser_rejects_ambiguous_values() {
        for invalid in ["1:60", "1:2", "1:02:03", "-1:00", "1440:01", "O1:00"] {
            assert_eq!(clock(invalid, 86400), None);
        }
        assert_eq!(clock("1440:00", 86400), Some(86400.0));
        assert_eq!(number("1,234", limits::SOULS), None);
        assert_eq!(number("4294967296", u32::MAX), None);
        let header = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";
        let word = |conf: &str, line: u32, value: &str| {
            format!("5\t1\t1\t1\t{line}\t1\t0\t0\t40\t20\t{conf}\t{value}\n")
        };
        assert_eq!(
            parse_tsv(&format!("{header}{}", word("94.0", 1, "0"))).as_deref(),
            Some("0")
        );
        assert_eq!(
            parse_tsv(&format!("{header}{}", word("84.9", 1, "9"))),
            None
        );
        assert_eq!(parse_tsv(&format!("{header}{}", word("NaN", 1, "9"))), None);
        assert_eq!(
            parse_tsv(&format!(
                "{header}{}{}",
                word("95", 1, "1"),
                word("95", 2, "2")
            )),
            None
        );
    }

    #[test]
    fn hud_invalid_frame_and_eof_never_replay() {
        let mut src = OfflineHudSource::new(vec![mock_crop(Clock)], Mock(Some("01:02")));
        let snapshot = src.poll().unwrap().unwrap();
        assert_eq!(snapshot.playing_seconds, Some(62.0));
        assert_eq!(
            crate::prepare(Some(&snapshot), &mut crate::Writer::new()),
            Err(crate::Status::Blocked)
        );
        assert_eq!(src.poll(), Ok(None));
        let mut src = OfflineHudSource::new(
            vec![mock_crop(Clock), mock_crop(Clock)],
            Mock(Some("01:02")),
        );
        assert_eq!(src.poll(), Err(SourceError::InvalidSample));
        assert_eq!(src.poll(), Ok(None));
        let mut src = OfflineHudSource::new(vec![mock_crop(Row(13, Kills))], Mock(Some("1")));
        assert_eq!(src.poll(), Err(SourceError::InvalidSample));
        let mut src = OfflineHudSource::new(
            vec![Crop {
                field: Clock,
                width: 2,
                height: 1,
                pixels: vec![0],
            }],
            Mock(Some("1")),
        );
        assert_eq!(src.poll(), Err(SourceError::InvalidSample));
        let mut src = OfflineHudSource::new(vec![mock_crop(Clock)], Mock(None));
        assert_eq!(src.poll(), Ok(None));
    }

    #[test]
    fn hud_local_only_and_conflicting_timer() {
        let s = cells(&[
            (Row(1, Team), "Own"),
            (Row(1, Local), "Local"),
            (Row(1, Health), "800"),
            (Row(1, MaxHealth), "1000"),
            (Row(1, AbilityPoints), "2"),
            (Row(2, Team), "Enemy"),
            (Row(2, Local), "Other"),
            (Row(2, Health), "900"),
            (Row(2, AbilityPoints), "3"),
            (Presence(Objective::Midboss), "Present"),
            (Countdown(Objective::Midboss), "00:42"),
            (Count(Objective::BridgeBuffs), "2"),
            (Presence(Objective::Rift), "unclear"),
        ]);
        assert_eq!(s.players[0].health, Some(800));
        assert_eq!(s.players[0].ability_points, Some(2));
        assert_eq!(s.players[1].health, None);
        assert_eq!(s.players[1].ability_points, None);
        assert_eq!(s.timers.midboss.unwrap().next_spawn_in, None);
        assert_eq!(s.timers.bridge_buffs.unwrap().count, Some(2));
        assert_eq!(s.timers.rift, None);
        let s = cells(&[
            (Row(1, Team), "Own"),
            (Row(1, Local), "Local"),
            (Row(1, Health), "10"),
            (Row(2, Local), "Local"),
        ]);
        assert_eq!(s.players[0].health, None);
    }

    // Actual raster OCR, not a mock. Requires local ImageMagick and Tesseract
    // with eng data. Never part of release runtime or ordinary CI dependencies.
    fn rendered_crop(field: Field, text: &str) -> Crop {
        let output = Command::new("convert")
            .args([
                "-size",
                "480x80",
                "xc:white",
                "-font",
                "DejaVu-Sans",
                "-pointsize",
                "36",
                "-fill",
                "black",
                "-gravity",
                "center",
                "-annotate",
                "0",
                text,
                "-depth",
                "8",
                "gray:-",
            ])
            .output()
            .expect("local ImageMagick");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 480 * 80);
        Crop {
            field,
            width: 480,
            height: 80,
            pixels: output.stdout,
        }
    }

    #[test]
    #[ignore = "requires local Tesseract eng and ImageMagick; offline synthetic pixels only"]
    fn hud_real_ocr_pixels() {
        let fields = [
            (Clock, "12:34"),
            (Mode, "Street Brawl"),
            (Pause, "Paused"),
            (HeroName, "Infernus"),
            (Row(1, Team), "Own"),
            (Row(1, Local), "Local"),
            (Row(1, Kills), "7"),
            (Row(1, Deaths), "2"),
            (Row(1, Assists), "11"),
            (Row(1, Souls), "12345"),
            (Row(1, Level), "15"),
            (Row(1, Alive), "Alive"),
            (Row(1, Health), "850"),
            (Row(1, MaxHealth), "1000"),
            (Row(1, AbilityPoints), "3"),
            (Row(2, Team), "Enemy"),
            (Row(2, Local), "Other"),
            (Row(2, Souls), "8765"),
            (Presence(Objective::Urn), "Absent"),
            (Countdown(Objective::Urn), "01:23"),
            (Count(Objective::BridgeBuffs), "2"),
        ];
        let frame = fields.iter().map(|(f, t)| rendered_crop(*f, t)).collect();
        let mut source = OfflineHudSource::new(frame, Tesseract);
        let s = source.poll().unwrap().unwrap();
        assert_eq!(s.playing_seconds, Some(754.0));
        assert_eq!(s.game_mode, Some(GameMode::StreetBrawl));
        assert_eq!(s.paused, Some(true));
        assert_eq!(s.local_hero_name.as_deref(), Some("Infernus"));
        assert_eq!(s.players.len(), 2);
        let p = &s.players[0];
        assert_eq!(
            (p.kills, p.deaths, p.assists, p.souls, p.level),
            (Some(7), Some(2), Some(11), Some(12345), Some(15))
        );
        assert_eq!(
            (p.alive, p.health, p.max_health, p.ability_points),
            (Some(true), Some(850), Some(1000), Some(3))
        );
        assert_eq!(s.players[1].souls, Some(8765));
        assert_eq!(s.timers.urn.as_ref().unwrap().present, Some(false));
        assert_eq!(s.timers.urn.as_ref().unwrap().next_spawn_in, Some(83.0));
        assert_eq!(s.timers.bridge_buffs.as_ref().unwrap().count, Some(2));
        assert_eq!(s.players[0].items, None);
        assert_eq!(s.players[0].ability_upgrades, None);
        assert_eq!(s.provenance, Provenance::Unknown);
        assert_eq!(
            crate::prepare(Some(&s), &mut crate::Writer::new()),
            Err(crate::Status::Blocked)
        );
        assert_eq!(source.poll(), Ok(None));
    }
}
