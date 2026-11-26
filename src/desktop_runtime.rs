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
        self.tick_with(pid, now, open, send_live)
    }
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
                        let message = match error {
                            FatalUpload::KeyRejected => "Verbindungsschlüssel abgelehnt. Bitte neuen Schlüssel im Overlay-Baukasten erstellen und hier speichern.",
                            FatalUpload::Conflict => "Server meldet einen Sitzungskonflikt. Bitte im Overlay-Baukasten einen neuen Schlüssel erstellen und hier speichern.",
                            FatalUpload::ObsoleteApi => error.message(),
                        };
                        self.fatal = Some(message.into());
                        self.status = message.into();
                        self.source = None;
                        self.client = None;
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
