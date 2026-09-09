#![cfg(all(feature = "admin", not(target_arch = "wasm32")))]
use fluxpro_editor::*;
use fluxpro_engine::{
    admin::{FluxproAdminService, PageRequest, ProcessInstanceFilter},
    db_service::FluxproDbServiceImpl,
    models::{
        commands::start_process_instance::StartProcessInstance,
        context_map::context_map::ContextMap, id_field::IdField,
    },
    service::process_service::{FluxproService, FluxproServiceImpl},
};
use sqlx::PgPool;
use std::sync::Arc;

#[sqlx::test(migrations = false)]
#[ignore = "requires a temporary PostgreSQL DATABASE_URL"]
async fn monitor_queries_and_publication_preserve_version_boundaries(pool: PgPool) {
    fluxpro_engine::migrations::migrate(&pool).await.unwrap();
    let service = FluxproServiceImpl::new(Arc::new(FluxproDbServiceImpl::new(pool.clone())));
    let admin = FluxproAdminService::new(pool.clone());
    let doc = EditorDocument::default()
        .prepare_publication(None, "1.0.0")
        .unwrap();
    service
        .publish_process_def_from_source(&doc.definition, &doc.to_project_yaml().unwrap(), None)
        .await
        .unwrap();
    for number in 0..3 {
        service
            .start_process_instance(
                doc.definition.key.clone(),
                StartProcessInstance {
                    process_id: IdField::new(format!("ORDER_{number}")).unwrap(),
                    version: None,
                    context: ContextMap::default(),
                },
            )
            .await
            .unwrap();
    }
    let source = admin
        .get_current_process_definition_by_key(doc.definition.key.get_id())
        .await
        .unwrap();
    let doc = fluxpro_editor::admin::monitor_document(source).unwrap();
    let page = admin
        .list_process_instances(ProcessInstanceFilter::default(), PageRequest::default())
        .await
        .unwrap();
    let instance = &page.items[0];
    sqlx::query("update fluxpro.process_instance set current_node_ref=(select uuid from fluxpro.process_def_node where process_def_uuid=$1 and node_id='start') where uuid=$2")
        .bind(instance.process_definition_uuid).bind(instance.uuid).execute(&pool).await.unwrap();
    sqlx::query("insert into fluxpro.process_instance_log(process_instance_uuid,level,event_type,source,message) select $1,'info','test','integration','event '||n from generate_series(1,280) n")
        .bind(instance.uuid).execute(&pool).await.unwrap();
    sqlx::query("insert into fluxpro.signal_history(process_id,signal_name,payload) select $1,'test',jsonb_build_object('n',n) from generate_series(1,280) n")
        .bind(&instance.token).execute(&pool).await.unwrap();
    sqlx::query("insert into fluxpro.process_incident(process_instance_uuid,task_uuid,task,kind,reason,attempt) values ($1,gen_random_uuid(),'{}','test','unresolved test incident',1)")
        .bind(instance.uuid).execute(&pool).await.unwrap();
    let mut request = MonitorRequest {
        scope: MonitorScope::from_document(&doc),
        revision: 5,
        active_counts_only: false,
        query: MonitorQuery {
            search: instance.uuid.to_string(),
            node_id: Some("start".into()),
            limit: 1,
            ..Default::default()
        },
        instances: vec![MonitorInstanceRequest {
            uuid: instance.uuid.to_string(),
            log_limit: 350,
            signal_limit: 350,
        }],
    };
    let snapshot = fluxpro_editor::admin::load_monitor_snapshot(&admin, request.clone()).await;
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert_eq!(snapshot.request, request);
    assert_eq!(snapshot.instances.total, 1);
    assert_eq!(snapshot.instances.items.len(), 1);
    assert_eq!(
        snapshot
            .node_counts
            .iter()
            .map(|c| c.instance_count)
            .sum::<u64>(),
        1
    );
    assert_eq!(
        snapshot.details[0].logs.items.len() as u64,
        snapshot.details[0].logs.total
    );
    assert!(snapshot.details[0].logs.items.len() >= 280);
    assert_eq!(snapshot.details[0].signals.items.len(), 280);
    assert_eq!(snapshot.details[0].signals.total, 280);
    assert_eq!(
        snapshot.node_counts.iter().map(|c| c.errors).sum::<u64>(),
        1
    );
    assert_eq!(
        snapshot.details[0].instance.issues[0].message,
        "unresolved test incident"
    );
    request.query.node_id = Some("finish".into());
    let empty = fluxpro_editor::admin::load_monitor_snapshot(&admin, request.clone()).await;
    assert_eq!(empty.instances.total, 0);
    assert_eq!(empty.node_counts, snapshot.node_counts);
    let mut changed = doc.clone();
    changed
        .positions
        .insert("start".into(), Position::new(444., 222.));
    changed.definition.name = "Updated definition".into();
    let published = changed.prepare_publication(Some(&doc), "1.0.1").unwrap();
    service
        .publish_process_def_from_source(
            &published.definition,
            &published.to_project_yaml().unwrap(),
            doc.definition.uuid,
        )
        .await
        .unwrap();
    assert!(
        service
            .create_process_def(&published.definition)
            .await
            .is_err(),
        "duplicate versions must fail"
    );
    let current = service
        .get_process(doc.definition.key.clone(), None)
        .await
        .unwrap();
    assert_eq!(current.version.as_str(), "1.0.1");
    assert_eq!(
        admin
            .get_process_instance(instance.uuid)
            .await
            .unwrap()
            .summary
            .process_definition_uuid,
        doc.definition.uuid.unwrap()
    );
    let reopened = fluxpro_editor::admin::monitor_document(
        admin
            .get_current_process_definition_by_key(doc.definition.key.get_id())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reopened.positions, published.positions);
    let newer = published.prepare_publication(Some(&doc), "1.0.2").unwrap();
    let newer_yaml = newer.to_project_yaml().unwrap();
    let (first, second) = tokio::join!(
        service.publish_process_def_from_source(
            &newer.definition,
            &newer_yaml,
            doc.definition.uuid
        ),
        service.publish_process_def_from_source(
            &newer.definition,
            &newer_yaml,
            doc.definition.uuid
        ),
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one concurrent publication should win"
    );
    assert!(
        service
            .publish_process_def_from_source(
                &published.definition,
                &published.to_project_yaml().unwrap(),
                doc.definition.uuid
            )
            .await
            .is_err()
    );
    assert!(
        service
            .publish_process_def_from_source(&newer.definition, &newer_yaml, None)
            .await
            .is_err(),
        "new process cannot reuse an existing key"
    );
    let restricted = fluxpro_editor::admin::load_definition_snapshot(&admin, request.clone()).await;
    assert!(restricted.error.is_none());
    assert!(restricted.instances.items.is_empty());
    assert!(restricted.details.is_empty());
    request.scope = MonitorScope::from_document(&reopened);
    let foreign = fluxpro_editor::admin::load_monitor_snapshot(&admin, request.clone()).await;
    assert!(foreign.error.is_some());
    assert!(foreign.details.is_empty());
    assert_eq!(foreign.request, request);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a temporary PostgreSQL DATABASE_URL"]
async fn active_node_counts_exclude_completed_but_keep_suspended(pool: PgPool) {
    fluxpro_engine::migrations::migrate(&pool).await.unwrap();
    let service = FluxproServiceImpl::new(Arc::new(FluxproDbServiceImpl::new(pool.clone())));
    let admin = FluxproAdminService::new(pool.clone());
    let document = EditorDocument::default()
        .prepare_publication(None, "1.0.0")
        .unwrap();
    service
        .create_process_def(&document.definition)
        .await
        .unwrap();
    let source = admin
        .get_current_process_definition_by_key("new_process")
        .await
        .unwrap();
    let definition_uuid = source.uuid;
    for (name, node, state) in [
        ("working", "start", "running"),
        ("done", "finish", "running"),
        ("incident", "finish", "suspended"),
    ] {
        sqlx::query("insert into fluxpro.process_instance(process_def_uuid,process_id,token,current_node_ref,execution_state) values ($1,$2,$2,(select uuid from fluxpro.process_def_node where process_def_uuid=$1 and node_id=$3),$4)")
            .bind(definition_uuid).bind(name).bind(node).bind(state).execute(&pool).await.unwrap();
    }
    let document = fluxpro_editor::admin::monitor_document(source).unwrap();
    let document: EditorDocument =
        serde_json::from_value(serde_json::to_value(document).unwrap()).unwrap();
    let request = MonitorRequest {
        scope: MonitorScope::from_document(&document),
        active_counts_only: true,
        ..Default::default()
    };
    let snapshot = fluxpro_editor::admin::load_monitor_snapshot(&admin, request.clone()).await;
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    assert_eq!(
        snapshot
            .node_counts
            .iter()
            .map(|c| c.instance_count)
            .sum::<u64>(),
        2
    );
    assert_eq!(
        snapshot
            .node_counts
            .iter()
            .find(|c| c.node_id == "finish")
            .unwrap()
            .instance_count,
        1
    );
    assert_eq!(
        snapshot.instances.total, 3,
        "list history is independently filterable"
    );
    let all = admin
        .get_process_node_instance_counts(definition_uuid)
        .await
        .unwrap();
    assert_eq!(all.iter().map(|c| c.instance_count).sum::<i64>(), 3);
    let mut filtered = request;
    filtered.query.node_id = Some("start".into());
    let filtered = fluxpro_editor::admin::load_monitor_snapshot(&admin, filtered).await;
    assert_eq!(filtered.instances.total, 1);
    assert_eq!(filtered.node_counts, snapshot.node_counts);
}
