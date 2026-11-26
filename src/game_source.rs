//! Narrow polling boundary: only currently observed, permitted game values.
use crate::game_model::GameSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    Unavailable,
    Unauthorized,
    Incompatible,
    InvalidSample,
}

/// No cached replay. No raw game data or arbitrary I/O in this interface.
/// Authorization and visibility must be established before returning a sample.
pub trait GameSource {
    fn poll(&mut self) -> Result<Option<GameSnapshot>, SourceError>;
}

#[cfg(test)]
pub struct SyntheticSource {
    samples: std::vec::IntoIter<Result<Option<GameSnapshot>, SourceError>>,
}

#[cfg(test)]
impl SyntheticSource {
    pub fn new(samples: Vec<Result<Option<GameSnapshot>, SourceError>>) -> Self {
        Self {
            samples: samples.into_iter(),
        }
    }
}

#[cfg(test)]
impl GameSource for SyntheticSource {
    fn poll(&mut self) -> Result<Option<GameSnapshot>, SourceError> {
        self.samples.next().unwrap_or(Ok(None)).map(|sample| {
            sample.map(|mut snapshot| {
                snapshot.provenance = crate::game_model::Provenance::Synthetic;
                snapshot
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_model::Provenance;

    #[test]
    fn synthetic_source_marks_samples_and_never_replays() {
        let mut source = SyntheticSource::new(vec![
            Ok(Some(GameSnapshot {
                provenance: Provenance::Authorized,
                ..Default::default()
            })),
            Err(SourceError::Unauthorized),
            Ok(None),
            Err(SourceError::Unavailable),
            Err(SourceError::Incompatible),
            Err(SourceError::InvalidSample),
        ]);
        assert_eq!(
            source.poll().unwrap().unwrap().provenance,
            Provenance::Synthetic
        );
        assert_eq!(source.poll(), Err(SourceError::Unauthorized));
        assert_eq!(source.poll(), Ok(None));
        assert_eq!(source.poll(), Err(SourceError::Unavailable));
        assert_eq!(source.poll(), Err(SourceError::Incompatible));
        assert_eq!(source.poll(), Err(SourceError::InvalidSample));
        assert_eq!(source.poll(), Ok(None));
        assert_eq!(source.poll(), Ok(None));
    }
}
