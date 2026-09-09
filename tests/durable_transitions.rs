//! PostgreSQL fault and concurrency tests. Run with DATABASE_URL and `--ignored`.
#![cfg(feature = "runtime")]

use async_trait::async_trait;
use chrono::{Duration, Utc};
use fluxpro_engine::db_service::task_execution::TaskExecutionChanges;
use fluxpro_engine::db_service::{FluxproDbService, FluxproDbServiceImpl};
use fluxpro_engine::engine::fluxpro_handlers::FluxproHandlersContainer;
use fluxpro_engine::engine::fluxpro_runner::{
    config::RunnerConfig, runner::EngineRunner, shutdown::Shutdown,
};
use fluxpro_engine::models::commands::{
    post_signal::PostSignal, start_process_instance::StartProcessInstance,
};
use fluxpro_engine::models::context_map::context_map::{ContextMap, ContextValue};
use fluxpro_engine::models::context_map::context_patcher::ContextPatcher;
use fluxpro_engine::models::handle_node_result::HandleNodeResult;
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::models::process_def::ProcessDefinition;
use fluxpro_engine::models::queue::queue_task::{
    FluxproQueueTask, FluxproQueueTaskDefinition, TaskProcessOutcome,
};
use fluxpro_engine::service::process_service::{FluxproService, FluxproServiceImpl};
use fluxpro_engine::traits::node_handlers::service_node_handler::FluxproServiceHandler;
use serde_json::json;
use sqlx::PgPool;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tokio::time::{Duration as StdDuration, sleep, timeout};
use uuid::Uuid;

fn id(value: &str) -> IdField {
    IdField::new(value).unwrap()
}

fn definition(service: bool) -> ProcessDefinition {
    let middle = if service {
        json!({"id":"work", "type":"ServiceTask", "handler":"patch", "next":"yes", "set_stage":"working"})
    } else {
        json!({"id":"work", "type":"Wait", "wait_for":{"signals":["approved","rejected"]},
            "timeout":{"after":"PT1H", "on_timeout":"expired"},
            "next":{"branches":[{"when":"ctx[\"_last_signal\"] == \"approved\"", "next":"yes"}], "default":"no"}})
    };
    serde_json::from_value(json!({
        "key":"durable_test", "name":"Durable execution test", "version":"1.0.0", "status":"active",
        "effective_from":"2020-01-01T00:00:00Z", "signals":["approved","rejected"],
        "stages":[{"id":"created","name":"Created","is_initial":true}, "working"],
        "nodes":[{"id":"start","type":"Start","next":"work"}, middle,
            {"id":"yes","type":"End"},{"id":"no","type":"End"},{"id":"expired","type":"End"}]
    }))
    .unwrap()
}

struct Fixture {
    pool: PgPool,
    db: Arc<FluxproDbServiceImpl>,
    service: Arc<FluxproServiceImpl>,
    wakeup: Arc<Notify>,
}

