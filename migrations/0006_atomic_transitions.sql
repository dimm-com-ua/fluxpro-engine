-- Each visit identifies one execution of a node, including repeated visits in loops.
alter table fluxpro.process_instance
    add column revision bigint not null default 0,
    add column node_visit_id uuid,
    add column wait_completed boolean not null default false;

update fluxpro.process_instance
set node_visit_id = gen_random_uuid()
where current_node_ref is not null;

-- Bind timeout and admitted signal tasks to the wait that created/accepted them.
-- Queue JSON remains compatible with the existing serialized task variants.
alter table fluxpro.queue_runner add column node_visit_id uuid;

update fluxpro.queue_runner q
set node_visit_id = case
    when n.node_id = q.task #>> '{ProcessEvent,node,id}'
         and not exists (
             select 1 from fluxpro.process_instance_log entered
             where entered.process_instance_uuid = i.uuid
               and entered.event_type = 'node.entered'
               and entered.node_id = n.node_id
               and entered.created_at > q.created_at
         ) then i.node_visit_id
    else gen_random_uuid()
end
from fluxpro.process_instance i
left join fluxpro.process_def_node n on n.uuid = i.current_node_ref
where q.task #>> '{ProcessEvent,process_token}' = i.token;

create index queue_runner_node_visit_idx
    on fluxpro.queue_runner(node_visit_id) where node_visit_id is not null;
