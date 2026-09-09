use fluxpro_editor::{BlockKind, EditorDocument, Position};
use serde_json::json;

const APPROVAL: &str = include_str!("../../../examples/definitions/approval.yaml");

#[test]
fn exceptional_routes_leave_separate_footer_icons() {
    let document = EditorDocument::from_yaml(APPROVAL).unwrap();
    let mut checked = 0;
    for edge in document.connections() {
        if let Some((offset, _, _)) = edge.special_outlet() {
            let source = document.positions[&edge.source];
            let x = source.x + offset;
            let y = source.y + document.node_height(&edge.source);
            assert_ne!(
                offset, 112.0,
                "exceptional routes must not use the ordinary +"
            );
            assert!(
                document
                    .connection_path(&edge)
                    .unwrap()
                    .starts_with(&format!("M {x} {y} C"))
            );
            checked += 1;
        }
    }
    assert!(checked >= 1);
    let outlets: Vec<_> = [
        "/on_error/next",
        "/on_error/compensate",
        "/timeout/on_timeout",
    ]
    .into_iter()
    .map(|pointer| {
        fluxpro_editor::Connection {
            source: "a".into(),
            target: "b".into(),
            label: String::new(),
            pointer: pointer.into(),
        }
        .special_outlet()
        .unwrap()
        .0 as u32
    })
    .collect();
    assert_eq!(
        outlets
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[test]
fn branch_rows_have_separate_outlets_and_layout_reserves_their_height() {
    let mut document = EditorDocument::from_yaml(APPROVAL).unwrap();
    document
        .connect("decision", "prepare", Some("ctx.retry"))
        .unwrap();
    document
        .connect("decision", "decision", Some("ctx.again"))
        .unwrap();
    document.auto_layout();
    let branches = document.branch_routes("decision");
    assert_eq!(branches.len(), 4);
    assert_eq!(branches.last().unwrap().label, "Otherwise");
    let source = document.positions["decision"];
    let definition_before = serde_json::to_value(&document.definition).unwrap();
    for (i, edge) in branches.iter().enumerate() {
        let (port, _) = document.branch_port(edge).unwrap();
        assert_eq!(port.x, source.x + 224.0);
        assert_eq!(port.y, source.y + 108.0 + i as f64 * 44.0);
        let path = document.connection_path(edge).unwrap();
        assert!(path.starts_with(&format!("M {} {} C", port.x, port.y)));
        assert!(!path.contains("NaN"));
        assert!(port.y < source.y + document.node_height("decision"));
    }
    assert_eq!(document.node_height("start"), 88.0);
    for node in &document.definition.nodes {
        let position = document.positions[node.id().get_id()];
        if position.y > source.y {
            assert!(position.y >= source.y + document.node_height("decision") + 96.0);
        }
    }
    assert_eq!(
        definition_before,
        serde_json::to_value(&document.definition).unwrap()
    );
}

#[test]
fn imports_all_existing_fixtures_and_preserves_runtime_semantics() {
    for yaml in [
        APPROVAL,
        include_str!("../../../tests/fixtures/tk_online.yaml"),
        include_str!("../../../tests/fixtures/cash_loan.yaml"),
    ] {
        let document = EditorDocument::from_yaml(yaml).unwrap();
        document.definition.validate().unwrap();
        let original = serde_json::to_value(&document.definition).unwrap();
        let reimported = EditorDocument::from_yaml(&document.to_process_yaml().unwrap()).unwrap();
        assert_eq!(
            original,
            serde_json::to_value(&reimported.definition).unwrap()
        );
        assert_eq!(document.positions.len(), document.definition.nodes.len());
        assert!(document.connections().iter().any(|e| e.label == "Timeout"));
    }
}

#[test]
fn project_roundtrip_preserves_layout_without_polluting_process_yaml() {
    let mut document = EditorDocument::from_yaml(APPROVAL).unwrap();
    document.move_node("review", Position::new(901.25, 302.5));
    let restored = EditorDocument::from_yaml(&document.to_project_yaml().unwrap()).unwrap();
    assert_eq!(document.positions, restored.positions);
    let process: serde_json::Value =
        serde_yaml::from_str(&document.to_process_yaml().unwrap()).unwrap();
    assert!(process.get("editor").is_none());
}

#[test]
fn layout_places_start_above_all_nodes_and_finishes_at_bottom() {
    let document = EditorDocument::from_yaml(APPROVAL).unwrap();
    let start = document.positions["start"];
    let max_y = document.positions.values().map(|p| p.y).fold(0.0, f64::max);
    for node in &document.definition.nodes {
        let p = document.positions[node.id().get_id()];
        if !node.is_start() {
            assert!(p.y > start.y);
        }
        if node.is_end() {
            assert_eq!(p.y, max_y);
        }
    }
    let mut positions = document
        .positions
        .values()
        .map(|p| (p.x as u64, p.y as u64))
        .collect::<Vec<_>>();
    positions.sort();
    positions.dedup();
    assert_eq!(positions.len(), document.definition.nodes.len());
}

#[test]
fn branches_default_timeouts_errors_and_compensation_are_distinct_edges() {
    let mut document = EditorDocument::from_yaml(APPROVAL).unwrap();
    document
        .connect("prepare", "expired", Some("ctx.amount > 100"))
        .unwrap();
    document.connect("prepare", "failed", None).unwrap();
    let value = serde_json::to_value(document.node("prepare").unwrap()).unwrap();
    assert_eq!(value["next"]["default"], "failed");
    assert_eq!(value["next"]["branches"][0]["next"], "expired");
    assert_eq!(value["on_error"]["next"], "failed");
    assert_eq!(value["retries"]["max"], 2);
    let mut value = value;
    value["on_error"]["compensate"] = json!("review");
    document
        .update_node_yaml("prepare", &serde_yaml::to_string(&value).unwrap())
        .unwrap();
    assert!(
        document
            .connections()
            .iter()
            .any(|e| e.label == "Compensate" && e.target == "review")
    );
    let branch = document
        .connections()
        .into_iter()
        .find(|e| e.source == "prepare" && e.label == "ctx.amount > 100")
        .unwrap();
    document.disconnect(&branch).unwrap();
    let value = serde_json::to_value(document.node("prepare").unwrap()).unwrap();
    assert!(value["next"]["branches"].as_array().unwrap().is_empty());
    assert_eq!(value["next"]["default"], "failed");
    assert_eq!(value["on_error"]["compensate"], "review");
    document.definition.validate().unwrap();
}

#[test]
fn removing_timeout_removes_configuration_and_not_the_success_path() {
    let mut document = EditorDocument::from_yaml(APPROVAL).unwrap();
    let timeout = document
        .connections()
        .into_iter()
        .find(|e| e.source == "review" && e.label == "Timeout")
        .unwrap();
    document.disconnect(&timeout).unwrap();
    let value = serde_json::to_value(document.node("review").unwrap()).unwrap();
    assert!(value["timeout"].is_null());
    assert_eq!(value["next"], "decision");
    document.definition.validate().unwrap();
}

#[test]
fn cycles_and_unreachable_nodes_have_finite_deterministic_layouts() {
    let mut document = EditorDocument::from_yaml(APPROVAL).unwrap();
    document
        .connect("decision", "prepare", Some("ctx.retry"))
        .unwrap();
    let id = document
        .add_block(BlockKind::Wait, Position::default())
        .unwrap();
    document.auto_layout();
    let first = document.positions.clone();
    document.auto_layout();
    assert_eq!(first, document.positions);
    assert!(
        document
            .diagnostics()
            .iter()
            .any(|m| m == &format!("{id}: not reachable from Start"))
    );
    assert!(document.connections().iter().all(|edge| {
        edge.path(&document.positions)
            .is_some_and(|p| !p.contains("NaN"))
    }));
}

#[test]
fn additions_create_declarations_and_do_not_overwrite_ids_or_routes() {
    let mut document = EditorDocument::default();
    let original_edges = document.connections();
    for kind in [
        BlockKind::UserTask,
        BlockKind::Wait,
        BlockKind::Gateway,
        BlockKind::ServiceTask,
        BlockKind::End,
    ] {
        let first = document.add_block(kind, Position::default()).unwrap();
        let second = document.add_block(kind, Position::default()).unwrap();
        assert_ne!(first, second);
    }
    assert!(
        document
            .add_block(BlockKind::Start, Position::default())
            .is_err()
    );
    assert_eq!(original_edges, document.connections());
    document.definition.validate().unwrap();
}

#[test]
fn malformed_and_duplicate_imports_are_rejected_but_drafts_are_editable() {
    assert!(EditorDocument::from_yaml("nodes: [").is_err());
    let duplicate = "key: demo\nname: Demo\nversion: 1.0.0\nstatus: draft\nnodes:\n- {id: a, type: End}\n- {id: a, type: End}";
    assert!(
        EditorDocument::from_yaml(duplicate)
            .unwrap_err()
            .contains("Duplicate")
    );
    let draft = EditorDocument::from_yaml(
        "key: demo\nname: Demo\nversion: 1.0.0\nstatus: draft\nnodes: []",
    )
    .unwrap();
    assert!(!draft.diagnostics().is_empty());
}

#[test]
fn mutations_reject_invalid_references_and_keep_node_identity_stable() {
    let mut document = EditorDocument::default();
    assert!(document.connect("finish", "start", None).is_err());
    assert!(document.connect("start", "missing", None).is_err());
    assert!(document.connect("start", "finish", Some("true")).is_err());
    assert!(document.remove_node("finish").is_err());
    assert!(
        document
            .update_node_yaml("finish", "id: changed\ntype: End")
            .is_err()
    );
    document.definition.validate().unwrap();
}

#[test]
fn embedding_a_typed_definition_preserves_its_database_identity() {
    let mut definition = EditorDocument::default().definition;
    definition.uuid = Some("f0bf905b-a64a-4c51-b58e-e1b0d5d2c504".parse().unwrap());
    let expected = definition.uuid;
    let document = EditorDocument::new(definition).unwrap();
    assert_eq!(document.definition.uuid, expected);
}
