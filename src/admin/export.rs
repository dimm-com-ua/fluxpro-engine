//! Markdown diagnostic reports containing definition, context, and execution history.

use super::{
    AdminResult, ExecutionLogEntry, FluxproAdminService, QUEUE_SELECT, QueueTaskSummary,
    SignalHistoryEntry,
};
use crate::models::queue::queue_task::FluxproQueueTask;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

struct TimelineEntry {
    at: DateTime<Utc>,
    order: u8,
    title: String,
    kind: &'static str,
    level: Option<String>,
    metadata: Vec<(String, String)>,
    payload: Value,
}

impl FluxproAdminService {
    /// Builds a portable, chronological diagnostic report without exposing the
    /// engine's persistence schema to the host application or console.
    pub async fn export_process_instance_markdown(
        &self,
        process_instance_uuid: Uuid,
    ) -> AdminResult<String> {
        let details = self.get_process_instance(process_instance_uuid).await?;
        let definition = self
            .get_process_definition(details.summary.process_definition_uuid)
            .await?;
        let logs = sqlx::query_as::<_, ExecutionLogEntry>(
            r#"select uuid, created_at, level, event_type, source, message, node_id,
                      handler_id, queue_task_uuid, attempt, error_kind, error_message, details
               from fluxpro.process_instance_log
               where process_instance_uuid = $1
               order by created_at, uuid"#,
        )
        .bind(process_instance_uuid)
        .fetch_all(&self.pool)
        .await?;
        let signals = sqlx::query_as::<_, SignalHistoryEntry>(
            r#"select h.uuid, h.created_at, h.process_id, h.signal_name, h.payload
               from fluxpro.signal_history h
               join fluxpro.process_instance i on i.token = h.process_id
               where i.uuid = $1
               order by h.created_at, h.uuid"#,
        )
        .bind(process_instance_uuid)
        .fetch_all(&self.pool)
        .await?;
        let queue_tasks = sqlx::query_as::<_, QueueTaskSummary>(QUEUE_SELECT)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .filter(|task| {
                serde_json::from_value::<FluxproQueueTask>(task.task.clone())
                    .is_ok_and(|task| task.process_token().get_id() == details.summary.token)
            })
            .collect::<Vec<_>>();

        let mut timeline = Vec::new();
        timeline.push(TimelineEntry {
            at: details.summary.created_at,
            order: 0,
            title: "Process instance created".into(),
            kind: "instance",
            level: Some("info".into()),
            metadata: vec![("Instance UUID".into(), process_instance_uuid.to_string())],
            payload: json!({}),
        });
        timeline.extend(details.stage_history.iter().map(|stage| TimelineEntry {
            at: stage.created_at,
            order: 1,
            title: format!(
                "Stage changed to {}",
                stage
                    .stage_name
                    .as_deref()
                    .or(stage.stage_id.as_deref())
                    .unwrap_or("unknown")
            ),
            kind: "stage",
            level: Some("info".into()),
            metadata: vec![
                (
                    "Stage ID".into(),
                    stage.stage_id.clone().unwrap_or_else(|| "—".into()),
                ),
                (
                    "Reason".into(),
                    stage.reason.clone().unwrap_or_else(|| "—".into()),
                ),
            ],
            payload: stage.context.clone(),
        }));
        timeline.extend(signals.iter().map(|signal| TimelineEntry {
            at: signal.created_at,
            order: 2,
            title: format!("Signal {} received", signal.signal_name),
            kind: "signal",
            level: Some("info".into()),
            metadata: vec![
                ("Signal UUID".into(), signal.uuid.to_string()),
                ("Process token".into(), signal.process_id.clone()),
            ],
            payload: signal.payload.clone(),
        }));
        timeline.extend(logs.iter().map(|log| TimelineEntry {
            at: log.created_at,
            order: 3,
            title: format!("{} — {}", log.event_type, log.message),
            kind: "execution",
            level: Some(log.level.clone()),
            metadata: vec![
                ("Source".into(), log.source.clone()),
                (
                    "Node".into(),
                    log.node_id.clone().unwrap_or_else(|| "—".into()),
                ),
                (
                    "Handler".into(),
                    log.handler_id.clone().unwrap_or_else(|| "—".into()),
                ),
                (
                    "Queue task".into(),
                    log.queue_task_uuid
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    "Attempt".into(),
                    log.attempt
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    "Error".into(),
                    log.error_message.clone().unwrap_or_else(|| "—".into()),
                ),
            ],
            payload: log.details.clone(),
        }));
        // Keep instance, stage, signal, and execution entries stable at equal timestamps.
        timeline.sort_by_key(|entry| (entry.at, entry.order));

        let warning_count = logs.iter().filter(|log| log.level == "warning").count();
        let error_count = logs
            .iter()
            .filter(|log| matches!(log.level.as_str(), "error" | "critical"))
            .count();
        let audit_status = if error_count > 0 {
            "CRITICAL"
        } else if warning_count > 0 {
            "WARNING"
        } else {
            "HEALTHY"
        };

        let mut output = String::new();
        output.push_str("# FluxPro process audit\n\n");
        output.push_str(&format!(
            "> Generated at {}. The report can contain process context and personal data; handle it according to your access policy.\n\n",
            timestamp(Utc::now())
        ));
        output.push_str("## Summary\n\n");
        markdown_table(
            &mut output,
            &[
                ("Audit status", audit_status.to_string()),
                ("Process", definition.key),
                ("Version", definition.version),
                ("Process ID", details.summary.process_id.clone()),
                ("Token", details.summary.token.clone()),
                ("Instance UUID", process_instance_uuid.to_string()),
                ("State", details.summary.state.clone()),
                (
                    "Current node",
                    details
                        .summary
                        .current_node_id
                        .clone()
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    "Current stage",
                    details
                        .summary
                        .current_stage_name
                        .clone()
                        .unwrap_or_else(|| "—".into()),
                ),
                ("Warnings", warning_count.to_string()),
                ("Errors/critical", error_count.to_string()),
                ("Pending queue tasks", queue_tasks.len().to_string()),
            ],
        );

        output.push_str("\n## Current node definition\n\n");
        json_block(
            &mut output,
            details.current_node.as_ref().unwrap_or(&Value::Null),
        );
        output.push_str("\n## Current context\n\n");
        if details.context_variables.is_empty() {
            output.push_str("_Context is empty._\n");
        } else {
            output.push_str("| Scope | Name | Value |\n|---|---|---|\n");
            for variable in &details.context_variables {
                output.push_str(&format!(
                    "| {} | `{}` | `{}` |\n",
                    table_cell(&variable.scope),
                    table_cell(variable.name.get_id()),
                    table_cell(
                        &serde_json::to_string(&variable.value).unwrap_or_else(|_| "null".into())
                    )
                ));
            }
        }

        output.push_str("\n## Pending queue tasks\n\n");
        if queue_tasks.is_empty() {
            output.push_str("_No pending tasks._\n");
        } else {
            for task in &queue_tasks {
                output.push_str(&format!(
                    "### {} — attempt {}\n\n- Run after: {}\n- State: {}\n\n",
                    task.uuid,
                    task.attempts,
                    timestamp(task.run_after),
                    task.state
                ));
                json_block(&mut output, &task.task);
                output.push('\n');
            }
        }

        output.push_str("\n## Complete chronological timeline\n\n");
        for entry in timeline {
            output.push_str(&format!(
                "### {} — {}\n\n",
                timestamp(entry.at),
                heading(&entry.title)
            ));
            output.push_str(&format!("- Kind: `{}`\n", entry.kind));
            if let Some(level) = entry.level {
                output.push_str(&format!("- Level: `{}`\n", table_cell(&level)));
            }
            for (key, value) in entry.metadata {
                output.push_str(&format!("- {}: {}\n", key, inline_value(&value)));
            }
            if entry.payload != json!({}) && entry.payload != Value::Null {
                output.push_str("\n");
                json_block(&mut output, &entry.payload);
            }
            output.push('\n');
        }
        Ok(output)
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn heading(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace('#', "\\#")
}

fn table_cell(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .replace('`', "\\`")
}

fn inline_value(value: &str) -> String {
    if value == "—" {
        value.into()
    } else {
        format!("`{}`", table_cell(value))
    }
}

fn markdown_table(output: &mut String, values: &[(&str, String)]) {
    output.push_str("| Field | Value |\n|---|---|\n");
    for (field, value) in values {
        output.push_str(&format!(
            "| {} | {} |\n",
            table_cell(field),
            inline_value(value)
        ));
    }
}

fn json_block(output: &mut String, value: &Value) {
    output.push_str("````json\n");
    output.push_str(&serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into()));
    output.push_str("\n````\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_helpers_escape_table_and_heading_content() {
        assert_eq!(table_cell("a|b\nc"), "a\\|b c");
        assert_eq!(heading("one\n#two"), "one \\#two");
    }
}
