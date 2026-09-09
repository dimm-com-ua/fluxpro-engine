//! Auth-agnostic server adapter. Hosts must authorize the scope before calling.
use crate::*;
use fluxpro_engine::admin::{
    ExecutionLogFilter, FluxproAdminService, PageRequest, ProcessDefinitionDetails,
};
use fluxpro_engine::models::id_field::IdField;
use serde::{Serialize, de::DeserializeOwned};

fn project<T: Serialize, U: DeserializeOwned>(value: T) -> anyhow::Result<U> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

/// Uses compiled runtime data as authority and imports saved layout only when valid.
pub fn monitor_document(source: ProcessDefinitionDetails) -> anyhow::Result<EditorDocument> {
    let mut document = EditorDocument::new(serde_json::from_value(source.definition)?)
        .map_err(anyhow::Error::msg)?;
    document.definition.uuid = Some(source.uuid);
    if let Some(layout) = source
        .source_definition
        .as_deref()
        .and_then(|yaml| EditorDocument::from_yaml(yaml).ok())
    {
        for (id, position) in layout.positions {
            if let Some(target) = document.positions.get_mut(&id) {
                *target = position;
            }
        }
    }
    Ok(document)
}

/// Loads an exact version, filtered page, independent counters and open histories.
/// Errors are echoed as snapshots so the component can retain its last good data.
/// Escalations are host-owned: enrich issues/counts and mark their availability
/// when a host provides an authoritative support-ticket source.
pub async fn load_monitor_snapshot(
    service: &FluxproAdminService,
    request: MonitorRequest,
) -> MonitorSnapshot {
    match load(service, &request, true).await {
        Ok(snapshot) => snapshot,
        Err(error) => MonitorSnapshot {
            request,
            error: Some(error.to_string()),
            ..Default::default()
        },
    }
}

/// Loads definition counts without querying instance identities, context or history.
pub async fn load_definition_snapshot(
    service: &FluxproAdminService,
    request: MonitorRequest,
) -> MonitorSnapshot {
    match load(service, &request, false).await {
        Ok(snapshot) => snapshot,
        Err(error) => MonitorSnapshot {
            request,
            error: Some(error.to_string()),
            ..Default::default()
        },
    }
}

async fn load(
    service: &FluxproAdminService,
    request: &MonitorRequest,
    include_instances: bool,
) -> anyhow::Result<MonitorSnapshot> {
    anyhow::ensure!(
        request.query.limit > 0 && request.query.limit <= 250,
        "Page size must be 1–250"
    );
    anyhow::ensure!(
        request.query.offset <= i64::MAX as u64,
        "Invalid page offset"
    );
    anyhow::ensure!(
        request.instances.len() <= 25,
        "At most 25 instance tabs can be loaded"
    );
    let uuid = request
        .scope
        .definition_uuid
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("A persisted definition UUID is required"))?
        .parse()?;
    let source = service.get_process_definition(uuid).await?;
    anyhow::ensure!(
        source.key == request.scope.key && source.version == request.scope.version,
        "Definition scope mismatch"
    );
    let counts = service.get_process_node_instance_counts(uuid).await?;
    if !include_instances {
        return Ok(MonitorSnapshot {
            request: request.clone(),
            observed_at: Some(chrono::Utc::now().to_rfc3339()),
            node_counts: counts
                .into_iter()
                .map(|count| MonitorNodeCount {
                    node_id: count.node_id,
                    instance_count: count.instance_count.max(0) as u64,
                    errors: count.error_count.max(0) as u64,
                    escalations: 0,
                })
                .collect(),
            ..Default::default()
        });
    }
    let page = service
        .list_process_instances(
            fluxpro_engine::admin::ProcessInstanceFilter {
                search: Some(request.query.search.clone()),
                process_definition_uuid: Some(uuid),
                state: request.query.state.clone(),
                node_id: request.query.node_id.clone(),
            },
            PageRequest {
                offset: request.query.offset as i64,
                limit: request.query.limit as i64,
            },
        )
        .await?;
    let mut instances: Vec<MonitorInstance> = project(page.items)?;
    for instance in &mut instances {
        load_issue(service, instance).await?;
    }
    let mut details = Vec::new();
    for wanted in &request.instances {
        // Validate identities before fetching histories or exposing any foreign data.
        let data = service.get_process_instance(wanted.uuid.parse()?).await?;
        anyhow::ensure!(
            data.summary.process_definition_uuid == uuid,
            "Instance belongs to another definition"
        );
        let mut instance: MonitorInstance = project(data.summary)?;
        load_issue(service, &mut instance).await?;
        let mut detail = MonitorInstanceDetails {
            instance,
            context: serde_json::to_value(data.context)?,
            context_variables: project(data.context_variables)?,
            current_node: data.current_node,
            stage_history: project(data.stage_history)?,
            ..Default::default()
        };
        // Each history remains independently paginated; never silently truncate at 250.
        match load_histories(service, wanted, &mut detail).await {
            Ok(()) => {}
            Err(error) => detail.error = Some(error.to_string()),
        }
        details.push(detail);
    }
    Ok(MonitorSnapshot {
        request: request.clone(),
        observed_at: Some(chrono::Utc::now().to_rfc3339()),
        node_counts: counts
            .into_iter()
            .map(|count| MonitorNodeCount {
                node_id: count.node_id,
                instance_count: count.instance_count.max(0) as u64,
                errors: count.error_count.max(0) as u64,
                escalations: 0,
            })
            .collect(),
        instances: MonitorPage {
            items: instances,
            total: page.total.max(0) as u64,
        },
        details,
        escalation_counts_available: false,
        error: None,
    })
}

