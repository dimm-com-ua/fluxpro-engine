# FluxPro Engine

`fluxpro-engine` is a feature-gated workflow and process engine for Rust.

The package is intentionally self-contained and can be copied to or cloned as
its own repository:

```text
https://github.com/dimm-com-ua/fluxpro-engine
```

The default build contains only process models, commands, context values, and
the service-handler contract. Enable only the integration layers an
application needs:

- `db` — PostgreSQL persistence through SQLx.
- `runtime` — process execution and the asynchronous task runner (includes
  `db`).
- `api` — Actix Web HTTP handlers (includes `runtime`).
- `admin` — read and safely manage process definitions, instances, contexts,
  stage history, and queued tasks (includes `db`).
- `full` — all backend integrations (kept compatible; does not enable the UI).
- `editor` — Leptos process authoring and monitoring components (Rust 1.88+).
- `editor-csr`, `editor-hydrate`, `editor-ssr` — editor plus the corresponding
  Leptos rendering mode. Choose one mode per build target.

```toml
[dependencies]
fluxpro-engine = "0.1"

# Or, for a service running the engine:
fluxpro-engine = { version = "0.1", features = ["runtime"] }
```

The crate has no dependency on the Lendiq application or its workspace crates.

## Visual process editor

The [`editor` module](docs/editor.md) exports embeddable `ProcessEditor` and
`ProcessMonitor` Leptos 0.8 components from this package:

```toml
fluxpro-engine = { version = "0.1.5", features = ["editor-csr"] }
```

```rust,ignore
use fluxpro_engine::editor::{EditorDocument, ProcessEditor, ProcessMonitor};
```

The editor provides drag-and-drop authoring, YAML import/export, layout,
undo/redo, and live instance monitoring. Add `admin` to `editor-ssr` for the
server-side administration adapter. Leptos and browser dependencies remain
optional; the default and backend `full` builds do not enable them.

## Documentation and examples

- [Node reference](docs/nodes.md): all six node types, fields, routing, signals,
  timeouts, retries, lifecycle hooks, and current runtime limitations.
- [Runnable examples](docs/examples.md): JSON/YAML validation, a service handler,
  a PostgreSQL workflow, and HTTP integration.
- [Business workflow catalog](docs/business/README.md): seven definitions and 23
  executable scenarios for delivery, repairs, education, lending, retail,
  insurance, and subscriptions.
- [Loan lifecycle](docs/business/loan-lifecycle.md): scoring, funding, repayment
  schedules, daily accrual, payment reconciliation, and verified closure.
- [Architecture and documentation style](docs/architecture.md): module
  responsibilities, persistence, queue execution, administration, and checks.

Validate a definition without a database:

```bash
cargo run --example validate_process
cargo run --features api --example validate_yaml
```

The [approval workflow](examples/definitions/approval.yaml) combines `Start`,
`ServiceTask`, `UserTask`, `Gateway`, `Wait`, and `End`. Generate the complete
Rust API reference with `cargo doc --features full,migration-repair,editor-ssr --no-deps`.

## Publishing

The repository is released through GitHub Releases. After creating a release
whose tag matches the crate version, the `Publish` workflow publishes the
package using the repository's `CARGO_REGISTRY_TOKEN` secret.

To validate a release locally:

```bash
cargo fmt --all -- --check
cargo test --features full,migration-repair,editor-ssr
cargo package
```

The crate's `Cargo.toml`, source, migrations, fixtures, licenses, and CI files
are all contained in this directory; no parent workspace files are required.

## Database schema

All engine-owned tables live in the dedicated `fluxpro` PostgreSQL schema.
Applications can apply the migrations embedded in the crate:

```rust,no_run
async fn migrate(pool: &sqlx::PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    fluxpro_engine::migrations::migrate(pool).await
}
```

For legacy databases blocked by duplicate version identities in migration 7,
enable `migration-repair` and use
`migrations::legacy_versions::{plan_legacy_versions, apply_legacy_version_plan}`.
This opt-in preview/apply API preserves definition UUIDs and instance bindings;
ordinary migrations never choose an ambiguous identity automatically.
See [the repair procedure](docs/legacy-version-repair.md) before upgrading.

The engine always uses explicitly qualified table names and does not modify or
depend on the connection's `search_path`.

## Administration

Enable `admin` to build an administration UI without coupling the engine to a
web framework:

```rust,no_run
use fluxpro_engine::admin::{
    FluxproAdminService, PageRequest, ProcessInstanceFilter,
};

async fn list_instances(pool: sqlx::PgPool) -> Result<(), fluxpro_engine::admin::AdminError> {
    let admin = FluxproAdminService::new(pool);
    let instances = admin
        .list_process_instances(ProcessInstanceFilter::default(), PageRequest::default())
        .await?;
    println!("{} instances", instances.total);
    Ok(())
}
```

Administrative queue retry and cancellation only affect tasks that do not have
a live lease. This prevents an operator action from racing an active worker.

## Queue runner

The runtime runner uses an atomic PostgreSQL dequeue with `FOR UPDATE SKIP
LOCKED`, so multiple application instances can compete for work without
serializing or executing the same lease concurrently. Active workers renew
their leases with a heartbeat; an abandoned lease is automatically available
to another instance after `task_lease_ms`.

Tasks committed by a handler wake a runner in the same engine immediately.
`idle_backoff_ms` remains the fallback polling interval for work committed by
another application instance.

For reliable failover, keep `heartbeat_interval_ms` comfortably below
`task_lease_ms`. The defaults are 20 seconds and 60 seconds respectively.

## Durable transitions

Queued execution commits context, node/stage state, wait completion, successor
tasks, and source-task acknowledgement in one PostgreSQL transaction. Lease and
instance-revision checks reject stale attempts. A signal and a timeout can close
a particular wait visit only once, including when the process revisits a node.
Database failures retain the source task for recovery after lease expiry.

Host handlers execute outside the transaction, so external effects still need
application idempotency. See [recovery and migration details](docs/architecture.md#atomic-transitions-and-recovery).
Stop and drain all old runners before applying migrations `0006`–`0007`.

Exhausted execution failures suspend the instance with a retained task and a
saved incident. Inspect it with `get_open_incident` and explicitly resume that
incident with `resume_instance`. Invalid routing expressions cannot select a
fallback. Handler patches persist removals atomically. Only active, effective
versions may start; published versions are immutable and `(key, version)` is
unique. Signals accept an optional stable `event_id` for durable deduplication
and `wait_visit_id` for targeting a particular wait visit. See the
[upgrade and recovery contract](docs/architecture.md#incidents-publication-and-event-identity).
