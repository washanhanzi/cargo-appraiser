//! Event handlers for the Appraiser controller.
//!
//! This module contains extracted event handlers from the main Appraiser loop,
//! organized by functionality for better maintainability.

mod audit;
mod context;
mod diagnostic;
mod document;
mod lsp_features;
mod resolve;

pub use audit::handle_audited;
pub use context::AppraiserContext;
pub use diagnostic::handle_cargo_diagnostic;
pub use document::{handle_changed, handle_closed, handle_opened_saved, handle_parse};
pub use lsp_features::{handle_code_action, handle_completion, handle_gd, handle_hover};
pub use resolve::{handle_cargo_lock_changed, handle_cargo_resolved, handle_ready_to_resolve};
