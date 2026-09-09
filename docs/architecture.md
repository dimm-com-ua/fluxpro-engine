# Architecture and maintenance

The engine separates serializable models from optional integration layers.

| Code | Responsibility | Feature |
| --- | --- | --- |
| `src/models` | Definitions, commands, context, IDs, queue payloads, handler results | Default |
| `src/traits` | Host service and lifecycle handler contract | Default |
| `src/db_models`, `src/db_service` | SQLx row decoding and PostgreSQL persistence | `db` |
| `src/migrations.rs` | Embedded schema migrations and isolated migration history | `db` |
| `src/archive.rs` | Technical-history export and deletion | `db` |
| `src/config`, `src/service` | Runtime defaults, dispatch, signals, and Rhai routing | `runtime` |
| `src/engine` | Handler registry, runtime assembly, queue workers, and shutdown | `runtime` |
| `src/api_handlers`, `src/impls` | Actix Web routes and HTTP error adaptation | `api` |
| `src/admin` | Pagination, inspection, health heuristics, Markdown export, queue operations | `admin` |

## Persistence and execution

Definitions are stored as compiled JSON plus node, stage, signal, and escalation
rows in the `fluxpro` schema. Original source text and version notes can be
retained alongside the compiled definition. An instance binds to one definition
version and has both an application business ID and a generated runtime token.
See [the node reference](nodes.md) for selection and execution constraints.

The queue stores node entry, timeout, and signal tasks. A dequeue atomically
claims one eligible item by locking its instance with `FOR UPDATE SKIP LOCKED`
and checking existing leases again in a separate READ COMMITTED statement.
A single-statement advisory lock cannot refresh an already-taken MVCC snapshot. The
`locked_by` database column is a lease-expiration timestamp despite its name.
Ownership-sensitive renewal, retry, and deletion compare the lease key.
Renewal locks the queue row first and then checks its deadline in a fresh statement;
waiting on a row lock cannot revive an expired lease.

The runner renews active leases, limits worker concurrency, and uses local
notifications to reduce polling latency. Notifications are not distributed;
other application instances rely on fallback polling. A worker completion also
wakes the runner because an already-queued successor can become eligible only
after the previous task releases its instance lease. Shutdown stops admission
and waits for active workers; handlers should avoid unbounded execution.

Node transitions and their durable execution events commit together. Failure to
write a transition event rolls back the transition and retains its source task.
Attempt-start and runner diagnostics remain best-effort writes outside that
transaction. They can describe attempts whose final transition did not commit.


## Atomic transitions and recovery

Startup creates the instance, its context, initial stage/history, and Start
queue item in one transaction. A failed startup cannot leave a partial instance.

For each claimed task, the PostgreSQL adapter briefly locks the instance and
queue row to read a consistent snapshot, verify its lease, and bind an eligible
signal to a wait visit. The handler then runs without an open transaction.
The commit locks the instance and task again, verifies the instance revision
and lease ownership, and writes all transition effects together. Context,
stage/history, node visit, wait completion, durable events, successors, and
source-task deletion or retry release either all commit or all roll back.
An expired or superseded attempt cannot overwrite a newer committed revision.

A wait has a UUID per node visit and an explicit completion flag. Signals
admitted at that wait and timeouts refer to that UUID. The first accepted event
closes the visit; later events cannot create another continuation. Reentering
the same node creates a new UUID. A handler retry keeps its task UUID and visit;
an intentional Repeat creates a fresh task and visit.

Snapshot reads, missing successors, and commit failures propagate to the runner.
The source task remains leased until its lease expires, when it can be reclaimed.
A retry after an uncertain commit outcome cannot commit the same deleted queue
item again. Business-handler failures still use the configured retry/on_error
policy; exhausted failures without an error route create a durable incident.

