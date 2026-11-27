//! Collector library and fail-closed CLI runtime.
//! Public DTO and source interfaces do not authorize a live provider.
#[cfg(any(windows, test))]
mod desktop_runtime;
pub mod game_model;
pub mod game_source;
#[cfg(test)]
mod hud_source;
pub mod independent_source;
pub mod limits;
#[cfg(test)]
mod model_wire_tests;
mod secret;
#[cfg(windows)]
mod windows_client;
#[cfg(windows)]
pub use windows_client::run_windows;
pub mod telemetry;

use std::collections::HashSet;
use std::io::{self, Write};
use std::mem::discriminant;
use std::thread;
use std::time::{Duration, SystemTime};

use game_model::{Eligibility, GameSnapshot, MatchContext, Provenance};
use game_source::{GameSource, SourceError};
use independent_source::BlockedSource;
use reqwest::blocking::Client;
use serde::Serialize;

use telemetry::{Observation, Payload};

#[cfg(not(feature = "internal-test"))]
const ENDPOINT: &str = "https://deutsche-deadlock-community.de/twitch/api/v3/live-match";
#[cfg(feature = "internal-test")]
const ENDPOINT: &str = "https://localhost:8443/twitch/api/v3/live-match";
const PERIOD: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RETRY: Duration = Duration::from_secs(60);
const VERSION: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = concat!("live-match-collector/", env!("CARGO_PKG_VERSION"));
const KEY_REJECTED: &str = "Verbindungsschlüssel abgelehnt. Erstelle im Overlay-Baukasten einen neuen Schlüssel und starte das Programm neu.";

struct Writer {
    id: uuid::Uuid,
    sequence: u64,
    epoch: u64,
    current: Option<u64>,
    seen: HashSet<u64>,
    expired_epoch: Option<u64>,
    exhausted: bool,
}

impl Writer {
    fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            sequence: 0,
            epoch: 0,
            current: None,
            seen: HashSet::new(),
            expired_epoch: None,
            exhausted: false,
        }
    }

    fn observe(&mut self, id: u64) -> Result<(), Status> {
        if self.exhausted {
            return Err(Status::CounterExhausted);
        }
        if self.current == Some(id) {
            return if self.expired_epoch == Some(self.epoch) {
                Err(Status::EpochExpired)
            } else {
                Ok(())
            };
        }
        if self.seen.contains(&id) {
            return Err(Status::OldMatch);
        }
        if self.seen.len() >= limits::MATCH_HISTORY {
            self.exhausted = true;
            return Err(Status::CounterExhausted);
        }
        self.epoch = next_counter(self.epoch)?;
        self.seen.insert(id);
        self.current = Some(id);
        self.expired_epoch = None;
        Ok(())
    }

    fn expire_current(&mut self) {
        self.expired_epoch = Some(self.epoch);
    }

    fn packet(&mut self, observation: Observation) -> Result<Payload, Status> {
        if self.exhausted {
            return Err(Status::CounterExhausted);
        }
        if self.expired_epoch == Some(self.epoch) {
            return Err(Status::EpochExpired);
        }
        if self.current.is_none() {
            return Err(Status::Unreadable);
        }
        self.sequence = next_counter(self.sequence)?;
        Ok(Payload::new(
            self.id,
            self.sequence,
            self.epoch,
            observation,
        ))
    }
}

fn next_counter(value: u64) -> Result<u64, Status> {
    value
        .checked_add(1)
        .filter(|n| *n <= limits::MAX_COUNTER)
        .ok_or(Status::CounterExhausted)
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Waiting,
    Blocked,
    ReadFailed,
    Outdated,
    Excluded,
    Unreadable,
    EpochExpired,
    OldMatch,
    CounterExhausted,
    InvalidBody,
    BodyTooLarge,
    Sending,
    Rejected(u16),
    Unreachable,
}

