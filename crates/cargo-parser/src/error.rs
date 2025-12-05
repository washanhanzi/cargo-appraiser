//! Error types for cargo-parser.

use std::fmt;

/// Error type for cargo resolution operations.
#[derive(Debug)]
pub struct CargoResolveError {
    kind: CargoResolveErrorKind,
    source: anyhow::Error,
}

#[derive(Debug)]
enum CargoResolveErrorKind {
    GlobalContext,
    Workspace,
    Resolve,
    CacheLock,
}

impl CargoResolveError {
    pub fn global_context(e: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: CargoResolveErrorKind::GlobalContext,
            source: e.into(),
        }
    }

    pub fn workspace(e: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: CargoResolveErrorKind::Workspace,
            source: e.into(),
        }
    }

    pub fn resolve(e: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: CargoResolveErrorKind::Resolve,
            source: e.into(),
        }
    }

    pub fn cache_lock(e: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: CargoResolveErrorKind::CacheLock,
            source: e.into(),
        }
    }

    /// Get the underlying error.
    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}

impl fmt::Display for CargoResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CargoResolveErrorKind::GlobalContext => {
                write!(f, "failed to create global context: {}", self.source)
            }
            CargoResolveErrorKind::Workspace => {
                write!(f, "failed to create workspace: {}", self.source)
            }
            CargoResolveErrorKind::Resolve => {
                write!(f, "failed to resolve dependencies: {}", self.source)
            }
            CargoResolveErrorKind::CacheLock => {
                write!(f, "failed to acquire package cache lock: {}", self.source)
            }
        }
    }
}

impl std::error::Error for CargoResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
