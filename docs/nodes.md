# Workflow node reference

This reference describes the schema in `src/models/process_def.rs` and the
behavior in `src/service/process_service.rs`. JSON and YAML use the same fields.
Node `type` values are case-sensitive.

| Type | Purpose | How it advances |
| --- | --- | --- |
| `Start` | Enter the workflow | Queues its required `next` node |
| `End` | Finish a workflow path | Invokes the completion hook; no successor |
| `ServiceTask` | Execute host application logic | Uses the handler outcome, retries, and `next` |
| `UserTask` | Present a host-rendered form | Accepts a configured signal or takes a timeout |
| `Wait` | Wait without presenting a form | Accepts a configured signal or takes a timeout |
| `Gateway` | Select a conditional path | Queues one branch or the fallback |

The [approval example](../examples/definitions/approval.yaml) combines all six
types. The [minimal JSON example](../examples/definitions/minimal.json) contains
only Start and End. Commands for validating and executing them are in the
[examples guide](examples.md).

For complete domain workflows, see the [business catalog](business/README.md):
seven definitions and 23 executable scenarios. The [loan guide](business/loan-lifecycle.md)
explains repeated Wait visits, reconciliation timers, and the boundary between
workflow context and an authoritative ledger.

## Definition prerequisites

A definition requires `key`, `name`, `version`, `status`, and `nodes`.
`version` accepts `1.2.3` or `v1.2.3` and serializes as `1.2.3`; status uses
`draft`, `active`, or `deprecated`. IDs match `[a-zA-Z_][a-zA-Z0-9_]*`.
Hyphens, dots, spaces, and a leading digit are invalid in IDs.

Declare referenced `signals`, `forms`, `stages`, and escalation topics at the
root. Each declaration category has its own uniqueness check. Signals and
stages support compact ID forms as well as object forms. Form declarations
contain `id` and `roles`. Escalation declarations describe host-rendered actions;
the engine does not render buttons or automatically emit their signals.

Exactly one object stage must have `is_initial: true`. Omitted `effective_from`
is assigned the registration time. Runtime selects only `active` versions within
their effective/deprecation dates, ordered by insertion index, optionally
restricted to an explicit version. It does not sort semantic version strings.
`(key, version)` is unique. Published definitions and their node/stage/signal
rows are immutable; register a new version to change behavior. Deprecation stops
new starts without changing the version bound to existing instances.

`validate()` requires exactly one Start, at least one End, and exactly one initial
stage. It rejects duplicate IDs, broken references, empty signal lists, malformed
or nonpositive relative timeouts/backoffs, and invalid effective-date ranges.
With `runtime`, it also compiles every condition before registration. `compile()`
performs the same checks and emits JSON. The model-only feature cannot check Rhai
syntax. `unreachable_nodes()` reports nodes without a path from Start; registration
logs these diagnostics rather than rejecting auxiliary nodes that hosts explicitly
enqueue. Registered host handlers are checked when execution invokes them.

## Common fields and execution order

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | Yes | Unique node ID within the definition |
| `type` | Yes | One of the six names above |
| `set_stage` | No | Declared stage ID, or an object with `stage` and optional `reason` |

For example, `set_stage: reviewing` changes the business stage. The object form
`set_stage: { stage: reviewing, reason: "Review requested" }` also passes the
reason to `on_stage_change`. Queued execution persists the stage and reason in
the same transaction as the node transition.

When processing a queued node, the engine reads a consistent instance snapshot,
invokes the previous form's hide callback, invokes stage and node callbacks,
and prepares the resulting changes. It then commits the current node, stage,
context, events, successors, and queue completion in one transaction. Callbacks
run before these changes become visible in PostgreSQL and may repeat if an
attempt fails. Initial-stage assignment during startup does not invoke
`on_stage_change`.
A committed End leaves that End as the persisted current node. Business-final
stages (`is_final: true`) also matter to administration and archival; use a
final stage on terminal nodes when that is the desired business state.

Successors are queued, not executed inline. The runner serializes work for one
instance while allowing different instances to run concurrently. Lease expiry
or retry can execute a handler again, so external effects need an application
idempotency strategy.

## Start

Start identifies the entry point and immediately queues `next`. A definition
must contain exactly one Start. It has no handler, signal wait, or timeout.

| Additional field | Required | Meaning |
| --- | --- | --- |
| `next` | Yes | Direct successor node ID |

```yaml
id: start
type: Start
set_stage: created
next: prepare
```

## End

End queues no successor. If configured, `on_process_complete` receives
`end_node_id` as a typed string argument. An End does not cancel every other
queued item automatically. Signals processed with an End current are not
deferred, and timeout events for other nodes do not advance that End.

There are no additional fields.

```yaml
id: completed_end
type: End
set_stage: completed
```

## ServiceTask

ServiceTask finds a host handler by the `handler` ID and calls
`FluxproServiceHandler::process_node` with the runtime token, current context,
and optional typed arguments. Register handlers before starting the runner.

