-- no-transaction
-- Queue-first dequeue uses a bounded due-time scan in scheduling order.
create index concurrently queue_runner_due_order_idx
    on fluxpro.queue_runner (run_after, created_at, uuid);
