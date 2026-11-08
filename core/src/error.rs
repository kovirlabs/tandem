use serde::{Serialize, Serializer};
use thiserror::Error;

use crate::providers::ProviderError;
use crate::router::privacy::PrivacyError;

/// Library-level error for the Tandem core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("can't {action} while the session is {phase}")]
    InvalidTransition { action: String, phase: String },
    #[error(transparent)]
    Privacy(#[from] PrivacyError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("storage error: {0}")]
    Store(#[from] rusqlite::Error),
    #[error("invalid tool call: {0}")]
    Tool(String),
    #[error("secret store error: {0}")]
    Secret(String),
    #[error("no {0} provider is configured")]
    NoProvider(&'static str),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// Commands return errors to the UI as plain strings.
impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
