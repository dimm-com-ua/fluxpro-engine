use fluxpro_editor::*;
use serde_json::json;

// Explicitly simulated data for exercising the public host callback contract.
pub fn snapshot(request: MonitorRequest) -> MonitorSnapshot {
    let scope = &request.scope;
    let instances = (1..=67)
        .map(|n| {
            let node = match n % 7 {
                0 => "completed_end",
                1 => {
                    if request.revision % 2 == 0 {
                        "review"
                    } else {
                        "prepare"
                    }
                }
                2 | 3 => "review",
                4 => "await_confirmation",
                5 => "decision",
                _ => "prepare",
            };
            let issue = if n % 11 == 0 {
                Some(MonitorIssue {
                    id: format!("incident-{n}"),
                    kind: "error".into(),
                    message: "Payment provider is unavailable; retry budget exhausted.".into(),
                    node_id: Some(node.into()),
                    topic: Some("handler.unavailable".into()),
                    created_at: Some("2026-09-09T09:20:00Z".into()),
                    details: json!({"attempt":3,"resolved_at":null}),
                })
            } else if n % 5 == 0 {
                Some(MonitorIssue {
                    id: format!("ticket-{n}"),
                    kind: "escalation".into(),
                    message: "Manual verification is required.".into(),
                    node_id: Some(node.into()),
                    topic: Some("approval_support".into()),
                    created_at: Some("2026-09-09T09:15:00Z".into()),
                    details: json!({"assigned_to":"support","priority":"high"}),
                })
            } else {
                None
            };
            MonitorInstance {
                uuid: format!("00000000-0000-4000-8000-{n:012}"),
                created_at: format!("2026-09-09T08:{:02}:00Z", n % 60),
                process_definition_uuid: scope
                    .definition_uuid
                    .clone()
                    .unwrap_or("demo-definition".into()),
                process_key: scope.key.clone(),
                process_version: scope.version.clone(),
                process_id: format!("APP-{n:04}"),
                token: format!("approval_token_{n:04}"),
                state: if issue.as_ref().is_some_and(|i| i.kind == "error") {
                    "suspended"
                } else if node == "completed_end" {
                    "completed"
                } else {
                    "running"
                }
                .into(),
                current_node_id: Some(node.into()),
                current_stage_id: Some(
                    if node == "completed_end" {
                        "completed"
                    } else {
                        "reviewing"
                    }
                    .into(),
                ),
                current_stage_name: Some(
                    if node == "completed_end" {
                        "Completed"
                    } else {
                        "Reviewing"
                    }
                    .into(),
                ),
                current_stage_reason: Some("Application received".into()),
                issues: issue.into_iter().collect(),
                metadata: std::collections::BTreeMap::from([
                    (
                        "node_visit_id".into(),
                        json!(format!("visit-{n}-{}", request.revision)),
                    ),
                    ("wait_completed".into(), json!(false)),
                ]),
            }
        })
        .collect::<Vec<_>>();
    let node_counts = [
        "start",
        "prepare",
        "review",
        "decision",
        "await_confirmation",
        "completed_end",
        "rejected_end",
        "expired",
        "failed",
    ]
    .into_iter()
    .map(|id| {
        let matches = instances
            .iter()
            .filter(|i| i.current_node_id.as_deref() == Some(id))
            .collect::<Vec<_>>();
        MonitorNodeCount {
            node_id: id.into(),
            instance_count: matches.len() as u64,
            errors: matches
                .iter()
                .filter(|i| i.issues.iter().any(|x| x.kind == "error"))
                .count() as u64,
            escalations: matches
                .iter()
                .filter(|i| i.issues.iter().any(|x| x.kind == "escalation"))
                .count() as u64,
        }
    })
    .collect();
    let matching = instances
        .iter()
        .filter(|i| request.query.matches(i))
        .cloned()
        .collect::<Vec<_>>();
    let details=request.instances.iter().filter_map(|r|{
        let instance=instances.iter().find(|i|i.uuid==r.uuid)?.clone();
        let logs=(0..140).take(r.log_limit as usize).map(|n|MonitorLogEntry {uuid:format!("log-{}-{n}",instance.uuid),created_at:format!("2026-09-09T{:02}:{:02}:00Z",10-n/60,59-n%60),level:if n==0 && !instance.issues.is_empty(){"warning"}else{"info"}.into(),event_type:if n%2==0{"node.entered"}else{"node.completed"}.into(),message:if n%2==0{"Entered workflow block"}else{"Completed handler and committed transition"}.into(),node_id:instance.current_node_id.clone(),data:std::collections::BTreeMap::from([("source".into(),json!("engine")),("attempt".into(),json!(1)),("details".into(),json!({"elapsed_ms":24+n}))])}).collect();
        let context=json!({"amount":{"number":25000},"currency":{"string":"UAH"},"approved":{"boolean":false},"customer":{"object":{"name":"Demo customer","tags":["returning","verified"]}},"_last_signal":{"string":"approved"}});
        Some(MonitorInstanceDetails {instance:instance.clone(),context:context.clone(),context_variables:vec![json!({"scope":"_","name":"amount","value":{"number":25000}}),json!({"scope":"review","name":"amount","value":{"number":20000}})],current_node:Some(json!({"id":instance.current_node_id,"type":match instance.current_node_id.as_deref(){Some("prepare")=>"ServiceTask",Some("decision")=>"Gateway",Some("await_confirmation")=>"Wait",Some("completed_end")=>"End",_=>"UserTask"}})),stage_history:vec![MonitorStageEntry{uuid:"stage-2".into(),created_at:"2026-09-09T09:10:00Z".into(),stage_id:Some("reviewing".into()),stage_name:Some("Reviewing".into()),reason:Some("Documents accepted".into()),context:context.clone()},MonitorStageEntry{uuid:"stage-1".into(),created_at:"2026-09-09T08:00:00Z".into(),stage_id:Some("created".into()),stage_name:Some("Created".into()),reason:Some("Application started".into()),context:json!({"amount":{"number":20000}})}],logs:MonitorPage{items:logs,total:140},signals:MonitorPage{items:(0..6).take(r.signal_limit as usize).map(|n|MonitorSignalEntry{uuid:format!("signal-{n}"),created_at:format!("2026-09-09T09:{:02}:00Z",30-n),signal_name:if n%2==0{"approved"}else{"confirmed"}.into(),payload:json!({"source":"demo","sequence":n})}).collect(),total:6},error:None})
    }).collect();
    MonitorSnapshot {
        escalation_counts_available: true,
        observed_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        node_counts,
        instances: MonitorPage {
            total: matching.len() as u64,
            items: matching
                .into_iter()
                .skip(request.query.offset as usize)
                .take(request.query.limit as usize)
                .collect(),
        },
        details,
        error: None,
        request,
    }
}
