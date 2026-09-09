-- Incident tasks remain in the queue; suspended instances are excluded from dequeue.
alter table fluxpro.process_instance add column execution_state text not null default 'running'
    check (execution_state in ('running', 'suspended'));
create table fluxpro.process_incident (
    uuid uuid primary key default gen_random_uuid(),
    process_instance_uuid uuid not null references fluxpro.process_instance(uuid),
    task_uuid uuid not null,
    task jsonb not null,
    kind text not null,
    reason text not null,
    attempt integer not null,
    created_at timestamptz not null default now(),
    resolved_at timestamptz
);
create unique index process_incident_open_uidx on fluxpro.process_incident(process_instance_uuid)
    where resolved_at is null;

-- Keep event identities independently of archivable signal history.
create table fluxpro.signal_receipt (
    process_instance_uuid uuid not null references fluxpro.process_instance(uuid),
    event_id text not null check (length(event_id) between 1 and 256),
    signal text not null,
    payload jsonb not null,
    requested_visit uuid,
    created_at timestamptz not null default now(),
    primary key (process_instance_uuid, event_id)
);
alter table fluxpro.signal_history add column event_id text;
alter table fluxpro.signal_history add column wait_visit_id uuid;

-- Validate lifecycle labels without changing legacy capitalization.
alter table fluxpro.process_definition add constraint process_definition_status_valid
    check (lower(status) in ('draft', 'active', 'deprecated'));

-- Fail rather than silently rewriting ambiguous existing version identities.
alter table fluxpro.process_definition add constraint process_definition_key_version_unique unique(key_, version);

create function fluxpro.protect_published_definition() returns trigger language plpgsql as $$
begin
    if lower(old.status) <> 'draft' then
        if TG_OP = 'DELETE' then
            raise exception 'published definitions cannot be deleted';
        end if;
        if (to_jsonb(new) - 'status' - 'deprecated_at') is distinct from
           (to_jsonb(old) - 'status' - 'deprecated_at') or lower(new.status) = 'draft' then
            raise exception 'published definitions are immutable; create a new version';
        end if;
    end if;
    if TG_OP = 'DELETE' then return old; end if;
    return new;
end $$;
create trigger protect_published_definition before update or delete on fluxpro.process_definition
    for each row execute function fluxpro.protect_published_definition();

create function fluxpro.protect_published_child() returns trigger language plpgsql as $$
declare parent_status text;
begin
    if TG_OP <> 'INSERT' then
        select status into parent_status from fluxpro.process_definition where uuid=old.process_def_uuid for update;
        if lower(parent_status) <> 'draft' then
            raise exception 'published definition declarations are immutable';
        end if;
    end if;
    if TG_OP <> 'DELETE' then
        select status into parent_status from fluxpro.process_definition where uuid=new.process_def_uuid for update;
        if lower(parent_status) <> 'draft' then
            raise exception 'published definition declarations are immutable';
        end if;
        return new;
    end if;
    return old;
end $$;
create trigger protect_published_child before insert or update or delete on fluxpro.process_def_node
    for each row execute function fluxpro.protect_published_child();
create trigger protect_published_child before insert or update or delete on fluxpro.process_stage
    for each row execute function fluxpro.protect_published_child();
create trigger protect_published_child before insert or update or delete on fluxpro.process_def_signal
    for each row execute function fluxpro.protect_published_child();
create trigger protect_published_child before insert or update or delete on fluxpro.process_def_escalations
    for each row execute function fluxpro.protect_published_child();
-- Prioritize the explicitly resumed task before unrelated queued signals or timers.
alter table fluxpro.process_instance add column recovery_task_uuid uuid;
