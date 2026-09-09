# Operational business examples

Use these definitions with the [shared integration contract](README.md#shared-integration-contract).
Each scenario's exact signals, outputs, and expected path are executable data in
[business.json](../../examples/scenarios/business.json).

## Pizza delivery

[Definition](../../examples/definitions/business/pizza_order.yaml).
Start with an `order_id`; the order service owns the basket, address, totals,
customer details, and inventory reservation. Store IDs in context rather than
copying sensitive customer records into workflow history.

The process reserves ingredients, requests payment, waits up to ten minutes,
submits a kitchen order, waits for readiness, dispatches delivery, and waits for
a confirmed delivery. Payment failure, preparation timeout, and delivery failure
all enter cancellation and payment reconciliation. A pending refund returns to
reconciliation after a callback or one-hour timeout; it does not immediately mark
the order cancelled.

| Handler | Required behavior / routing output |
| --- | --- |
| `reserve_ingredients` | Reserve under the order key; return Boolean `stock_available` |
| `create_payment_intent` | Create/reuse the order's payment intent and persist its identity |
| `submit_kitchen_order` | Submit once per order version; refuse an externally cancelled order |
| `dispatch_courier` | Create/reuse delivery; refuse a stale dispatch command |
| `cancel_order_and_reconcile_payment` | Fence cancellation, release reservations, reconcile capture/refund, return Boolean `refund_pending` |

`refund_pending=false` means no refund obligation remains or it has been durably
resolved. It must not mean “refund API request sent.” Payment and delivery events
come from trusted integrations. A kitchen/provider callback may arrive before
the Wait exists, so the host inbox must retain and redeliver its notification
when the accepting visit exists. Cancellation must handle a capture that arrives
after the payment timeout or even after End.

Scenarios: `pizza_delivered`, `pizza_out_of_stock`, `pizza_payment_timeout`,
`pizza_delivery_refund`. The last scenario proves the refund callback routes
through reconciliation, not directly to a successful cancellation.

## Home technician

[Definition](../../examples/definitions/business/home_repair.yaml).
Start with `appointment_id`; the host owns address, requested slot, diagnosis,
quote versions, technician assignments, and invoicing.

Booking either finds a technician or cancels the appointment. Arrival is bounded
by four hours; the customer accepts or declines the diagnosis-based quote. Repair
completion opens an acceptance form, where the customer can request rework.
Acceptance leads to an invoice, payment wait, authoritative payment verification,
and completion. An eight-hour repair delay creates an investigation; an unpaid
invoice produces a weekly reminder. These durations are illustrative service
policies, not universal appointment commitments.

| Handler | Required behavior / routing output |
| --- | --- |
| `book_technician` | Atomically reserve a slot; Boolean `technician_found` |
| `prepare_repair_quote` | Persist diagnosis, scope, price, and quote version for the form |
| `authorize_repair` | Verify the accepted quote version and authorize work once |
| `investigate_repair_delay` | Create/update the existing service case and reconcile repair status |
| `arrange_rework` | Create a distinct authorized rework version, not another original repair |
| `issue_repair_invoice` | Create/reuse the appointment invoice |
| `remind_repair_payment` | Send once per configured reminder period |
| `verify_repair_payment` | Read settlement and return Boolean `invoice_settled` |
| `cancel_appointment` | Release slot and settle any call-out/deposit obligation before success |

Scenarios: `repair_completed`, `repair_rework_and_payment_reminder`,
`repair_quote_declined`, `repair_no_technician`. Every return to `repair_wait`,
`acceptance`, or `payment` creates a new visit. A response to an older acceptance
form must not accept the new rework. Investigation must republish already known
repair completion into the current wait when appropriate; otherwise the operator
continues owning the open case.

## English school

[Definition](../../examples/definitions/business/english_school.yaml).
Start with `enrolment_id`; the LMS owns placement answers, selected course, tuition,
lesson identities, attendance, assessment results, and certificate records.

Placement produces a course offer. Acceptance creates an invoice; verified tuition
starts learning. Each completed lesson triggers progress reconciliation. The next
lesson is scheduled until the course is complete; absence triggers outreach. The
final exam produces a certificate on success or a remedial lesson on failure.
Missing exam submissions cause a reminder and a new exam visit.

| Handler | Required behavior / routing output |
| --- | --- |
| `assess_placement_and_offer` | Grade placement and persist a course/level offer |
| `issue_tuition_invoice` | Create/reuse the enrolment invoice |
| `confirm_paid_enrolment` | Verify payment and capacity; Boolean `tuition_settled` |
| `schedule_next_lesson` | Book the next incomplete lesson; repeated calls reuse that lesson |
| `reconcile_lesson_progress` | Read durable attendance; Booleans `course_complete`, `needs_outreach` on every call |
| `contact_absent_student` | Create an outreach task without inventing attendance |
| `remind_final_exam` | Notify once per reminder period |
| `grade_final_exam` | Read the authorized submission; Boolean `exam_passed` |
| `assign_remedial_lesson` | Persist a remedial assignment and update curriculum progress |
| `issue_certificate` | Issue once per enrolment and qualification version |
| `release_seat_and_reconcile_tuition` | Release capacity and resolve or durably hand off financial obligations |

The attendance timeout reads the LMS even if its notification was lost. Rebooking
a lesson must not create duplicates, and repeated attendance records must not
increment progress twice. Exam and enrolment signals must come from an authorized
student and be tied to the displayed visit.

Scenarios: `school_graduation`, `school_absence_and_remedial_exam`,
`school_offer_declined`. The demo's two lesson results are a short simulation of a
course; the YAML has no hard-coded lesson count. Pauses, transfers, and withdrawals
after enrolment require additional authored routes and domain refund policy.

## Online purchase return

[Definition](../../examples/definitions/business/retail_return.yaml).
Start with `return_id`; the commerce system owns the original purchase, return
items, eligibility policy, parcel identity, inspection record, and refund amount.

| Handler | Required behavior / routing output |
| --- | --- |
| `check_return_eligibility` | Evaluate configured policy; Boolean `return_eligible` |
| `issue_return_label` | Create/reuse the return's shipment label |
| `locate_return_parcel` | Query tracking and open/update an exception case after fourteen days |
| `inspect_returned_item` | Read warehouse inspection; Boolean `inspection_passed` |
| `request_return_refund` | Submit using a stable return/refund key |
| `reconcile_return_refund` | Read provider settlement; Boolean `refund_settled` |
| `notify_return_rejection` | Persist reason and resolve custody/return-shipping obligations |

Scenarios: `return_refund_reconciled`, `return_ineligible`. Receipt does not itself
prove refund eligibility. Refund completion requires reconciliation, including
when no callback arrives. Lost parcels intentionally stay with exception handling;
the sample does not silently reject them because a timer elapsed.

## Insurance claim

[Definition](../../examples/definitions/business/insurance_claim.yaml).
Start with `claim_id`; the claims system owns policy, loss details, evidence,
adjuster authority, decision reasons, and settlement instructions.

| Handler | Required behavior / routing output |
| --- | --- |
| `verify_policy_coverage` | Read applicable policy data; Boolean `covered` |
| `request_missing_evidence` | Create a reminder for the current evidence request |
| `escalate_claim_review` | Assign/update an overdue adjuster case |
| `request_claim_settlement` | Verify approved decision and submit once per settlement version |
| `reconcile_claim_settlement` | Read settlement confirmation; Boolean `settlement_paid` |
| `record_claim_rejection` | Record an authorized reason and deliver the decision |

Scenarios: `claim_settled_after_escalation`, `claim_rejected`. Neither a coverage
flag from an untrusted client nor a timer authorizes payment. Evidence requests
and assessment escalations repeat; legal deadlines, appeals, partial settlements,
and fraud investigation are host/product policies requiring further routes.

## Subscription renewal

[Definition](../../examples/definitions/business/subscription_renewal.yaml).
Start with `subscription_id` and `billing_period`. The host scheduler creates one
instance per due billing period using an enforced business uniqueness constraint.
The workflow starts a charge, reconciles the invoice, and either extends service,
waits during grace, or suspends unpaid access.

| Handler | Required behavior / routing output |
| --- | --- |
| `request_subscription_charge` | Create/reuse a charge for this subscription and billing period |
| `reconcile_subscription_invoice` | Read invoice and absolute grace deadline; Booleans `invoice_paid`, `grace_expired` |
| `send_billing_reminder` | Deduplicate by invoice and reminder period |
| `extend_subscription_period` | Grant this period once; do not add another month on replay |
| `suspend_unpaid_subscription` | Recheck payment and fence the access change against newer entitlements |

Scenarios: `subscription_renewed`, `subscription_paid_in_grace`,
`subscription_grace_expired`. Payment is evaluated before grace expiration so a
settled invoice is not suspended merely because reconciliation ran late. A `P1D`
timer polls the invoice; it does not define the billing calendar or restart the
grace period. If a payment arrives after suspension/End, the billing domain must
restore access through its correction policy or a new workflow.
