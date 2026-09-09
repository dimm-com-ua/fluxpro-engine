//! Checks that shipped examples and node-reference snippets remain compatible with the schema.

use fluxpro_engine::models::process_def::ProcessDefinition;

#[test]
fn minimal_json_example_compiles() {
    let definition: ProcessDefinition =
        serde_json::from_str(include_str!("../examples/definitions/minimal.json")).unwrap();
    let compiled = definition.compile().unwrap();
    let round_trip: ProcessDefinition = serde_json::from_value(compiled).unwrap();
    round_trip.validate().unwrap();
    assert!(round_trip.stages.iter().any(|stage| stage.is_initial()));
}

#[cfg(feature = "api")]
mod yaml_examples {
    use super::*;
    use fluxpro_engine::db_service::FluxproDbServiceImpl;
    use fluxpro_engine::models::context_map::context_map::{ContextMapBuilder, ContextValue};
    use fluxpro_engine::models::id_field::IdField;
    use fluxpro_engine::models::process_def::{Next, Node};
    use fluxpro_engine::service::process_service::{FluxproService, FluxproServiceImpl};
    use std::collections::HashSet;
    use std::sync::Arc;

    fn approval() -> ProcessDefinition {
        serde_yaml::from_str(include_str!("../examples/definitions/approval.yaml")).unwrap()
    }

    #[test]
    fn approval_example_covers_every_node_type() {
        let definition = approval();
        definition.validate().unwrap();
        assert!(definition.stages.iter().any(|stage| stage.is_initial()));
        let types: HashSet<_> = definition
            .nodes
            .iter()
            .map(|node| match node {
                Node::Start { .. } => "Start",
                Node::End { .. } => "End",
                Node::ServiceTask { .. } => "ServiceTask",
                Node::UserTask { .. } => "UserTask",
                Node::Wait { .. } => "Wait",
                Node::Gateway { .. } => "Gateway",
            })
            .collect();
        assert_eq!(types.len(), 6);
    }

    #[test]
    fn reference_nodes_parse_and_validate_in_the_documented_workflow() {
        let reference = include_str!("../docs/nodes.md");
        let mut snippets = 0;
        for block in reference.split("```yaml\n").skip(1) {
            let yaml = block.split("```").next().unwrap();
            let node: Node = serde_yaml::from_str(yaml).unwrap();
            let mut definition = approval();
            let index = definition
                .nodes
                .iter()
                .position(|item| item.id() == node.id())
                .expect("reference snippets must use IDs declared in the approval example");
            definition.nodes[index] = node;
            definition.validate().unwrap();
            snippets += 1;
        }
        assert_eq!(
            snippets, 7,
            "validate all six node types and the inline routing example"
        );
    }

    #[tokio::test]
    async fn documented_signal_routes_execute_with_the_runtime_evaluator() {
        // Expression evaluation does not query PostgreSQL; the pool stays disconnected.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://localhost/unused_documentation_test")
            .unwrap();
        let service = FluxproServiceImpl::new(Arc::new(FluxproDbServiceImpl::new(pool)));
        let fixture: ProcessDefinition =
            serde_yaml::from_str(include_str!("fixtures/cash_loan.yaml")).unwrap();
        let mut nodes = approval().nodes;
        nodes.extend(fixture.nodes);
        nodes.extend(
            include_str!("../docs/nodes.md")
                .split("```yaml\n")
                .skip(1)
                .map(|block| {
                    serde_yaml::from_str::<Node>(block.split("```").next().unwrap()).unwrap()
                }),
        );
        let mut expressions = HashSet::new();
        for node in nodes {
            match node {
                Node::Gateway { branches, .. } => {
                    expressions.extend(branches.into_iter().map(|branch| branch.when));
                }
                Node::UserTask {
                    next: Some(Next::Routes(routes)),
                    ..
                }
                | Node::Wait {
                    next: Some(Next::Routes(routes)),
                    ..
                }
                | Node::ServiceTask {
                    next: Some(Next::Routes(routes)),
                    ..
                } => {
                    expressions.extend(routes.branches.into_iter().map(|branch| branch.when));
                }
                _ => {}
            }
        }
        assert!(!expressions.is_empty());
        for expression in expressions {
            let mut matched = false;
            for signal in [
                "approved",
                "rejected",
                "calculator_interacted",
                "terms_selected",
            ] {
                let context = ContextMapBuilder::new()
                    .set(
                        IdField::new("_last_signal").unwrap(),
                        ContextValue::string(signal.into()),
                    )
                    .build();
                let result = service.eval_bool_with_context(&expression, &context).await;
                assert!(
                    result.is_some(),
                    "invalid documented expression: {expression}"
                );
                matched |= result == Some(true);
            }
            assert!(
                matched,
                "documented route never matches an example signal: {expression}"
            );
        }
    }
}
