//! Prepares workflow effects outside transactions and commits them through the persistence fence.

use super::*;
use crate::db_service::task_execution::{
    ScheduledTask, TaskExecutionChanges, TaskExecutionSnapshot,
};
use crate::models::process_def::{NodeTimeout, TaskHandler};

fn event(
    task: &FluxproQueueTaskDefinition,
    kind: &str,
    message: impl Into<String>,
) -> ExecutionLogEvent {
    let mut event = ExecutionLogEvent::info(kind, "process_service", message);
    event.queue_task_uuid = Some(task.uuid);
    event.attempt = Some(task.attempts);
    event
}

fn node(snapshot: &TaskExecutionSnapshot, id: &IdField) -> anyhow::Result<Node> {
    snapshot
        .definition
        .nodes
        .iter()
        .find(|node| node.id() == id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("node {id} missing from instance definition"))
}

fn queue_node(
    changes: &mut TaskExecutionChanges,
    token: &IdField,
    node: Node,
    at: Option<DateTime<Utc>>,
) {
    changes.tasks.push(ScheduledTask {
        task: FluxproQueueTask::ProcessNode {
            process_token: token.clone(),
            node,
        },
        run_after: at,
        visit_id: None,
        cancel_signals: vec![],
    });
}

async fn select(
    _service: &FluxproServiceImpl,
    context: &ContextMap,
    branches: &[Branch],
    default: Option<&IdField>,
) -> anyhow::Result<IdField> {
    for branch in branches {
        if crate::service::expressions::evaluate_condition(&branch.when, context)? {
            return Ok(branch.next.clone());
        }
    }
    default
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No default branch found"))
}

async fn next(
    service: &FluxproServiceImpl,
    snapshot: &TaskExecutionSnapshot,
    changes: &mut TaskExecutionChanges,
    token: &IdField,
    successor: Option<&Next>,
) -> anyhow::Result<()> {
    if let Some(successor) = successor {
        let target = match successor {
            Next::To(target) => target.clone(),
            Next::Routes(routes) => {
                select(
                    service,
                    changes.context.as_ref().unwrap_or(&snapshot.context),
                    &routes.branches,
                    routes.default.as_ref(),
                )
                .await?
            }
        };
        queue_node(changes, token, node(snapshot, &target)?, None);
    }
    Ok(())
}

#[derive(Debug)]
struct UncertainHostOperation(&'static str);
impl std::fmt::Display for UncertainHostOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for UncertainHostOperation {}

async fn invoke(
    service: &FluxproServiceImpl,
    task: &FluxproQueueTaskDefinition,
    snapshot: &TaskExecutionSnapshot,
    registry: &FluxproHandlersContainer,
    handler_id: &IdField,
    target: &IdField,
    phase: &'static str,
    args: Option<&ContextMap>,
) -> anyhow::Result<HandleNodeResult> {
    let handler = registry
        .get_handler(handler_id)
        .ok_or_else(|| anyhow::anyhow!("handler {handler_id} is not registered"))?;
    let execution = crate::traits::node_handlers::service_node_handler::HandlerExecution {
        operation_id: format!("{}:{phase}", task.uuid),
        task_id: task.uuid,
        node_id: target.clone(),
        phase,
        instance_revision: snapshot.revision,
        attempt: task.attempts,
    };
    let future = handler.process_node_with_execution(
        task.task.process_token(),
        &snapshot.context,
        args,
        &execution,
    );
    tokio::pin!(future);
    // Catch panics at the async boundary so a bad handler does not lease/replay forever.
    let guarded = std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => poll,
            Err(_) => {
                std::task::Poll::Ready(Err(UncertainHostOperation("handler.panicked").into()))
            }
        }
    });
    match tokio::time::timeout(service.handler_timeout, guarded).await {
        Ok(result) => result,
        Err(_) => Err(UncertainHostOperation("handler.timed_out").into()),
    }
}

