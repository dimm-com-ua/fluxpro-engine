//! Shared business catalog and a PostgreSQL-backed stepper with simulated host services.
//!
//! This module is deliberately demo code. Its in-memory response cache is not a
//! durable receipt store, and it never calculates money or calls a provider.
use anyhow::{Context, ensure};
use async_trait::async_trait;
use fluxpro_engine::db_service::{FluxproDbService, FluxproDbServiceImpl};
use fluxpro_engine::engine::fluxpro_handlers::FluxproHandlersContainer;
use fluxpro_engine::models::commands::{
    post_signal::PostSignal, start_process_instance::StartProcessInstance,
};
use fluxpro_engine::models::context_map::{
    context_map::ContextMap, context_patcher::ContextPatcher,
};
use fluxpro_engine::models::handle_node_result::HandleNodeResult;
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::models::process_def::{Node, ProcessDefinition};
use fluxpro_engine::models::queue::queue_task::{FluxproQueueTask, TaskProcessOutcome};
use fluxpro_engine::service::process_service::{FluxproService, FluxproServiceImpl};
use fluxpro_engine::traits::node_handlers::service_node_handler::{
    FluxproServiceHandler, HandlerExecution,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const DEFINITIONS: &[(&str, &str)] = &[
    (
        "pizza_order",
        include_str!("../definitions/business/pizza_order.yaml"),
    ),
    (
        "home_repair",
        include_str!("../definitions/business/home_repair.yaml"),
    ),
    (
        "english_school",
        include_str!("../definitions/business/english_school.yaml"),
    ),
    (
        "loan_lifecycle",
        include_str!("../definitions/business/loan_lifecycle.yaml"),
    ),
    (
        "retail_return",
        include_str!("../definitions/business/retail_return.yaml"),
    ),
    (
        "insurance_claim",
        include_str!("../definitions/business/insurance_claim.yaml"),
    ),
    (
        "subscription_renewal",
        include_str!("../definitions/business/subscription_renewal.yaml"),
    ),
];

/// A script supplies external observations, while the engine chooses every route.
#[derive(Clone, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub workflow: String,
    pub context: ContextMap,
    #[serde(default)]
    pub responses: HashMap<String, Vec<ContextMap>>,
    pub steps: Vec<Step>,
    /// Complete expected node sequence, including repeated visits.
    pub expected_nodes: String,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Step {
    Signal {
        at: String,
        name: String,
        #[serde(default)]
        duplicate: bool,
    },
    Timeout {
        at: String,
    },
}

pub fn scenarios() -> anyhow::Result<Vec<Scenario>> {
    Ok(serde_json::from_str(include_str!(
        "../scenarios/business.json"
    ))?)
}

pub fn definition(name: &str) -> anyhow::Result<ProcessDefinition> {
    let (_, source) = DEFINITIONS
        .iter()
        .find(|(key, _)| *key == name)
        .with_context(|| format!("unknown workflow {name}"))?;
    Ok(serde_yaml::from_str(source)?)
}

#[derive(Default)]
struct Simulation {
    responses: HashMap<String, VecDeque<ContextMap>>,
    receipts: HashMap<String, ContextMap>,
}

struct SimulatedHandler {
    name: IdField,
    state: Arc<Mutex<Simulation>>,
}

#[async_trait]
impl FluxproServiceHandler for SimulatedHandler {
    fn get_name(&self) -> IdField {
        self.name.clone()
    }

    async fn process_node(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        anyhow::bail!("the queued runtime must supply execution metadata")
    }

    async fn process_node_with_execution(
        &self,
        _: &IdField,
        _: &ContextMap,
        _: Option<&ContextMap>,
        execution: &HandlerExecution,
    ) -> anyhow::Result<HandleNodeResult> {
        let mut state = self.state.lock().unwrap();
        let patch = if let Some(previous) = state.receipts.get(&execution.operation_id) {
            previous.clone()
        } else {
            let response = if let Some(script) = state.responses.get_mut(self.name.get_id()) {
                script
                    .pop_front()
                    .with_context(|| format!("{} ran more times than scripted", self.name))?
            } else {
                ContextMap::default()
            };
            state
                .receipts
                .insert(execution.operation_id.clone(), response.clone());
            response
        };
        let mut builder = ContextPatcher::builder();
        for (key, value) in patch.0 {
            builder = builder.set(key, value);
        }
        Ok(HandleNodeResult::success_with_patcher(builder.build()))
    }
}

/// Runs only in a disposable database without other runners. Records are retained.
/// Timers are moved forward explicitly; the real queue dispatcher commits transitions.
pub async fn run(pool: PgPool, scenario: Scenario) -> anyhow::Result<IdField> {
    fluxpro_engine::migrations::migrate(&pool).await?;
    let existing: i64 = sqlx::query_scalar("select count(*) from fluxpro.queue_runner")
        .fetch_one(&pool)
        .await?;
    ensure!(
        existing == 0,
        "business demo requires an idle disposable database without other runners"
    );
    let mut definition = definition(&scenario.workflow)?;
    definition.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    // Separate demo runs must not overwrite a previously published definition.
    definition.key = IdField::new(format!("{}_{}", definition.key, Uuid::new_v4().simple()))?;
    let names: HashSet<_> = definition
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::ServiceTask { handler, .. } => Some(handler.clone()),
            _ => None,
        })
        .collect();
    for name in scenario.responses.keys() {
        ensure!(
            names.contains(&IdField::new(name)?),
            "unknown scripted handler {name}"
        );
    }
    let state = Arc::new(Mutex::new(Simulation {
        responses: scenario
            .responses
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect(),
        ..Default::default()
    }));
    let handlers = Arc::new(FluxproHandlersContainer::new(
        names
            .into_iter()
            .map(|name| {
                Arc::new(SimulatedHandler {
                    name,
                    state: state.clone(),
                }) as Arc<dyn FluxproServiceHandler + Send + Sync>
            })
            .collect(),
    ));
    let db = Arc::new(FluxproDbServiceImpl::new(pool.clone()));
    let service = FluxproServiceImpl::new(db.clone());
    service
        .create_process_def(&definition)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let token = service
        .start_process_instance(
            definition.key.clone(),
            StartProcessInstance {
                process_id: IdField::generate(),
                version: Some(definition.version.clone()),
                context: scenario.context,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut visited = vec![];
    drain(&db, &service, &handlers, &token, &mut visited).await?;
    for (index, step) in scenario.steps.into_iter().enumerate() {
        let (Step::Signal { ref at, .. } | Step::Timeout { ref at }) = step;
        let node = service
            .get_process_instance_current_node(&token)
            .await?
            .context("missing current node")?;
        ensure!(
            node.id().get_id() == at,
            "{}: expected {at}, reached {}",
            scenario.name,
            node.id()
        );
        let visit: Uuid =
            sqlx::query_scalar("select node_visit_id from fluxpro.process_instance where token=$1")
                .bind(token.get_id())
                .fetch_one(&pool)
                .await?;
        match step {
            Step::Signal {
                name, duplicate, ..
            } => {
                let signal = PostSignal {
                    event_id: Some(format!("{}:{index}", scenario.name)),
                    wait_visit_id: Some(visit),
                    signal: IdField::new(name)?,
                    context: ContextMap::default(),
                };
                service
                    .post_signal(token.clone(), signal.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if duplicate {
                    service
                        .post_signal(token.clone(), signal)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
            }
            Step::Timeout { .. } => {
                let affected = sqlx::query("update fluxpro.queue_runner set run_after=clock_timestamp()-interval '1 second' where node_visit_id=$1 and task ? 'ProcessEvent'")
                    .bind(visit).execute(&pool).await?.rows_affected();
                ensure!(affected == 1, "expected one timeout for this visit");
            }
        }
        drain(&db, &service, &handlers, &token, &mut visited).await?;
    }
    let expected: Vec<_> = scenario.expected_nodes.split_whitespace().collect();
    ensure!(
        visited == expected,
        "{}: node sequence differs\nexpected: {expected:?}\nactual: {visited:?}",
        scenario.name
    );
    let current = service
        .get_process_instance_current_node(&token)
        .await?
        .context("missing final node")?;
    ensure!(current.is_end(), "scenario did not finish at an End");
    ensure!(
        service.get_open_incident(&token).await?.is_none(),
        "scenario has an incident"
    );
    ensure!(
        state
            .lock()
            .unwrap()
            .responses
            .values()
            .all(|v| v.is_empty()),
        "some scripted responses were unused"
    );
    let queued: i64 = sqlx::query_scalar("select count(*) from fluxpro.queue_runner")
        .fetch_one(&pool)
        .await?;
    ensure!(queued == 0, "finished scenario retained queue tasks");
    Ok(token)
}

async fn drain(
    db: &FluxproDbServiceImpl,
    service: &FluxproServiceImpl,
    handlers: &Arc<FluxproHandlersContainer>,
    token: &IdField,
    visited: &mut Vec<String>,
) -> anyhow::Result<()> {
    for _ in 0..100 {
        let Some(task) = db.fetch_queue_task(&IdField::generate(), 30_000).await? else {
            return Ok(());
        };
        ensure!(
            task.task.process_token() == token,
            "another process is running in the demo database"
        );
        if let FluxproQueueTask::ProcessNode { node, .. } = &task.task {
            visited.push(node.id().to_string());
        }
        ensure!(
            service.process_task(task, handlers.clone()).await? == TaskProcessOutcome::Completed,
            "scenario handler failed or suspended"
        );
    }
    anyhow::bail!("scenario failed to reach a wait within 100 tasks")
}
