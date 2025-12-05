//! # cargo-parser
//!
//! Resolve cargo dependencies and provide fast indexed lookups.
//!
//! This crate runs cargo's resolution process and builds an index for O(1)
//! lookups by composite key (table, platform, name).
//!
//! ## Overview
//!
//! The `cargo-parser` crate complements `toml-parser`:
//! - **toml-parser**: Parses Cargo.toml (desired state with positions)
//! - **cargo-parser**: Resolves dependencies via cargo (actual state)
//!
//! Use `DependencyLookupKey` to map between them.
//!
//! ## Data Model
//!
//! For each dependency declared in Cargo.toml, `CargoIndex` provides:
//! - `package`: The resolved/installed Package (if installed)
//! - `available_versions`: All versions from registry (for completion/hover)
//! - `latest_matched_summary`: Latest version compatible with version requirement
//! - `latest_summary`: Absolute latest version (may be incompatible)
//!
//! ## Example
//!
//! ```ignore
//! use cargo_parser::{CargoIndex, DependencyLookupKey, DependencyTable};
//! use std::path::Path;
//!
//! // Resolve dependencies
//! let index = CargoIndex::resolve(Path::new("Cargo.toml"))?;
//!
//! // O(1) lookup by composite key
//! let key = DependencyLookupKey::new(DependencyTable::Dependencies, None, "serde");
//! if let Some(resolved) = index.get(&key) {
//!     if let Some(pkg) = &resolved.package {
//!         println!("Installed: {}", pkg.version());
//!     }
//!     if resolved.has_compatible_upgrade() {
//!         println!("Upgrade available!");
//!     }
//! }
//! ```
//!
//! ## Complexity
//!
//! | Operation | Complexity |
//! |-----------|------------|
//! | `resolve()` | O(n) + network I/O |
//! | `get()` by key | O(1) |
//! | `iter()` | O(n) |

mod error;
mod index;
mod query;

pub use error::CargoResolveError;
pub use index::CargoIndex;
pub use query::{dep_kind_to_table, DependencyLookupKey, DependencyTable, ResolvedDependency};

// Re-export cargo types for convenience
pub use cargo::core::{Package, Summary};
