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
- `full` — all currently available integrations.

```toml
[dependencies]
fluxpro-engine = "0.1"

# Or, for a service running the engine:
fluxpro-engine = { version = "0.1", features = ["runtime"] }
```

The crate has no dependency on the Lendiq application or its workspace crates.

## Publishing

The repository is released through GitHub Releases. After creating a release
whose tag matches the crate version, the `Publish` workflow publishes the
package using the repository's `CARGO_REGISTRY_TOKEN` secret.

To validate a release locally:

```bash
cargo fmt --all -- --check
cargo test --all-features
cargo package
```

The crate's `Cargo.toml`, source, migrations, fixtures, licenses, and CI files
are all contained in this directory; no parent workspace files are required.

## Database schema

All engine-owned tables live in the dedicated `fluxpro` PostgreSQL schema.
Applications can apply the migrations embedded in the crate:

```rust,no_run
fluxpro_engine::migrations::migrate(&pool).await?;
```

The engine always uses explicitly qualified table names and does not modify or
depend on the connection's `search_path`.

## Administration

Enable `admin` to build an administration UI without coupling the engine to a
web framework:

```rust,no_run
use fluxpro_engine::admin::{
    FluxproAdminService, PageRequest, ProcessInstanceFilter,
};

let admin = FluxproAdminService::new(pool);
let instances = admin
    .list_process_instances(ProcessInstanceFilter::default(), PageRequest::default())
    .await?;
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