async fn hook(
    service: &FluxproServiceImpl,
    task: &FluxproQueueTaskDefinition,
    target: &IdField,
    phase: &'static str,
    snapshot: &TaskExecutionSnapshot,
    registry: &FluxproHandlersContainer,
    _token: &IdField,
    handler: Option<&TaskHandler>,
    args: ContextMap,
) -> anyhow::Result<()> {
    if let Some(handler) = handler {
        let id = handler.handler();
        let result = invoke(
            service,
            task,
            snapshot,
            registry,
            &id,
            target,
            phase,
            Some(&args),
        )
        .await
        .with_context(|| format!("lifecycle handler {id} failed"))?;
        anyhow::ensure!(
            matches!(result.status, HandleResultStatus::Success(_)),
            "lifecycle handler {id} returned {}",
            result.status.name()
        );
    }
    Ok(())
}

fn timeout(
    task: &FluxproQueueTaskDefinition,
    snapshot: &TaskExecutionSnapshot,
    changes: &mut TaskExecutionChanges,
    origin: &Node,
    timer: Option<&NodeTimeout>,
    wait: Option<&WaitFor>,
) -> anyhow::Result<()> {
    if let Some(timer) = timer {
        node(snapshot, &timer.on_timeout)?;
        changes.tasks.push(ScheduledTask {
            task: FluxproQueueTask::ProcessEvent {
                process_token: task.task.process_token().clone(),
                node: origin.clone(),
                on_time: timer.on_timeout.clone(),
            },
            run_after: Some(timer.after_now()?),
            visit_id: Some(task.uuid),
            cancel_signals: wait.map(WaitFor::signals).unwrap_or_default(),
        });
    }
    Ok(())
}

