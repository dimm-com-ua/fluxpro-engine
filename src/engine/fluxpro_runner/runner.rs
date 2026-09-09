//! Queue consumption with per-task heartbeats and bounded worker concurrency.

use crate::engine::fluxpro_handlers::FluxproHandlersContainer;
use crate::engine::fluxpro_runner::config::RunnerConfig;
use crate::engine::fluxpro_runner::shutdown::Shutdown;
use crate::models::execution_log::ExecutionLogEvent;
use crate::models::id_field::IdField;
use crate::models::queue::queue_task::{FluxproQueueTaskDefinition, TaskProcessOutcome};
use crate::service::process_service::FluxproService;
use chrono::Utc;
use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant as StdInstant;
use tokio::select;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

/// Concurrent queue consumer that renews leases and drains workers on shutdown.
#[derive(Clone)]
pub struct EngineRunner {
    service: Arc<dyn FluxproService + Send + Sync>,
    queue_wakeup: Arc<Notify>,
    cfg: RunnerConfig,
}

impl EngineRunner {
    /// Builds a runner using the supplied service, notifier, and worker settings.
    pub fn new(
        service: Arc<dyn FluxproService + Send + Sync>,
        queue_wakeup: Arc<Notify>,
        cfg: RunnerConfig,
    ) -> Self {
        Self {
            service,
            queue_wakeup,
            cfg,
        }
    }

    /// Processes queued work until shutdown, then waits for active workers.
    ///
    /// # Errors
    ///
    /// Rejects zero concurrency and invalid heartbeat/lease timing. Individual
    /// worker failures are logged; their remaining leases can later expire.
    pub async fn run_until_stopped(&self, shutdown: Shutdown) -> anyhow::Result<()> {
        anyhow::ensure!(self.cfg.concurrency > 0, "runner concurrency must be > 0");
        anyhow::ensure!(
            self.cfg.heartbeat_interval_ms > 0
                && self.cfg.heartbeat_interval_ms < self.cfg.task_lease_ms,
            "heartbeat interval must be > 0 and shorter than the task lease"
        );

        let mut rx = shutdown.subscribe();
        let mut workers = JoinSet::new();
        let health_monitor = tokio::spawn(monitor_queue_health(
            self.service.clone(),
            shutdown.subscribe(),
        ));

        loop {
            while let Some(res) = workers.try_join_next() {
                log_worker_result(res);
            }

            if workers.len() >= self.cfg.concurrency {
                select! {
                    _ = rx.changed() => { if *rx.borrow() { break; } },
                    completed = workers.join_next() => {
                        if let Some(res) = completed { log_worker_result(res); }
                    }
                }
                continue;
            }

            let lock_key = IdField::generate();
            // A monotonic deadline measured before dequeue is conservative and
            // does not depend on clock agreement between the host and PostgreSQL.
            let initial_deadline = Instant::now() + Duration::from_millis(self.cfg.task_lease_ms);

            select! {
                _ = rx.changed() => { if *rx.borrow() { break; } }
                dequeued = self.service.fetch_queue_task(&lock_key, self.cfg.task_lease_ms) => {
                    match dequeued {
                        Ok(Some(task)) => {
                            let svc = self.service.clone();
                            let container = self.cfg.container.clone();
                            let heartbeat_interval_ms = self.cfg.heartbeat_interval_ms;
                            let task_lease_ms = self.cfg.task_lease_ms;
                            workers.spawn(run_one(
                                svc,
                                task,
                                container,
                                heartbeat_interval_ms,
                                task_lease_ms,
                                initial_deadline,
                            ));
                        }
                        Ok(None) => {
                            debug!("no tasks to process");
                            if wait_for_queue_activity(
                                &mut rx,
                                &mut workers,
                                &self.queue_wakeup,
                                Duration::from_millis(self.cfg.idle_backoff_ms),
                            )
                            .await
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("dequeue_task error: {e}. backing off");
                            sleep(Duration::from_millis(self.cfg.idle_backoff_ms)).await;
                        }
                    }
                }
            }
        }

        info!("shutdown requested; waiting workers = {}", workers.len());
        while let Some(res) = workers.join_next().await {
            log_worker_result(res);
        }
        let _ = health_monitor.await;
        info!("the runner stopped gracefully");
        Ok(())
    }
}

