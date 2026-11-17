-- no-transaction
-- Probe live sibling leases without scanning unrelated process tasks.
create index concurrently queue_runner_process_lease_idx
    on fluxpro.queue_runner (
        (coalesce(task #>> '{process_node,process_token}',
                  task #>> '{ProcessEvent,process_token}',
                  task #>> '{ProcessSignal,process_token}')),
        locked_by
    ) where lock_key <> '';