async fn enter(
    service: &FluxproServiceImpl,
    task: &FluxproQueueTaskDefinition,
    snapshot: &TaskExecutionSnapshot,
    changes: &mut TaskExecutionChanges,
    target: &Node,
    registry: &FluxproHandlersContainer,
) -> anyhow::Result<()> {
    let token = task.task.process_token();
    let hooks = snapshot.definition.special_handlers.as_ref();
    // A retry of the same queue item continues its existing visit. Entry hooks
    // that already committed are not repeated merely because the handler failed.
    if snapshot.visit_id != Some(task.uuid) {
        if let Some(Node::UserTask { form, .. }) = &snapshot.current_node {
            hook(
                service,
                task,
                target.id(),
                "hide_form",
                snapshot,
                registry,
                token,
                hooks.and_then(|h| h.on_hide_form.as_ref()),
                ContextMapBuilder::new()
                    .set(
                        IdField::new("form_id")?,
                        ContextValue::id_field(form.clone()),
                    )
                    .build(),
            )
            .await?;
        }
        changes.enter_node = Some(target.id().clone());
        let mut entered = event(
            task,
            "node.entered",
            format!("Entered node {}", target.id()),
        );
        entered.node_id = Some(target.id().to_string());
        entered.details = json!({ "node": target, "visit_id": task.uuid });
        changes.events.push(entered);
        if let Some(stage) = target.set_stage() {
            let stage = stage.stage_with_reason();
            anyhow::ensure!(
                snapshot
                    .definition
                    .stages
                    .iter()
                    .any(|item| item.id() == &stage.stage),
                "stage {} is not declared",
                stage.stage
            );
            hook(
                service,
                task,
                target.id(),
                "stage",
                snapshot,
                registry,
                token,
                hooks.and_then(|h| h.on_stage_change.as_ref()),
                ContextMapBuilder::new()
                    .set(IdField::new("stage")?, ContextValue::object(json!(stage)))
                    .build(),
            )
            .await?;
            changes.events.push(event(
                task,
                "stage.changed",
                format!("Process moved to stage {}", stage.stage),
            ));
            changes.stage = Some(stage);
        }
    }
    let mut handling = event(
        task,
        "node.handling_started",
        format!("Handling node {}", target.id()),
    );
    handling.node_id = Some(target.id().to_string());
    changes.events.push(handling);
    match target {
        Node::Start { next, .. } => queue_node(changes, token, node(snapshot, next)?, None),
        Node::End { .. } => {
            hook(
                service,
                task,
                target.id(),
                "complete",
                snapshot,
                registry,
                token,
                hooks.and_then(|h| h.on_process_complete.as_ref()),
                ContextMapBuilder::new()
                    .set(
                        IdField::new("end_node_id")?,
                        ContextValue::string(target.id().to_string()),
                    )
                    .build(),
            )
            .await?;
        }
        Node::Gateway {
            branches,
            next: fallback,
            ..
        } => {
            let selected = select(service, &snapshot.context, branches, fallback.as_ref()).await?;
            queue_node(changes, token, node(snapshot, &selected)?, None);
        }
        Node::Wait {
            wait_for,
            timeout: timer,
            ..
        } => timeout(
            task,
            snapshot,
            changes,
            target,
            timer.as_ref(),
            Some(wait_for),
        )?,
        Node::UserTask {
            form,
            wait_for,
            timeout: timer,
            ..
        } => {
            timeout(
                task,
                snapshot,
                changes,
                target,
                timer.as_ref(),
                wait_for.as_ref(),
            )?;
            let form = snapshot
                .definition
                .forms
                .iter()
                .find(|item| &item.id == form)
                .ok_or_else(|| anyhow::anyhow!("form {form} is not declared"))?;
            hook(
                service,
                task,
                target.id(),
                "show_form",
                snapshot,
                registry,
                token,
                hooks.and_then(|h| h.on_show_form.as_ref()),
                ContextMapBuilder::new()
                    .set(
                        IdField::new("form_id")?,
                        ContextValue::id_field(form.id.clone()),
                    )
                    .set(
                        IdField::new("roles")?,
                        ContextValue::array(
                            form.roles
                                .iter()
                                .cloned()
                                .map(ContextValue::id_field)
                                .collect(),
                        ),
                    )
                    .build(),
            )
            .await?;
        }
        Node::ServiceTask {
            handler,
            args,
            next: successor,
            retries,
            on_error,
            ..
        } => {
            let mut started = event(
                task,
                "handler.started",
                format!("Handler {handler} started"),
            );
            started.node_id = Some(target.id().to_string());
            started.handler_id = Some(handler.to_string());
            service.log_best_effort(token, started).await;
            let result = invoke(
                service,
                task,
                snapshot,
                registry,
                handler,
                target.id(),
                "service",
                args.as_ref(),
            )
            .await;
            if let Err(error) = &result {
                if error.downcast_ref::<UncertainHostOperation>().is_some() {
                    return Err(result.err().expect("checked error"));
                }
            }
            let result = match result {
                Ok(HandleNodeResult {
                    status: HandleResultStatus::Failure,
                }) => Err(anyhow::anyhow!("handler {handler} returned failure")),
                Ok(HandleNodeResult {
                    status: HandleResultStatus::IllegalState(state),
                }) => Err(anyhow::anyhow!(
                    "handler {handler} reported illegal state: {state}"
                )),
                Ok(HandleNodeResult {
                    status: HandleResultStatus::HandlerNotExists,
                }) => Err(anyhow::anyhow!("handler {handler} reported not exists")),
                result => result,
            };
            match result {
                Ok(HandleNodeResult { status }) => {
                    let mut completed = event(
                        task,
                        "handler.completed",
                        format!("Handler {handler} returned {}", status.name()),
                    );
                    completed.handler_id = Some(handler.to_string());
                    completed.node_id = Some(target.id().to_string());
                    completed.details = json!({ "result": status.name() });
                    match &status {
                        HandleResultStatus::Repeat(at) => {
                            completed.level = ExecutionLogLevel::Warning;
                            completed.event_type = "handler.repeat_requested".into();
                            completed.details["repeat_at"] = json!(at);
                        }
                        HandleResultStatus::HandlerNotExists => {
                            completed.level = ExecutionLogLevel::Warning;
                            completed.event_type = "handler.reported_not_exists".into();
                        }
                        _ => {}
                    }
                    changes.events.push(completed);
                    match status {
                        HandleResultStatus::Success(patch) => {
                            let mut context = snapshot.context.clone();
                            context.apply_patcher(&patch);
                            changes.context = Some(context);
                            changes.context_patch = Some(patch);
                            next(service, snapshot, changes, token, successor.as_ref()).await?;
                        }
                        HandleResultStatus::HandlerNotExists => {
                            next(service, snapshot, changes, token, successor.as_ref()).await?
                        }
                        HandleResultStatus::Repeat(at) => {
                            queue_node(changes, token, target.clone(), Some(at))
                        }
                        HandleResultStatus::Failure | HandleResultStatus::IllegalState(_) => {
                            unreachable!("failure outcomes normalized above")
                        }
                    }
                }
                Err(error) => {
                    let mut failed =
                        ExecutionLogEvent::error("handler.attempt_failed", "service_task", &error);
                    failed.node_id = Some(target.id().to_string());
                    failed.handler_id = Some(handler.to_string());
                    failed.queue_task_uuid = Some(task.uuid);
                    failed.attempt = Some(task.attempts);
                    if let Some(retries) = retries
                        && i64::from(task.attempts) <= i64::from(retries.max)
                    {
                        failed.level = ExecutionLogLevel::Warning;
                        changes.retry_at = Some(retries.next_attempt_at()?);
                        changes.events.push(event(
                            task,
                            "queue_task.retry_scheduled",
                            "Handler attempt scheduled for retry",
                        ));
                    } else {
                        failed.event_type = "queue_task.retries_exhausted".into();
                        if let Some(target) =
                            on_error.as_ref().and_then(|error| error.next.as_ref())
                        {
                            queue_node(changes, token, node(snapshot, target)?, None);
                        } else {
                            changes.incident =
                                Some(("handler.retries_exhausted".into(), format!("{error:#}")));
                        }
                    }
                    changes.events.push(failed);
                }
            }
        }
    }
    Ok(())
}

