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