impl Fixture {
    async fn new(pool: PgPool, definition: ProcessDefinition) -> Self {
        fluxpro_engine::migrations::migrate(&pool).await.unwrap();
        let db = Arc::new(FluxproDbServiceImpl::new(pool.clone()));
        let wakeup = Arc::new(Notify::new());
        let service = Arc::new(FluxproServiceImpl::with_queue_wakeup(
            db.clone(),
            wakeup.clone(),
            Default::default(),
        ));
        service.create_process_def(&definition).await.unwrap();
        Self {
            pool,
            db,
            service,
            wakeup,
        }
    }
    async fn start(&self) -> IdField {
        self.service
            .start_process_instance(id("durable_test"), command())
            .await
            .unwrap()
    }
    async fn claim(&self) -> FluxproQueueTaskDefinition {
        self.db
            .fetch_queue_task(&IdField::generate(), 30_000)
            .await
            .unwrap()
            .expect("a due task")
    }
    async fn run(&self) -> FluxproQueueTaskDefinition {
        let task = self.claim().await;
        self.service
            .process_task(task.clone(), registry())
            .await
            .unwrap();
        task
    }
    async fn waiting(&self) -> IdField {
        let token = self.start().await;
        self.run().await;
        self.run().await;
        assert_eq!(self.current(&token).await.as_deref(), Some("work"));
        token
    }
    async fn current(&self, token: &IdField) -> Option<String> {
        self.service
            .get_process_instance_current_node(token)
            .await
            .unwrap()
            .map(|n| n.id().to_string())
    }
    async fn signal(&self, token: &IdField, name: &str) {
        self.service
            .post_signal(
                token.clone(),
                PostSignal {
                    event_id: None,
                    wait_visit_id: None,
                    signal: id(name),
                    context: ContextMap::default(),
                },
            )
            .await
            .unwrap();
    }
    async fn count(&self, query: &str) -> i64 {
        sqlx::query_scalar(query)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn source_exists(&self, uuid: Uuid) -> bool {
        sqlx::query_scalar("select exists(select 1 from fluxpro.queue_runner where uuid=$1)")
            .bind(uuid)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn open(&self) -> bool {
        sqlx::query_scalar("select not wait_completed from fluxpro.process_instance")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn command() -> StartProcessInstance {
    StartProcessInstance {
        process_id: id("business_instance"),
        version: None,
        context: ContextMap([(id("original"), ContextValue::number(1))].into()),
    }
}
fn registry() -> Arc<FluxproHandlersContainer> {
    Arc::new(FluxproHandlersContainer::default())
}

struct PatchHandler(Arc<AtomicUsize>);
#[async_trait]
impl FluxproServiceHandler for PatchHandler {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(HandleNodeResult::success_with_patcher(
            ContextPatcher::builder()
                .set_bool(id("processed"), true)
                .build(),
        ))
    }
}
fn patch_registry() -> (Arc<FluxproHandlersContainer>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(FluxproHandlersContainer::new(vec![Arc::new(PatchHandler(
            calls.clone(),
        ))])),
        calls,
    )
}

async fn trigger(pool: &PgPool, when: &str, body: &str) {
    let sql = format!(
        "create function fluxpro.test_fault() returns trigger language plpgsql as $$ begin {body} end $$; \
        create trigger test_fault {when} on fluxpro.queue_runner for each row execute function fluxpro.test_fault();"
    );
    sqlx::raw_sql(&sql).execute(pool).await.unwrap();
}
async fn clear_trigger(pool: &PgPool) {
    sqlx::raw_sql(
        "drop trigger test_fault on fluxpro.queue_runner; drop function fluxpro.test_fault();",
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn startup_rolls_back_instance_context_stage_and_queue_on_error(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    trigger(
        &f.pool,
        "before insert",
        "raise exception 'injected queue insertion failure';",
    )
    .await;
    assert!(
        f.service
            .start_process_instance(id("durable_test"), command())
            .await
            .is_err()
    );
    for table in [
        "process_instance",
        "process_instance_context_variable",
        "process_instance_stage_log",
        "queue_runner",
        "process_instance_log",
    ] {
        assert_eq!(
            f.count(&format!("select count(*) from fluxpro.{table}"))
                .await,
            0,
            "partial startup in {table}"
        );
    }
    clear_trigger(&f.pool).await;
    f.start().await;
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner").await,
        1
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.process_instance_context_variable")
            .await,
        1
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn late_commit_error_rolls_back_handler_effects_and_replay_cannot_duplicate_successor(
    pool: PgPool,
) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let (handlers, calls) = patch_registry();
    trigger(
        &f.pool,
        "before delete",
        "raise exception 'injected task completion failure';",
    )
    .await;
    assert!(
        f.service
            .process_task(task.clone(), handlers.clone())
            .await
            .is_err()
    );
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_context_variable where name='processed'"
        )
        .await,
        0
    );
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await, 0);
    assert_eq!(
        f.count("select count(*) from fluxpro.process_instance_stage_log")
            .await,
        1
    );
    clear_trigger(&f.pool).await;
    f.service
        .process_task(task.clone(), handlers.clone())
        .await
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "external handler may repeat after rollback"
    );
    assert!(
        f.service
            .process_task(task.clone(), handlers)
            .await
            .is_err()
    );
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await, 1);
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_context_variable where name='processed'"
        )
        .await,
        1
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.process_instance_stage_log")
            .await,
        2
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn queued_opposing_signals_complete_a_wait_only_once(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    f.signal(&token, "rejected").await;
    f.run().await;
    assert!(!f.open().await);
    assert_eq!(f.current(&token).await.as_deref(), Some("work"));
    // Both signals precede the queued successor; the second must not reopen the wait.
    let second = f.run().await;
    assert!(matches!(
        second.task,
        FluxproQueueTask::ProcessSignal { .. }
    ));
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'process_node'")
            .await,
        1
    );
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_log where event_type='signal.accepted'"
        )
        .await,
        1
    );
    assert_eq!(
        f.service
            .get_process_instance_context(&token)
            .await
            .unwrap()
            .as_string(&id("_last_signal"))
            .as_deref(),
        Some("approved")
    );
    f.run().await;
    assert_eq!(f.current(&token).await.as_deref(), Some("yes"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn timeout_wins_before_queued_signal_without_creating_two_paths(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    sqlx::query("update fluxpro.queue_runner set run_after=now()-interval '1 minute' where task ? 'ProcessEvent'").execute(&f.pool).await.unwrap();
    let timer = f.run().await;
    assert!(matches!(timer.task, FluxproQueueTask::ProcessEvent { .. }));
    f.run().await;
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='expired'").await, 1);
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await, 0);
    f.run().await;
    assert_eq!(f.current(&token).await.as_deref(), Some("expired"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn signal_completion_rollback_restores_timeout_and_wait(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    let task = f.claim().await;
    trigger(&f.pool, "before delete", "if OLD.task ? 'ProcessSignal' then raise exception 'injected signal completion failure'; end if; return OLD;").await;
    assert!(
        f.service
            .process_task(task.clone(), registry())
            .await
            .is_err()
    );
    assert!(f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        1
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_cancel_task")
            .await,
        2
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'process_node'")
            .await,
        0
    );
    assert!(
        f.service
            .get_process_instance_context(&token)
            .await
            .unwrap()
            .as_string(&id("_last_signal"))
            .is_none()
    );
    clear_trigger(&f.pool).await;
    f.service.process_task(task, registry()).await.unwrap();
    assert!(!f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        0
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_cancel_task")
            .await,
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn database_read_errors_keep_timeout_task_recoverable(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    sqlx::query("update fluxpro.queue_runner set run_after=now() where task ? 'ProcessEvent'")
        .execute(&f.pool)
        .await
        .unwrap();
    let task = f.claim().await;
    sqlx::query("alter table fluxpro.process_def_node rename to temporarily_unavailable_nodes")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.service
            .process_task(task.clone(), registry())
            .await
            .is_err()
    );
    assert!(
        f.service
            .queue_next_node(
                &token,
                &fluxpro_engine::models::process_def::Next::To(id("yes"))
            )
            .await
            .is_err()
    );
    assert!(f.source_exists(task.uuid).await);
    assert!(f.open().await);
    sqlx::query("alter table fluxpro.temporarily_unavailable_nodes rename to process_def_node")
        .execute(&f.pool)
        .await
        .unwrap();
    f.service.process_task(task, registry()).await.unwrap();
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='expired'").await, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn old_timeout_and_signal_cannot_complete_a_later_visit_to_the_same_node(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let old_timer: Uuid =
        sqlx::query_scalar("select uuid from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    f.signal(&token, "approved").await;
    let work = f.service.get_node(&token, &id("work")).await.unwrap();
    f.service
        .queue_node(&token, &work, Some(Utc::now() - Duration::minutes(1)))
        .await
        .unwrap();
    f.run().await;
    f.run().await; // The old signal is bound to the earlier visit and is discarded.
    sqlx::query(
        "update fluxpro.queue_runner set run_after=now()-interval '1 minute' where uuid=$1",
    )
    .bind(old_timer)
    .execute(&f.pool)
    .await
    .unwrap();
    f.run().await;
    assert!(f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'process_node'")
            .await,
        0
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        1
    );
    // A distinct signal can complete the new visit normally.
    f.signal(&token, "rejected").await;
    f.run().await;
    assert!(!f.open().await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn early_signal_is_delivered_after_wait_entry(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.start().await;
    f.signal(&token, "approved").await;
    f.run().await; // Start; the early signal precedes work in the queue.
    let signal = f.claim().await;
    assert!(matches!(
        signal.task,
        FluxproQueueTask::ProcessSignal { .. }
    ));
    assert_eq!(
        f.service
            .process_task(signal.clone(), registry())
            .await
            .unwrap(),
        TaskProcessOutcome::RetryScheduled
    );
    f.run().await;
    sqlx::query("update fluxpro.queue_runner set run_after=now() where uuid=$1")
        .bind(signal.uuid)
        .execute(&f.pool)
        .await
        .unwrap();
    f.run().await;
    assert!(!f.open().await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn expired_owner_cannot_commit_after_task_is_reclaimed(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    f.start().await;
    let old = f.claim().await;
    let snapshot = f.db.load_task_execution(&old).await.unwrap();
    sqlx::query(
        "update fluxpro.queue_runner set locked_by=now()-interval '1 second' where uuid=$1",
    )
    .bind(old.uuid)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        f.db.commit_task_execution(&old, &snapshot, TaskExecutionChanges::default())
            .await
            .is_err()
    );
    let new = f.claim().await;
    assert_eq!(old.uuid, new.uuid);
    assert_ne!(old.lock_key, new.lock_key);
    assert!(
        f.db.commit_task_execution(&old, &snapshot, TaskExecutionChanges::default())
            .await
            .is_err()
    );
    f.service.process_task(new, registry()).await.unwrap();
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner").await,
        1
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn concurrent_replay_commits_only_one_transition(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    f.start().await;
    let task = f.claim().await;
    let (a, b) = tokio::join!(
        f.service.process_task(task.clone(), registry()),
        f.service.process_task(task, registry())
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='work'").await, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn revision_change_rejects_stale_context_commit(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.start().await;
    let task = f.claim().await;
    let snapshot = f.db.load_task_execution(&task).await.unwrap();
    f.db.add_context_variable(&token, "_", &id("original"), &ContextValue::number(2))
        .await
        .unwrap();
    let changes = TaskExecutionChanges {
        context: Some(snapshot.context.clone()),
        ..Default::default()
    };
    assert!(
        f.db.commit_task_execution(&task, &snapshot, changes)
            .await
            .is_err()
    );
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(
        f.service
            .get_process_instance_context(&token)
            .await
            .unwrap()
            .as_number(&id("original")),
        Some(2)
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn independent_workers_cannot_claim_two_tasks_for_one_instance(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    f.signal(&token, "rejected").await;
    let other = FluxproDbServiceImpl::new(f.pool.clone());
    let a_key = IdField::generate();
    let b_key = IdField::generate();
    let (a, b) = tokio::join!(
        f.db.fetch_queue_task(&a_key, 30_000),
        other.fetch_queue_task(&b_key, 30_000)
    );
    assert_eq!(
        usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
        1
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn heartbeat_progresses_while_commit_holds_the_queue_row_lock(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    trigger(
        &f.pool,
        "before delete",
        "perform pg_sleep(0.2); return OLD;",
    )
    .await;
    let (handlers, _) = patch_registry();
    let mut config = RunnerConfig::with_handlers(handlers);
    config.heartbeat_interval_ms = 20;
    config.task_lease_ms = 5_000;
    config.idle_backoff_ms = 10;
    let runner = EngineRunner::new(f.service.clone(), f.wakeup.clone(), config);
    let shutdown = Shutdown::new();
    let stop = shutdown.clone();
    let worker = tokio::spawn(async move { runner.run_until_stopped(stop).await });
    let completion = timeout(StdDuration::from_secs(10), async {
        loop {
            if f.current(&token).await.as_deref() == Some("yes")
                && f.count("select count(*) from fluxpro.queue_runner").await == 0
            {
                break;
            }
            sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await;
    shutdown.trigger();
    if completion.is_err() {
        worker.abort();
    }
    assert!(
        completion.is_ok(),
        "heartbeat blocked the transaction it was meant to protect"
    );
    worker.await.unwrap().unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn backend_disconnect_during_completion_rolls_back_the_transition(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let (handlers, _) = patch_registry();
    trigger(
        &f.pool,
        "before delete",
        "perform pg_sleep(10); return OLD;",
    )
    .await;
    let service = f.service.clone();
    let attempt = task.clone();
    let run = tokio::spawn(async move { service.process_task(attempt, handlers).await });
    let pid = timeout(StdDuration::from_secs(5), async {
        loop {
            let pid: Option<i32> = sqlx::query_scalar("select pid from pg_stat_activity where datname=current_database() \
                and pid<>pg_backend_pid() and wait_event='PgSleep' and query like 'delete from fluxpro.queue_runner%' limit 1")
                .fetch_optional(&f.pool).await.unwrap();
            if let Some(pid) = pid { break pid; }
            sleep(StdDuration::from_millis(10)).await;
        }
    }).await.unwrap();
    sqlx::query("select pg_terminate_backend($1)")
        .bind(pid)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(run.await.unwrap().is_err());
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_context_variable where name='processed'"
        )
        .await,
        0
    );
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await, 0);
    clear_trigger(&f.pool).await;
    f.service
        .process_task(task, patch_registry().0)
        .await
        .unwrap();
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn migration_preserves_current_timeouts_and_rejects_known_earlier_visits(pool: PgPool) {
    for migration in [
        include_str!("../migrations/0001_initial.sql"),
        include_str!("../migrations/0002_execution_log.sql"),
        include_str!("../migrations/0003_process_definition_source.sql"),
        include_str!("../migrations/0004_process_definition_version_comment.sql"),
        include_str!("../migrations/0005_technical_log_archive_indexes.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    let db = Arc::new(FluxproDbServiceImpl::new(pool.clone()));
    let definition = definition(false);
    let definition_uuid = db
        .create_service_process_def(&definition, definition.compile().unwrap(), None)
        .await
        .unwrap();
    let token = db
        .create_process_instance(definition_uuid, id("legacy_instance"))
        .await
        .unwrap();
    sqlx::query("update fluxpro.process_instance set current_node_ref=(select uuid from fluxpro.process_def_node where process_def_uuid=$1 and node_id='work') where token=$2")
        .bind(definition_uuid).bind(token.get_id()).execute(&pool).await.unwrap();
    let work = definition
        .nodes
        .iter()
        .find(|node| node.id() == &id("work"))
        .unwrap()
        .clone();
    let payload = serde_json::to_value(FluxproQueueTask::ProcessEvent {
        process_token: token.clone(),
        node: work,
        on_time: id("expired"),
    })
    .unwrap();
    let old: Uuid = sqlx::query_scalar("insert into fluxpro.queue_runner (task,created_at,run_after) values ($1,now()-interval '1 hour',now()+interval '1 hour') returning uuid")
        .bind(&payload).fetch_one(&pool).await.unwrap();
    sqlx::query("insert into fluxpro.process_instance_log (process_instance_uuid,level,event_type,source,message,node_id,created_at) select uuid,'info','node.entered','test','reentered','work',now()-interval '30 minutes' from fluxpro.process_instance")
        .execute(&pool).await.unwrap();
    let current: Uuid = sqlx::query_scalar(
        "insert into fluxpro.queue_runner (task,run_after) values ($1,now()) returning uuid",
    )
    .bind(payload)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0006_atomic_transitions.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let visit: Uuid = sqlx::query_scalar("select node_visit_id from fluxpro.process_instance")
        .fetch_one(&pool)
        .await
        .unwrap();
    let current_visit: Uuid =
        sqlx::query_scalar("select node_visit_id from fluxpro.queue_runner where uuid=$1")
            .bind(current)
            .fetch_one(&pool)
            .await
            .unwrap();
    let old_visit: Uuid =
        sqlx::query_scalar("select node_visit_id from fluxpro.queue_runner where uuid=$1")
            .bind(old)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(visit, current_visit);
    assert_ne!(visit, old_visit);
    sqlx::raw_sql(include_str!("../migrations/0007_runtime_safety.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let service = FluxproServiceImpl::new(db.clone());
    let task = db
        .fetch_queue_task(&IdField::generate(), 30_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.uuid, current);
    service.process_task(task, registry()).await.unwrap();
    let count: i64 = sqlx::query_scalar("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='expired'").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn missing_persisted_successor_does_not_acknowledge_source(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    f.start().await;
    f.run().await;
    let task = f.claim().await;
    // Simulate storage corruption beneath the publication guard.
    sqlx::query("alter table fluxpro.process_def_node disable trigger protect_published_child")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("delete from fluxpro.process_def_node where node_id='yes'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.service
            .process_task(task.clone(), patch_registry().0)
            .await
            .is_err()
    );
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_context_variable where name='processed'"
        )
        .await,
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn completed_wait_without_successor_stays_closed(pool: PgPool) {
    let mut definition = definition(false);
    if let fluxpro_engine::models::process_def::Node::Wait { next, .. } = &mut definition.nodes[1] {
        *next = None;
    }
    let f = Fixture::new(pool, definition).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    f.run().await;
    f.signal(&token, "rejected").await;
    f.run().await;
    assert!(!f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner").await,
        0
    );
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_log where event_type='signal.accepted'"
        )
        .await,
        1
    );
}

struct FailOnceHandler(AtomicUsize);
#[async_trait]
impl FluxproServiceHandler for FailOnceHandler {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            anyhow::bail!("transient host failure");
        }
        Ok(HandleNodeResult::success())
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn handler_retry_reuses_visit_without_duplicate_stage_entry(pool: PgPool) {
    let mut definition = definition(true);
    if let fluxpro_engine::models::process_def::Node::ServiceTask { retries, .. } =
        &mut definition.nodes[1]
    {
        *retries = Some(fluxpro_engine::models::process_def::Retries {
            max: 1,
            backoff: "PT1S".into(),
        });
    }
    let f = Fixture::new(pool, definition).await;
    f.start().await;
    f.run().await;
    let task = f.claim().await;
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        FailOnceHandler(AtomicUsize::new(0)),
    )]));
    assert_eq!(
        f.service
            .process_task(task.clone(), handlers.clone())
            .await
            .unwrap(),
        TaskProcessOutcome::RetryScheduled
    );
    sqlx::query("update fluxpro.queue_runner set run_after=now() where uuid=$1")
        .bind(task.uuid)
        .execute(&f.pool)
        .await
        .unwrap();
    let retry = f.claim().await;
    assert_eq!(retry.uuid, task.uuid);
    assert_eq!(retry.attempts, 2);
    f.service.process_task(retry, handlers).await.unwrap();
    assert_eq!(
        f.count("select count(*) from fluxpro.process_instance_stage_log")
            .await,
        2
    );
    assert_eq!(f.count("select count(*) from fluxpro.process_instance_log where event_type='node.entered' and node_id='work'").await,1);
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await,1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn failed_form_hook_does_not_publish_a_partial_wait(pool: PgPool) {
    let mut raw = serde_json::to_value(definition(false)).unwrap();
    raw["nodes"][1]["type"] = json!("UserTask");
    raw["nodes"][1]["form"] = json!("review");
    raw["forms"] = json!([{"id":"review","roles":["reviewer"]}]);
    raw["special_handlers"] = json!({"on_show_form":"patch"});
    let f = Fixture::new(pool, serde_json::from_value(raw).unwrap()).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        FailOnceHandler(AtomicUsize::new(0)),
    )]));
    assert_eq!(
        f.service
            .process_task(task.clone(), handlers.clone())
            .await
            .unwrap(),
        TaskProcessOutcome::RetryScheduled
    );
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        0
    );
    sqlx::query("update fluxpro.queue_runner set run_after=now() where uuid=$1")
        .bind(task.uuid)
        .execute(&f.pool)
        .await
        .unwrap();
    let retry = f.claim().await;
    f.service.process_task(retry, handlers).await.unwrap();
    assert_eq!(f.current(&token).await.as_deref(), Some("work"));
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        1
    );
    f.signal(&token, "approved").await;
    f.signal(&token, "rejected").await;
    f.run().await;
    f.run().await;
    assert!(!f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'process_node'")
            .await,
        1
    );
}

async fn make_due(f: &Fixture, uuid: Uuid) {
    sqlx::query("update fluxpro.queue_runner set run_after=now() where uuid=$1")
        .bind(uuid)
        .execute(&f.pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn exhausted_handler_is_suspended_and_concurrent_resume_replays_once(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    assert_eq!(
        f.service
            .process_task(task.clone(), registry())
            .await
            .unwrap(),
        TaskProcessOutcome::Suspended
    );
    let incident = f.db.get_open_incident(&token).await.unwrap().unwrap();
    assert_eq!(incident.kind, "handler.retries_exhausted");
    assert!(incident.reason.contains("not registered"));
    assert_eq!(incident.task_uuid, task.uuid);
    assert!(f.source_exists(task.uuid).await);
    f.signal(&token, "approved").await;
    assert!(
        f.db.fetch_queue_task(&id("other_worker"), 30_000)
            .await
            .unwrap()
            .is_none()
    );
    let (a, b) = tokio::join!(
        f.db.resume_instance(&token, incident.uuid),
        f.db.resume_instance(&token, incident.uuid)
    );
    assert_ne!(a.unwrap(), b.unwrap());
    let resumed = f.claim().await;
    assert_eq!(resumed.uuid, task.uuid);
    assert_eq!(resumed.attempts, 1);
    let (handlers, calls) = patch_registry();
    assert_eq!(
        f.service.process_task(resumed, handlers).await.unwrap(),
        TaskProcessOutcome::Completed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(f.db.get_open_incident(&token).await.unwrap().is_none());
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await,1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn failed_incident_write_rolls_back_suspension_and_retains_lease(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    sqlx::raw_sql("create function fluxpro.reject_incident() returns trigger language plpgsql as $$ begin raise exception 'incident storage failed'; end $$; create trigger fault before insert on fluxpro.process_incident for each row execute function fluxpro.reject_incident();")
        .execute(&f.pool).await.unwrap();
    assert!(
        f.service
            .process_task(task.clone(), registry())
            .await
            .is_err()
    );
    assert!(f.db.get_open_incident(&token).await.unwrap().is_none());
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
    assert!(f.source_exists(task.uuid).await);
    sqlx::query("drop trigger fault on fluxpro.process_incident")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        f.service.process_task(task, registry()).await.unwrap(),
        TaskProcessOutcome::Suspended
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn lifecycle_errors_have_a_bounded_budget_and_an_incident(pool: PgPool) {
    let mut raw = serde_json::to_value(definition(false)).unwrap();
    raw["nodes"][1]["type"] = json!("UserTask");
    raw["nodes"][1]["form"] = json!("review");
    raw["forms"] = json!([{"id":"review","roles":[]}]);
    raw["special_handlers"] = json!({"on_show_form":"missing"});
    let f = Fixture::new(pool, serde_json::from_value(raw).unwrap()).await;
    let token = f.start().await;
    f.run().await;
    for attempt in 1..=3 {
        let task = f.claim().await;
        let result = f
            .service
            .process_task(task.clone(), registry())
            .await
            .unwrap();
        assert_eq!(
            result,
            if attempt == 3 {
                TaskProcessOutcome::Suspended
            } else {
                TaskProcessOutcome::RetryScheduled
            }
        );
        if attempt < 3 {
            make_due(&f, task.uuid).await;
        }
    }
    assert_eq!(
        f.db.get_open_incident(&token).await.unwrap().unwrap().kind,
        "execution.retries_exhausted"
    );
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn invalid_expression_registration_does_not_write_definition(pool: PgPool) {
    fluxpro_engine::migrations::migrate(&pool).await.unwrap();
    let db = Arc::new(FluxproDbServiceImpl::new(pool.clone()));
    let svc =
        FluxproServiceImpl::with_queue_wakeup(db, Arc::new(Notify::new()), Default::default());
    let mut raw = serde_json::to_value(definition(false)).unwrap();
    raw["nodes"][1]["next"]["branches"][0]["when"] = json!("ctx._last_signal ==");
    assert!(matches!(
        svc.create_process_def(&serde_json::from_value(raw).unwrap())
            .await,
        Err(fluxpro_engine::models::process_def_error::CreateProcessError::ValidationError(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from fluxpro.process_definition")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn signal_condition_error_cannot_choose_fallback_or_consume_wait(pool: PgPool) {
    let mut raw = serde_json::to_value(definition(false)).unwrap();
    raw["nodes"][1]["next"]["branches"][0]["when"] = json!("ctx.decision > 0");
    let f = Fixture::new(pool, serde_json::from_value(raw).unwrap()).await;
    let token = f.waiting().await;
    f.signal(&token, "approved").await;
    let task = f.claim().await;
    assert_eq!(
        f.service
            .process_task(task.clone(), registry())
            .await
            .unwrap(),
        TaskProcessOutcome::Suspended
    );
    let incident = f.db.get_open_incident(&token).await.unwrap().unwrap();
    assert_eq!(incident.kind, "condition.failed");
    assert!(f.open().await);
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'process_node'")
            .await,
        0
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessEvent'")
            .await,
        1
    );
    let context = f
        .service
        .get_process_instance_context(&token)
        .await
        .unwrap();
    assert!(!context.0.contains_key(&id("_last_signal")));
    f.db.save_process_instance_context(
        &token,
        &ContextMap([(id("decision"), ContextValue::number(1))].into()),
    )
    .await
    .unwrap();
    f.db.resume_instance(&token, incident.uuid).await.unwrap();
    assert_eq!(f.run().await.uuid, task.uuid);
    assert!(!f.open().await);
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await,1);
}

struct RemoveHandler;
#[async_trait]
impl FluxproServiceHandler for RemoveHandler {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        Ok(HandleNodeResult::success_with_patcher(
            ContextPatcher::builder()
                .set_number(id("original"), 2)
                .remove(id("original"))
                .remove(id("retained"))
                .set_number(id("retained"), 3)
                .remove(id("absent"))
                .build(),
        ))
    }
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn ordered_patch_removals_persist_and_rollback_with_successor(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(RemoveHandler)]));
    trigger(&f.pool, "before delete", "raise exception 'ack failed';").await;
    assert!(
        f.service
            .process_task(task.clone(), handlers.clone())
            .await
            .is_err()
    );
    let before = f
        .service
        .get_process_instance_context(&token)
        .await
        .unwrap();
    assert_eq!(before.0[&id("original")], ContextValue::number(1));
    assert!(!before.0.contains_key(&id("retained")));
    clear_trigger(&f.pool).await;
    f.service.process_task(task, handlers).await.unwrap();
    let after = f
        .service
        .get_process_instance_context(&token)
        .await
        .unwrap();
    assert!(!after.0.contains_key(&id("original")));
    assert_eq!(after.0[&id("retained")], ContextValue::number(3));
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.process_instance_context_variable where name='original'"
        )
        .await,
        0
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn version_selection_ignores_drafts_future_and_deprecated_versions(pool: PgPool) {
    use fluxpro_engine::models::process_def::ProcessStatus;
    use fluxpro_engine::models::version_id::VersionId;
    let f = Fixture::new(pool, definition(false)).await;
    for (version, status, from) in [
        (
            "2.0.0",
            ProcessStatus::Draft,
            Utc::now() - Duration::days(1),
        ),
        (
            "3.0.0",
            ProcessStatus::Active,
            Utc::now() + Duration::days(1),
        ),
        (
            "4.0.0",
            ProcessStatus::Deprecated,
            Utc::now() - Duration::days(1),
        ),
    ] {
        let mut d = definition(false);
        d.version = VersionId::new(version).unwrap();
        d.status = status;
        d.effective_from = Some(from);
        f.service.create_process_def(&d).await.unwrap();
        assert!(
            f.service
                .get_process(id("durable_test"), Some(d.version))
                .await
                .is_err()
        );
    }
    assert_eq!(
        f.service
            .get_process(id("durable_test"), None)
            .await
            .unwrap()
            .version
            .as_str(),
        "1.0.0"
    );
    let token = f.start().await;
    sqlx::query("update fluxpro.process_definition set status='Deprecated' where version='1.0.0'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.service
            .get_process(id("durable_test"), None)
            .await
            .is_err()
    );
    f.run().await;
    f.run().await;
    assert_eq!(f.current(&token).await.as_deref(), Some("work"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn concurrent_registration_is_unique_and_publication_is_immutable(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let mut d = definition(false);
    d.version = fluxpro_engine::models::version_id::VersionId::new("2.0.0").unwrap();
    let (a, b) = tokio::join!(
        f.service.create_process_def(&d),
        f.service.create_process_def(&d)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        f.count("select count(*) from fluxpro.process_definition where version='2.0.0'")
            .await,
        1
    );
    for query in [
        "update fluxpro.process_definition set definition='{}'",
        "update fluxpro.process_definition set status='Draft'",
        "delete from fluxpro.process_definition",
        "update fluxpro.process_def_node set definition='{}'",
        "delete from fluxpro.process_def_node",
        "update fluxpro.process_stage set is_initial=false",
        "insert into fluxpro.process_def_signal(process_def_uuid,signal_id) select uuid,'extra' from fluxpro.process_definition",
    ] {
        assert!(
            sqlx::query(query).execute(&f.pool).await.is_err(),
            "accepted mutation: {query}"
        );
    }
}

fn identified_signal(event: &str, name: &str) -> PostSignal {
    PostSignal {
        event_id: Some(event.into()),
        wait_visit_id: None,
        signal: id(name),
        context: ContextMap::default(),
    }
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn event_identity_survives_interleaving_and_history_cleanup(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let a = identified_signal("event-a", "approved");
    assert!(f.db.enqueue_signal_if_new(&token, &a).await.unwrap());
    assert!(
        f.db.enqueue_signal_if_new(&token, &identified_signal("event-b", "rejected"))
            .await
            .unwrap()
    );
    assert!(!f.db.enqueue_signal_if_new(&token, &a).await.unwrap());
    sqlx::query("delete from fluxpro.signal_history")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!f.db.enqueue_signal_if_new(&token, &a).await.unwrap());
    let mut conflict = a.clone();
    conflict
        .context
        .0
        .insert(id("changed"), ContextValue::number(1));
    assert!(f.db.enqueue_signal_if_new(&token, &conflict).await.is_err());
    let mut conflict = a.clone();
    conflict.wait_visit_id = Some(Uuid::new_v4());
    assert!(f.db.enqueue_signal_if_new(&token, &conflict).await.is_err());
    assert!(
        f.db.enqueue_signal_if_new(&token, &identified_signal("event-a", "rejected"))
            .await
            .is_err()
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessSignal'")
            .await,
        2
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn concurrent_event_delivery_creates_one_receipt_and_task(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let signal = identified_signal("same-event", "approved");
    let (a, b) = tokio::join!(
        f.db.enqueue_signal_if_new(&token, &signal),
        f.db.enqueue_signal_if_new(&token, &signal)
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert_eq!(
        f.count("select count(*) from fluxpro.signal_receipt").await,
        1
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.signal_history").await,
        1
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.queue_runner where task ? 'ProcessSignal'")
            .await,
        1
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn failed_signal_enqueue_does_not_consume_event_identity(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let signal = identified_signal("retry-event", "approved");
    trigger(
        &f.pool,
        "before insert",
        "raise exception 'queue unavailable';",
    )
    .await;
    assert!(f.db.enqueue_signal_if_new(&token, &signal).await.is_err());
    assert_eq!(
        f.count("select count(*) from fluxpro.signal_receipt").await,
        0
    );
    assert_eq!(
        f.count("select count(*) from fluxpro.signal_history").await,
        0
    );
    clear_trigger(&f.pool).await;
    assert!(f.db.enqueue_signal_if_new(&token, &signal).await.unwrap());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn equal_payloads_can_complete_later_visits_but_explicit_old_visit_cannot(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let visit: Uuid = sqlx::query_scalar("select node_visit_id from fluxpro.process_instance")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let first = identified_signal("first", "approved");
    f.db.enqueue_signal_if_new(&token, &first).await.unwrap();
    f.run().await;
    f.run().await;
    f.service
        .queue_node(&token, &definition(false).nodes[1], None)
        .await
        .unwrap();
    f.run().await;
    let mut stale = identified_signal("delayed", "approved");
    stale.wait_visit_id = Some(visit);
    f.db.enqueue_signal_if_new(&token, &stale).await.unwrap();
    f.run().await;
    assert!(f.open().await);
    assert!(!f.db.enqueue_signal_if_new(&token, &first).await.unwrap());
    let second = identified_signal("second", "approved");
    assert!(f.db.enqueue_signal_if_new(&token, &second).await.unwrap());
    f.run().await;
    assert!(!f.open().await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn legacy_signal_without_event_id_is_an_independent_delivery(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    let signal: PostSignal =
        serde_json::from_value(json!({"signal":"approved","context":{}})).unwrap();
    assert!(signal.event_id.is_none());
    assert!(f.db.enqueue_signal_if_new(&token, &signal).await.unwrap());
    assert!(f.db.enqueue_signal_if_new(&token, &signal).await.unwrap());
    f.run().await;
    f.run().await;
    assert_eq!(f.count("select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='yes'").await,1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn configured_retry_exhaustion_routes_or_suspends_without_losing_task(pool: PgPool) {
    use fluxpro_engine::models::process_def::{Node, OnError, Retries};
    let mut d = definition(true);
    if let Node::ServiceTask {
        retries, on_error, ..
    } = &mut d.nodes[1]
    {
        *retries = Some(Retries {
            max: 2,
            backoff: "PT1S".into(),
        });
        *on_error = Some(OnError {
            next: Some(id("no")),
            compensate: None,
        });
    }
    let f = Fixture::new(pool, d).await;
    let token = f.start().await;
    f.run().await;
    let mut source = None;
    for attempt in 1..=3 {
        let task = f.claim().await;
        if let Some(source) = source {
            assert_eq!(source, task.uuid);
        } else {
            source = Some(task.uuid);
        }
        assert_eq!(task.attempts, attempt);
        assert_eq!(
            f.service
                .process_task(task.clone(), registry())
                .await
                .unwrap(),
            if attempt == 3 {
                TaskProcessOutcome::Completed
            } else {
                TaskProcessOutcome::RetryScheduled
            }
        );
        if attempt < 3 {
            make_due(&f, task.uuid).await;
        }
    }
    assert!(f.db.get_open_incident(&token).await.unwrap().is_none());
    assert!(!f.source_exists(source.unwrap()).await);
    assert_eq!(
        f.count(
            "select count(*) from fluxpro.queue_runner where task #>> '{process_node,node,id}'='no'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn failed_resume_rolls_back_resolution_state_and_retry_budget(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    f.service
        .process_task(task.clone(), registry())
        .await
        .unwrap();
    let incident = f.service.get_open_incident(&token).await.unwrap().unwrap();
    sqlx::raw_sql("create function fluxpro.reject_resume() returns trigger language plpgsql as $$ begin if new.event_type='instance.resumed' then raise exception 'resume audit unavailable'; end if; return new; end $$; create trigger fault before insert on fluxpro.process_instance_log for each row execute function fluxpro.reject_resume();")
        .execute(&f.pool).await.unwrap();
    assert!(
        f.service
            .resume_instance(&token, incident.uuid)
            .await
            .is_err()
    );
    assert_eq!(
        f.service
            .get_open_incident(&token)
            .await
            .unwrap()
            .unwrap()
            .uuid,
        incident.uuid
    );
    assert_eq!(
        f.count("select attempts::bigint from fluxpro.queue_runner")
            .await,
        1
    );
    assert!(
        f.db.fetch_queue_task(&id("worker"), 30_000)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("drop trigger fault on fluxpro.process_instance_log")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.service
            .resume_instance(&token, incident.uuid)
            .await
            .unwrap()
    );
    assert!(
        !f.service
            .resume_instance(&token, incident.uuid)
            .await
            .unwrap()
    );
    assert_eq!(f.claim().await.uuid, task.uuid);
}

#[cfg(feature = "admin")]
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn admin_reports_suspension_and_prevents_cancelling_incident_task(pool: PgPool) {
    use fluxpro_engine::admin::{
        FluxproAdminService, PageRequest, ProcessInstanceFilter, QueueTaskFilter,
    };
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    f.service
        .process_task(task.clone(), registry())
        .await
        .unwrap();
    let admin = FluxproAdminService::new(f.pool.clone());
    assert!(!admin.cancel_queue_task(task.uuid).await.unwrap());
    let incident = admin.get_open_incident(&token).await.unwrap().unwrap();
    let instances = admin
        .list_process_instances(ProcessInstanceFilter::default(), PageRequest::default())
        .await
        .unwrap();
    assert_eq!(instances.items[0].state, "suspended");
    let tasks = admin
        .list_queue_tasks(QueueTaskFilter::default(), PageRequest::default())
        .await
        .unwrap();
    assert_eq!(tasks.items[0].state, "suspended");
    admin.resume_instance(&token, incident.uuid).await.unwrap();
    assert!(!admin.cancel_queue_task(task.uuid).await.unwrap());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn explicit_context_patch_removes_keys_and_fences_stale_execution(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let snapshot = f.db.load_task_execution(&task).await.unwrap();
    f.db.apply_process_instance_patch(
        &token,
        &ContextPatcher::builder().remove(id("original")).build(),
    )
    .await
    .unwrap();
    assert!(
        !f.db
            .get_process_instance_context(&token)
            .await
            .unwrap()
            .0
            .contains_key(&id("original"))
    );
    assert!(
        f.db.commit_task_execution(&task, &snapshot, TaskExecutionChanges::default())
            .await
            .is_err()
    );
    assert!(f.source_exists(task.uuid).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn upgrade_rejects_duplicate_versions_without_deleting_legacy_data(pool: PgPool) {
    for migration in [
        include_str!("../migrations/0001_initial.sql"),
        include_str!("../migrations/0002_execution_log.sql"),
        include_str!("../migrations/0003_process_definition_source.sql"),
        include_str!("../migrations/0004_process_definition_version_comment.sql"),
        include_str!("../migrations/0005_technical_log_archive_indexes.sql"),
        include_str!("../migrations/0006_atomic_transitions.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    let db = FluxproDbServiceImpl::new(pool.clone());
    let d = definition(false);
    for _ in 0..2 {
        db.create_service_process_def(&d, d.compile().unwrap(), None)
            .await
            .unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    let error = sqlx::raw_sql(include_str!("../migrations/0007_runtime_safety.sql"))
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("process_definition_key_version_unique")
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from fluxpro.process_definition")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("select count(*) from information_schema.columns where table_schema='fluxpro' and table_name='process_instance' and column_name='execution_state'").fetch_one(&pool).await.unwrap(),0);
}

#[cfg(feature = "api")]
#[path = "support/submitted_processes.rs"]
mod submitted_processes;

#[cfg(feature = "api")]
#[path = "support/business_processes.rs"]
mod business_processes;

struct UncertainHandler {
    panic: bool,
}
#[async_trait]
impl FluxproServiceHandler for UncertainHandler {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        if self.panic {
            panic!("injected host panic");
        }
        std::future::pending::<()>().await;
        unreachable!()
    }
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn hung_handler_suspends_without_on_error_route_or_automatic_retry(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let service = FluxproServiceImpl::new(f.db.clone())
        .with_handler_timeout(StdDuration::from_millis(20))
        .unwrap();
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        UncertainHandler { panic: false },
    )]));
    let outcome = timeout(
        StdDuration::from_secs(1),
        service.process_task(task.clone(), handlers),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(outcome, TaskProcessOutcome::Suspended);
    assert!(f.source_exists(task.uuid).await);
    assert_eq!(
        f.db.get_open_incident(&token).await.unwrap().unwrap().kind,
        "handler.timed_out"
    );
    assert_eq!(f.current(&token).await.as_deref(), Some("start"));
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn panicking_handler_creates_incident_instead_of_restarting_forever(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    let token = f.start().await;
    f.run().await;
    let task = f.claim().await;
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        UncertainHandler { panic: true },
    )]));
    assert_eq!(
        f.service
            .process_task(task.clone(), handlers)
            .await
            .unwrap(),
        TaskProcessOutcome::Suspended
    );
    assert_eq!(
        f.db.get_open_incident(&token).await.unwrap().unwrap().kind,
        "handler.panicked"
    );
    assert!(f.source_exists(task.uuid).await);
}
struct IdentifiedHandler(
    Arc<
        std::sync::Mutex<
            Vec<fluxpro_engine::traits::node_handlers::service_node_handler::HandlerExecution>,
        >,
    >,
);
#[async_trait]
impl FluxproServiceHandler for IdentifiedHandler {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        panic!("legacy entry unexpectedly called")
    }
    async fn process_node_with_execution(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
        execution: &fluxpro_engine::traits::node_handlers::service_node_handler::HandlerExecution,
    ) -> anyhow::Result<HandleNodeResult> {
        self.0.lock().unwrap().push(execution.clone());
        Ok(HandleNodeResult::success())
    }
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn replay_passes_same_operation_key_to_host(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    f.start().await;
    f.run().await;
    let task = f.claim().await;
    let calls = Arc::new(std::sync::Mutex::new(vec![]));
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        IdentifiedHandler(calls.clone()),
    )]));
    trigger(&f.pool, "before delete", "raise exception 'lost ack';").await;
    assert!(
        f.service
            .process_task(task.clone(), handlers.clone())
            .await
            .is_err()
    );
    clear_trigger(&f.pool).await;
    f.service
        .process_task(task.clone(), handlers)
        .await
        .unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].operation_id, calls[1].operation_id);
    assert_eq!(calls[0].task_id, task.uuid);
    assert_eq!(calls[0].node_id, id("work"));
    assert_eq!(calls[0].instance_revision, calls[1].instance_revision);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn many_dequeuers_never_lease_two_tasks_for_one_instance(pool: PgPool) {
    let f = Fixture::new(pool, definition(false)).await;
    let token = f.waiting().await;
    for _ in 0..16 {
        f.signal(&token, "approved").await;
    }
    let mut workers = tokio::task::JoinSet::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(16));
    for _ in 0..16 {
        let db = f.db.clone();
        let barrier = barrier.clone();
        workers.spawn(async move {
            barrier.wait().await;
            db.fetch_queue_task(&IdField::generate(), 30_000)
                .await
                .unwrap()
        });
    }
    let mut leased = 0;
    while let Some(result) = workers.join_next().await {
        if result.unwrap().is_some() {
            leased += 1;
        }
    }
    assert_eq!(leased, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn blocked_heartbeat_cannot_resurrect_an_expired_lease(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    f.start().await;
    f.run().await;
    let task = f.claim().await;
    sqlx::query("update fluxpro.queue_runner set locked_by=clock_timestamp()+interval '100 milliseconds' where uuid=$1").bind(task.uuid).execute(&f.pool).await.unwrap();
    let mut block = f.pool.begin().await.unwrap();
    sqlx::query("select uuid from fluxpro.queue_runner where uuid=$1 for update")
        .bind(task.uuid)
        .fetch_one(&mut *block)
        .await
        .unwrap();
    let db = f.db.clone();
    let key = task.lock_key.clone();
    let uuid = task.uuid;
    let renewal = tokio::spawn(async move { db.renew_queue_task_lock(uuid, &key, 30_000).await });
    sleep(StdDuration::from_millis(160)).await;
    block.commit().await.unwrap();
    assert!(!renewal.await.unwrap().unwrap());
    let reclaimed = f.claim().await;
    assert_eq!(reclaimed.uuid, task.uuid);
    assert_ne!(reclaimed.lock_key, task.lock_key);
}

struct PendingHost {
    started: Arc<Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
struct DropMarker(Arc<std::sync::atomic::AtomicBool>);
impl Drop for DropMarker {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl FluxproServiceHandler for PendingHost {
    fn get_name(&self) -> IdField {
        id("patch")
    }
    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        let _marker = DropMarker(self.dropped.clone());
        self.started.notify_one();
        std::future::pending::<()>().await;
        unreachable!()
    }
}
#[sqlx::test(migrations = false)]
#[ignore = "requires PostgreSQL DATABASE_URL; CI runs this suite explicitly"]
async fn runner_cancels_host_when_heartbeat_is_blocked_past_deadline(pool: PgPool) {
    let f = Fixture::new(pool, definition(true)).await;
    f.start().await;
    f.run().await;
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(PendingHost {
        started: started.clone(),
        dropped: dropped.clone(),
    })]));
    let runner = EngineRunner::new(
        f.service.clone(),
        f.wakeup.clone(),
        RunnerConfig {
            concurrency: 1,
            idle_backoff_ms: 5,
            task_lease_ms: 200,
            heartbeat_interval_ms: 20,
            container: handlers,
        },
    );
    let shutdown = Shutdown::new();
    let stop = shutdown.clone();
    let worker = tokio::spawn(async move { runner.run_until_stopped(stop).await });
    timeout(StdDuration::from_secs(2), started.notified())
        .await
        .unwrap();
    let mut block = f.pool.begin().await.unwrap();
    let source:Uuid=sqlx::query_scalar("select uuid from fluxpro.queue_runner where task #>> '{process_node,node,id}'='work' for update").fetch_one(&mut *block).await.unwrap();
    timeout(StdDuration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    shutdown.trigger();
    timeout(StdDuration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    block.commit().await.unwrap();
    assert!(f.source_exists(source).await);
}
