//! # audit-parser
//!
//! Parse `cargo audit` output and provide fast indexed lookups.
//!
//! This crate runs `cargo audit` and builds an index for O(1)
//! lookups by (crate_name, version).
//!
//! ## Overview
//!
//! The `audit-parser` crate complements `toml-parser` and `cargo-parser`:
//! - **toml-parser**: Parses Cargo.toml (desired state with positions)
//! - **cargo-parser**: Resolves dependencies via cargo (actual state)
//! - **audit-parser**: Runs cargo audit (security vulnerabilities)
//!
//! ## Example
//!
//! ```ignore
//! use audit_parser::AuditIndex;
//! use std::path::Path;
//!
//! // Run cargo audit and parse results
//! let index = AuditIndex::audit(
//!     Path::new("/path/to/Cargo.lock"),
//!     &["my-workspace-member"],
//!     "cargo",
//! ).await?;
//!
//! // O(1) lookup by (name, version)
//! if let Some(issues) = index.get("serde", "1.0.0") {
//!     for issue in issues {
//!         println!("{}: {}", issue.id, issue.title);
//!     }
//! }
//!
//! // O(1) lookup by name only
//! if let Some(issues) = index.get_by_name("tokio") {
//!     println!("Found {} issues for tokio", issues.len());
//! }
//!
//! // Check if a crate has issues
//! if index.has_issues("crossbeam-channel", "0.5.13") {
//!     println!("crossbeam-channel 0.5.13 has security issues!");
//! }
//! ```
//!
//! ## Complexity
//!
//! | Operation | Complexity |
//! |-----------|------------|
//! | `audit()` | O(n) + process I/O |
//! | `get(name, version)` | O(1) |
//! | `get_by_name(name)` | O(1) |
//! | `iter()` | O(n) |

mod error;
mod index;
mod issue;
mod parser;

pub use error::AuditError;
pub use index::{AuditIndex, AuditLookupKey};
pub use issue::{AuditIssue, AuditKind};
pub use parser::parse_audit_output;