impl Status {
    fn patience(&self) -> u8 {
        match self {
            Status::Sending
            | Status::InvalidBody
            | Status::BodyTooLarge
            | Status::EpochExpired
            | Status::OldMatch
            | Status::CounterExhausted => 1,
            Status::Rejected(_) | Status::Unreachable | Status::Blocked => 3,
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
            Status::Blocked => "Keine autorisierte Deadlock-Live-Datenquelle verfügbar. Es wird nichts übertragen.".to_owned(),
            Status::ReadFailed => "Deadlock läuft, aber die Spieldaten lassen sich gerade nicht lesen. Es wird weiter versucht. Bleibt das so, prüfe, ob es eine neuere Version des Collectors gibt.".to_owned(),
            Status::Outdated => "Deadlock hat sich seit dieser Programmversion verändert. Es wird nichts übertragen. Prüfe, ob es eine neuere Version des Collectors gibt.".to_owned(),
            Status::Excluded => "Für dieses Match lädt Deadlock selbst keine Statistiken hoch. Der Collector sendet deshalb nichts.".to_owned(),
            Status::Unreadable => "Match erkannt, aber die Spieldaten sind nicht vollständig lesbar. Solange wird nichts übertragen.".to_owned(),
            Status::InvalidBody => "Die Live-Daten konnten nicht als JSON erstellt werden. Es wird nichts gesendet.".to_owned(),
            Status::BodyTooLarge => format!("Die Live-Daten überschreiten die zulässige Größe von {} Bytes. Es wird nichts gesendet.", limits::BODY_BYTES),
            Status::EpochExpired => "Die Übertragung dieses Matches ist abgelaufen. Der Collector wartet mit demselben Schlüssel auf das nächste Match.".to_owned(),
            Status::OldMatch => "Ein früher erkanntes Match wurde erneut erkannt. Es wird nichts gesendet.".to_owned(),
            Status::CounterExhausted => "Der Übertragungszähler ist ausgeschöpft. Erstelle einen neuen Verbindungsschlüssel und starte den Collector neu.".to_owned(),
            Status::Sending => "Der Server hat Live-Daten bestätigt. Die Anzeige kann bei unveränderten Spieldaten trotzdem ausblenden.".to_owned(),
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

fn prepare(snapshot: Option<&GameSnapshot>, writer: &mut Writer) -> Result<Observation, Status> {
    let Some(s) = snapshot else {
        return Err(Status::Waiting);
    };
    if s.provenance != Provenance::Authorized
        && !(cfg!(test) && s.provenance == Provenance::Synthetic)
    {
        return Err(Status::Blocked);
    }
    if s.context != MatchContext::LivePlaying {
        return Err(Status::Waiting);
    }
    let id = s
        .match_token
        .filter(|id| *id > 0)
        .ok_or(Status::Unreadable)?;
    writer.observe(id)?;
    match s.eligibility {
        Eligibility::Eligible => {}
        Eligibility::Excluded => return Err(Status::Excluded),
        Eligibility::Unknown => return Err(Status::Unreadable),
    }
    telemetry::observation(s).ok_or(Status::Unreadable)
}

fn source_status(error: SourceError) -> Status {
    match error {
        SourceError::Unavailable => Status::ReadFailed,
        SourceError::Unauthorized => Status::Blocked,
        SourceError::Incompatible => Status::Outdated,
        SourceError::InvalidSample => Status::Unreadable,
    }
}

fn read(
    source: &mut (impl GameSource + ?Sized),
    writer: &mut Writer,
) -> Result<Observation, Status> {
    let sample = source.poll().map_err(source_status)?;
    prepare(sample.as_ref(), writer)
}

fn finish(code: i32) -> ! {
    std::process::exit(code)
}

fn leave(message: &str, code: i32) -> ! {
    let _ = writeln!(io::stderr(), "{message}");
    finish(code)
}

fn guarded_run(run: impl FnOnce() + std::panic::UnwindSafe) {
    std::panic::set_hook(Box::new(|_| {
        let _ = writeln!(
            io::stderr(),
            "Unerwarteter Programmfehler (Collector {VERSION}). Bitte melde die Version und den Fehler."
        );
    }));
    if let Err(payload) = std::panic::catch_unwind(run) {
        std::mem::forget(payload);
        finish(1);
    }
}

#[derive(Debug, PartialEq)]
enum FatalUpload {
    KeyRejected,
    Conflict,
    ObsoleteApi,
}

impl FatalUpload {
    fn message(&self) -> &'static str {
        match self {
            Self::KeyRejected => KEY_REJECTED,
            Self::Conflict => "Die Übertragung steht im Konflikt mit einer früheren Sitzung oder Match-Reihenfolge. Beende andere Collector-Instanzen, erstelle einen neuen Verbindungsschlüssel und starte neu.",
            Self::ObsoleteApi => "Die Collector-API-Version wird nicht unterstützt. Prüfe, ob es eine passende Collector-Version gibt.",
        }
    }
}

fn outcome(code: Option<u16>) -> Result<Status, FatalUpload> {
    match code {
        Some(204) => Ok(Status::Sending),
        Some(401) => Err(FatalUpload::KeyRejected),
        Some(409) => Err(FatalUpload::Conflict),
        Some(410) => Err(FatalUpload::ObsoleteApi),
        Some(code) => Ok(Status::Rejected(code)),
        None => Ok(Status::Unreachable),
    }
}

fn encode_body(data: &impl Serialize) -> Result<Vec<u8>, Status> {
    struct BoundedBody {
        bytes: Vec<u8>,
        exceeded: bool,
    }
    impl Write for BoundedBody {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.len() > limits::BODY_BYTES - self.bytes.len() {
                self.exceeded = true;
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut body = BoundedBody {
        bytes: Vec::with_capacity(limits::BODY_BYTES),
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut body, data);
    if body.exceeded {
        return Err(Status::BodyTooLarge);
    }
    result.map_err(|_| Status::InvalidBody)?;
    Ok(body.bytes)
}

fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    let delay = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse().unwrap_or(u64::MAX))
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .unwrap_or_default()
    };
    Some(delay.min(MAX_RETRY))
}

fn upload(
    client: &Client,
    endpoint: &str,
    token: &str,
    data: &impl Serialize,
) -> Result<(Status, Option<Duration>), FatalUpload> {
    let body = match encode_body(data) {
        Ok(body) => body,
        Err(status) => return Ok((status, None)),
    };
    let sent = client
        .post(endpoint)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send();
    match sent {
        Ok(response) => {
            let status = if response.status().as_u16() == 409
                && response
                    .headers()
                    .get_all("live-match-conflict")
                    .iter()
                    .count()
                    == 1
                && response
                    .headers()
                    .get("live-match-conflict")
                    .is_some_and(|value| value.as_bytes() == b"epoch-expired")
            {
                Status::EpochExpired
            } else {
                outcome(Some(response.status().as_u16()))?
            };
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| retry_after(value, SystemTime::now()));
            Ok((status, delay))
        }
        Err(_) => Ok((Status::Unreachable, None)),
    }
}

struct RetryPacing {
    delay: Duration,
}

impl RetryPacing {
    fn new() -> Self {
        Self { delay: PERIOD }
    }

    fn after(&mut self, status: &Status, hint: Option<Duration>) -> Duration {
        if status.failing() {
            self.delay = (self.delay * 2).min(MAX_RETRY);
            self.delay.max(hint.unwrap_or_default()).min(MAX_RETRY)
        } else {
            if *status == Status::Sending {
                self.delay = PERIOD;
            }
            PERIOD
        }
    }
}

fn client_builder() -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
}

/// Start the collector CLI; exits before key input when the source is blocked.
pub fn run_cli() {
    guarded_run(run);
}

