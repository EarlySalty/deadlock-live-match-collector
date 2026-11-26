//! No live data source is authorized for the required field coverage yet.
//! This boundary must not be replaced with a guessed or silently simulated provider.
use crate::game_model::GameSnapshot;
use crate::game_source::{GameSource, SourceError};

pub struct BlockedSource;

impl BlockedSource {
    pub fn open() -> Result<Self, SourceError> {
        Err(SourceError::Unauthorized)
    }
}

impl GameSource for BlockedSource {
    fn poll(&mut self) -> Result<Option<GameSnapshot>, SourceError> {
        Err(SourceError::Unauthorized)
    }
}
