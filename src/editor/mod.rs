//! Embeddable workflow authoring and live monitoring for Leptos 0.8.
//!
//! Enable `editor-csr`, `editor-hydrate`, or `editor-ssr` to match the host application. The UI
//! uses only engine models; no server or database is required.
//! See `docs/editor.md` for embedding and the runnable browser example.

mod components;
mod declarations;
mod document;
mod monitor;
mod monitor_view;
mod settings;
mod viewport;
pub use monitor::*;
pub use monitor_view::ProcessMonitor;

pub use components::ProcessEditor;
pub use document::{BlockKind, Connection, EditorDocument, Position};

/// Scoped styles, also inserted by [`ProcessEditor`].
pub const EDITOR_CSS: &str = include_str!("editor.css");

/// Optional server-side adapter for the engine administration service.
#[cfg(feature = "admin")]
pub mod admin;

mod publication;
