// copied from /cargo/crates/cargo-util-schemas/src/restricted_names.rs

use thiserror::Error;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range};

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
            TomlError::InvalidFeatureName(e) => Some((
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
            TomlError::InvalidProfileName(e) => Some((
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
    #[error("feature name {0} starts with `dep:`")]
    StartWithDepColon(String),
    #[error("feature name {0} contains `/`")]
    ContainSlash(String),
    #[error("feature name {0}'s first character must be letters or `_` or digit")]
    InvalidFirstCharacter(String),
    #[error("feature name {0} must only contain numbers, `+`, `-`, `_`, `.` or letters")]
    InvalidCharacter(String),
}

pub fn validate_feature_name(name: &str) -> Result<(), TomlError> {
    if name.starts_with("dep:") {
        return Err(InvalidFeatureName::StartWithDepColon(name.to_string()).into());
    }
    if name.contains('/') {
        return Err(InvalidFeatureName::ContainSlash(name.to_string()).into());
    }
    let mut chars = name.chars();
    if let Some(ch) = chars.next() {
        if !(unicode_xid::UnicodeXID::is_xid_start(ch) || ch == '_' || ch.is_ascii_digit()) {
            return Err(InvalidFeatureName::InvalidFirstCharacter(name.to_string()).into());
        }
    }
    for ch in chars {
        if !(unicode_xid::UnicodeXID::is_xid_continue(ch) || ch == '-' || ch == '+' || ch == '.') {
            return Err(InvalidFeatureName::InvalidCharacter(name.to_string()).into());
        }
    }
    Ok(())
}

impl From<InvalidFeatureName> for TomlError {
    fn from(value: InvalidFeatureName) -> Self {
        TomlError::InvalidFeatureName(value)
    }
}

#[derive(Debug, thiserror::Error, Clone)]
pub enum InvalidProfileName {
    #[error("profile name {0} only allow letters, numbers, underscore, and hyphen")]
    InvalidCharacter(String),
    #[error("use `dev` to configure the default development profile")]
    DebugReserved,
    #[error("use [profile.dev.build-override] or and [profile.release.build-override] to configure build dependency settings")]
    BuildOverrideReserved,
    #[error("profile name {0} is reserved")]
    KeywordReserved(String),
}

pub fn validate_profile_name(name: &str) -> Result<(), TomlError> {
    if let Some(ch) = name
        .chars()
        .find(|ch| !ch.is_alphanumeric() && *ch != '_' && *ch != '-')
    {
        return Err(InvalidProfileName::InvalidCharacter(name.to_string()).into());
    }

    let lower_name = name.to_lowercase();
    if lower_name == "debug" {
        return Err(InvalidProfileName::DebugReserved.into());
    }
    if lower_name == "build-override" {
        return Err(InvalidProfileName::BuildOverrideReserved.into());
    }

    // These are some arbitrary reservations. We have no plans to use
    // these, but it seems safer to reserve a few just in case we want to
    // add more built-in profiles in the future. We can also uses special
    // syntax like cargo:foo if needed. But it is unlikely these will ever
    // be used.
    if matches!(
        lower_name.as_str(),
        "build"
            | "check"
            | "clean"
            | "config"
            | "fetch"
            | "fix"
            | "install"
            | "metadata"
            | "package"
            | "publish"
            | "report"
            | "root"
            | "run"
            | "rust"
            | "rustc"
            | "rustdoc"
            | "target"
            | "tmp"
            | "uninstall"
    ) || lower_name.starts_with("cargo")
    {
        return Err(InvalidProfileName::KeywordReserved(name.to_string()).into());
    }

    Ok(())
}

impl From<InvalidProfileName> for TomlError {
    fn from(value: InvalidProfileName) -> Self {
        TomlError::InvalidProfileName(value)
    }
}