pub(super) async fn process(
    service: &FluxproServiceImpl,
    task: FluxproQueueTaskDefinition,
    registry: Arc<FluxproHandlersContainer>,
) -> anyhow::Result<TaskProcessOutcome> {
    let snapshot = service.db_service.load_task_execution(&task).await?;
    let token = task.task.process_token();
    let mut changes = TaskExecutionChanges::default();
    let prepared: anyhow::Result<()> = async {
    match &task.task {
        FluxproQueueTask::ProcessNode { node, .. } => {
            let declared = self::node(&snapshot, node.id())?;
            anyhow::ensure!(serde_json::to_value(node)? == serde_json::to_value(&declared)?, "queued node differs from its published definition");
            enter(service, &task, &snapshot, &mut changes, &declared, &registry).await?;
        }
        FluxproQueueTask::ProcessEvent {
            node: origin,
            on_time,
            ..
        } => {
            anyhow::ensure!(
                snapshot.task_visit_id.is_some(),
                "timeout task has no originating visit"
            );
            if snapshot.task_visit_id == snapshot.visit_id
                && !snapshot.wait_completed
                && snapshot
                    .current_node
                    .as_ref()
                    .is_some_and(|node| node.id() == origin.id())
            {
                changes.close_wait = true;
                queue_node(&mut changes, token, node(&snapshot, on_time)?, None);
                changes.events.push(event(
                    &task,
                    "timeout.accepted",
                    "Timeout completed the current wait",
                ));
            } else {
                changes.events.push(event(
                    &task,
                    "timeout.stale",
                    "Timeout belongs to a completed or earlier visit",
                ));
            }
        }
        FluxproQueueTask::ProcessSignal { signal, .. } => {
            let waiting = snapshot
                .current_node
                .as_ref()
                .and_then(|node| waiting_transition(node, &signal.signal));
            if let Some((current, successor)) = waiting
                && !snapshot.wait_completed
                && snapshot.task_visit_id.is_some()
                && snapshot.task_visit_id == snapshot.visit_id
            {
                changes.close_wait = true;
                let mut context = snapshot.context.clone();
                context.0.extend(signal.context.0.clone());
                context.0.insert(
                    IdField::new("_last_signal")?,
                    ContextValue::string(signal.signal.to_string()),
                );
                changes.context = Some(context);
                next(service, &snapshot, &mut changes, token, successor.as_ref()).await?;
                let mut accepted = event(
                    &task,
                    "signal.accepted",
                    format!("Signal {} was accepted", signal.signal),
                );
                accepted.node_id = Some(current.id().to_string());
                accepted.details =
                    json!({ "signal": signal.signal, "visit_id": snapshot.visit_id });
                changes.events.push(accepted);
            } else if snapshot.task_visit_id.is_some()
                || !can_wait_for_future_signal(snapshot.current_node.as_ref())
            {
                changes.events.push(event(
                    &task,
                    "signal.not_accepted",
                    "Signal belongs to a completed wait or terminal instance",
                ));
            } else if let Some(retry) = service
                .signal_retry_policy
                .decision(task.created_at, Utc::now())
            {
                changes.retry_at = Some(retry.retry_at);
                if should_log_signal_deferral(task.attempts) {
                    let mut deferred = event(
                        &task,
                        "signal.deferred",
                        format!("Signal {} is waiting for a compatible node", signal.signal),
                    );
                    deferred.level = ExecutionLogLevel::Warning;
                    deferred.details = json!({ "signal": signal.signal, "retry_at": retry.retry_at, "retry_phase": retry.phase, "expires_at": retry.expires_at });
                    changes.events.push(deferred);
                }
            } else {
                let mut expired = event(
                    &task,
                    "signal.expired",
                    format!("Signal {} exceeded its delivery deadline", signal.signal),
                );
                expired.level = ExecutionLogLevel::Critical;
                changes.events.push(expired);
            }
        }
    }
    Ok(())
    }.await;
    if let Err(error) = prepared {
        // Discard every proposed effect: a failed signal route must not consume
        // its wait or merge its payload, and a failed node must not advance.
        changes = TaskExecutionChanges::default();
        if let Some(uncertain) = error.downcast_ref::<UncertainHostOperation>() {
            changes.incident = Some((uncertain.0.into(), format!("{error:#}")));
        } else if error
            .downcast_ref::<crate::service::expressions::ConditionError>()
            .is_some()
        {
            changes.incident = Some(("condition.failed".into(), format!("{error:#}")));
        } else if task.attempts >= 3 {
            changes.incident = Some(("execution.retries_exhausted".into(), format!("{error:#}")));
        } else {
            changes.retry_at = Some(Utc::now() + chrono::Duration::seconds(5));
            changes.events.push(ExecutionLogEvent::error(
                "execution.attempt_failed",
                "process_service",
                &error,
            ));
        }
    }
    if let Some((kind, reason)) = &changes.incident {
        let mut stopped = event(&task, "instance.suspended", reason);
        stopped.level = ExecutionLogLevel::Critical;
        stopped.error_kind = Some(kind.clone());
        stopped.error_message = Some(reason.clone());
        changes.events.push(stopped);
    }
    for pending in &changes.tasks {
        if let FluxproQueueTask::ProcessNode { node, .. } = &pending.task {
            let mut queued = event(
                &task,
                "node.queued",
                format!("Node {} queued for execution", node.id()),
            );
            queued.node_id = Some(node.id().to_string());
            queued.details = json!({ "run_after": pending.run_after });
            changes.events.push(queued);
        }
    }
    let outcome = if changes.incident.is_some() {
        TaskProcessOutcome::Suspended
    } else if changes.retry_at.is_some() {
        TaskProcessOutcome::RetryScheduled
    } else {
        TaskProcessOutcome::Completed
    };
    service
        .db_service
        .commit_task_execution(&task, &snapshot, changes)
        .await?;
    service.queue_wakeup.notify_one();
    Ok(outcome)
}
