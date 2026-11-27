//! Source-independent lifecycle; all observation and transport remain fail-closed.
use super::*;
use std::time::Instant;
use zeroize::Zeroizing;

pub(crate) const IDLE: Duration = Duration::from_secs(2);
pub(crate) struct Runtime {
    pid: Option<u32>,
    source: Option<Box<dyn GameSource>>,
    token: Option<Zeroizing<String>>,
    client: Option<Client>,
    writer: Writer,
    leased: bool,
    session_next: Instant,
    pacing: RetryPacing,
    next: Instant,
    status: String,
    fatal: Option<String>,
}
impl Runtime {
    pub(crate) fn new(token: Option<Zeroizing<String>>) -> Self {
        Self {
            pid: None,
            source: None,
            token,
            client: None,
            writer: Writer::new(),
            leased: false,
            session_next: Instant::now(),
            pacing: RetryPacing::new(),
            next: Instant::now(),
            status: "Warte auf Deadlock.".into(),
            fatal: None,
        }
    }
    pub(crate) fn set_token(&mut self, token: Option<Zeroizing<String>>) {
        if self.token.as_deref() == token.as_deref() {
            return;
        }
        self.token = token;
        self.client = None;
        self.source = None;
        self.writer = Writer::new();
        self.leased = false;
        self.session_next = Instant::now();
        self.pacing = RetryPacing::new();
        self.fatal = None;
        self.next = Instant::now();
    }
    pub(crate) fn tick(
        &mut self,
        pid: Result<Option<u32>, ()>,
        now: Instant,
        open: impl FnOnce() -> Result<Box<dyn GameSource>, SourceError>,
    ) -> &str {
        self.tick_with_session(pid, now, open, acquire_live, send_live)
    }
    fn tick_with_session(
        &mut self,
        pid: Result<Option<u32>, ()>,
        now: Instant,
        open: impl FnOnce() -> Result<Box<dyn GameSource>, SourceError>,
        acquire: impl FnOnce(&mut Option<Client>, &str) -> Result<SessionReply, SendError>,
        send: impl FnOnce(
            &mut Option<Client>,
            &str,
            &Payload,
        ) -> Result<(Status, Option<Duration>), SendError>,
    ) -> &str {
        let pid = match pid {
            Ok(pid) => pid,
            Err(()) => {
                self.source = None;
                self.client = None;
                self.pid = None;
                self.status =
                    "Deadlock-Prozessprüfung fehlgeschlagen. Keine Übertragung; erneuter Versuch."
                        .into();
                return &self.status;
            }
        };
        if pid != self.pid {
            self.source = None;
            self.client = None;
            self.pid = pid;
            self.next = now;
            self.pacing = RetryPacing::new();
        }
        if pid.is_none() {
            self.status = "Wartemodus – Deadlock ist geschlossen.".into();
            return &self.status;
        }
        if self.token.is_none() {
            self.status = "Deadlock läuft – bitte Verbindungsschlüssel speichern.".into();
            return &self.status;
        }
        if let Some(fatal) = &self.fatal {
            self.status = fatal.clone();
            return &self.status;
        }
        if now < self.next {
            return &self.status;
        }
        if self.source.is_none() {
            match open() {
                Ok(source) => self.source = Some(source),
                Err(error) => {
                    self.status = source_status(error).message();
                    self.next = now + Duration::from_secs(5);
                    return &self.status;
                }
            }
        }
        let (status, hint) = match read(self.source.as_mut().unwrap().as_mut(), &mut self.writer) {
            Ok(_) if !self.leased => {
                if now < self.session_next {
                    self.status = "Warte auf sichere Server-Sitzung; kein Upload.".into();
                    return &self.status;
                }
                match acquire(&mut self.client, self.token.as_ref().unwrap()) {
                    Ok(SessionReply::Granted(grant)) => {
                        self.writer.id = grant.writer_id;
                        self.writer.sequence = grant.next_sequence - 1;
                        // The current eligible match has already been observed. Its first
                        // upload must use the server-reserved next epoch, not increment it.
                        self.writer.epoch = grant.next_match_epoch;
                        self.leased = true;
                        self.pacing = RetryPacing::new();
                        self.next = now.max(Instant::now()) + PERIOD;
                        self.status =
                            "Server-Sitzung bestätigt; warte auf frische Spielwerte.".into();
                    }
                    Ok(SessionReply::Wait(delay)) => {
                        self.session_next = now.max(Instant::now()) + delay;
                        self.next = self.session_next;
                        self.status = "Server-Sitzung noch nicht verfügbar; erneuter Versuch ohne Schlüsselwechsel.".into();
                    }
                    Err(SendError::Setup(reason)) => {
                        self.status = reason.into();
                        self.next = now + Duration::from_secs(10);
                    }
                    Err(SendError::Fatal(error)) => self.stop_fatal(error),
                }
                return &self.status;
            }
            Ok(observation) => match self.writer.packet(observation) {
                Ok(data) => match send(&mut self.client, self.token.as_ref().unwrap(), &data) {
                    Ok((Status::EpochExpired, hint)) => {
                        self.writer.expire_current();
                        (Status::EpochExpired, hint)
                    }
                    Ok(result) => result,
                    Err(SendError::Setup(reason)) => {
                        self.status = reason.into();
                        self.next = now + Duration::from_secs(10);
                        return &self.status;
                    }
                    Err(SendError::Fatal(error)) => {
                        self.stop_fatal(error);
                        return &self.status;
                    }
                },
                Err(status) => (status, None),
            },
            Err(status) => {
                if matches!(
                    status,
                    Status::Blocked | Status::ReadFailed | Status::Outdated
                ) {
                    self.source = None;
                }
                (status, None)
            }
        };
        if status == Status::CounterExhausted {
            self.fatal = Some(status.message());
            self.source = None;
        }
        self.next = now + self.pacing.after(&status, hint);
        self.status = status.message();
        &self.status
    }
    fn stop_fatal(&mut self, error: FatalUpload) {
        let message = match error {
            FatalUpload::KeyRejected => "Verbindungsschlüssel abgelehnt. Bitte neuen Schlüssel im Overlay-Baukasten erstellen und hier speichern.",
            FatalUpload::Conflict => "Server meldet einen Sitzungskonflikt. Bitte im Overlay-Baukasten einen neuen Schlüssel erstellen und hier speichern.",
            FatalUpload::ObsoleteApi => error.message(),
        };
        self.fatal = Some(message.into());
        self.status = message.into();
        self.source = None;
        self.client = None;
    }

