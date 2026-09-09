//! Embedded, versioned migrations for the dedicated `fluxpro` schema.

use sqlx::Executor;
use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

/// Embedded database migrations required by Fluxpro.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Creates or updates the dedicated `fluxpro` PostgreSQL schema.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    let mut connection = pool.acquire().await?;
    connection
        .execute("create schema if not exists fluxpro")
        .await?;
    // Keep the crate's migration history isolated from the host application's
    // `_sqlx_migrations` table. This also prevents version collisions when the
    // engine is published and embedded into another project.
    connection
        .execute("set search_path to fluxpro, public")
        .await?;
    MIGRATOR.run(&mut *connection).await
}
