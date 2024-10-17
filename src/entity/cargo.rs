use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Url,
};

use super::{Dependency, TomlKey};

pub struct CargoError {
    pub kind: CargoErrorKind,
    pub inner: anyhow::Error,
}

impl CargoError {
    pub fn other(e: anyhow::Error) -> Self {
        CargoError {
            kind: CargoErrorKind::Other,
            inner: e,
        }
    }

    pub fn crate_name(&self) -> Option<&str> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(name) => Some(name),
            CargoErrorKind::VersionNotFound(name) => Some(name),
            CargoErrorKind::FailedToSelectVersion(name) => Some(name),
            _ => None,
        }
    }
}

impl CargoError {
    pub fn diagnostic(
        &self,
        uri: &Url,
        key: Option<&TomlKey>,
        dep: Option<&Dependency>,
    ) -> Option<Diagnostic> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(name) => {
                let key = key?;
                Some(Diagnostic {
                    range: key.range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    code: None,
                    code_description: None,
                    source: Some("cargo".to_string()),
                    message: format!("No matching package named `{}` found", name),
                    related_information: Some(vec![DiagnosticRelatedInformation {
                        location: Location::new(uri.clone(), key.range),
                        message: "hahahah".to_string(),
                    }]),
                    tags: None,
                    data: None,
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum CargoErrorKind {
    NoMatchingPackage(String),
    VersionNotFound(String),
    FailedToSelectVersion(String),
    CyclicDependency,
    Other,
}

impl From<anyhow::Error> for CargoError {
    fn from(e: anyhow::Error) -> Self {
        let error_message = e.to_string();

        if error_message.starts_with("no matching package named") {
            let Some(package_name) = error_message.split('`').nth(1) else {
                return CargoError {
                    kind: CargoErrorKind::Other,
                    inner: e,
                };
            };
            return CargoError {
                kind: CargoErrorKind::NoMatchingPackage(package_name.to_string()),
                inner: e,
            };
        }

        if error_message.starts_with("failed to select a version for the requirement") {
            //failed to select a version for the requirement `serde = "^2"`
            let Some(package_with_version) = error_message.split('`').nth(1) else {
                return CargoError {
                    kind: CargoErrorKind::Other,
                    inner: e,
                };
            };
            let Some(package_name) = package_with_version.split_whitespace().next() else {
                return CargoError {
                    kind: CargoErrorKind::Other,
                    inner: e,
                };
            };
            return CargoError {
                kind: CargoErrorKind::VersionNotFound(package_name.to_string()),
                inner: e,
            };
        }

        if error_message.starts_with("failed to select a version for") {
            let Some(package_name) = error_message.split('`').nth(1) else {
                return CargoError {
                    kind: CargoErrorKind::Other,
                    inner: e,
                };
            };
            return CargoError {
                kind: CargoErrorKind::FailedToSelectVersion(package_name.to_string()),
                inner: e,
            };
        }

        if error_message.contains("cyclic package dependency") {
            return CargoError {
                kind: CargoErrorKind::CyclicDependency,
                inner: e,
            };
        }

        CargoError {
            kind: CargoErrorKind::Other,
            inner: e,
        }
    }
}
