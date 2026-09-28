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

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Auth error: {0}")]
    Auth(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    /// Caller-supplied input was rejected before any work was done: an
    /// out-of-range number, an over-long string, a malformed identifier.
    ///
    /// This is deliberately distinct from [`ConduitError::Other`]. Both
    /// serialise to a bare string on the JS side, so without a dedicated
    /// variant "you typed too much" and "something broke internally" are the
    /// same token and the UI can only show a generic failure. The serialized
    /// form is prefixed with [`VALIDATION_PREFIX`] so the frontend can branch
    /// on it without a schema change:
    ///
    /// ```ts
    /// if (String(err).startsWith('VALIDATION: ')) showFieldError(String(err));
    /// else showGenericError(err);
    /// ```
    #[error("{VALIDATION_PREFIX}{0}")]
    Validation(String),

    #[error("{0}")]
    Other(String),
}

/// Serialized prefix of [`ConduitError::Validation`], part of the JS-facing
/// contract. Kept next to the variant so the two cannot drift.
pub const VALIDATION_PREFIX: &str = "VALIDATION: ";

pub type Result<T> = std::result::Result<T, ConduitError>;

// Tauri commands require their error type to implement `serde::Serialize`.
// Serialize as the human-readable Display string so the frontend keeps
// receiving the same message shapes it saw when commands returned `String`.
impl serde::Serialize for ConduitError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// Implement conversion to String for legacy String-error call sites.
impl From<ConduitError> for String {
    fn from(err: ConduitError) -> String {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `VALIDATION: ` prefix is a contract with the frontend, not an
    /// implementation detail: it is the only thing that lets the UI tell
    /// "you typed too much" apart from "something broke". If this test fails,
    /// update the frontend branch in the same pass.
    #[test]
    fn validation_serializes_with_the_documented_prefix() {
        let err = ConduitError::Validation("device_name must be 64 characters or fewer".into());
        let as_string = err.to_string();
        assert!(
            as_string.starts_with(VALIDATION_PREFIX),
            "Validation must serialize with {VALIDATION_PREFIX:?}, got {as_string:?}"
        );
        assert_eq!(
            serde_json::to_string(&err).expect("serialise"),
            "\"VALIDATION: device_name must be 64 characters or fewer\"",
            "Tauri sends the Display string, so the prefix must survive serde too"
        );
    }

    /// Every other variant must NOT claim to be a validation error, or the
    /// frontend would render internal failures as a field-level correction.
    #[test]
    fn non_validation_variants_are_not_claiming_the_prefix() {
        let others: Vec<ConduitError> = vec![
            ConduitError::Other("boom".into()),
            ConduitError::Storage("locked".into()),
            ConduitError::Auth("nope".into()),
            ConduitError::Protocol("bad frame".into()),
            ConduitError::DeviceNotFound("d1".into()),
            ConduitError::Encryption("bad nonce".into()),
        ];
        for err in others {
            assert!(
                !err.to_string().starts_with(VALIDATION_PREFIX),
                "{err:?} must not be mistaken for a validation error"
            );
        }
    }

    /// `From<ConduitError> for String` is what the legacy `-> Result<_, String>`
    /// call sites use; the prefix has to survive that conversion too.
    #[test]
    fn validation_converts_to_string_with_the_prefix() {
        let converted: String = ConduitError::Validation("too long".into()).into();
        assert_eq!(converted, "VALIDATION: too long");
    }
}