async fn monitor_queue_health(
    service: Arc<dyn FluxproService + Send + Sync>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {
                if *shutdown.borrow() {
                    break;
                }
                match service.get_queue_health().await {
                    Ok(snapshot) => {
                        let oldest_age = snapshot.oldest_ready_at
                            .map(|created_at| Utc::now().signed_duration_since(created_at).num_seconds().max(0))
                            .unwrap_or(0);
                        if oldest_age >= 60 {
                            warn!(
                                "fluxpro.queue.health degraded ready_tasks={} locked_tasks={} oldest_ready_age_seconds={}",
                                snapshot.ready_tasks,
                                snapshot.locked_tasks,
                                oldest_age
                            );
                        } else {
                            info!(
                                "fluxpro.queue.health ready_tasks={} locked_tasks={} oldest_ready_age_seconds={}",
                                snapshot.ready_tasks,
                                snapshot.locked_tasks,
                                oldest_age
                            );
                        }
                    }
                    Err(error) => warn!("fluxpro.queue.health failed: {error:#}"),
                }
            }
        }
    }
}

/// Waits until the queue may have become fetchable again.
///
/// A worker completion matters here because a successor task can already be queued while the
/// current task still owns the per-process lock. In that case the enqueue notification wakes the
/// runner too early, and completing the current worker is what makes the successor eligible.
/// Returns `true` when shutdown was requested.
async fn wait_for_queue_activity(
    shutdown: &mut watch::Receiver<bool>,
    workers: &mut JoinSet<anyhow::Result<()>>,
    queue_wakeup: &Notify,
    idle_backoff: Duration,
) -> bool {
    let has_active_workers = !workers.is_empty();

    select! {
        _ = shutdown.changed() => *shutdown.borrow(),
        completed = workers.join_next(), if has_active_workers => {
            if let Some(res) = completed {
                log_worker_result(res);
            }
            false
        },
        _ = queue_wakeup.notified() => {
            debug!("queue runner woken by a newly committed task");
            false
        },
        _ = sleep(idle_backoff) => false,
    }
}

async fn run_one(
    svc: Arc<dyn FluxproService + Send + Sync>,
    task: FluxproQueueTaskDefinition,
    container: Arc<FluxproHandlersContainer>,
    heartbeat_interval_ms: u64,
    task_lease_ms: u64,
    initial_deadline: Instant,
) -> anyhow::Result<()> {
    let task_id = task.uuid;
    let lock_key = task.lock_key.clone();
    let task_kind = task.kind();
    let process_token = task.task.process_token().clone();
    let attempt = task.attempts;
    let started_at = StdInstant::now();
    let suppress_repeated_signal_lifecycle = matches!(
        &task.task,
        crate::models::queue::queue_task::FluxproQueueTask::ProcessSignal { .. }
    ) && attempt > 1;
    info!("start task {} ({})", task.uuid, task.kind());

    if !suppress_repeated_signal_lifecycle {
        let mut started = ExecutionLogEvent::info(
            "queue_task.started",
            "queue_runner",
            format!("Queue task {task_kind} started"),
        );
        started.queue_task_uuid = Some(task_id);
        started.attempt = Some(attempt);
        started.details =
            serde_json::json!({ "task_kind": task_kind.clone(), "lock_key": lock_key.clone() });
        if let Err(error) = svc.record_execution_log(&process_token, started).await {
            warn!("could not persist queue task start event: {error:#}");
        }
    }

    let process = svc.process_task(task, container);
    tokio::pin!(process);
    // Poll lease maintenance alongside execution. Awaiting a renewal inside a
    // select branch would stop polling the commit that currently owns its row lock.
    let heartbeat = maintain_lease(
        svc.clone(),
        task_id,
        lock_key,
        heartbeat_interval_ms,
        task_lease_ms,
        initial_deadline,
    );
    tokio::pin!(heartbeat);

    loop {
        select! {
            res = &mut process => {
                let duration_ms = started_at.elapsed().as_millis();
                if let Err(ref e) = res {
                    error!("task {task_id} ({task_kind}) failed: {e}");
                }
                let event = match &res {
                    Ok(TaskProcessOutcome::Completed) => {
                        let mut event = ExecutionLogEvent::info(
                            "queue_task.completed",
                            "queue_runner",
                            format!("Queue task {task_kind} completed"),
                        );
                        event.queue_task_uuid = Some(task_id);
                        event.attempt = Some(attempt);
                        event
                    }
                    Ok(TaskProcessOutcome::RetryScheduled) => {
                        let mut event = ExecutionLogEvent::info(
                            "queue_task.released_for_retry",
                            "queue_runner",
                            format!("Queue task {task_kind} released for retry"),
                        );
                        event.queue_task_uuid = Some(task_id);
                        event.attempt = Some(attempt);
                        event
                    }
                    Ok(TaskProcessOutcome::Suspended) => {
                        let mut event = ExecutionLogEvent::info("queue_task.suspended", "queue_runner", "Task retained for explicit recovery");
                        event.queue_task_uuid = Some(task_id);
                        event
                    }
                    Err(error) => {
                        let mut event = ExecutionLogEvent::error(
                            "queue_task.failed",
                            "queue_runner",
                            error,
                        );
                        event.message = format!("Queue task {task_kind} failed");
                        event.queue_task_uuid = Some(task_id);
                        event.attempt = Some(attempt);
                        event.details["task_kind"] = serde_json::json!(task_kind.clone());
                        event
                    }
                };
                if (!suppress_repeated_signal_lifecycle || res.is_err())
                    && let Err(error) = svc.record_execution_log(&process_token, event).await
                {
                    warn!("could not persist queue task completion event: {error:#}");
                }
                info!(
                    "fluxpro.queue_task.finished task_id={} task_kind={} attempt={} duration_ms={} outcome={}",
                    task_id,
                    task_kind,
                    attempt,
                    duration_ms,
                    match &res {
                        Ok(TaskProcessOutcome::Completed) => "completed",
                        Ok(TaskProcessOutcome::RetryScheduled) => "retry_scheduled",
                        Ok(TaskProcessOutcome::Suspended) => "suspended",
                        Err(_) => "failed",
                    }
                );
                return res.map(|_| ());
            }
            result = &mut heartbeat => return result,
        }
    }
}

