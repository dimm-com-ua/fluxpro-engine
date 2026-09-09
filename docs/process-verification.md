# Verification of the submitted Cash Loan and TK Online processes

This review does **not** certify exactly-once external effects or unconditional
progress. The submitted definitions are preserved under `tests/fixtures/submitted`.
Their HTTP wrappers were removed; YAML and expression text were preserved.

## Engine changes and evidence

- Dequeue locks the instance before checking leases in a fresh statement. Sixteen
  concurrent dequeuers are tested against one instance with multiple queued signals.
- Heartbeat renewal cannot revive a lease that expired while awaiting a row lock.
  Runner deadlines use monotonic time and include a blocked renewal.
- Async host calls have a configurable deadline (five minutes by default).
  Timeout and unwinding panic retain the task and suspend the instance instead of
  automatically repeating an external operation whose outcome is unknown.
- Handlers receive a stable per-task/per-phase operation ID through the additive
  `process_node_with_execution` API. A forced acknowledgement failure verifies
  that replay uses the same key. The host must actually deduplicate that key.
- Legacy `ctx._last_signal` expressions are accepted, with string-aware normalization.
  Invalid expressions and absent required keys still produce incidents.
- A queued node must match its published definition before invoking its handler.

The PostgreSQL tests use controlled handlers, not ERP, payment, signing, message,
or application-stage services. They check the full Cash Loan success sequence
and stage history, TK success, TK escalation/return/rejection, manual decision,
reminder and payout escalation paths. Every explicit branch in both original
schemas is exercised with a matching input and checked against earlier branches.
Other tests inject database errors, lost connections, lease expiration, duplicate
signals, and concurrent recovery. These tests provide bounded evidence, not a
proof that arbitrary host code or an unavailable external system will progress.

Verification on 2026-09-09 passed 38 tests including Rustdoc examples, plus 49
explicitly enabled PostgreSQL integration tests. The runnable workflow example,
default-feature tests, database-only build, all-feature examples, formatting, and
Rustdoc with warnings denied also passed. PostgreSQL used an isolated temporary
cluster. Engine checks used a source-identical temporary package without the
workspace section, because the editor workspace was being changed concurrently;
this is not a verification of the editor or the complete application workspace.

## Definition and caller issues

| Observation | Consequence / required contract |
| --- | --- |
| Cash startup supplies only `product_name`; its first route reads `is_phone_confirmed` | The strict evaluator suspends if the handler does not supply the flag. The reviewed Cash definition guards the optional flag with `ctx.contains(...)`. |
| `calculator_interacted` and `terms_selected` can both arrive at `select_terms_open` before the first transition | Both are bound to the same visit. The first closes it; the second is discarded. A regression test exposes this unresolved producer/engine contract. The client must wait for the next visit, or an explicit rule for carrying this event forward must be agreed and implemented. |
| The phone signal HTTP example uses `request_id` in the instance URL | This endpoint expects the returned runtime token (`current_instance`), not the business ID. |
| The next HTTP signal URL contains `//fluxpro` | Use the canonical single-slash path; do not depend on proxy normalization. |
| Existing signal examples omit `event_id` and `wait_visit_id` | Each request is independent; a delayed event may be admitted to a later compatible wait. Producers must use stable event IDs and explicit visit IDs where intent belongs to one particular visit. |
| `credit_info_open`, `select_terms_open`, and `wait_documents_to_sign` have no timeout | They may wait indefinitely by definition. This is not a queue deadlock; guaranteed business completion would require an explicit deadline or external event. |
| `on_error` invokes `stop_order` and then routes through another `stop_order` node | Two calls are explicitly prescribed. Idempotent business behavior is required; the engine must not silently remove an authored node. |
| Cleanup occurs in both service nodes and `on_process_complete` | Multiple logical cleanup calls are authored; task deduplication cannot remove them without changing the process. |
| Cash reminder routes from a UserTask through a ServiceTask into a Wait | The engine calls `on_hide_form` on departure. The host/UI must support delivering `terms_viewed` during the subsequent technical Wait; UI availability was not verified. |
| Both submitted versions expire on 2026-12-31 | New starts after that date are intentionally excluded. Existing instances remain bound to their original version. |

## Reviewed copies

`examples/definitions/reviewed/cash_loan.yaml` proposes version 1.1.26 with an
explicit optional-phone guard. `tk_online.yaml` proposes 0.0.8. Both copies use
canonical bracket access. No route target, retry count, timeout, form, handler,
or stage assignment was changed. Choose unused version numbers before publishing;
the engine refuses duplicate identities. Neither definition was published here.

The original Cash startup missing its phone flag and the rapid calculator event
sequence have dedicated tests demonstrating their current outcomes. Passing those
tests documents a limitation; it does not mean these outcomes meet the desired UX.

## External integration boundary

A related local `lendiq-rs` checkout was inspected read-only. Its dependency names
published `fluxpro-engine = "0.1.0"`; deployment of this workspace build is unverified.
The local SetStage handler calls `set_order_stage`, which invokes
`add_lend_order_stage` independently of the engine transaction, without an operation
ID or engine-revision fence. The procedure appends history and updates `crt_stage`.
Consequently, external stage consistency cannot be guaranteed by the engine alone.

Payout preparation calls the application's `create_task_once` with an order-based
key. The subsequent payout/provider path still needs separate verification of
stable request identity, durable receipts after queue cleanup, and recovery after
an uncertain provider response. No payment, ERP request, or production database
operation was performed during this review.

Before claiming readiness, connect the reviewed engine version and migrations,
implement/verify idempotency and revision fencing at the external write boundary,
and settle the rapid-calculator delivery contract. Unbounded guarantees would
remain incorrect even after those changes: outages and missing required events
must produce a recoverable, observable state rather than a false success.

## Additional business examples

The [business catalog](business/README.md) adds seven definitions and 23 scripted
PostgreSQL scenarios. On 2026-09-09, all 72 PostgreSQL tests and 39 ordinary/Rustdoc
tests passed with the engine-only isolation described above. Example builds,
formatting, strict Rustdoc, and CLI runs for pizza refunds and extended loan
servicing also passed. These additions document and test orchestration with
simulated integrations; they do not resolve the outstanding calculator contract
or certify the real application's financial and external-stage handlers.