    #[cfg(test)]
    fn tick_with(
        &mut self,
        pid: Result<Option<u32>, ()>,
        now: Instant,
        open: impl FnOnce() -> Result<Box<dyn GameSource>, SourceError>,
        send: impl FnOnce(
            &mut Option<Client>,
            &str,
            &Payload,
        ) -> Result<(Status, Option<Duration>), SendError>,
    ) -> &str {
        self.tick_with_session(pid, now, open, |_, _| panic!("unexpected session"), send)
    }
}

#[derive(serde::Deserialize)]
struct SessionGrant {
    writer_id: uuid::Uuid,
    next_sequence: u64,
    next_match_epoch: u64,
}

enum SessionReply {
    Granted(SessionGrant),
    Wait(Duration),
}

fn acquire_live(client: &mut Option<Client>, token: &str) -> Result<SessionReply, SendError> {
    if client.is_none() {
        *client = Some(desktop_client().map_err(SendError::Setup)?);
    }
    request_session(client.as_ref().unwrap(), ENDPOINT, token).map_err(SendError::Fatal)
}

fn request_session(
    client: &Client,
    endpoint: &str,
    token: &str,
) -> Result<SessionReply, FatalUpload> {
    use std::io::Read;
    // A lost or malformed success response may already have reserved a lease.
    let wait = || Ok(SessionReply::Wait(MAX_RETRY));
    let Ok(response) = client
        .post(format!("{endpoint}/session"))
        .bearer_auth(token)
        .body(Vec::new())
        .send()
    else {
        return wait();
    };
    match response.status().as_u16() {
        200 => {
            let mut bytes = Vec::new();
            if response.take(1025).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 {
                return wait();
            }
            let Ok(grant) = serde_json::from_slice::<SessionGrant>(&bytes) else {
                return wait();
            };
            if grant.writer_id.get_version() != Some(uuid::Version::Random)
                || grant.writer_id.get_variant() != uuid::Variant::RFC4122
                || grant.next_sequence != 2
                || grant.next_match_epoch != 2
            {
                return wait();
            }
            Ok(SessionReply::Granted(grant))
        }
        401 => Err(FatalUpload::KeyRejected),
        410 => Err(FatalUpload::ObsoleteApi),
        409 if response
            .headers()
            .get_all("live-match-conflict")
            .iter()
            .count()
            == 1
            && response
                .headers()
                .get("live-match-conflict")
                .is_some_and(|v| v.as_bytes() == b"writer-busy") =>
        {
            wait()
        }
        409 => Err(FatalUpload::Conflict),
        429 | 503 => {
            let hint = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| retry_after(v, SystemTime::now()))
                .unwrap_or(MAX_RETRY);
            Ok(SessionReply::Wait(hint.max(PERIOD)))
        }
        _ => wait(),
    }
}

