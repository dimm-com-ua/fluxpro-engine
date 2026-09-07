alter table fluxpro.process_definition
    add column if not exists version_comment text;

update fluxpro.process_definition
set version_comment = definition #>> '{metadata,comment}'
where version_comment is null
  and nullif(definition #>> '{metadata,comment}', '') is not null;