Heartbeat renewal and task execution are polled independently so a renewal
waiting for the commit's queue-row lock cannot prevent that commit from running.
Lease maintenance has a monotonic deadline, including while renewal is blocked.
The initial deadline starts before dequeue, avoiding host/database clock skew.

These guarantees apply to the runner's `process_task` path. Direct `handle_node`
and individual persistence helpers remain low-level APIs, not substitutes for a
queued atomic transition. Custom database adapters must implement
`load_task_execution`, `commit_task_execution`, and `start_process_instance_atomic`;
the default implementations fail explicitly rather than silently falling back
to non-atomic execution. Adapters must also implement atomic context patches,
event-identity admission, and incident inspection/recovery.

Host handlers and lifecycle hooks run outside the transaction. External side
effects can repeat after a failed or disconnected attempt; use application
idempotency keys for HTTP requests, payments, messages, and similar effects.
Hooks run with the input snapshot and proposed arguments before state is committed.
Handlers should return context patches rather than independently mutating the
instance: direct concurrent mutations change its revision and reject the pending
transition.

Each queued host invocation has a five-minute deadline by default. Configure it
with `FluxProEngine::create_with_handler_timeout` or
`FluxproServiceImpl::with_handler_timeout`. Timeout or an unwinding panic suspends
immediately: the external outcome may be uncertain, so neither automatic retry
nor `on_error.next` is safe. Cooperative async execution is required; blocking
code that never yields and process aborts cannot be preempted by a Tokio timer.

The handler trait's additive `process_node_with_execution` method receives a
stable `operation_id`, task/node identity, phase, revision, and attempt. Its
default calls the legacy method. Hosts must persist idempotency receipts at the
actual side-effect boundary and reject stale revisions for external stage writes.
Passing metadata is not an exactly-once external execution guarantee. Lifecycle
phases have distinct keys, while replay and explicit resume preserve the key.

### Upgrade existing installations

Stop and drain **all** runners before applying migrations `0006`–`0007`, then restart
them using the new code. Old workers do not participate in the revision/visit
protocol and must not execute alongside the new workers during migration.
The migration adds metadata columns without changing queue JSON or definition
formats. Existing current nodes receive visit IDs; their pending timeouts are
bound to those visits. Timeouts known from execution history to predate a newer
entry into the same node receive an obsolete identity. Without historical entry
records, older same-node visits cannot be reconstructed unambiguously.

## Incidents, publication, and event identity

An exhausted ServiceTask without `on_error.next` suspends the whole instance.
Other execution preparation failures (including lifecycle hooks) retry at most
three total attempts with a five-second delay. Rhai condition errors suspend
immediately. SQL snapshot/commit failures continue to retain their source task
for lease recovery; a database outage is not silently converted into a consumed
business failure.

Suspension writes `process_incident`, releases but retains the source queue item,
and marks the instance `suspended` in the same fenced transaction. No pending
signal, timer, or node for that instance is dequeued while suspended. A failed
condition discards all proposed effects, including a signal's context patch and
wait completion. Incidents are separate from archivable execution logs.

Use `FluxproService::get_open_incident(token)` to inspect the reason, task payload,
and attempt. After fixing the handler or input context, call
`resume_instance(token, incident.uuid)`. This operation atomically resolves only
that incident, resets the source task's attempts, and wakes the local runner.
Duplicate or stale resume commands return false. The retained task keeps its UUID
and executes before other queued work; generic queue cancellation cannot delete
an unresolved or recovering incident task. `FluxproAdminService` exposes the same
inspection/recovery operations, using polling to wake runners on other instances.

Published definitions cannot be edited or deleted, including their child rows.
Registration serializes by process key and enforces a unique `(key, version)`.
Creating a definition with `status: active` publishes it atomically; `draft`
versions are stored but cannot start, including explicit version requests.
Use a new version for behavioral changes. Runtime selects the latest registered
active version within its dates; existing instances continue their original
version after deprecation. An omitted effective date uses registration time.