| Additional field | Required | Meaning |
| --- | --- | --- |
| `handler` | Yes | Registered handler ID |
| `args` | No | `ContextMap` passed unchanged to the handler |
| `next` | No | Direct successor or conditional routes |
| `retries` | No | `{ max, backoff }` for failed queue attempts |
| `on_error` | No | Error routing metadata; only `next` is executed |

```yaml
id: prepare
type: ServiceTask
handler: prepare_request
args:
  channel: { string: demo }
retries:
  max: 2
  backoff: PT5S
next: review
on_error:
  next: failed
```

Handler results have these effects:

| Outcome | Effect |
| --- | --- |
| `Success(patch)` | Persists ordered sets/removals atomically with `next` |
| `Failure` | Produces an execution error eligible for retry |
| `IllegalState(message)` | Produces an execution error eligible for retry |
| `Repeat(at)` | Queues a new execution of the same node at `at`; does not follow `next` |
| `HandlerNotExists` | Queued execution treats this as a handler failure |
| Returned `Err(...)` | Produces an execution error eligible for retry |

An unregistered handler and a `HandlerNotExists` result are both errors in
queued execution. If `next` is absent after success, the
engine queues nothing and the service node remains current.

`retries.max` counts retries after the initial attempt. With `max: 2`, normal
processing permits three attempts. `backoff` is a fixed ISO 8601 duration, not
exponential backoff. After retries are exhausted, `on_error.next` is queued if
present; otherwise the instance becomes `suspended`, with its source task and
a durable incident retained for explicit recovery. `on_error.compensate` is validated as a node reference but is
not executed. `Repeat` creates a fresh queue item and does not consume this
failed-attempt retry limit.

Retries and error routing belong to `process_task`; a direct call to
`handle_node` does not provide them.

## UserTask

UserTask exposes a declared form through optional host callbacks. The engine
itself does not render a form or authorize its roles. It waits for any accepted
signal, or for a configured timeout. Signal payloads update the context before
`next` is selected.

| Additional field | Required | Meaning |
| --- | --- | --- |
| `form` | Yes | ID declared in root `forms` |
| `wait_for` | No | `{ signal: id }` or `{ signals: [id, ...] }` |
| `timeout` | No | Deadline and `on_timeout` target |
| `next` | No | Successor after an accepted signal |
| `on_error` | No | Parsed and reference-validated, but not executed for UserTask |

```yaml
id: review
type: UserTask
form: approval_form
set_stage: reviewing
wait_for:
  signals: [approved, rejected]
timeout:
  after: PT15M
  on_timeout: expired
next: decision
```

On entry, `on_show_form` receives `form_id` as an ID value and `roles` as an array
of ID values. Before entering a subsequent queued node, `on_hide_form` receives
`form_id`. With neither `wait_for` nor `timeout`, the node remains waiting;
`next` alone does not advance it. A timeout-only user task is allowed.

A relative timeout starts on entry into this node, not when the user opens a
page. To start timing after a page-open signal, first use an untimed UserTask
that accepts that signal, then transition to a second timed UserTask. See the
[signal timeout fixture](../tests/fixtures/tk_online.yaml).

## Wait

Wait accepts signals without presenting a new form or calling form hooks for
itself. A preceding UserTask is still hidden during the transition into Wait.

| Additional field | Required | Meaning |
| --- | --- | --- |
| `wait_for` | Yes | One signal or a list accepting any one signal |
| `timeout` | No | Deadline and `on_timeout` target |
| `next` | No | Successor after an accepted signal |

```yaml
id: await_confirmation
type: Wait
wait_for:
  signal: confirmed
timeout:
  after: PT1H
  on_timeout: expired
next: completed_end
```

Wait supports neither a form nor service-task retry/error routing. A list of
signals means **any one**, not all. Omitting `next` leaves the current node in
place after a signal is accepted and its timeout is canceled, but its visit is
closed and cannot accept another signal. A new entry into the node creates a
new visit.

## Gateway

Gateway evaluates ordered Rhai boolean expressions against the current context
and selects at most one successor.

| Additional field | Required | Meaning |
| --- | --- | --- |
| `gateway` | Yes | `XOR`; retain this explicit field in existing definitions |
| `branches` | No | Ordered `{ when, next }` objects; defaults to an empty list |
| `next` | No | Fallback node ID |

```yaml
id: decision
type: Gateway
gateway: XOR
branches:
  - when: 'ctx["_last_signal"] == "approved"'
    next: await_confirmation
next: rejected_end
```

`XOR` selects the first expression that evaluates to `true` and uses `next` if
no branch is selected. If there is no selected branch and no fallback,
execution retries and then suspends. A condition compilation/evaluation error
suspends immediately, retaining the task and selecting no fallback. An absent
context key or a non-boolean result is also an error. Guard optional keys with
`ctx.contains("optional") && ctx.optional`.

The definition format remains `type: Gateway` with an explicit `gateway: XOR`,
ordered `branches`, and an optional fallback `next`. Existing XOR definitions
require no schema changes. The former `AND` mode is no longer supported;
definitions specifying `gateway: AND` are rejected during deserialization.

