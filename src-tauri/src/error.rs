/// Serializable error type for all Tauri command returns.
/// Every variant maps to a user-visible string via thiserror::Error.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Custom(String),
}

// Tauri requires errors returned from commands to implement serde::Serialize.
// The pattern is to serialize the error as its Display string.
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

// Convenience: allow ? on String errors
impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Custom(s)
    }
}
