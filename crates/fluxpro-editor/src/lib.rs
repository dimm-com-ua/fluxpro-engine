//! Embeddable workflow authoring and live monitoring for Leptos 0.8.
//!
//! Enable `csr`, `hydrate`, or `ssr` to match the host application. The engine
//! dependency uses only its models; no server or database is required.
//! See the crate README for embedding and the runnable browser example.

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
