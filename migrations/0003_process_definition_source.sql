alter table fluxpro.process_definition
    add column if not exists source_definition text;