Use expressions such as `ctx.amount >= 100` or
`ctx["_last_signal"] == "approved"`. Access keys beginning with an underscore
using brackets. The engine also accepts legacy `ctx._last_signal` by normalizing
it before Rhai compilation, without changing stored expression text. This is Rhai,
not CEL. Primitive context
wrappers are unwrapped; arrays are converted recursively, and dates and
timestamps become strings. An Object is passed as an opaque `serde_json::Value`,
not recursively converted into a Rhai map: do not assume nested expressions
such as `ctx.customer.name` work for it.

Prefer double-quoted strings inside the expression. The compatibility normalizer
converts single-quoted strings and the legacy reserved property outside quoted
literals/comments, preserving escaped quotes. Each evaluation has a 50,000-operation limit and configured
expression depth limits of 64/32. There are no host service-handler calls in the
expression scope.

## Shared successor routes

`ServiceTask.next`, `UserTask.next`, and `Wait.next` accept either a node ID or
an object with `branches` and `default`. Object routes always use XOR rules,
regardless of other Gateway nodes in the process.

This alternate UserTask demonstrates routing without a separate Gateway:

```yaml
id: review
type: UserTask
form: approval_form
wait_for:
  signals: [approved, rejected]
next:
  branches:
    - when: 'ctx["_last_signal"] == "approved"'
      next: await_confirmation
  default: rejected_end
```

The first true branch wins. `branches` defaults to empty; `default` is optional,
but no match without a default is an execution error. Gateway fallbacks use the
field `next`, while these route objects use `default`.

## Timeouts and signals

A timeout requires `on_timeout`, a **node ID**, and either `after` (an ISO 8601
duration such as `PT15M`) or `at` (a UTC timestamp such as
`2027-01-01T12:00:00Z`). If both are supplied, `at` wins. A past timestamp makes
the queued event immediately eligible. Missing or malformed timing fails at
execution, not definition validation. There is no cron timer support.

Timeouts are durable queue events and can execute after their deadline when
workers are busy. A signal or timeout atomically closes one node visit and
queues its continuation. An accepted signal cancels the visit's remaining
timeouts in that same transaction. Other events bound to the closed visit are
consumed without advancing the process, even before its successor executes.
Timeouts carry a visit ID, so a timer from an earlier visit cannot advance a
later visit to the same node. Timeout routing does not emit a signal or set
`_last_signal`.

Accepted signals merge their typed payload into context and set
`ctx["_last_signal"]` to the signal name before routing. Signals arriving before
a compatible wait are deferred according to the delivery policy, up to 26 hours
by default. Admission binds signals to a currently compatible wait visit;
early signals bind when they first encounter a compatible wait. Signals bound
to a completed or earlier visit are discarded rather than deferred into a new
visit. Signals reaching an End are not deferred. Expired signals are logged
and consumed.

Signal submission accepts optional `event_id` and `wait_visit_id` fields:

```json
{
  "event_id": "request-42-approved",
  "wait_visit_id": "3c8b8db5-c746-4915-b65b-2c56da8a1d76",
  "signal": "approved",
  "context": { "approved": { "boolean": true } }
}
```

Use a stable producer `event_id` (1–256 characters) for retries. The same identity
and payload within an instance are acknowledged without another queue task,
even after intervening events or signal-history archival. Reusing an identity
with a different signal, payload, or requested visit returns an error. A separate
receipt table retains identities for the lifetime of the instance.

Omitting `event_id` remains valid JSON, but each request is an independent event:
content equality no longer suppresses legitimate repeated actions. Different IDs
can carry identical payloads. Wait completion still permits only one continuation.

`wait_visit_id` optionally pins delivery to the visit returned by instance
administration (`node_visit_id`). An obsolete visit is discarded and cannot close
a later visit to the same node. Without it, admission binds to the currently
compatible visit; an early signal binds when a compatible wait is reached.

## Lifecycle hooks and context persistence

All lifecycle hooks use the same handler registry as ServiceTask. A reference
may be a plain ID or `{ handler: id }`. Hook arguments are typed `ContextMap`
values. Queued hooks must return `Success`; errors and other outcomes retry up
to three total attempts, five seconds apart, and then create an incident. Hook
context patches are ignored. Queued execution invokes callbacks before committing its database
transition; callbacks receive the proposed arguments and the input context
snapshot. They should tolerate repeated calls after a rollback or lost lease.
The engine cannot atomically commit an external HTTP request with PostgreSQL.

Context wire values keep explicit type wrappers:

```json
{
  "amount": { "number": 100 },
  "approved": { "boolean": true },
  "customer_name": { "string": "Ada" },
  "customer_id": { "id_field": "customer_1" }
}
```

Other wrappers are `float`, `date`, `datetime` (alias `date_time`), `array`, and
`object`. Context accessors require the exact type; `as_number` does not coerce
strings or floats.

Handler context patches persist operations in order in the `_` scope: `Set`
upserts a key and `Remove` deletes it. They commit with the queued transition;
an error rolls back both the patch and its continuation. The low-level
`apply_process_instance_patch` method also supports atomic removals and advances
the instance revision. `save_process_instance_context` remains an explicit merge
API: omitted keys are preserved. Runtime reads flatten scopes by name; use unique
names across scopes when relying on those reads.
