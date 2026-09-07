create table if not exists fluxpro.process_instance_log (
    uuid uuid primary key not null default gen_random_uuid(),
    created_at timestamptz not null default now(),
    process_instance_uuid uuid not null references fluxpro.process_instance(uuid) on delete cascade,
    level varchar(16) not null,
    event_type varchar(80) not null,
    source varchar(120) not null,
    message text not null,
    node_id varchar(255),
    handler_id varchar(255),
    queue_task_uuid uuid,
    attempt integer,
    error_kind varchar(255),
    error_message text,
    details jsonb not null default '{}'::jsonb
);

create index if not exists process_instance_log_instance_created_idx
    on fluxpro.process_instance_log(process_instance_uuid, created_at desc);
create index if not exists process_instance_log_level_idx
    on fluxpro.process_instance_log(level);
create index if not exists process_instance_log_event_type_idx
    on fluxpro.process_instance_log(event_type);
create index if not exists process_instance_log_queue_task_idx
    on fluxpro.process_instance_log(queue_task_uuid)
    where queue_task_uuid is not null;
create index if not exists process_instance_log_error_idx
    on fluxpro.process_instance_log(process_instance_uuid, created_at desc)
    where level in ('error', 'critical');
