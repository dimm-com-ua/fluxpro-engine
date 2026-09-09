//! Run with DATABASE_URL and --features runtime,migration-repair -- --ignored.
#![cfg(all(feature = "migration-repair", feature = "runtime"))]

use fluxpro_engine::migrations::legacy_versions::{
    LegacyVersionRepairOutcome as Outcome, apply_legacy_version_plan, plan_legacy_versions,
};
use fluxpro_engine::migrations::{MIGRATOR, migrate};
use serde_json::{Value, json};
use sqlx::{Executor, PgPool, migrate::Migrator};
use std::borrow::Cow;
use uuid::Uuid;

async fn legacy_schema(pool: &PgPool) {
    let mut connection = pool.acquire().await.unwrap();
    connection.execute("create schema fluxpro").await.unwrap();
    connection
        .execute("set search_path to fluxpro, public")
        .await
        .unwrap();
    Migrator {
        migrations: Cow::Owned(MIGRATOR.iter().filter(|m| m.version < 7).cloned().collect()),
        ..Migrator::DEFAULT
    }
    .run(&mut *connection)
    .await
    .unwrap();
}

async fn definition(pool: &PgPool, key: &str, version: &str, index: i32) -> Uuid {
    let document = json!({
        "key": key, "name": format!("Revision {index}"), "version": version,
        "status": "active", "effective_from": null,
        "nodes": [{"id": "start", "type": "Start", "next": "end"}, {"id": "end", "type": "End"}],
        "stages": [], "signals": [], "special_handlers": null,
        "unknown_legacy_field": {"preserve": true}
    });
    let mut yaml_document = document.clone();
    yaml_document["editor"] = json!({"positions": {"start": {"x": 12.5, "y": 42}}});
    let yaml = format!(
        "# Original formatting is backed up\n{}",
        serde_yaml::to_string(&yaml_document).unwrap()
    );
    sqlx::query_scalar(
        "insert into fluxpro.process_definition(key_,version,index_id,status,effective_from,definition,source_definition)
         values ($1,$2,$3,'Active','2020-01-01',$4,$5) returning uuid",
    ).bind(key).bind(version).bind(index).bind(document).bind(yaml).fetch_one(pool).await.unwrap()
}