fn run() {
    eprintln!("Deadlock Live Match Collector {VERSION}");
    if std::env::args_os().len() != 1 {
        leave(
            "Dieses Programm akzeptiert keine Befehlszeilenargumente.",
            2,
        );
    }
    // No source can be opened until the authorization and field-coverage gate is met.
    // Fail before asking for a key, creating an HTTP client or contacting any server.
    let mut source = match BlockedSource::open() {
        Ok(source) => source,
        Err(error) => leave(&source_status(error).message(), 1),
    };
    #[cfg(feature = "internal-test")]
    eprintln!("Interner Test: ausschließlich HTTPS über localhost:8443.");
    let builder = client_builder().https_only(true);
    #[cfg(feature = "internal-test")]
    let builder = {
        let certificate = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|dir| dir.join("test-ca.pem")))
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| reqwest::Certificate::from_pem(&bytes).ok())
            .unwrap_or_else(|| {
                leave(
                    "Interner Test: gültige test-ca.pem neben der EXE erforderlich.",
                    1,
                )
            });
        builder
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate)
    };
    let client = match builder.build() {
        Ok(client) => client,
        Err(_) => leave(
            "Die sichere Verbindung konnte nicht eingerichtet werden.",
            1,
        ),
    };
    let token = match secret::read_token() {
        Ok(token) => token,
        Err(reason) => leave(reason, 1),
    };
    let mut announcer = Announcer::new();
    let mut pacing = RetryPacing::new();
    let mut writer = Writer::new();
    eprintln!("Warte auf ein laufendes Deadlock-Match. Beenden mit Strg+C.");
    loop {
        let (status, hint) = match read(&mut source, &mut writer) {
            Ok(observation) => match writer.packet(observation) {
                Ok(data) => match upload(&client, ENDPOINT, &token, &data) {
                    Ok((Status::EpochExpired, hint)) => {
                        writer.expire_current();
                        (Status::EpochExpired, hint)
                    }
                    Ok(result) => result,
                    Err(fatal) => {
                        drop(token);
                        leave(fatal.message(), 1)
                    }
                },
                Err(status) => {
                    drop(token);
                    leave(&status.message(), 1)
                }
            },
            Err(Status::CounterExhausted) => {
                drop(token);
                leave(&Status::CounterExhausted.message(), 1)
            }
            Err(status) => (status, None),
        };
        let delay = pacing.after(&status, hint);
        if let Some(message) = announcer.observe(status) {
            eprintln!("{message}");
        }
        thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_model::{
        AbilityUpgrade, GameMode, ObjectiveTimer, PlayerSample, StructureKind, StructureSample,
        StructureState, Team, TeamSample, TimerSource,
    };
    use game_source::SyntheticSource;
    use serde_json::{json, Value};

    #[test]
    fn source_poll_uses_observed_clock_and_never_creates_progress() {
        let mut s = snapshot();
        s.paused = Some(false);
        let mut paused = s.clone();
        paused.paused = Some(true);
        let mut source =
            SyntheticSource::new(vec![Ok(Some(s.clone())), Ok(Some(paused)), Ok(Some(s))]);
        for expected_pause in [false, true, false] {
            let sample = source.poll().unwrap().unwrap();
            let b = serde_json::to_value(telemetry::Payload::new(
                uuid::Uuid::new_v4(),
                1,
                1,
                telemetry::observation(&sample).unwrap(),
            ))
            .unwrap();
            assert_eq!(b["game_seconds"], 42);
            assert_eq!(b["paused"], expected_pause);
        }
        assert_eq!(source.poll().unwrap(), None);
    }

    fn snapshot() -> GameSnapshot {
        GameSnapshot {
            match_token: Some(123),
            expected_player_count: Some(2),
            context: MatchContext::LivePlaying,
            eligibility: Eligibility::Eligible,
            provenance: Provenance::Synthetic,
            game_mode: Some(GameMode::Normal),
            playing_seconds: Some(42.8),
            local_hero_name: Some("Infernus".to_owned()),
            players: vec![
                PlayerSample {
                    is_local: Some(true),
                    team: Some(Team::Own),
                    hero_id: Some(1),
                    ..Default::default()
                },
                PlayerSample {
                    team: Some(Team::Enemy),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn observation(s: &GameSnapshot) -> Result<Observation, Status> {
        prepare(Some(s), &mut Writer::new())
    }

    fn timer(source: TimerSource, present: bool, seconds: f32) -> ObjectiveTimer {
        ObjectiveTimer {
            present: Some(present),
            count: Some(usize::from(present)),
            next_spawn_in: Some(seconds),
            source,
        }
    }

    fn packet(s: &GameSnapshot, writer: &mut Writer) -> Value {
        let sample = prepare(Some(s), writer).unwrap();
        let data = writer.packet(sample).unwrap();
        serde_json::from_slice(&encode_body(&data).unwrap()).unwrap()
    }

    #[test]
    fn v3_extended_payload_preserves_game_values_and_excludes_private_fields() {
        let mut writer = Writer::new();
        writer.id = uuid::Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let mut s = snapshot();
        let first = packet(&s, &mut writer);
        assert_eq!(first["schema_version"], 3);
        assert_eq!(first["hero"], "Infernus");
        assert_eq!(first["game_seconds"], 42);
        assert_eq!(first["game_mode"], "normal");
        for key in ["kills", "deaths", "assists", "team_souls", "enemy_souls"] {
            assert!(first[key].is_null());
        }
        s.match_token = Some(987654321);
        s.players[0].kills = Some(990);
        s.players[0].deaths = Some(991);
        s.players[0].assists = Some(992);
        s.players[0].souls = Some(999_999);
        s.players[0].items = Some(vec![123456789]);
        s.players[0].hero_damage = Some(1234);
        s.players[1].hero_id = Some(2);
        s.players[1].slot = Some(7);
        s.players[1].level = Some(42);
        s.players[1].alive = Some(true);
        s.players[1].souls = Some(888_888);
        s.players[1].hero_damage = Some(123456789);
        s.players[1].items = Some(vec![987654321]);
        s.paused = Some(false);
        s.timers.midboss = Some(timer(TimerSource::Game, false, 90.0));
        let body = packet(&s, &mut writer);
        assert_eq!(body["sequence"], 2);
        assert_eq!(body["match_epoch"], 2);
        assert_eq!(body["kills"], 990);
        assert_eq!(body["deaths"], 991);
        assert_eq!(body["assists"], 992);
        assert_eq!(body["team_souls"], 999999);
        assert_eq!(body["enemy_souls"], 888888);
        assert_eq!(body["details"]["schema_version"], 1);
        assert_eq!(body["details"]["own_loadout"]["items"], json!([123456789]));
        assert_eq!(body["details"]["scoreboard"][0]["hero_damage"], 1234);
        assert_eq!(body["details"]["scoreboard"][1]["hero_id"], 2);
        assert_eq!(body["details"]["scoreboard"][1]["level"], 42);
        assert!(body["details"]["scoreboard"][1]["hero_damage"].is_null());
        assert_eq!(
            body["details"]["objectives"]["midboss"]["next_spawn_in_seconds"],
            90
        );
        let serialized = body.to_string();
        for key in [
            "match_id",
            "raw_update_tick",
            "steam_id",
            "name",
            "position",
            "controller",
            "pawn",
            "received_at",
            "ultimate_cooldown",
        ] {
            assert!(
                !serialized.contains(&format!("\"{key}\"")),
                "unexpected {key}"
            );
        }
        assert!(!serialized.contains("987654321"));
    }

    #[test]
    fn historical_schema2_vectors_match_independent_contract_checks() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/live-match-api-v3-vectors.json"
        ))
        .unwrap();
        let schema: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/live-match-api-v3.schema.json"
        ))
        .unwrap();
        let cases = vectors["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 22);
        for case in cases {
            let body = &case["body"];
            assert_eq!(
                vector_status(body, &schema),
                case["status"].as_u64().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    fn vector_status(body: &Value, schema: &Value) -> u64 {
        let Some(object) = body.as_object() else {
            return 422;
        };
        let properties = schema["properties"].as_object().unwrap();
        if object.keys().any(|key| !properties.contains_key(key))
            || schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|key| !object.contains_key(key.as_str().unwrap()))
        {
            return 422;
        }
        for key in ["schema_version", "sequence", "match_epoch", "game_seconds"] {
            if body[key].as_u64().is_none() {
                return 422;
            }
        }
        let (Some(hero), Some(writer)) = (body["hero"].as_str(), body["writer_id"].as_str()) else {
            return 422;
        };
        if !body["paused"].is_null() && !body["paused"].is_boolean() {
            return 422;
        }
        let boss = &body["midboss"];
        if !boss.is_null() {
            let Some(object) = boss.as_object() else {
                return 422;
            };
            if object
                .keys()
                .any(|key| !["present", "source", "next_spawn_in_seconds"].contains(&key.as_str()))
                || !boss["present"].is_boolean()
                || !boss["source"].is_string()
                || (!boss["next_spawn_in_seconds"].is_null()
                    && boss["next_spawn_in_seconds"].as_u64().is_none())
            {
                return 422;
            }
        }
        let canonical = uuid::Uuid::parse_str(writer).ok().is_some_and(|id| {
            id.get_version_num() == 4
                && id.get_variant() == uuid::Variant::RFC4122
                && id.to_string() == writer
        });
        let bounded = |key: &str| {
            let n = body[key].as_u64().unwrap();
            (properties[key]["minimum"].as_u64().unwrap()
                ..=properties[key]["maximum"].as_u64().unwrap())
                .contains(&n)
        };
        if body["schema_version"] != properties["schema_version"]["const"]
            || !canonical
            || !bounded("sequence")
            || !bounded("match_epoch")
            || !bounded("game_seconds")
            || !(1..=80).contains(&hero.len())
            || hero.trim() != hero
            || hero.chars().any(char::is_control)
        {
            return 400;
        }
        if !boss.is_null()
            && (boss["source"] != "game"
                || boss["next_spawn_in_seconds"]
                    .as_u64()
                    .is_some_and(|n| n > 7200 || boss["present"] == true))
        {
            return 400;
        }
        204
    }

    #[test]
    fn expired_epoch_waits_for_genuine_new_match_without_rotation() {
        let mut writer = Writer::new();
        let mut s = snapshot();
        let first = packet(&s, &mut writer);
        writer.expire_current();
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::EpochExpired));
        s.playing_seconds = Some(89.0);
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::EpochExpired));
        assert_eq!(writer.sequence, 1);
        s.match_token = Some(456);
        let next = packet(&s, &mut writer);
        assert_eq!(next["writer_id"], first["writer_id"]);
        assert_eq!(next["sequence"], 2);
        assert_eq!(next["match_epoch"], 2);
        s.match_token = Some(123);
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::OldMatch));
    }

    #[test]
    fn expanded_maximum_payload_and_unknown_team_values() {
        let mut s = snapshot();
        s.expected_player_count = Some(12);
        let base = s.players[1].clone();
        while s.players.len() < 12 {
            s.players.push(base.clone());
        }
        for (i, row) in s.players.iter_mut().enumerate() {
            row.team = Some(if i < 6 { Team::Own } else { Team::Enemy });
            row.slot = Some((i + 1) as u32);
            row.hero_id = Some(if i == 0 { 1 } else { u32::MAX });
            row.level = Some(limits::LEVEL);
            row.souls = Some(limits::SOULS / 6);
            row.kills = Some(limits::SUMMARY_SCORE);
            row.deaths = Some(limits::SUMMARY_SCORE);
            row.assists = Some(limits::SUMMARY_SCORE);
            row.last_hits = Some(limits::STAT);
            row.denies = Some(limits::STAT);
            row.hero_damage = Some(limits::STAT);
            row.objective_damage = Some(limits::STAT);
            row.healing = Some(limits::STAT);
            row.alive = Some(true);
        }
        let local = &mut s.players[0];
        local.items = Some(vec![u32::MAX; limits::ITEMS]);
        local.ability_upgrades = Some(
            (0..limits::ABILITY_UPGRADES as u32)
                .map(|i| AbilityUpgrade {
                    item_id: u32::MAX - i,
                    unlocked: true,
                    points: limits::ABILITY_POINTS,
                })
                .collect(),
        );
        local.health = Some(1_000_000);
        local.max_health = Some(1_000_000);
        local.ability_points = Some(limits::STAT);
        local.creep_souls = Some(limits::STAT);
        local.secured_souls = Some(limits::STAT);
        local.denied_souls = Some(limits::STAT);
        local.neutral_souls = Some(limits::STAT);
        local.farm_baseline = Some(limits::STAT);
        local.kill_streak = Some(limits::STAT);
        s.teams = [Team::Own, Team::Enemy]
            .into_iter()
            .map(|team| TeamSample {
                team,
                complete: true,
                structures_alive: Some(128),
                ability_points: Some(limits::STAT),
                last_hits: Some(limits::STAT),
                denies: Some(limits::STAT),
                hero_damage: Some(limits::STAT),
                objective_damage: Some(limits::STAT),
                healing: Some(limits::STAT),
                rejuvenators: Some(1000),
            })
            .collect();
        s.structures = (0..limits::STRUCTURE_ROWS)
            .map(|_| StructureSample {
                kind: StructureKind::Guardian,
                team: Some(Team::Own),
                lane: Some(8),
                state: StructureState::Standing,
            })
            .collect();
        s.timers.midboss = Some(timer(TimerSource::Game, false, 7200.0));
        s.timers.urn = Some(timer(TimerSource::Game, false, 7200.0));
        s.timers.rift = Some(timer(TimerSource::Game, false, 7200.0));
        s.timers.bridge_buffs = Some(timer(TimerSource::Schedule, false, 7200.0));
        let mut writer = Writer::new();
        writer.id = uuid::Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let data = next_test_payload(&mut writer, &s);
        let bytes = encode_body(&data).unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let expected = bytes.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let actual = read_mock_post(&mut stream);
            assert_eq!(actual, expected);
            if std::env::var_os("COLLECTOR_TEST_EMIT_SYNTHETIC").is_some() {
                println!(
                    "COLLECTOR_SYNTHETIC_BODY={}",
                    String::from_utf8(actual).unwrap()
                );
            }
            write!(
                stream,
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let client = client_builder().build().unwrap();
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &data),
            Ok((Status::Sending, None))
        );
        server.join().unwrap();
        assert_eq!(body["details"]["scoreboard"].as_array().unwrap().len(), 12);
        assert_eq!(body["details"]["teams"].as_array().unwrap().len(), 2);
        assert_eq!(body["details"]["structures"].as_array().unwrap().len(), 48);
        assert_eq!(
            body["details"]["own_loadout"]["items"]
                .as_array()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            body["details"]["own_loadout"]["ability_upgrades"]
                .as_array()
                .unwrap()
                .len(),
            16
        );
        assert_eq!(
            body["details"]["objectives"]["bridge_buffs"]["estimated_spawn_in_seconds"],
            7200
        );
        let size = bytes.len();
        eprintln!("Extended synthetic maximal shape: {size} bytes");
        assert!(size < limits::BODY_BYTES);
        s.players[1].souls = None;
        s.teams[0].complete = false;
        let partial = packet(&s, &mut writer);
        assert!(partial["team_souls"].is_null());
        assert!(partial["details"]["teams"][0]["souls"].is_null());
        assert!(partial["details"]["teams"][0]["hero_damage"].is_null());
        assert_eq!(partial["details"]["teams"][0]["kills"], 6000);
    }

    #[test]
    fn missing_or_invalid_hero_name_cannot_create_upload() {
        let mut writer = Writer::new();
        let mut s = snapshot();
        for name in [
            None,
            Some(String::new()),
            Some("   ".into()),
            Some("x\ny".into()),
            Some("ü".repeat(41)),
        ] {
            s.local_hero_name = name;
            assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Unreadable));
            assert_eq!(writer.sequence, 0);
        }
        s.local_hero_name = Some(" Infernus ".into());
        let body = packet(&s, &mut writer);
        assert_eq!(body["hero"], "Infernus");
        assert_eq!(body["sequence"], 1);
        assert_eq!(body["match_epoch"], 1);
    }

    #[test]
    fn local_eligibility_and_roster_still_fail_closed_without_scores() {
        assert!(observation(&snapshot()).is_ok());
        let mut s = snapshot();
        s.context = MatchContext::Other;
        assert_eq!(observation(&s), Err(Status::Waiting));
        let mut s = snapshot();
        s.players[0].is_local = Some(false);
        assert_eq!(observation(&s), Err(Status::Unreadable));
        let mut s = snapshot();
        s.players[1].is_local = Some(true);
        assert_eq!(observation(&s), Err(Status::Unreadable));
        let mut s = snapshot();
        s.players[0].is_spectator = true;
        assert_eq!(observation(&s), Err(Status::Unreadable));
        let mut s = snapshot();
        s.match_token = Some(0);
        assert_eq!(observation(&s), Err(Status::Unreadable));
        let mut s = snapshot();
        for expected in [None, Some(0), Some(12), Some(u32::MAX)] {
            s.expected_player_count = expected;
            assert_eq!(observation(&s), Err(Status::Unreadable));
        }
        for invalid in [None, Some(Team::Unknown), Some(Team::Own)] {
            let mut s = snapshot();
            s.players[1].team = invalid;
            assert_eq!(observation(&s), Err(Status::Unreadable));
        }
        let mut s = snapshot();
        s.players[1].is_spectator = true;
        assert_eq!(observation(&s), Err(Status::Unreadable));
        s.players[1].is_spectator = false;
        assert!(observation(&s).is_ok());
        for hero in [None, Some(0)] {
            s.players[0].hero_id = hero;
            assert_eq!(observation(&s), Err(Status::Unreadable));
        }
        let mut s = snapshot();
        for now in [None, Some(f32::NAN), Some(f32::INFINITY), Some(86_401.0)] {
            s.playing_seconds = now;
            assert_eq!(observation(&s), Err(Status::Unreadable));
        }
    }

    #[test]
    fn pause_and_midboss_are_only_observed_and_never_fabricate_freshness() {
        let mut s = snapshot();
        let mut writer = Writer::new();
        s.paused = Some(false);
        s.timers.midboss = Some(timer(TimerSource::Game, false, 41.2));
        let first = packet(&s, &mut writer);
        assert_eq!(
            first["midboss"],
            json!({"present":false,"source":"game","next_spawn_in_seconds":42})
        );
        s.paused = Some(true);
        let paused = packet(&s, &mut writer);
        s.playing_seconds = Some(42.8);
        let unchanged = packet(&s, &mut writer);
        assert_eq!(paused["game_seconds"], unchanged["game_seconds"]);
        assert_eq!(paused["paused"], unchanged["paused"]);
        assert_eq!(unchanged["sequence"], 3);
        assert!(unchanged.get("raw_update_tick").is_none());
        s.paused = None;
        assert!(packet(&s, &mut writer).get("paused").is_none());
        s.paused = Some(false);
        s.timers.midboss = Some(timer(TimerSource::Game, true, 20.0));
        let resumed = packet(&s, &mut writer);
        assert_eq!(resumed["paused"], false);
        assert_eq!(resumed["midboss"], json!({"present":true,"source":"game"}));
        for source in [TimerSource::Schedule, TimerSource::Unknown] {
            s.timers.midboss = Some(timer(source, false, 90.0));
            assert!(packet(&s, &mut writer).get("midboss").is_none());
        }
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 7200.5] {
            s.timers.midboss = Some(timer(TimerSource::Game, false, invalid));
            assert_eq!(
                packet(&s, &mut writer)["midboss"],
                json!({"present":false,"source":"game"})
            );
        }
        s.timers.midboss = Some(timer(TimerSource::Game, false, 7200.0));
        assert_eq!(
            packet(&s, &mut writer)["midboss"]["next_spawn_in_seconds"],
            7200
        );
        s.timers.midboss.as_mut().unwrap().present = None;
        assert!(packet(&s, &mut writer).get("midboss").is_none());
        s.timers.midboss = Some(timer(TimerSource::Game, false, 90.0));
        for mode in [None, Some(GameMode::StreetBrawl)] {
            s.game_mode = mode;
            assert!(packet(&s, &mut writer).get("midboss").is_none());
        }
    }

    #[test]
    fn writer_epoch_sequence_survive_detach_timeout_and_reject_old_match() {
        let mut writer = Writer::new();
        let mut s = snapshot();
        let first = packet(&s, &mut writer);
        assert_eq!(first["sequence"], 1);
        assert_eq!(first["match_epoch"], 1);
        assert_eq!(prepare(None, &mut writer), Err(Status::Waiting));
        let next = packet(&s, &mut writer);
        assert_eq!(next["sequence"], 2);
        assert_eq!(next["match_epoch"], 1);
        assert_eq!(next["writer_id"], first["writer_id"]);
        s.match_token = Some(456);
        s.players[0].hero_id = None;
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Unreadable));
        s.players[0].hero_id = Some(1);
        let new = packet(&s, &mut writer);
        assert_eq!(new["sequence"], 3);
        assert_eq!(new["match_epoch"], 2);
        s.match_token = Some(123);
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::OldMatch));
        assert_eq!(writer.sequence, 3);
        s.match_token = Some(456);
        let latest = packet(&s, &mut writer);
        assert_eq!(latest["sequence"], 4);
        assert_eq!(latest["match_epoch"], 2);
        let mut restart = Writer::new();
        let restarted = packet(&s, &mut restart);
        assert_ne!(restarted["writer_id"], first["writer_id"]);
        assert_eq!(restarted["sequence"], 1);
        assert_eq!(restarted["match_epoch"], 1);
        for id in [writer.id, restart.id] {
            assert_eq!(id.get_version_num(), 4);
            assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
            assert_eq!(id.to_string(), id.to_string().to_ascii_lowercase());
        }
        writer.sequence = limits::MAX_COUNTER - 1;
        assert_eq!(packet(&s, &mut writer)["sequence"], limits::MAX_COUNTER);
        let sample = prepare(Some(&s), &mut writer).unwrap();
        assert_eq!(writer.packet(sample), Err(Status::CounterExhausted));
        writer.epoch = limits::MAX_COUNTER;
        s.match_token = Some(789);
        assert_eq!(
            prepare(Some(&s), &mut writer),
            Err(Status::CounterExhausted)
        );
        assert_eq!(writer.epoch, limits::MAX_COUNTER);
    }

    #[test]
    fn writer_history_exhaustion_is_permanent_and_never_evicts() {
        let mut writer = Writer::new();
        for id in 1..=limits::MATCH_HISTORY as u64 {
            assert_eq!(writer.observe(id), Ok(()));
        }
        assert_eq!(writer.observe(1), Err(Status::OldMatch));
        assert_eq!(writer.observe(limits::MATCH_HISTORY as u64), Ok(()));
        assert_eq!(
            writer.observe(limits::MATCH_HISTORY as u64 + 1),
            Err(Status::CounterExhausted)
        );
        assert_eq!(writer.seen.len(), limits::MATCH_HISTORY);
        assert_eq!(writer.observe(1), Err(Status::CounterExhausted));
        assert_eq!(
            writer.observe(limits::MATCH_HISTORY as u64),
            Err(Status::CounterExhausted)
        );
        assert!(matches!(
            writer.packet(observation(&snapshot()).unwrap()),
            Err(Status::CounterExhausted)
        ));
    }

    #[test]
    fn raw_snapshot_bounds_and_known_duplicates_fail_closed() {
        let mut s = snapshot();
        s.players
            .resize(limits::SCOREBOARD_ROWS + 1, PlayerSample::default());
        assert_eq!(observation(&s), Err(Status::Unreadable));
        let mut s = snapshot();
        s.teams.resize(limits::TEAM_ROWS + 1, TeamSample::default());
        s.structures
            .resize(limits::STRUCTURE_ROWS + 1, StructureSample::default());
        s.players[0].items = Some(vec![0; limits::ITEMS + 1]);
        s.players[0].ability_upgrades = Some(vec![
            AbilityUpgrade::default();
            limits::ABILITY_UPGRADES + 1
        ]);
        let b = packet(&s, &mut Writer::new());
        assert_eq!(b["details"]["teams"], json!([]));
        assert_eq!(b["details"]["structures"], json!([]));
        assert!(b["details"]["own_loadout"]["items"].is_null());
        assert!(b["details"]["own_loadout"]["ability_upgrades"].is_null());
        let mut s = snapshot();
        s.teams = vec![
            TeamSample {
                team: Team::Own,
                ..Default::default()
            };
            2
        ];
        assert_eq!(
            packet(&s, &mut Writer::new())["details"]["teams"],
            json!([])
        );
        s.players[0].slot = Some(1);
        s.players[1].slot = Some(1);
        assert!(observation(&s).is_ok());
        s.players.push(PlayerSample {
            team: Some(Team::Own),
            slot: Some(1),
            ..Default::default()
        });
        s.expected_player_count = Some(3);
        assert_eq!(observation(&s), Err(Status::Unreadable));
    }

    #[test]
    fn duplicate_ability_ids_omit_ambiguous_optional_group() {
        let mut s = snapshot();
        s.players[0].ability_upgrades = Some(vec![
            AbilityUpgrade {
                item_id: 1,
                unlocked: true,
                points: 1
            };
            2
        ]);
        let data = packet(&s, &mut Writer::new());
        assert!(data["details"]["own_loadout"]["ability_upgrades"].is_null());
    }

    #[test]
    fn proxy_environment_is_ignored_by_shared_client_builder() {
        const CHILD: &str = "COLLECTOR_PROXY_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::proxy_environment_is_ignored_by_shared_client_builder",
                ])
                .env(CHILD, "1")
                .env("HTTP_PROXY", "http://127.0.0.1:1")
                .env("http_proxy", "http://127.0.0.1:1")
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .env("https_proxy", "http://127.0.0.1:1")
                .env("ALL_PROXY", "http://127.0.0.1:1")
                .env("all_proxy", "http://127.0.0.1:1")
                .env_remove("NO_PROXY")
                .env_remove("no_proxy")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let client = client_builder().build().unwrap();
        let sender = thread::spawn(move || {
            upload(
                &client,
                &endpoint,
                "synthetic-test-only",
                &serde_json::json!({"test": true}),
            )
        });
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("direct connection missing: {e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _ = read_mock_post(&mut stream);
        write!(
            stream,
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        assert_eq!(sender.join().unwrap(), Ok((Status::Sending, None)));
    }

    #[test]
    fn panic_guard_exits_without_disclosing_payload_or_waiting_on_piped_input() {
        const CHILD: &str = "COLLECTOR_TEST_PANIC_CHILD";
        if std::env::var_os(CHILD).is_some() {
            guarded_run(|| panic!("synthetic confidential panic payload"));
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::panic_guard_exits_without_disclosing_payload_or_waiting_on_piped_input",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(&format!("Collector {VERSION}")));
        assert!(!stderr.contains("synthetic confidential panic payload"));
        assert!(!stderr.contains("Drücke die Eingabetaste"));
    }

    #[test]
    fn retry_backoff_is_bounded_and_resets_only_on_success() {
        let mut pacing = RetryPacing::new();
        for (status, seconds) in [
            (Status::Rejected(404), 6),
            (Status::Rejected(429), 12),
            (Status::Rejected(503), 24),
            (Status::Unreachable, 48),
            (Status::Rejected(500), 60),
            (Status::Unreachable, 60),
        ] {
            assert_eq!(pacing.after(&status, None), Duration::from_secs(seconds));
        }
        assert_eq!(pacing.after(&Status::Waiting, None), PERIOD);
        assert_eq!(pacing.after(&Status::Unreachable, None), MAX_RETRY);
        assert_eq!(pacing.after(&Status::Sending, Some(MAX_RETRY)), PERIOD);
        assert_eq!(
            pacing.after(&Status::Rejected(429), Some(Duration::from_secs(999))),
            MAX_RETRY
        );
        assert_eq!(
            pacing.after(&Status::Rejected(429), Some(Duration::from_secs(1))),
            Duration::from_secs(12)
        );
        assert_eq!(pacing.after(&Status::Sending, None), PERIOD);
        assert_eq!(
            pacing.after(&Status::Unreachable, None),
            Duration::from_secs(6)
        );
    }

    #[test]
    fn retry_after_supports_seconds_dates_and_invalid_values() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(retry_after("0", now), Some(Duration::ZERO));
        assert_eq!(retry_after(" 21 ", now), Some(Duration::from_secs(21)));
        assert_eq!(
            retry_after("9999999999999999999999999", now),
            Some(MAX_RETRY)
        );
        assert_eq!(
            retry_after(&httpdate::fmt_http_date(now + Duration::from_secs(15)), now),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            retry_after(
                &httpdate::fmt_http_date(now + Duration::from_secs(600)),
                now
            ),
            Some(MAX_RETRY)
        );
        assert_eq!(
            retry_after(&httpdate::fmt_http_date(now - Duration::from_secs(15)), now),
            Some(Duration::ZERO)
        );
        for invalid in ["", "-1", "tomorrow", "1.5"] {
            assert_eq!(retry_after(invalid, now), None);
        }
    }

    fn read_mock_post(stream: &mut std::net::TcpStream) -> Vec<u8> {
        use std::io::{BufRead, BufReader, Read};
        stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
        let mut reader = BufReader::new(stream);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line.to_ascii_lowercase());
        }
        assert!(headers.starts_with("post /twitch/api/v3/live-match http/1.1"));
        assert!(headers.contains(&format!(
            "user-agent: {}\r\n",
            USER_AGENT.to_ascii_lowercase()
        )));
        assert!(headers.contains("content-type: application/json\r\n"));
        assert!(headers.contains("authorization: bearer synthetic-test-only\r\n"));
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(length <= limits::BODY_BYTES);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        body
    }

    fn next_test_payload(writer: &mut Writer, s: &GameSnapshot) -> Payload {
        let observation = prepare(Some(s), writer).unwrap();
        writer.packet(observation).unwrap()
    }

    #[test]
    fn upload_sends_exact_json_bytes_and_version_and_reads_retry_after() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let mut writer = Writer::new();
        let s = snapshot();
        let payloads: Vec<_> = (0..5).map(|_| next_test_payload(&mut writer, &s)).collect();
        let expected: Vec<_> = payloads.iter().map(|p| encode_body(p).unwrap()).collect();
        let server = thread::spawn(move || {
            for (index, response) in [
                "429 Too Many Requests",
                "204 No Content",
                "401 Unauthorized",
                "409 Conflict",
                "410 Gone",
            ]
            .into_iter()
            .enumerate()
            {
                let (mut stream, _) = listener.accept().unwrap();
                let body = read_mock_post(&mut stream);
                assert_eq!(body, expected[index]);
                assert_eq!(
                    serde_json::from_slice::<Value>(&body).unwrap()["sequence"],
                    index + 1
                );
                write!(stream, "HTTP/1.1 {response}\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let client = client_builder().build().unwrap();
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &payloads[0]),
            Ok((Status::Rejected(429), Some(Duration::from_secs(1))))
        );
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &payloads[1]),
            Ok((Status::Sending, Some(Duration::from_secs(1))))
        );
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &payloads[2]),
            Err(FatalUpload::KeyRejected)
        );
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &payloads[3]),
            Err(FatalUpload::Conflict)
        );
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &payloads[4]),
            Err(FatalUpload::ObsoleteApi)
        );
        server.join().unwrap();
        let fresh = next_test_payload(&mut writer, &s);
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &fresh),
            Ok((Status::Unreachable, None))
        );
    }

    #[test]
    fn only_exact_epoch_expired_conflict_header_allows_waiting_for_next_match() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let server = thread::spawn(move || {
            for headers in [
                "Live-Match-Conflict: epoch-expired\r\n",
                "",
                "Live-Match-Conflict: writer-conflict\r\n",
                "Live-Match-Conflict: epoch-expired\r\nLive-Match-Conflict: writer-conflict\r\n",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let _body = read_mock_post(&mut stream);
                write!(stream, "HTTP/1.1 409 Conflict\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let client = client_builder().build().unwrap();
        let mut writer = Writer::new();
        let s = snapshot();
        assert_eq!(
            upload(
                &client,
                &endpoint,
                "synthetic-test-only",
                &next_test_payload(&mut writer, &s)
            ),
            Ok((Status::EpochExpired, None))
        );
        for _ in 0..3 {
            assert_eq!(
                upload(
                    &client,
                    &endpoint,
                    "synthetic-test-only",
                    &next_test_payload(&mut writer, &s)
                ),
                Err(FatalUpload::Conflict)
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn timeout_never_resends_the_same_packet_and_next_sample_is_freshly_read() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let (release, wait) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_body: Value = serde_json::from_slice(&read_mock_post(&mut first)).unwrap();
            assert_eq!(first_body["sequence"], 1);
            assert_eq!(first_body["game_seconds"], 42);
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            drop(first);
            let (mut second, _) = listener.accept().unwrap();
            let second_body: Value = serde_json::from_slice(&read_mock_post(&mut second)).unwrap();
            assert_eq!(second_body["sequence"], 2);
            assert_eq!(second_body["game_seconds"], 45);
            assert_eq!(second_body["writer_id"], first_body["writer_id"]);
            assert_eq!(second_body["match_epoch"], first_body["match_epoch"]);
            write!(
                second,
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let client = client_builder()
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        let mut writer = Writer::new();
        let mut s = snapshot();
        let first = next_test_payload(&mut writer, &s);
        let result = upload(&client, &endpoint, "synthetic-test-only", &first);
        release.send(()).unwrap();
        assert_eq!(result, Ok((Status::Unreachable, None)));
        s.playing_seconds = Some(45.8);
        let second = next_test_payload(&mut writer, &s);
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &second),
            Ok((Status::Sending, None))
        );
        server.join().unwrap();
    }

    #[test]
    fn excluded_or_incomplete_new_match_cannot_reopen_an_older_epoch() {
        let mut writer = Writer::new();
        let mut s = snapshot();
        assert_eq!(packet(&s, &mut writer)["match_epoch"], 1);
        s.match_token = Some(456);
        s.eligibility = Eligibility::Excluded;
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Excluded));
        s.match_token = Some(123);
        s.eligibility = Eligibility::Eligible;
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::OldMatch));
        s.match_token = Some(789);
        s.players.clear();
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Unreadable));
        let mut s = snapshot();
        s.match_token = Some(456);
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::OldMatch));
        s.match_token = Some(789);
        let new = packet(&s, &mut writer);
        assert_eq!(new["match_epoch"], 3);
        assert_eq!(new["sequence"], 2);
    }

    #[test]
    fn json_body_limit_counts_utf8_and_escaping_and_rejects_before_network() {
        let exact = format!("{}{}", "ü".repeat((limits::BODY_BYTES - 4) / 2), "\"");
        assert_eq!(encode_body(&exact).unwrap().len(), limits::BODY_BYTES);
        assert_eq!(encode_body(&format!("{exact}x")), Err(Status::BodyTooLarge));
        assert_eq!(
            encode_body(&"a".repeat(limits::BODY_BYTES - 3))
                .unwrap()
                .len(),
            limits::BODY_BYTES - 1
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let client = client_builder().build().unwrap();
        assert_eq!(
            upload(
                &client,
                &endpoint,
                "synthetic-test-only",
                &format!("{exact}x")
            ),
            Ok((Status::BodyTooLarge, None))
        );
        let invalid = std::collections::BTreeMap::from([(vec![1, 2], 3)]);
        assert_eq!(encode_body(&invalid), Err(Status::InvalidBody));
        assert_eq!(
            upload(&client, &endpoint, "synthetic-test-only", &invalid),
            Ok((Status::InvalidBody, None))
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn source_error_and_detach_never_replay_previous_sample() {
        let mut writer = Writer::new();
        let mut source = SyntheticSource::new(vec![
            Ok(Some(snapshot())),
            Err(SourceError::Unavailable),
            Ok(None),
            Err(SourceError::Incompatible),
            Err(SourceError::InvalidSample),
            Err(SourceError::Unauthorized),
            Ok(Some(snapshot())),
        ]);
        assert!(read(&mut source, &mut writer).is_ok());
        for expected in [
            Status::ReadFailed,
            Status::Waiting,
            Status::Outdated,
            Status::Unreadable,
            Status::Blocked,
        ] {
            assert_eq!(read(&mut source, &mut writer), Err(expected));
        }
        assert!(read(&mut source, &mut writer).is_ok());
        assert_eq!(writer.epoch, 1);
        assert_eq!(read(&mut source, &mut writer), Err(Status::Waiting));
    }

    #[test]
    fn names_the_reason_when_nothing_is_sent() {
        assert_eq!(prepare(None, &mut Writer::new()), Err(Status::Waiting));
        let mut s = snapshot();
        s.players[0].hero_id = None;
        assert_eq!(
            prepare(Some(&s), &mut Writer::new()),
            Err(Status::Unreadable)
        );
        s.eligibility = Eligibility::Excluded;
        assert_eq!(prepare(Some(&s), &mut Writer::new()), Err(Status::Excluded));
        s.context = MatchContext::Other;
        assert_eq!(prepare(Some(&s), &mut Writer::new()), Err(Status::Waiting));
    }

    #[test]
    fn any_drift_stops_the_upload_inside_and_outside_a_match() {
        let mut writer = Writer::new();
        let mut source = SyntheticSource::new(vec![
            Err(SourceError::Incompatible),
            Err(SourceError::Incompatible),
            Ok(None),
        ]);
        assert_eq!(read(&mut source, &mut writer), Err(Status::Outdated));
        assert_eq!(read(&mut source, &mut writer), Err(Status::Outdated));
        assert_eq!(read(&mut source, &mut writer), Err(Status::Waiting));
        assert_eq!(writer.sequence, 0);
        assert_eq!(writer.epoch, 0);
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
    fn authorization_block_has_no_raw_error_or_fallback() {
        assert!(matches!(
            BlockedSource::open(),
            Err(SourceError::Unauthorized)
        ));
        let mut source = BlockedSource;
        let mut writer = Writer::new();
        for _ in 0..3 {
            assert_eq!(read(&mut source, &mut writer), Err(Status::Blocked));
        }
        assert_eq!(writer.sequence, 0);
        assert_eq!(writer.epoch, 0);
        let mut s = snapshot();
        s.provenance = Provenance::Unknown;
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Blocked));
        assert_eq!(writer.epoch, 0);
        let mut announcer = Announcer::new();
        assert_eq!(announcer.observe(Status::Blocked), None);
        assert_eq!(announcer.observe(Status::Blocked), None);
        assert!(announcer
            .observe(Status::Blocked)
            .unwrap()
            .contains("Keine autorisierte"));
        assert_eq!(announcer.observe(Status::Blocked), None);
    }

    #[test]
    fn unknown_eligibility_and_context_never_upload() {
        let mut s = snapshot();
        s.eligibility = Eligibility::Unknown;
        let mut writer = Writer::new();
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Unreadable));
        assert_eq!(writer.epoch, 1);
        assert_eq!(writer.sequence, 0);
        s.eligibility = Eligibility::Eligible;
        s.context = MatchContext::Unknown;
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Waiting));
        assert_eq!(writer.sequence, 0);
    }

    #[test]
    fn missing_name_after_valid_sample_cannot_reuse_cached_name() {
        let mut writer = Writer::new();
        let mut invalid = snapshot();
        invalid.local_hero_name = None;
        let mut source =
            SyntheticSource::new(vec![Ok(Some(snapshot())), Ok(Some(invalid)), Ok(None)]);
        let sample = read(&mut source, &mut writer).unwrap();
        writer.packet(sample).unwrap();
        assert_eq!(read(&mut source, &mut writer), Err(Status::Unreadable));
        assert_eq!(read(&mut source, &mut writer), Err(Status::Waiting));
        assert_eq!(writer.sequence, 1);
    }

    #[test]
    fn unknown_source_cannot_displace_valid_match_epoch() {
        let mut writer = Writer::new();
        assert_eq!(packet(&snapshot(), &mut writer)["match_epoch"], 1);
        let mut s = snapshot();
        s.provenance = Provenance::Unknown;
        s.match_token = Some(456);
        assert_eq!(prepare(Some(&s), &mut writer), Err(Status::Blocked));
        assert_eq!(packet(&snapshot(), &mut writer)["match_epoch"], 1);
    }

    #[test]
    fn fatal_upload_statuses_stop_without_fallback() {
        assert_eq!(outcome(Some(204)), Ok(Status::Sending));
        assert_eq!(outcome(Some(401)), Err(FatalUpload::KeyRejected));
        assert_eq!(outcome(Some(409)), Err(FatalUpload::Conflict));
        assert_eq!(outcome(Some(410)), Err(FatalUpload::ObsoleteApi));
        for code in [200, 201, 301, 400, 403, 404, 408, 413, 415, 422, 429, 503] {
            assert_eq!(outcome(Some(code)), Ok(Status::Rejected(code)));
        }
        assert_eq!(outcome(None), Ok(Status::Unreachable));
    }
}
