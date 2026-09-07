create schema if not exists fluxpro;

create table if not exists fluxpro.process_definition (
    uuid uuid primary key not null default gen_random_uuid(),
    created_at timestamptz not null default now(),
    key_ text not null,
    version text not null,
    index_id integer not null default 0,
    status text not null,
    effective_from timestamptz not null,
    deprecated_at timestamptz,
    definition jsonb not null
);
create index if not exists process_definition_key_version_idx on fluxpro.process_definition (key_, version);
create index if not exists process_definition_status_idx on fluxpro.process_definition (status);
create index if not exists process_definition_effective_from_idx on fluxpro.process_definition (effective_from);
create index if not exists process_definition_deprecated_at_idx on fluxpro.process_definition (deprecated_at);

create table if not exists fluxpro.process_stage (
    uuid uuid primary key not null default gen_random_uuid(),
    process_def_uuid uuid references fluxpro.process_definition(uuid),
    stage_id varchar(40) not null,
    name varchar(200),
    is_initial bool,
    is_final bool
);
create index if not exists process_stage_process_def_uuid_idx on fluxpro.process_stage(process_def_uuid);
create index if not exists process_stage_stage_id_idx on fluxpro.process_stage(stage_id);
create index if not exists process_stage_is_initial_idx on fluxpro.process_stage(is_initial);
create unique index if not exists process_stage_process_def_stage_id_uidx
    on fluxpro.process_stage(process_def_uuid, stage_id);

create table if not exists fluxpro.process_def_signal (
    uuid uuid primary key not null default gen_random_uuid(),
    process_def_uuid uuid references fluxpro.process_definition(uuid),
    signal_id varchar not null
);
create index if not exists process_def_signal_process_def_uuid_idx on fluxpro.process_def_signal(process_def_uuid);
create index if not exists process_def_signal_signal_id_idx on fluxpro.process_def_signal(signal_id);
create unique index if not exists process_def_signal_process_def_signal_id_uidx
    on fluxpro.process_def_signal(process_def_uuid, signal_id);

create table if not exists fluxpro.process_def_node (
    uuid uuid primary key not null default gen_random_uuid(),
    process_def_uuid uuid references fluxpro.process_definition(uuid),
    node_id varchar not null,
    definition jsonb default '{}'::jsonb
);
create index if not exists process_def_node_process_def_uuid_idx on fluxpro.process_def_node(process_def_uuid);
create index if not exists process_def_node_node_id_idx on fluxpro.process_def_node(node_id);
create unique index if not exists process_def_node_process_def_node_id_uidx
    on fluxpro.process_def_node(process_def_uuid, node_id);

create table if not exists fluxpro.process_instance (
    uuid uuid primary key not null default gen_random_uuid(),
    created_at timestamptz not null default now(),
    process_def_uuid uuid references fluxpro.process_definition(uuid),
    process_id varchar not null unique,
    token varchar(255) not null unique default gen_random_uuid(),
    context jsonb not null default '{}'::jsonb,
    current_stage uuid references fluxpro.process_stage(uuid),
    current_stage_reason text,
    current_node_ref uuid references fluxpro.process_def_node(uuid)
);
create index if not exists process_instance_process_def_uuid_idx on fluxpro.process_instance(process_def_uuid);
create index if not exists process_instance_process_id_idx on fluxpro.process_instance(process_id);
create index if not exists process_instance_current_stage_idx on fluxpro.process_instance(current_stage);
create index if not exists process_instance_current_node_ref_idx on fluxpro.process_instance(current_node_ref);

create table if not exists fluxpro.process_instance_stage_log (
    uuid uuid primary key not null default gen_random_uuid(),
    process_instance_uuid uuid references fluxpro.process_instance(uuid),
    stage_uuid uuid references fluxpro.process_stage(uuid),
    reason varchar(200),
    created_at timestamptz not null default now(),
    context jsonb not null default '{}'::jsonb
);
create index if not exists process_instance_stage_log_process_instance_uuid_idx
    on fluxpro.process_instance_stage_log(process_instance_uuid);
create index if not exists process_instance_stage_log_stage_uuid_idx
    on fluxpro.process_instance_stage_log(stage_uuid);

create table if not exists fluxpro.process_instance_context_variable (
    uuid uuid primary key not null default gen_random_uuid(),
    process_instance_uuid uuid references fluxpro.process_instance(uuid),
    scope varchar not null,
    name varchar not null,
    value jsonb not null
);
create index if not exists process_instance_context_variable_process_instance_uuid_idx
    on fluxpro.process_instance_context_variable(process_instance_uuid);
create index if not exists process_instance_context_variable_scope_idx
    on fluxpro.process_instance_context_variable(scope);
create index if not exists process_instance_context_variable_name_idx
    on fluxpro.process_instance_context_variable(name);
create unique index if not exists process_instance_context_variable_process_scope_name_uidx
    on fluxpro.process_instance_context_variable(process_instance_uuid, scope, name);

create table if not exists fluxpro.queue_runner (
    uuid uuid primary key default gen_random_uuid(),
    created_at timestamptz not null default now(),
    task jsonb not null default '{}'::jsonb,
    run_after timestamptz not null default now(),
    attempts integer not null default 0,
    lock_key text not null default '',
    locked_at timestamptz,
    locked_by timestamptz
);
create index if not exists queue_runner_available_idx
    on fluxpro.queue_runner(run_after, locked_by, created_at);
create index if not exists queue_runner_lock_key_idx on fluxpro.queue_runner(lock_key);

create table if not exists fluxpro.queue_cancel_task (
    uuid uuid primary key default gen_random_uuid(),
    process_token varchar(255),
    node_id varchar(255),
    event_id varchar(255),
    task_uuid uuid references fluxpro.queue_runner(uuid)
);
create index if not exists queue_cancel_task_task_uuid_idx on fluxpro.queue_cancel_task(task_uuid);
create index if not exists queue_cancel_task_process_node_event_idx
    on fluxpro.queue_cancel_task(process_token, node_id, event_id);

create table if not exists fluxpro.event_schedule (
    uuid uuid primary key default gen_random_uuid(),
    process_instance_uuid uuid references fluxpro.process_instance(uuid) not null,
    process_def_node_uuid uuid not null,
    scheduled_time timestamptz not null,
    on_time text not null
);
create index if not exists event_schedule_scheduled_time_idx on fluxpro.event_schedule(scheduled_time);
create index if not exists event_schedule_process_instance_idx on fluxpro.event_schedule(process_instance_uuid);

create table if not exists fluxpro.process_def_escalations (
    uuid uuid primary key default gen_random_uuid(),
    process_def_uuid uuid references fluxpro.process_definition(uuid),
    escalation_id text,
    definition jsonb default '{}'::jsonb
);
create unique index if not exists process_def_escalations_process_escalation_uidx
    on fluxpro.process_def_escalations(process_def_uuid, escalation_id);

create table if not exists fluxpro.signal_history (
    uuid uuid primary key default gen_random_uuid(),
    created_at timestamptz not null default now(),
    process_id text not null,
    signal_name text not null,
    payload jsonb default '{}'::jsonb
);
create index if not exists signal_history_created_at_idx on fluxpro.signal_history(created_at);
create index if not exists signal_history_process_id_idx on fluxpro.signal_history(process_id);
create index if not exists signal_history_signal_name_idx on fluxpro.signal_history(signal_name);