async fn rows(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar("select to_jsonb(d) from fluxpro.process_definition d order by uuid")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires temporary PostgreSQL"]
async fn repair_preserves_graph_bindings_selection_and_upgrades(pool: PgPool) {
    legacy_schema(&pool).await;
    let first = definition(&pool, "cash_loan", "1.1.10", 1).await;
    let retained = definition(&pool, "cash_loan", "1.1.10", 2).await;
    let future = definition(&pool, "cash_loan", "1.1.10", 3).await;
    sqlx::query("update fluxpro.process_definition set effective_from='2999-01-01' where uuid=$1")
        .bind(future)
        .execute(&pool)
        .await
        .unwrap();
    definition(&pool, "cash_loan", "1.10.0", 4).await;
    let other = definition(&pool, "unrelated", "9.0.0", 1).await;
    let node: Uuid = sqlx::query_scalar("insert into fluxpro.process_def_node(process_def_uuid,node_id,definition) values ($1,'start','{}') returning uuid")
        .bind(first).fetch_one(&pool).await.unwrap();
    let instance: Uuid = sqlx::query_scalar("insert into fluxpro.process_instance(process_def_uuid,process_id,current_node_ref,context) values ($1,'existing',$2,'{\"keep\":true}') returning uuid")
        .bind(first).bind(node).fetch_one(&pool).await.unwrap();
    let before = rows(&pool).await;
    let selected_before: Uuid = sqlx::query_scalar("select uuid from fluxpro.process_definition where key_='cash_loan' and lower(status)='active' and effective_from<=now() order by index_id desc limit 1")
        .fetch_one(&pool).await.unwrap();
    assert!(
        migrate(&pool)
            .await
            .unwrap_err()
            .to_string()
            .contains("process_definition_key_version_unique")
    );
    let plan = plan_legacy_versions(&pool).await.unwrap();
    assert_eq!(plan.changes.len(), 2);
    assert_eq!(plan.original_rows.len(), 4);
    assert!(plan.changes.iter().all(|c| c.retained_uuid == retained));
    assert_eq!(plan.changes[0].new_version, "1.10.1");
    assert_eq!(plan.changes[1].new_version, "1.10.2");
    assert_eq!(before, rows(&pool).await, "preview is read-only");
    let restored_plan = serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
    assert_eq!(
        apply_legacy_version_plan(&pool, &restored_plan)
            .await
            .unwrap(),
        Outcome::Applied(2)
    );
    let after = rows(&pool).await;
    for original in &before {
        let updated = after
            .iter()
            .find(|r| r["uuid"] == original["uuid"])
            .unwrap();
        let mut normalized = updated.clone();
        normalized["version"] = original["version"].clone();
        normalized["definition"]["version"] = original["definition"]["version"].clone();
        normalized["source_definition"] = original["source_definition"].clone();
        assert_eq!(&normalized, original, "no other row/graph field changed");
        let mut yaml: Value =
            serde_yaml::from_str(updated["source_definition"].as_str().unwrap()).unwrap();
        assert_eq!(yaml["version"], updated["version"]);
        yaml["version"] = original["version"].clone();
        let original_yaml: Value =
            serde_yaml::from_str(original["source_definition"].as_str().unwrap()).unwrap();
        assert_eq!(
            yaml, original_yaml,
            "unknown YAML and editor fields preserved"
        );
    }
    assert_eq!(
        before.iter().find(|r| r["uuid"] == other.to_string()),
        after.iter().find(|r| r["uuid"] == other.to_string())
    );
    assert_eq!(
        apply_legacy_version_plan(&pool, &plan).await.unwrap(),
        Outcome::AlreadyApplied
    );
    migrate(&pool).await.unwrap();
    migrate(&pool).await.unwrap();
    assert_eq!(
        apply_legacy_version_plan(&pool, &plan).await.unwrap(),
        Outcome::AlreadyApplied
    );
    let binding: (Uuid, Uuid, Value) = sqlx::query_as("select process_def_uuid,current_node_ref,context from fluxpro.process_instance where uuid=$1")
        .bind(instance).fetch_one(&pool).await.unwrap();
    assert_eq!(binding, (first, node, json!({"keep":true})));
    let selected_after: Uuid = sqlx::query_scalar("select uuid from fluxpro.process_definition where key_='cash_loan' and lower(status)='active' and effective_from<=now() order by index_id desc limit 1")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(selected_before, selected_after);
    let error = sqlx::query("update fluxpro.process_definition set version='99.0.0' where uuid=$1")
        .bind(first)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("immutable"));
    assert!(
        plan_legacy_versions(&pool)
            .await
            .unwrap()
            .changes
            .is_empty()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires temporary PostgreSQL"]
async fn stale_plan_and_new_versions_are_rejected(pool: PgPool) {
    legacy_schema(&pool).await;
    definition(&pool, "flow", "0.0.1", 1).await;
    let id = definition(&pool, "flow", "0.0.1", 2).await;
    let plan = plan_legacy_versions(&pool).await.unwrap();
    definition(&pool, "flow", "0.0.2", 3).await;
    let before = rows(&pool).await;
    assert!(
        apply_legacy_version_plan(&pool, &plan)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed since preview")
    );
    assert_eq!(before, rows(&pool).await);
    let plan = plan_legacy_versions(&pool).await.unwrap();
    sqlx::query("update fluxpro.process_definition set definition=jsonb_set(definition,'{name}','\"new name\"') where uuid=$1")
        .bind(id).execute(&pool).await.unwrap();
    let before = rows(&pool).await;
    assert!(apply_legacy_version_plan(&pool, &plan).await.is_err());
    assert_eq!(before, rows(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires temporary PostgreSQL"]
async fn concurrent_repair_is_idempotent_and_sql_failure_rolls_back(pool: PgPool) {
    legacy_schema(&pool).await;
    definition(&pool, "flow", "1.0.0", 1).await;
    definition(&pool, "flow", "1.0.0", 2).await;
    definition(&pool, "flow", "1.0.0", 3).await;
    let plan = plan_legacy_versions(&pool).await.unwrap();
    pool.execute("create function fluxpro.reject_test_repair() returns trigger language plpgsql as $$ begin if new.version='1.0.2' then raise exception 'injected repair failure'; end if; return new; end $$;
                  create trigger reject_test_repair before update on fluxpro.process_definition for each row execute function fluxpro.reject_test_repair()")
        .await.unwrap();
    let before = rows(&pool).await;
    assert!(
        apply_legacy_version_plan(&pool, &plan)
            .await
            .unwrap_err()
            .to_string()
            .contains("injected repair failure")
    );
    assert_eq!(before, rows(&pool).await);
    pool.execute("drop trigger reject_test_repair on fluxpro.process_definition")
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        apply_legacy_version_plan(&pool, &plan),
        apply_legacy_version_plan(&pool, &plan)
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert!(outcomes.contains(&Outcome::Applied(2)));
    assert!(outcomes.contains(&Outcome::AlreadyApplied));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires temporary PostgreSQL"]
async fn invalid_yaml_and_tampered_plan_cannot_write(pool: PgPool) {
    legacy_schema(&pool).await;
    let id = definition(&pool, "flow", "1.0.0", 1).await;
    definition(&pool, "flow", "1.0.0", 2).await;
    let mut plan = plan_legacy_versions(&pool).await.unwrap();
    plan.changes[0].new_version = "50.0.0".into();
    let before = rows(&pool).await;
    assert!(
        apply_legacy_version_plan(&pool, &plan)
            .await
            .unwrap_err()
            .to_string()
            .contains("modified")
    );
    assert_eq!(before, rows(&pool).await);
    sqlx::query(
        "update fluxpro.process_definition set source_definition='broken: [' where uuid=$1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let before = rows(&pool).await;
    assert!(
        plan_legacy_versions(&pool)
            .await
            .unwrap_err()
            .to_string()
            .contains("Invalid source YAML")
    );
    assert_eq!(before, rows(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires temporary PostgreSQL"]
async fn clean_databases_need_no_repair(pool: PgPool) {
    let plan = plan_legacy_versions(&pool).await.unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        apply_legacy_version_plan(&pool, &plan).await.unwrap(),
        Outcome::AlreadyApplied
    );
    migrate(&pool).await.unwrap();
    let plan = plan_legacy_versions(&pool).await.unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(
        apply_legacy_version_plan(&pool, &plan).await.unwrap(),
        Outcome::AlreadyApplied
    );
}
