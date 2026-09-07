use super::{
    AdminResult, FluxproAdminService, INSTANCE_SELECT, Page, PageRequest, normalize_filter,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    Healthy,
    Warning,
    Critical,
}

impl AuditSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessAuditFilter {
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessAuditIssue {
    pub kind: String,
    pub title: String,
    pub details: String,
    pub occurred_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessAuditSummary {
    pub process_instance_uuid: Uuid,
    pub process_key: String,
    pub process_version: String,
    pub process_id: String,
    pub token: String,
    pub state: String,
    pub current_node_id: Option<String>,
    pub current_node_type: Option<String>,
    pub current_stage_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub current_node_entered_at: Option<DateTime<Utc>>,
    pub last_activity_at: DateTime<Utc>,
    pub warning_count: i64,
    pub error_count: i64,
    pub incident_count: i64,
    pub severity: AuditSeverity,
    pub issues: Vec<ProcessAuditIssue>,
}

#[derive(Debug, FromRow)]
struct ProcessAuditRow {
    process_instance_uuid: Uuid,
    process_key: String,
    process_version: String,
    process_id: String,
    token: String,
    state: String,
    current_node_id: Option<String>,
    current_node_type: Option<String>,
    current_node_definition: Option<Value>,
    current_stage_name: Option<String>,
    created_at: DateTime<Utc>,
    current_node_entered_at: Option<DateTime<Utc>>,
    last_activity_at: DateTime<Utc>,
    warning_count: i64,
    error_count: i64,
    incident_count: i64,
    latest_incident_type: Option<String>,
    latest_incident_message: Option<String>,
    latest_incident_at: Option<DateTime<Utc>>,
}

impl FluxproAdminService {
    pub async fn list_process_audits(
        &self,
        filter: ProcessAuditFilter,
        page: PageRequest,
    ) -> AdminResult<Page<ProcessAuditSummary>> {
        let page = page.normalized();
        let search = normalize_filter(filter.search);
        let total = sqlx::query_scalar::<_, i64>(&format!(
            "select count(*) from ({INSTANCE_SELECT}) instance_admin \
             where $1::text is null or process_id ilike '%' || $1 || '%' \
                or token ilike '%' || $1 || '%' or process_key ilike '%' || $1 || '%'"
        ))
        .bind(&search)
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query_as::<_, ProcessAuditRow>(&format!(
            r#"
            with instance_admin as ({INSTANCE_SELECT}),
            log_stats as (
                select process_instance_uuid,
                       count(*) filter (where level = 'warning')::bigint as warning_count,
                       count(*) filter (where level in ('error', 'critical'))::bigint as error_count,
                       count(*) filter (where level in ('warning', 'error', 'critical'))::bigint as incident_count,
                       max(created_at) as last_log_at
                from fluxpro.process_instance_log
                group by process_instance_uuid
            ),
            last_node as (
                select distinct on (process_instance_uuid)
                       process_instance_uuid, created_at as entered_at
                from fluxpro.process_instance_log
                where event_type = 'node.entered'
                order by process_instance_uuid, created_at desc, uuid desc
            ),
            latest_incident as (
                select distinct on (process_instance_uuid)
                       process_instance_uuid, event_type, message, created_at
                from fluxpro.process_instance_log
                where level in ('warning', 'error', 'critical')
                order by process_instance_uuid, created_at desc, uuid desc
            ),
            signal_stats as (
                select i.uuid as process_instance_uuid, max(h.created_at) as last_signal_at
                from fluxpro.process_instance i
                join fluxpro.signal_history h on h.process_id = i.token
                group by i.uuid
            ),
            stage_stats as (
                select process_instance_uuid, max(created_at) as last_stage_at
                from fluxpro.process_instance_stage_log
                group by process_instance_uuid
            )
            select a.uuid as process_instance_uuid,
                   a.process_key, a.process_version, a.process_id, a.token, a.state,
                   a.current_node_id, a.current_node_type,
                   n.definition as current_node_definition,
                   a.current_stage_name, a.created_at,
                   ln.entered_at as current_node_entered_at,
                   greatest(
                       a.created_at,
                       coalesce(ls.last_log_at, a.created_at),
                       coalesce(ss.last_signal_at, a.created_at),
                       coalesce(st.last_stage_at, a.created_at)
                   ) as last_activity_at,
                   coalesce(ls.warning_count, 0)::bigint as warning_count,
                   coalesce(ls.error_count, 0)::bigint as error_count,
                   coalesce(ls.incident_count, 0)::bigint as incident_count,
                   li.event_type as latest_incident_type,
                   li.message as latest_incident_message,
                   li.created_at as latest_incident_at
            from instance_admin a
            left join fluxpro.process_def_node n on n.uuid = a.current_node_uuid
            left join log_stats ls on ls.process_instance_uuid = a.uuid
            left join last_node ln on ln.process_instance_uuid = a.uuid
            left join latest_incident li on li.process_instance_uuid = a.uuid
            left join signal_stats ss on ss.process_instance_uuid = a.uuid
            left join stage_stats st on st.process_instance_uuid = a.uuid
            where $1::text is null or a.process_id ilike '%' || $1 || '%'
               or a.token ilike '%' || $1 || '%' or a.process_key ilike '%' || $1 || '%'
            order by
                case when coalesce(ls.error_count, 0) > 0 then 0
                     when coalesce(ls.warning_count, 0) > 0 then 1 else 2 end,
                greatest(a.created_at, coalesce(ls.last_log_at, a.created_at)) desc,
                a.uuid
            offset $2 limit $3
            "#
        ))
        .bind(&search)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;

        let items = rows.into_iter().map(classify_audit_row).collect();
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }
}

fn classify_audit_row(row: ProcessAuditRow) -> ProcessAuditSummary {
    let mut severity = if row.error_count > 0 {
        AuditSeverity::Critical
    } else if row.warning_count > 0 {
        AuditSeverity::Warning
    } else {
        AuditSeverity::Healthy
    };
    let mut issues = Vec::new();
    if row.incident_count > 0 {
        issues.push(ProcessAuditIssue {
            kind: row
                .latest_incident_type
                .clone()
                .unwrap_or_else(|| "execution.warning".into()),
            title: if row.error_count > 0 {
                "Execution error".into()
            } else {
                "Non-standard execution".into()
            },
            details: row.latest_incident_message.clone().unwrap_or_else(|| {
                format!("{} warning/error event(s) recorded", row.incident_count)
            }),
            occurred_at: row.latest_incident_at,
        });
    }
    let stuck_issue = detect_stuck_issue(&row);
    let has_stuck_issue = stuck_issue.is_some();
    if let Some(stuck) = stuck_issue {
        severity = AuditSeverity::Critical;
        issues.push(stuck);
    }

    ProcessAuditSummary {
        process_instance_uuid: row.process_instance_uuid,
        process_key: row.process_key,
        process_version: row.process_version,
        process_id: row.process_id,
        token: row.token,
        state: row.state,
        current_node_id: row.current_node_id,
        current_node_type: row.current_node_type,
        current_stage_name: row.current_stage_name,
        created_at: row.created_at,
        current_node_entered_at: row.current_node_entered_at,
        last_activity_at: row.last_activity_at,
        warning_count: row.warning_count,
        error_count: row.error_count,
        incident_count: row.incident_count + i64::from(has_stuck_issue),
        severity,
        issues,
    }
}

fn detect_stuck_issue(row: &ProcessAuditRow) -> Option<ProcessAuditIssue> {
    if row.state == "completed" {
        return None;
    }
    let entered_at = row.current_node_entered_at.unwrap_or(row.created_at);
    let age = Utc::now()
        .signed_duration_since(entered_at)
        .max(Duration::zero());
    let node_type = row.current_node_type.as_deref().unwrap_or_default();
    let allowed = match node_type.to_ascii_lowercase().as_str() {
        "" | "start" => Duration::minutes(5),
        "servicetask" | "gateway" => Duration::minutes(15),
        "usertask" | "wait" => current_node_timeout(row.current_node_definition.as_ref())
            .map(|timeout| timeout + Duration::minutes(5))
            .unwrap_or_else(|| Duration::hours(26)),
        "end" => return None,
        _ => Duration::hours(26),
    };
    (age > allowed).then(|| ProcessAuditIssue {
        kind: "process.stuck".into(),
        title: "Process may be stuck".into(),
        details: format!(
            "Current node has not changed for {} minutes; expected maximum is {} minutes",
            age.num_minutes(),
            allowed.num_minutes()
        ),
        occurred_at: Some(entered_at + allowed),
    })
}

fn current_node_timeout(definition: Option<&Value>) -> Option<Duration> {
    let after = definition?.get("timeout")?.get("after")?.as_str()?;
    let parsed = after.parse::<iso8601::Duration>().ok()?;
    Duration::from_std(std::time::Duration::from(parsed)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso_timeout_from_node_definition() {
        let node = serde_json::json!({"timeout": {"after": "PT24H"}});
        assert_eq!(current_node_timeout(Some(&node)), Some(Duration::hours(24)));
    }
}
