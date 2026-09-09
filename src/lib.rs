//! Workflow definitions, typed context values, and optional execution integrations.
//!
//! The default build exposes models and the service-handler contract. Enable `db`
//! for PostgreSQL persistence, `runtime` for execution, `api` for Actix Web routes,
//! `admin` for inspection and maintenance, or `full` for all backend integrations.
//! Enable `editor` for Leptos authoring and monitoring components (Rust 1.88+).
//!
//! Start with [`models::process_def::ProcessDefinition`] and
//! [`traits::node_handlers::service_node_handler::FluxproServiceHandler`].
//! The repository's `docs/nodes.md` describes every node and its runtime behavior.

#[cfg(feature = "editor")]
pub mod editor;

pub mod models;
pub mod traits;

#[cfg(feature = "admin")]
pub mod admin;
#[cfg(feature = "api")]
pub mod api_handlers;
#[cfg(feature = "db")]
pub mod archive;
#[cfg(feature = "runtime")]
pub mod config;
#[cfg(feature = "db")]
pub mod db_models;
#[cfg(feature = "db")]
pub mod db_service;
#[cfg(feature = "runtime")]
pub mod engine;
#[cfg(feature = "api")]
pub mod impls;
#[cfg(feature = "db")]
pub mod migrations;
#[cfg(feature = "runtime")]
pub mod service;