Migration `0007` refuses existing duplicate `(key, version)` pairs rather than
choosing or deleting a version. Before upgrading, inspect duplicates with:

```sql
SELECT key_, version, count(*)
FROM fluxpro.process_definition
GROUP BY key_, version HAVING count(*) > 1;
```

Resolve ambiguous identities with an explicit data migration that preserves
instance bindings, then retry the schema upgrade. Existing definitions remain
readable; invalid expressions in already published versions produce
incidents instead of silently choosing fallback. Newly registered definitions
must pass the stricter validator. The legacy `ctx._last_signal` spelling is
normalized to bracket access outside string literals and comments, so existing
valid routes keep working without rewriting persisted definitions.

`PostSignal` JSON remains compatible with requests omitting the new `event_id`
and `wait_visit_id` fields. Rust struct literals must add these optional fields
(or use deserialization). Requests without an ID are independent deliveries;
producers requiring deduplication must persist and reuse their event IDs. Equal
content with different IDs is allowed. Identity conflicts fail without updating
history, receipts, or the queue. See [signal examples](nodes.md#timeouts-and-signals).

## Administration and archival

Administration APIs return normalized pages, derived instance states, context,
node/stage metadata, and execution history. Queue retry and cancellation only
affect unlocked or expired tasks. Health audits combine incident severity and
node-age thresholds; a "stuck" issue is a heuristic, not proof of deadlock.
Markdown exports provide a portable report of an instance and its history.

Archival is a host-managed two-step process: fetch an ordered batch, persist and
verify it externally, then delete exactly the archived row IDs before the
cutoff. `delete_archived` cannot verify external storage itself. Signal history
still retains the latest signal for an instance whose stage is not final. Event
deduplication now uses `signal_receipt`, which archival never deletes. Fetch limits are clamped to 1–1,000.

## Comment and documentation style

Use English throughout source comments, Rustdoc, fixtures, and project guides.
Use `//!` for module purpose and `///` for public types, variants, fields, traits,
and methods. Start with a concise sentence describing behavior. Explain timing,
units, defaults, error conditions, serialization, and limitations where relevant.
Use backticks for identifiers and `# Examples` / `# Errors` for substantial
Rustdoc sections. Avoid repeating method signatures or listing derived traits.

Use `//` inside implementations to explain non-obvious invariants or choices,
particularly queue ownership, transaction boundaries, context merging, and
signal ordering. Do not narrate obvious assignments or preserve disabled code
as comments. Tests should have descriptive behavior names; add a comment only
when the scenario needs explanation.

Keep examples executable. Default-feature examples must not require optional
integrations; declare `required-features` in `Cargo.toml` for examples that do.
Use `no_run` for Rustdoc examples requiring a database and runnable doctests for
pure model behavior. Document reserved fields and current behavior explicitly.

Applied SQL migrations are immutable because SQLx tracks their checksums.
Document existing schema behavior here or in the persistence API rather than
editing old migration files solely to add comments.

## Validation

```bash
cargo fmt --all -- --check
cargo test
cargo test --all-features
cargo check --all-features --examples
cargo rustdoc --all-features --lib -- -D missing-docs -D rustdoc::broken_intra_doc_links -D rustdoc::invalid_rust_codeblocks
```

The test suite validates the shipped definitions and the YAML node snippets in
this reference. Pure examples can also be run using [the example commands](examples.md).
Run the PostgreSQL integration suite against a test server whose user can create
and drop databases:

```bash
DATABASE_URL=postgres://localhost/postgres cargo test --all-features --test durable_transitions -- --ignored --test-threads=4
```

SQLx gives each test a separate database. The suite injects SQL failures,
terminates a test backend during commit, races workers and wait events, checks
lease/revision fencing, and tests migration of existing timers. It is ignored
in the database-free default run and explicitly required by CI's PostgreSQL job.
The PostgreSQL workflow demo is also an integration exercise; compilation alone
does not establish database execution correctness.
