create index if not exists signal_history_process_created_uuid_idx
    on fluxpro.signal_history(process_id, created_at desc, uuid desc);

create index if not exists signal_history_created_uuid_archive_idx
    on fluxpro.signal_history(created_at, uuid);

create index if not exists process_instance_log_created_uuid_archive_idx
    on fluxpro.process_instance_log(created_at, uuid);
