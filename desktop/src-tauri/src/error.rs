use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConduitError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Lock poisoned: {0}")]
    LockPoisoned(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ConduitError>;

// Implement conversion to String for Tauri commands
impl From<ConduitError> for String {
    fn from(err: ConduitError) -> String {
        err.to_string()
    }
}