enum SendError {
    Setup(&'static str),
    Fatal(FatalUpload),
}
fn send_live(
    client: &mut Option<Client>,
    token: &str,
    data: &Payload,
) -> Result<(Status, Option<Duration>), SendError> {
    if client.is_none() {
        *client = Some(desktop_client().map_err(SendError::Setup)?);
    }
    upload(client.as_ref().unwrap(), ENDPOINT, token, data).map_err(SendError::Fatal)
}
fn desktop_client() -> Result<Client, &'static str> {
    let builder = client_builder().https_only(true);
    #[cfg(feature = "internal-test")]
    let builder = {
        let certificate = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|dir| dir.join("test-ca.pem")))
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| reqwest::Certificate::from_pem(&bytes).ok())
            .ok_or("Interner Test: gültige test-ca.pem neben der EXE erforderlich.")?;
        builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate)
    };
    builder
        .build()
        .map_err(|_| "Die sichere Verbindung konnte nicht eingerichtet werden.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    struct Probe(Rc<Cell<u32>>, Rc<Cell<u32>>);
    impl GameSource for Probe {
        fn poll(&mut self) -> Result<Option<GameSnapshot>, SourceError> {
            self.0.set(self.0.get() + 1);
            Ok(None)
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.1.set(self.1.get() + 1);
        }
    }
    #[test]
    fn lifecycle_never_opens_in_idle_and_drops_on_exit_or_pid_change() {
        let now = Instant::now();
        let mut r = Runtime::new(Some(Zeroizing::new("test".into())));
        let polls = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let open = || Ok(Box::new(Probe(polls.clone(), drops.clone())) as Box<dyn GameSource>);
        assert!(r.tick(Ok(None), now, || panic!()).contains("geschlossen"));
        r.tick(Ok(Some(10)), now, open);
        assert_eq!(polls.get(), 1);
        r.tick(Ok(Some(10)), now + IDLE, || panic!());
        assert_eq!(polls.get(), 1);
        r.tick(Ok(Some(11)), now + IDLE, open);
        assert_eq!(drops.get(), 1);
        r.tick(Ok(None), now + IDLE, || panic!());
        assert_eq!(drops.get(), 2);
        assert!(r.client.is_none());
    }
    #[test]
    fn missing_source_waits_without_network_and_retries() {
        let now = Instant::now();
        let mut r = Runtime::new(Some(Zeroizing::new("test".into())));
        let blocked = || Err(SourceError::Unauthorized);
        assert!(r
            .tick(Ok(Some(1)), now, blocked)
            .contains("Keine autorisierte"));
        assert!(r
            .tick(Ok(Some(1)), now + IDLE, || panic!())
            .contains("Keine autorisierte"));
        assert!(r
            .tick(Ok(Some(1)), now + Duration::from_secs(5), blocked)
            .contains("Keine autorisierte"));
        assert!(r.client.is_none());
        r.set_token(None);
        assert!(r
            .tick(Ok(Some(1)), now + Duration::from_secs(6), || panic!())
            .contains("Verbindungsschlüssel"));
    }
    #[test]
    fn detection_failure_drops_source_and_does_not_send() {
        let now = Instant::now();
        let mut r = Runtime::new(Some(Zeroizing::new("test".into())));
        let drops = Rc::new(Cell::new(0));
        r.tick(Ok(Some(1)), now, || {
            Ok(Box::new(Probe(Rc::new(Cell::new(0)), drops.clone())))
        });
        assert!(r.tick(Err(()), now, || panic!()).contains("fehlgeschlagen"));
        assert_eq!(drops.get(), 1);
        assert!(r.client.is_none());
    }
    #[test]
    fn session_http_contract_and_retries() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/twitch/api/v3/live-match",
            listener.local_addr().unwrap()
        );
        let id = uuid::Uuid::new_v4();
        let server = thread::spawn(move || {
            let valid = format!(r#"{{"writer_id":"{id}","next_sequence":2,"next_match_epoch":2}}"#);
            for (code, headers, body) in [
                (
                    "409 Conflict",
                    "Live-Match-Conflict: writer-busy\r\nRetry-After: 60\r\n",
                    "",
                ),
                ("429 Too Many Requests", "Retry-After: 7\r\n", ""),
                ("200 OK", "", "invalid"),
                ("200 OK", "", valid.as_str()),
                ("401 Unauthorized", "", ""),
                ("410 Gone", "", ""),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line.to_ascii_lowercase());
                }
                assert!(request.starts_with("post /twitch/api/v3/live-match/session http/1.1\r\n"));
                assert!(request.contains("authorization: bearer synthetic-test-only\r\n"));
                assert!(request.contains("content-length: 0\r\n"));
                write!(stream, "HTTP/1.1 {code}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let client = client_builder().build().unwrap();
        let ask = || request_session(&client, &endpoint, "synthetic-test-only");
        assert!(matches!(ask(), Ok(SessionReply::Wait(d)) if d == MAX_RETRY));
        assert!(matches!(ask(), Ok(SessionReply::Wait(d)) if d == Duration::from_secs(7)));
        assert!(matches!(ask(), Ok(SessionReply::Wait(d)) if d == MAX_RETRY));
        assert!(
            matches!(ask(), Ok(SessionReply::Granted(g)) if g.writer_id == id && g.next_sequence == 2 && g.next_match_epoch == 2)
        );
        assert!(matches!(ask(), Err(FatalUpload::KeyRejected)));
        assert!(matches!(ask(), Err(FatalUpload::ObsoleteApi)));
        server.join().unwrap();
        assert!(matches!(ask(), Ok(SessionReply::Wait(d)) if d == MAX_RETRY));
    }

    #[test]
    fn session_lease_waits_across_pid_changes_and_uses_fresh_sample() {
        let mut r = Runtime::new(Some(Zeroizing::new("test".into())));
        let now = Instant::now();
        let id = uuid::Uuid::new_v4();
        let open = || {
            let mut s = snapshot_for_session();
            s.playing_seconds = Some(20.0);
            let mut next = s.clone();
            next.playing_seconds = Some(23.0);
            Ok(Box::new(game_source::SyntheticSource::new(vec![
                Ok(Some(s)),
                Ok(Some(next)),
            ])) as Box<dyn GameSource>)
        };
        r.tick_with_session(
            Ok(Some(1)),
            now,
            open,
            |_, _| Ok(SessionReply::Wait(MAX_RETRY)),
            |_, _, _| panic!(),
        );
        let retry = r.session_next;
        r.tick_with_session(
            Ok(None),
            now + IDLE,
            || panic!(),
            |_, _| panic!(),
            |_, _, _| panic!(),
        );
        r.tick_with_session(
            Ok(Some(2)),
            now + IDLE * 2,
            open,
            |_, _| panic!(),
            |_, _, _| panic!(),
        );
        r.tick_with_session(
            Ok(None),
            retry,
            || panic!(),
            |_, _| panic!(),
            |_, _, _| panic!(),
        );
        r.tick_with_session(
            Ok(Some(3)),
            retry,
            open,
            |_, _| {
                Ok(SessionReply::Granted(SessionGrant {
                    writer_id: id,
                    next_sequence: 2,
                    next_match_epoch: 2,
                }))
            },
            |_, _, _| panic!(),
        );
        let next = r.next;
        r.tick_with_session(
            Ok(Some(3)),
            next,
            || panic!(),
            |_, _| panic!(),
            |_, _, payload| {
                let p = serde_json::to_value(payload).unwrap();
                assert_eq!(p["writer_id"], id.to_string());
                assert_eq!(p["sequence"], 2);
                assert_eq!(p["match_epoch"], 2);
                assert_eq!(p["game_seconds"], 23);
                Ok((Status::Sending, None))
            },
        );
        assert!(r.leased);
        r.tick(Ok(None), next + IDLE, || panic!());
        assert!(r.leased);
    }

    fn snapshot_for_session() -> GameSnapshot {
        use crate::game_model::{GameMode, PlayerSample, Team};
        GameSnapshot {
            match_token: Some(9),
            context: MatchContext::LivePlaying,
            eligibility: Eligibility::Eligible,
            provenance: Provenance::Synthetic,
            expected_player_count: Some(2),
            game_mode: Some(GameMode::Normal),
            playing_seconds: Some(20.0),
            local_hero_name: Some("Infernus".into()),
            players: vec![
                PlayerSample {
                    is_local: Some(true),
                    team: Some(Team::Own),
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
        }
    }

    #[test]
    fn network_retry_reads_fresh_observation_and_retains_writer() {
        use crate::game_model::{GameMode, PlayerSample, Team};
        let now = Instant::now();
        let mut r = Runtime::new(Some(Zeroizing::new("test".into())));
        let sample = |seconds| GameSnapshot {
            match_token: Some(9),
            context: MatchContext::LivePlaying,
            eligibility: Eligibility::Eligible,
            provenance: Provenance::Synthetic,
            expected_player_count: Some(2),
            game_mode: Some(GameMode::Normal),
            playing_seconds: Some(seconds),
            local_hero_name: Some("Infernus".into()),
            players: vec![
                PlayerSample {
                    is_local: Some(true),
                    team: Some(Team::Own),
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
        r.leased = true; // This test isolates post-handshake transport retries.
        let mut first = None;
        assert!(r
            .tick_with(
                Ok(Some(1)),
                now,
                || Ok(Box::new(game_source::SyntheticSource::new(vec![
                    Ok(Some(sample(20.0))),
                    Ok(Some(sample(26.0))),
                    Ok(Some(sample(29.0)))
                ]))),
                |_, _, payload| {
                    first = Some(serde_json::to_value(payload).unwrap());
                    Ok((Status::Unreachable, None))
                }
            )
            .contains("nicht erreichbar"));
        r.tick_with(Ok(Some(1)), now + IDLE, || panic!(), |_, _, _| panic!());
        assert!(r
            .tick_with(
                Ok(Some(1)),
                now + Duration::from_secs(6),
                || panic!(),
                |_, _, payload| {
                    let p = serde_json::to_value(payload).unwrap();
                    let f = first.as_ref().unwrap();
                    assert_eq!(p["writer_id"], f["writer_id"]);
                    assert_eq!(p["sequence"], 2);
                    assert_eq!(p["game_seconds"], 26);
                    Ok((Status::Sending, None))
                }
            )
            .contains("bestätigt"));
        assert!(r
            .tick_with(
                Ok(Some(1)),
                now + Duration::from_secs(9),
                || panic!(),
                |_, _, _| Err(SendError::Fatal(FatalUpload::KeyRejected))
            )
            .contains("abgelehnt"));
        r.tick_with(
            Ok(Some(1)),
            now + Duration::from_secs(15),
            || panic!(),
            |_, _, _| panic!(),
        );
        r.set_token(Some(Zeroizing::new("new".into())));
        assert!(r.fatal.is_none());
        r.tick_with(
            Ok(None),
            now + Duration::from_secs(16),
            || panic!(),
            |_, _, _| panic!(),
        );
    }
}
