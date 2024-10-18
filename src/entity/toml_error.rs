// copied from /cargo/crates/cargo-util-schemas/src/restricted_names.rs

use thiserror::Error;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range, Url};

#[derive(Debug, Error, Clone)]
pub struct TomlParsingError {
    pub id: String,
    #[source]
    source: TomlError,
    pub range: Range,
}

impl std::fmt::Display for TomlParsingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl TomlParsingError {
    pub fn new(id: String, source: TomlError, range: Range) -> Self {
        TomlParsingError { id, source, range }
    }

    pub fn diagnostic(self) -> Option<(String, Diagnostic)> {
        match self.source {
            TomlError::InvalidCrateName(e) => Some((
                self.id,
                Diagnostic {
                    range: self.range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: None,
                    code_description: None,
                    source: Some("cargo-appraiser".to_string()),
                    message: e.to_string(),
                    related_information: None,
                    tags: None,
                    data: None,
                },
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Error, Clone)]
pub enum TomlError {
    #[error(transparent)]
    InvalidCrateName(InvalidCrateName),
    #[error(transparent)]
    InvalidFeatureName(InvalidFeatureName),
    #[error(transparent)]
    InvalidProfileName(InvalidProfileName),
}

impl From<InvalidCrateName> for TomlError {
    fn from(value: InvalidCrateName) -> Self {
        TomlError::InvalidCrateName(value)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum InvalidCrateName {
    #[error("crate name {0} cannot start with digit")]
    StartWithDigit(String),
    #[error("crate name {0}'s first character must be letters or `_`")]
    InvalidFirstCharacter(String),
    #[error("crate name {0} must only contain numbers, `-`, `_` or letters")]
    InvalidCharacter(String),
}

pub fn validate_crate_name(name: &str) -> Result<(), TomlError> {
    let mut chars = name.chars();
    if let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            // A specific error for a potentially common case.
            return Err(InvalidCrateName::StartWithDigit(name.to_string()).into());
        }
        if !(unicode_xid::UnicodeXID::is_xid_start(ch) || ch == '_') {
            return Err(InvalidCrateName::InvalidFirstCharacter(name.to_string()).into());
        }
    }
    for ch in chars {
        if !(unicode_xid::UnicodeXID::is_xid_continue(ch) || ch == '-') {
            return Err(InvalidCrateName::InvalidCharacter(name.to_string()).into());
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum InvalidFeatureName {
    #[error("todo")]
    StartWithDepColon,
    #[error("todo")]
    ContainSlash,
    #[error("todo")]
    InvalidFirstCharacter,
    #[error("todo")]
    InvalidCharacter,
}

impl From<InvalidFeatureName> for TomlError {
    fn from(value: InvalidFeatureName) -> Self {
        TomlError::InvalidFeatureName(value)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum InvalidProfileName {
    #[error("todo")]
    InvalidCharacter,
    #[error("todo")]
    DebugReserved,
    #[error("todo")]
    BuildOverrideReserved,
    #[error("todo")]
    KeywordReserved,
}

impl From<InvalidProfileName> for TomlError {
    fn from(value: InvalidProfileName) -> Self {
        TomlError::InvalidProfileName(value)
    }
}
