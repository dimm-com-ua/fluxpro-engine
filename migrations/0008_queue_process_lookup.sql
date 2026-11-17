-- no-transaction
-- Match the persisted task tags without changing existing JSON payloads.
create index concurrently queue_runner_process_ready_idx
    on fluxpro.queue_runner (
        (coalesce(task #>> '{process_node,process_token}',
                  task #>> '{ProcessEvent,process_token}',
                  task #>> '{ProcessSignal,process_token}')),
        run_after, created_at, uuid
    );
