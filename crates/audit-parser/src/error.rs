//! Error types for audit-parser.

use thiserror::Error;

/// Errors that can occur during cargo audit operations.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Failed to spawn the cargo audit process
    #[error("Failed to spawn cargo audit: {0}")]
    SpawnFailed(String),

    /// Cargo audit returned empty output
    #[error("cargo audit returned empty output")]
    EmptyOutput,

    /// Failed to parse cargo audit output
    #[error("Failed to parse cargo audit output: {0}")]
    ParseFailed(String),
}