async fn maintain_lease(
    svc: Arc<dyn FluxproService + Send + Sync>,
    task_id: uuid::Uuid,
    lock_key: String,
    heartbeat_interval_ms: u64,
    task_lease_ms: u64,
    initial_deadline: Instant,
) -> anyhow::Result<()> {
    let heartbeat_period = Duration::from_millis(heartbeat_interval_ms);
    let lease_period = Duration::from_millis(task_lease_ms);
    let mut lease_deadline = initial_deadline;
    let mut heartbeat = interval_at(Instant::now() + heartbeat_period, heartbeat_period);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::time::timeout_at(lease_deadline, heartbeat.tick())
            .await
            .map_err(|_| anyhow::anyhow!("task {task_id} lease expired"))?;
        let renewal_started = Instant::now();
        let renewal = tokio::time::timeout_at(
            lease_deadline,
            svc.renew_queue_task_lock(task_id, &lock_key, task_lease_ms),
        )
        .await
        .map_err(|_| anyhow::anyhow!("task {task_id} lease renewal exceeded deadline"))?;
        match renewal {
            Ok(true) => lease_deadline = renewal_started + lease_period,
            Ok(false) => anyhow::bail!("lost ownership of task {task_id}"),
            Err(error) => {
                if Instant::now() >= lease_deadline {
                    anyhow::bail!(
                        "could not renew task {task_id} before its lease expired: {error:#}"
                    );
                }
                warn!("failed to renew lease for task {task_id}: {error:#}");
            }
        }
    }
}

fn log_worker_result(res: Result<anyhow::Result<()>, tokio::task::JoinError>) {
    match res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => error!("task worker failed: {e:#}"),
        Err(e) => error!("task worker join error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    #[tokio::test]
    async fn completed_worker_interrupts_idle_backoff() {
        let shutdown = Shutdown::new();
        let mut shutdown_rx = shutdown.subscribe();
        let queue_wakeup = Notify::new();
        let mut workers = JoinSet::new();
        workers.spawn(async { Ok(()) });

        let shutdown_requested = timeout(
            Duration::from_secs(1),
            wait_for_queue_activity(
                &mut shutdown_rx,
                &mut workers,
                &queue_wakeup,
                Duration::from_secs(60),
            ),
        )
        .await
        .expect("worker completion must wake the runner before idle backoff elapses");

        assert!(!shutdown_requested);
        assert!(workers.is_empty());
    }
}