async fn load_issue(
    service: &FluxproAdminService,
    instance: &mut MonitorInstance,
) -> anyhow::Result<()> {
    let token =
        IdField::new(&instance.token).map_err(|_| anyhow::anyhow!("Invalid runtime token"))?;
    if let Some(incident) = service.get_open_incident(&token).await? {
        instance.issues.push(MonitorIssue {
            id: incident.uuid.to_string(),
            kind: "error".into(),
            message: incident.reason.clone(),
            node_id: instance.current_node_id.clone(),
            topic: Some(incident.kind.clone()),
            created_at: Some(incident.created_at.to_rfc3339()),
            details: serde_json::to_value(incident)?,
        });
    }
    Ok(())
}

async fn load_histories(
    service: &FluxproAdminService,
    wanted: &MonitorInstanceRequest,
    detail: &mut MonitorInstanceDetails,
) -> anyhow::Result<()> {
    let uuid = wanted.uuid.parse()?;
    anyhow::ensure!(
        wanted.log_limit <= 10_000 && wanted.signal_limit <= 10_000,
        "History prefix is limited to 10000 events"
    );
    while detail.logs.items.len() < wanted.log_limit as usize {
        let page = service
            .list_process_instance_logs(
                uuid,
                ExecutionLogFilter::default(),
                PageRequest {
                    offset: detail.logs.items.len() as i64,
                    limit: (wanted.log_limit as i64 - detail.logs.items.len() as i64).min(250),
                },
            )
            .await?;
        detail.logs.total = page.total.max(0) as u64;
        let empty = page.items.is_empty();
        detail
            .logs
            .items
            .extend(project::<_, Vec<MonitorLogEntry>>(page.items)?);
        if empty || detail.logs.items.len() as u64 >= detail.logs.total {
            break;
        }
    }
    while detail.signals.items.len() < wanted.signal_limit as usize {
        let page = service
            .list_process_instance_signals(
                uuid,
                PageRequest {
                    offset: detail.signals.items.len() as i64,
                    limit: (wanted.signal_limit as i64 - detail.signals.items.len() as i64)
                        .min(250),
                },
            )
            .await?;
        detail.signals.total = page.total.max(0) as u64;
        let empty = page.items.is_empty();
        detail
            .signals
            .items
            .extend(project::<_, Vec<MonitorSignalEntry>>(page.items)?);
        if empty || detail.signals.items.len() as u64 >= detail.signals.total {
            break;
        }
    }
    Ok(())
}
