use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Range, Url,
};

use super::{Dependency, SymbolTree, TomlKey};

use thiserror::Error;

#[derive(Debug, Error)]
pub struct CargoError {
    #[source]
    pub source: anyhow::Error,
    pub kind: CargoErrorKind,
}

impl std::fmt::Display for CargoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl CargoError {
    pub fn other(e: anyhow::Error) -> Self {
        CargoError {
            kind: CargoErrorKind::Other,
            source: e,
        }
    }

    pub fn unparsed_workspace_error(e: anyhow::Error) -> Self {
        CargoError {
            kind: CargoErrorKind::UnParsedWorkspaceError,
            source: e,
        }
    }

    pub fn crate_name(&self) -> Option<&str> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(name) => Some(name),
            CargoErrorKind::VersionNotFound(name, _) => Some(name),
            CargoErrorKind::FailedToSelectVersion(name) => Some(name),
            CargoErrorKind::CyclicDependency => None,
            CargoErrorKind::UnParsedWorkspaceError => None,
            CargoErrorKind::Other => None,
        }
    }
}

impl CargoError {
    pub fn diagnostic(
        self,
        keys: &[&TomlKey],
        deps: &[&Dependency],
        tree: &SymbolTree,
    ) -> Option<Vec<(String, Diagnostic)>> {
        match &self.kind {
            CargoErrorKind::NoMatchingPackage(_) => {
                //TODO multiple keys
                //it's same for all package names
                let key = keys.first()?;
                Some(vec![(
                    key.id.to_string(),
                    Diagnostic {
                        range: key.range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        code: None,
                        code_description: None,
                        source: Some("cargo".to_string()),
                        message: self.to_string(),
                        related_information: None,
                        tags: None,
                        data: None,
                    },
                )])
            }
            CargoErrorKind::VersionNotFound(_, _) => {
                //TODO multiple deps
                //check dep req and summaries?
                Some(
                    deps.iter()
                        .filter(|d| d.matched_summary.is_none())
                        .filter_map(|d| {
                            let req = d.unresolved.as_ref()?.version_req().to_string();
                            let error_msg = self.to_string();

                            // Check if the requirement in the error message matches the dependency's requirement
                            if error_msg.contains(&format!("`{} = \"{}\"", d.name, req)) {
                                let version = d.version.as_ref()?.id.as_str();
                                let range = tree.entries.get(version)?.range;
                                Some((
                                    version.to_string(),
                                    Diagnostic {
                                        range,
                                        severity: Some(DiagnosticSeverity::ERROR),
                                        code: None,
                                        code_description: None,
                                        source: Some("cargo".to_string()),
                                        message: error_msg,
                                        related_information: None,
                                        tags: None,
                                        data: None,
                                    },
                                ))
                            } else {
                                None
                            }
                        })
                        .collect(),
                )
            }
            CargoErrorKind::FailedToSelectVersion(_) => {
                //TODO multiple deps
                //check features
                None
            }
            _ => None,
        }
    }
}

#[derive(Error, Debug)]
pub enum CargoErrorKind {
    #[error("no matching package named `{0}` found")]
    NoMatchingPackage(String),
    #[error("failed to select a version for the requirement `{1}`")]
    VersionNotFound(String, String),
    #[error("failed to select a version for `{0}`")]
    FailedToSelectVersion(String),
    #[error("cyclic dependency detected")]
    CyclicDependency,
    #[error("unparsed workspace error")]
    UnParsedWorkspaceError,
    #[error("unparsed error")]
    Other,
}

pub fn from_resolve_error(e: anyhow::Error) -> CargoError {
    let error_message = e.to_string();

    // no matching package named `aserde` found
    // location searched: registry `crates-io`
    // required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
    if error_message.starts_with("no matching package named") {
        let Some(package_name) = error_message.split('`').nth(1) else {
            return CargoError {
                kind: CargoErrorKind::Other,
                source: e,
            };
        };
        return CargoError {
            kind: CargoErrorKind::NoMatchingPackage(package_name.to_string()),
            source: e,
        };
    }

    // failed to select a version for the requirement `serde = "^2"`
    // candidate versions found which didn't match: 1.0.210, 1.0.209, 1.0.208, ...
    // location searched: crates.io index
    // required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
    // if you are looking for the prerelease package it needs to be specified explicitly
    // serde = { version = "1.0.172-alpha.0" }
    if error_message.starts_with("failed to select a version for the requirement") {
        let Some(package_with_version) = error_message.split('`').nth(1) else {
            return CargoError {
                kind: CargoErrorKind::Other,
                source: e,
            };
        };
        let Some(package_name) = package_with_version.split_whitespace().next() else {
            return CargoError {
                kind: CargoErrorKind::Other,
                source: e,
            };
        };
        return CargoError {
            kind: CargoErrorKind::VersionNotFound(
                package_name.to_string(),
                package_with_version.to_string(),
            ),
            source: e,
        };
    }

    // failed to select a version for `serde`.
    // ... required by package `hello-rust v0.1.0 (/Users/jingyu/tmp/hello-rust)`
    // versions that meet the requirements `^1` (locked to 1.0.210) are: 1.0.210
    //
    // the package `hello-rust` depends on `serde`, with features: `de1rive` but `serde` does not have these features.
    if error_message.starts_with("failed to select a version for") {
        let Some(package_name) = error_message.split('`').nth(1) else {
            return CargoError {
                kind: CargoErrorKind::Other,
                source: e,
            };
        };
        return CargoError {
            kind: CargoErrorKind::FailedToSelectVersion(package_name.to_string()),
            source: e,
        };
    }

    // cyclic package dependency: package `A v0.0.0 (registry `https://example.com/`)` depends on itself. Cycle:
    // package `A v0.0.0 (registry `https://example.com/`)`
    //     ... which satisfies dependency `A = \"*\"` of package `C v0.0.0 (registry `https://example.com/`)`
    //     ... which satisfies dependency `C = \"*\"` of package `A v0.0.0 (registry `https://example.com/`)`\
    if error_message.contains("cyclic package dependency") {
        return CargoError {
            kind: CargoErrorKind::CyclicDependency,
            source: e,
        };
    }

    CargoError {
        kind: CargoErrorKind::Other,
        source: e,
    }
}
