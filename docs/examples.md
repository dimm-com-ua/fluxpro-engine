# Examples

Run these commands from the repository root. The first three examples need no
PostgreSQL server and do not change external state.

| Command | Demonstrates |
| --- | --- |
| `cargo run --example validate_process` | Parsing, validation, and JSON compilation with default features |
| `cargo run --features api --example validate_yaml` | Validation of a YAML workflow containing every node type |
| `cargo run --features runtime --example service_handler` | A host handler returning a typed context patch |
| `cargo run --features api --example run_workflow` | Migrations, definition registration, runner startup, signals, and shutdown |
| `cargo run --features api --example business_workflows -- list` | Lists 23 business scenarios without a database |
| `cargo run --features api --example business_workflows -- validate` | Validates seven business definitions without a database |
| `cargo run --features api --example business_workflows -- run pizza_delivery_refund` | Executes a scenario in disposable PostgreSQL with simulated integrations |

YAML parsing uses the optional `serde_yaml` dependency currently enabled by the
`api` feature. Applications that only need model parsing may instead use their
own YAML parser with the default model types.

## Run the complete workflow

Set `DATABASE_URL` to a PostgreSQL database in which you want to create the
`fluxpro` schema and example records, then run:

```bash
cargo run --features api --example run_workflow
```

The example applies embedded migrations, creates a definition, starts an
instance with a generated business ID, and runs `prepare_request`. It sends
`approved` when the review form node is current, then `confirmed` when the
technical Wait is current. It waits for the successful End before requesting
runner shutdown. It polls with a 30-second execution timeout; shutdown then
waits for active workers to finish. Definition, instance, context, and history
records remain in the database for inspection.

The definition declares a form but no form lifecycle callbacks. It therefore
demonstrates routing without a graphical UI. A real host renders its UI through
`on_show_form` and `on_hide_form` implementations.

## Implement and register a handler

See [the handler implementation](../examples/support/mod.rs),
[local handler execution](../examples/service_handler.rs), and
[runner setup](../examples/run_workflow.rs).

A handler implements `get_name` and asynchronous `process_node`. Match the
registered name to the definition's `handler` field. On success, return
`HandleNodeResult::success_with_patcher`; on a failed attempt, return an error
or a failure outcome. Use `repeat(at)` for intentional rescheduling rather than
failure retry. Consult [the node reference](nodes.md#servicetask) for each
outcome's behavior.

Queued execution also calls `process_node_with_execution`, whose default delegates
to `process_node`. Override it to use the stable operation ID and instance revision
at the external write boundary. The [business stepper](../examples/support/business.rs)
demonstrates this API with an explicitly non-durable simulation cache. Production
receipts and side effects need the atomicity described in the
[business integration contract](business/README.md#shared-integration-contract).

## Business scenarios

The [catalog](business/README.md) provides complete definitions, handler contracts,
startup data, exact event sequences, and expected node paths for pizza delivery,
home repairs, English school enrolment, the loan lifecycle, retail returns,
insurance claims, and subscription renewal. Its 23 PostgreSQL scenarios cover
success, rejection, reminders, compensation, and repeated servicing visits.

Start with `business_workflows -- validate`, then follow the catalog's disposable
database instructions. These examples simulate host services; they neither call
payment providers nor implement interest calculations. Existing signal-delivery
limitations remain documented in the [verification report](process-verification.md).

## HTTP integration

Enable `api` and register `api_handlers::config::config` with Actix Web.
The current extractors require `web::Data<Arc<FluxProEngine>>` for definition
creation and startup, and `web::Data<FluxProEngine>` for signal posting.
Both can reference the same engine:

```rust,no_run
use actix_web::{web, App};
use fluxpro_engine::engine::fluxpro_engine::FluxProEngine;
use std::sync::Arc;

fn app_data(engine: Arc<FluxProEngine>) {
    let _app = App::new()
        .app_data(web::Data::new(engine.clone()))
        .app_data(web::Data::<FluxProEngine>::from(engine))
        .configure(fluxpro_engine::api_handlers::config::config);
}
```

| Method and path | Request | Success response |
| --- | --- | --- |
| `PUT /fluxpro/process_definitions/create` | UTF-8 YAML definition | JSON with `status` and persisted definition `process_id` |
| `POST /fluxpro/process/{process_id}/start` | JSON startup command; URL ID is a definition key | JSON `process_instance_id` containing the runtime token |
| `POST /fluxpro/instance/{process_id}/post_signal` | JSON signal; URL ID is the runtime token | Empty HTTP 200 |

Example startup body:

```json
{
  "process_id": "approval_request_1",
  "version": "1.0.0",
  "context": { "amount": { "number": 100 } }
}
```

Example signal body:

```json
{
  "event_id": "approval-request-1-reviewed",
  "signal": "approved",
  "context": { "reviewer": { "string": "Ada" } }
}
```

The definition endpoint reports parsing and creation errors as HTTP 400.
Startup and signal errors currently use the shared HTTP 500 adapter. The host
application supplies authentication, authorization, rendering, and server
startup; the crate registers routes only.

## Inspect and resume a suspended instance

Apply migrations during deployment. Inspect the saved failure before changing
anything; after fixing its cause, pass the reviewed incident UUID to resume:

```bash
DATABASE_URL=postgres://localhost/workflow cargo run --features runtime --example recover_incident -- INSTANCE_TOKEN
DATABASE_URL=postgres://localhost/workflow cargo run --features runtime --example recover_incident -- INSTANCE_TOKEN REVIEWED_INCIDENT_UUID
```

The [recovery example](../examples/recover_incident.rs) does not apply migrations
or run handlers. It releases the retained task to existing runners; a stale or
already resolved incident returns `false`. Do not generate a new signal event ID
when retrying the same producer event. Omit `event_id` only when every submission
should be treated as an independent delivery.

## Submitted production process review

The reviewed [Cash Loan](../examples/definitions/reviewed/cash_loan.yaml) and
[TK Online](../examples/definitions/reviewed/tk_online.yaml) definitions are
proposed new versions, not published deployments. Read the
[verification report](process-verification.md) before using them.
