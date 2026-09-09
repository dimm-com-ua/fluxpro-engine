# Business workflow catalog

These seven definitions are complete routing examples for the current engine.
They demonstrate 23 executable business scenarios, including failures, reminders,
compensation, repeated visits, and loan servicing. All comments and reference
material use English, consistently with the engine documentation.

| Business | Definition | Guide | Executable scenarios |
| --- | --- | --- | --- |
| Pizza delivery | [pizza_order.yaml](../../examples/definitions/business/pizza_order.yaml) | [Restaurant orders](operations.md#pizza-delivery) | Delivered; no stock; payment timeout; delivery failure and refund |
| Home technician | [home_repair.yaml](../../examples/definitions/business/home_repair.yaml) | [Appointments and repairs](operations.md#home-technician) | Completed; rework and reminders; declined quote; no technician |
| English school | [english_school.yaml](../../examples/definitions/business/english_school.yaml) | [Enrolment and learning](operations.md#english-school) | Graduation; absence and remedial exam; declined enrolment |
| Loan lifecycle | [loan_lifecycle.yaml](../../examples/definitions/business/loan_lifecycle.yaml) | [Origination and servicing](loan-lifecycle.md) | Repaid; manual review, delayed funding, overdue servicing and closure recheck; scoring decline; expired offer; failed funding |
| Online retail | [retail_return.yaml](../../examples/definitions/business/retail_return.yaml) | [Returns](operations.md#online-purchase-return) | Parcel delay and refund reconciliation; ineligible return |
| Insurance | [insurance_claim.yaml](../../examples/definitions/business/insurance_claim.yaml) | [Claims](operations.md#insurance-claim) | Missing evidence, escalation and settlement; rejected claim |
| Subscription | [subscription_renewal.yaml](../../examples/definitions/business/subscription_renewal.yaml) | [Billing periods](operations.md#subscription-renewal) | Immediate renewal; payment during grace; expired grace period |

## Validate and execute

List scenarios or validate every definition without connecting to PostgreSQL:

```bash
cargo run -p fluxpro-engine --features api --example business_workflows -- list
cargo run -p fluxpro-engine --features api --example business_workflows -- validate
```

For execution, set `DATABASE_URL` to a **disposable PostgreSQL database with no
other runners**. The demo applies migrations and retains definitions and history.
It refuses to start when queue tasks already exist. It creates a unique definition
key for each run, so repeated demos do not overwrite published definitions.

```bash
cargo run -p fluxpro-engine --features api --example business_workflows -- run pizza_delivery_refund
cargo run -p fluxpro-engine --features api --example business_workflows -- run repair_rework_and_payment_reminder
cargo run -p fluxpro-engine --features api --example business_workflows -- run school_absence_and_remedial_exam
cargo run -p fluxpro-engine --features api --example business_workflows -- run loan_manual_review_overdue_and_closure_recheck
```

The [stepper](../../examples/support/business.rs) uses the real persisted queue
and `process_task` dispatcher. It drains tasks until a wait, submits an event
bound to the current visit, or makes that visit's timeout due. It verifies the
complete expected node sequence and checks for incidents and leftover tasks.
It is not a background runner or a wall-clock scheduling benchmark.

The [scenario data](../../examples/scenarios/business.json) contains startup
context, ordered simulated handler responses, event/timer steps, and expected
node sequences. Signals marked `duplicate` are posted twice with the same event
ID. Handler results are simulated: no payments, repairs, messages, scoring,
interest calculations, or provider integrations happen. The response cache is
in memory and demonstrates the metadata API, not production deduplication.

Run the 23 business integration tests against a disposable database:

```bash
cargo test -p fluxpro-engine --all-features --test durable_transitions business_processes -- --ignored --test-threads=4
```

The existing CI PostgreSQL job includes these tests automatically. Passing them
verifies orchestration for the scripted inputs, not the correctness of a real
bank ledger, kitchen, LMS, insurer, or payment provider.

## Shared integration contract

Each YAML node names a host handler that must be registered. The handler tables
in the guides identify required routing outputs. Commands without routing outputs
still need real implementations: an empty successful result in the demo is a
simulation, not an implementation to deploy. Validate mandatory startup fields
before starting a real process; the YAML schema does not enforce a business input
schema. Amounts, when needed, are integer minor units with an explicit currency.
Financial calculations belong in an authoritative decimal ledger, not Rhai floats.

For each external write, override `process_node_with_execution` and persist the
operation ID **with the side effect and original result** in the owning system.
If that system is remote, use its idempotency API plus a durable outbox/inbox and
reconciliation. A receipt written before the effect can lose work; one written
after it without atomicity can repeat work. Use the instance token and revision
to fence stale external state updates. The operation ID covers a task/phase;
separate visits have separate IDs, so business-level uniqueness is also required.

Examples of business keys:

| Operation | Durable business identity |
| --- | --- |
| Create payment, refund, or delivery | Order/payment/delivery ID and command kind |
| Repair invoice or rework | Appointment and invoice/rework version |
| Lesson booking or certificate | Enrolment and lesson/qualification version |
| Loan disbursement | Loan and authorised disbursement ID |
| Loan accrual | Loan, business date, and calculation version |
| Apply incoming payment | Provider and transaction ID |
| Retail refund | Return and authorised refund ID |
| Insurance payout | Claim and settlement version |
| Subscription charge or entitlement | Subscription and billing period |

Only return success when the command has reached its documented durable boundary.
For example, requesting a refund may succeed after durable submission, while a
separate reconciliation node waits for settlement. A node that ends cancellation
must first confirm compensation or durably transfer its remaining obligations to
a monitored owner. An unknown provider outcome must be queried with the same
business key. Do not issue a fresh payment to discover what happened.

A `UserTask` declares a form; the host provides rendering and lifecycle callbacks.
These definitions omit `special_handlers` so the headless demo needs no UI.
Register `on_show_form`, `on_hide_form`, `on_stage_change`, and
`on_process_complete` as needed, with the same idempotency requirements as service
handlers. `roles` is descriptive metadata; the host must enforce authorization.

## Signals and repeated waits

A signal can close only one wait visit. Use a stable producer `event_id` and the
`wait_visit_id` associated with the form or action. A duplicate transport delivery
keeps the same event ID. A new business event gets a new one. Obtain the runtime
token from startup; do not substitute a business order ID in the instance URL.
The demo reads the visit from PostgreSQL; a real host should expose that identity
to authorized clients in its form/task API.

```json
{
  "event_id": "kitchen:order-1001:ready:v1",
  "wait_visit_id": "00000000-0000-4000-8000-000000000001",
  "signal": "pizza_ready",
  "context": {}
}
```

Replace the sample visit UUID with the actual accepting visit. An HTTP 200 means
admission, not completion of the transition. Do not send the next form's event
before that form's visit exists. Two distinct signals admitted to one visit do
not automatically become two future transitions; see the outstanding calculator
case in the [verification report](../process-verification.md).

Persist payment, attendance, shipment, and other external facts in a durable
inbox before trying to notify the engine. For business facts that must survive a
missed wake-up, reconciliation must read that inbox. A sender may retry the same
event safely, but retrying an event already recorded as stale does not retarget
it. Reconciliation may emit a new wake-up bound to a newly observed visit; this
is a new notification about the same stored fact, not a second monetary posting.
For state-specific actions such as accepting a quote, use the exact visit instead
of carrying the event into an unrelated later form.

Never trust client-supplied flags such as `loan_paid_off` or `invoice_settled`.
Allowlist signal payload fields, authenticate the producer, verify signatures,
and have the named handler read authoritative domain data before acting.

## Timers, incidents, and terminal states

Timeouts are lower bounds: processing can occur later after downtime or queue
backlog. A recurring `P1D` wait is not a business calendar scheduler. Loan accrual
uses an external business-date cursor; subscription instances use a host billing
calendar. Timers and reminders do not prove settlement, attendance, or delivery.

All waits here have timeout routes, but some routes intentionally repeat until an
external obligation is resolved. That is an observable business wait, not a
promise of eventual completion. Set operational alert thresholds for repeated
reminders, unresolved payouts/refunds, and open incidents. Keep recovery explicit
when the outcome of an external action is uncertain. Async handler timeout and
panic suspend execution; missing events or a permanently failed provider cannot
be fixed by a stronger completion guarantee.

An End stops this workflow path. It cannot consume a late payment or reverse an
external shipment. The owning domain service must reconcile late facts, reject
stale commands, and open a correction workflow if needed. These examples do not
change the unresolved behavior described in the earlier verification report.
