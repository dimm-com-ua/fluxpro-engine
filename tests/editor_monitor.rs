#![cfg(feature = "editor")]
#[path = "../examples/editor-demo/monitor_data.rs"]
mod monitor_data;
use fluxpro_engine::editor::*;

#[test]
fn monitor_request_survives_server_function_form_transport() {
    // Match the Console server function's argument nesting and PostUrl codec.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Arguments {
        jwt_token: String,
        request: MonitorRequest,
        definition_only: bool,
    }
    for instances in [
        vec![],
        vec![MonitorInstanceRequest {
            uuid: "00000000-0000-4000-8000-000000000005".into(),
            log_limit: 50,
            signal_limit: 25,
        }],
    ] {
        let args = Arguments {
            jwt_token: "test-token".into(),
            request: MonitorRequest {
                scope: MonitorScope {
                    key: "cash_loan".into(),
                    version: "1.1.25".into(),
                    definition_uuid: Some("00000000-0000-4000-8000-000000000001".into()),
                },
                revision: 7,
                active_counts_only: true,
                instances,
                ..Default::default()
            },
            definition_only: false,
        };
        let encoded = serde_qs::to_string(&args).unwrap();
        if args.request.instances.is_empty() {
            assert!(!encoded.contains("instances"));
        }
        let decoded: Arguments = serde_qs::Config::new(5, false)
            .deserialize_str(&encoded)
            .unwrap();
        assert_eq!(decoded, args);
    }
}

#[test]
fn counts_are_version_wide_while_search_and_pages_only_affect_the_list() {
    let request = MonitorRequest {
        scope: MonitorScope {
            key: "approval_demo".into(),
            version: "1.0.0".into(),
            definition_uuid: None,
        },
        revision: 1,
        ..Default::default()
    };
    let first = monitor_data::snapshot(request.clone());
    assert_eq!(first.instances.total, 67);
    assert_eq!(first.instances.items.len(), 25);
    assert_eq!(
        first
            .node_counts
            .iter()
            .map(|c| c.instance_count)
            .sum::<u64>(),
        67
    );
    let mut next = request.clone();
    next.query.offset = 25;
    let second = monitor_data::snapshot(next);
    assert_eq!(second.instances.items[0].process_id, "APP-0026");
    assert_eq!(first.node_counts, second.node_counts);
    let mut search = request;
    search.query.search = "TOKEN_0055".into();
    let result = monitor_data::snapshot(search);
    assert_eq!(result.instances.total, 1);
    assert_eq!(result.instances.items[0].process_id, "APP-0055");
    assert_eq!(result.node_counts, first.node_counts);
    assert!(
        result.instances.items[0]
            .issues
            .iter()
            .any(|i| i.kind == "error")
    );
}
#[test]
fn open_instance_history_pagination_keeps_context_and_stage_snapshots_distinct() {
    let mut request = MonitorRequest {
        scope: MonitorScope {
            key: "approval_demo".into(),
            version: "1.0.0".into(),
            definition_uuid: None,
        },
        instances: vec![MonitorInstanceRequest {
            uuid: "00000000-0000-4000-8000-000000000005".into(),
            log_limit: 50,
            signal_limit: 2,
        }],
        ..Default::default()
    };
    let first = monitor_data::snapshot(request.clone());
    let detail = &first.details[0];
    assert_eq!(detail.logs.items.len(), 50);
    assert_eq!(detail.logs.total, 140);
    assert_eq!(detail.signals.items.len(), 2);
    assert_ne!(detail.context, detail.stage_history[1].context);
    assert_eq!(detail.context_variables.len(), 2);
    request.instances[0].log_limit = 150;
    request.instances[0].signal_limit = 100;
    let full = monitor_data::snapshot(request);
    assert_eq!(full.details[0].logs.items.len(), 140);
    assert_eq!(full.details[0].signals.items.len(), 6);
}

#[test]
fn persisted_document_identity_survives_transport_but_not_portable_yaml() {
    let mut document = EditorDocument::default();
    let uuid = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    document.definition.uuid = Some(uuid);
    let json = serde_json::to_value(&document).unwrap();
    assert_eq!(json["definition"]["uuid"], uuid.to_string());
    let loaded: EditorDocument = serde_json::from_value(json).unwrap();
    assert_eq!(
        MonitorScope::from_document(&loaded),
        MonitorScope::from_document(&document)
    );
    assert_eq!(loaded.definition.uuid, Some(uuid));
    assert!(
        !document
            .to_project_yaml()
            .unwrap()
            .contains(&uuid.to_string())
    );
    assert!(
        serde_json::to_value(&document.definition)
            .unwrap()
            .get("uuid")
            .is_none()
    );
    let new_document: EditorDocument =
        serde_json::from_value(serde_json::to_value(EditorDocument::default()).unwrap()).unwrap();
    assert!(new_document.definition.uuid.is_none());
}
