//! Black-box proof: neither regular nor internal-test binaries solicit a key without
//! an authorized source. No real network or game access in this test.
use std::process::{Command, Stdio};

#[test]
fn binary_stops_before_key_input_when_source_not_authorized() {
    let output = Command::new(env!("CARGO_BIN_EXE_live-match-collector"))
        .stdin(Stdio::null())
        .output()
        .expect("start built collector");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Keine autorisierte Deadlock-Live-Datenquelle"));
    assert!(!stderr.contains("Verbindungsschlüssel"));
    assert!(!stderr.contains("Warte auf ein laufendes"));
}
#[test]
fn binary_rejects_arguments_before_source_or_key_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_live-match-collector"))
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .expect("start built collector");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("keine Befehlszeilenargumente"));
    assert!(!stderr.contains("Verbindungsschlüssel"));
}

#[test]
fn library_source_gate_remains_closed_and_production_rejects_synthetic_provenance() {
    use live_match_collector::{
        game_model::{
            Eligibility, GameMode, GameSnapshot, MatchContext, PlayerSample, Provenance, Team,
        },
        game_source::{GameSource, SourceError},
        independent_source::BlockedSource,
        telemetry,
    };
    assert!(matches!(
        BlockedSource::open(),
        Err(SourceError::Unauthorized)
    ));
    assert_eq!(BlockedSource.poll(), Err(SourceError::Unauthorized));
    let mut snapshot = GameSnapshot {
        match_token: Some(1),
        context: MatchContext::LivePlaying,
        eligibility: Eligibility::Eligible,
        provenance: Provenance::Synthetic,
        expected_player_count: Some(2),
        game_mode: Some(GameMode::Normal),
        playing_seconds: Some(42.0),
        local_hero_name: Some("Infernus".into()),
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
    };
    assert!(telemetry::observation(&snapshot).is_none());
    snapshot.provenance = Provenance::Unknown;
    assert!(telemetry::observation(&snapshot).is_none());
}
