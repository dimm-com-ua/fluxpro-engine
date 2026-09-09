//! Preserves explicit XOR definitions while rejecting the removed AND mode.

use fluxpro_engine::models::process_def::{GatewayKind, Node, ProcessDefinition};
use serde_json::{Value, json};

fn legacy_xor_node() -> Value {
    json!({
        "id": "credit_info_route",
        "type": "Gateway",
        "gateway": "XOR",
        "branches": [
            {
                "when": "ctx._last_signal == 'terms_viewed'",
                "next": "close_ticket_credit_info"
            },
            {
                "when": "ctx._last_signal == 'operator_credit_info_resolution' && ctx.operator_action == 'reject'",
                "next": "finalize_reject"
            }
        ],
        "next": "finalize_reject"
    })
}

fn assert_legacy_xor_compatible(node: Node) {
    assert!(matches!(
        &node,
        Node::Gateway {
            gateway: GatewayKind::XOR,
            ..
        }
    ));
    let original = legacy_xor_node();
    let serialized = serde_json::to_value(&node).unwrap();
    for field in ["id", "type", "gateway", "branches", "next"] {
        assert_eq!(serialized[field], original[field], "changed field {field}");
    }

    let mut definition: ProcessDefinition = serde_json::from_value(json!({
        "key": "legacy_xor",
        "name": "Legacy XOR workflow",
        "version": "1.0.0",
        "status": "active",
        "stages": [{"id":"created","name":"Created","is_initial":true}],
        "nodes": [
            { "id": "start", "type": "Start", "next": "credit_info_route" },
            { "id": "close_ticket_credit_info", "type": "End" },
            { "id": "finalize_reject", "type": "End" }
        ]
    }))
    .unwrap();
    definition.nodes.push(node);
    // Legacy reserved-property syntax is normalized without changing stored YAML.
    definition.validate().unwrap();
    let compiled = serde_json::to_value(&definition).unwrap();
    assert_eq!(compiled["nodes"][3]["gateway"], "XOR");
    // Schema compatibility preserves expression text; Rhai syntax is a separate concern.
    assert_eq!(compiled["nodes"][3]["branches"], original["branches"]);
}

#[test]
fn existing_json_gateway_keeps_explicit_xor_and_ordered_routes() {
    assert_legacy_xor_compatible(serde_json::from_value(legacy_xor_node()).unwrap());
}

#[test]
fn removed_and_mode_is_rejected_in_json() {
    let mut node = legacy_xor_node();
    node["gateway"] = json!("AND");
    let error = serde_json::from_value::<Node>(node).unwrap_err();
    assert!(error.to_string().contains("unknown variant `AND`"));
}

#[cfg(feature = "api")]
#[test]
fn existing_yaml_gateway_keeps_explicit_xor_and_ordered_routes() {
    let yaml = r#"
id: credit_info_route
type: Gateway
gateway: XOR
branches:
  - when: "ctx._last_signal == 'terms_viewed'"
    next: close_ticket_credit_info
  - when: "ctx._last_signal == 'operator_credit_info_resolution' && ctx.operator_action == 'reject'"
    next: finalize_reject
next: finalize_reject
"#;
    assert_legacy_xor_compatible(serde_yaml::from_str(yaml).unwrap());
    let error =
        serde_yaml::from_str::<Node>(&yaml.replace("gateway: XOR", "gateway: AND")).unwrap_err();
    assert!(error.to_string().contains("unknown variant `AND`"));
}
