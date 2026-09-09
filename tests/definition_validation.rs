//! Validation is available without a database; runtime adds Rhai compilation.
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::models::process_def::{NodeTimeout, ProcessDefinition};
use serde_json::json;

fn sample() -> serde_json::Value {
    serde_json::from_str(include_str!("../examples/definitions/minimal.json")).unwrap()
}
#[test]
fn validation_rejects_missing_or_multiple_initial_stages() {
    let mut raw = sample();
    raw["stages"] = json!([]);
    assert!(
        serde_json::from_value::<ProcessDefinition>(raw.clone())
            .unwrap()
            .validate()
            .is_err()
    );
    raw["stages"] = json!([{"id":"one","name":"One","is_initial":true},{"id":"two","name":"Two","is_initial":true}]);
    assert!(
        serde_json::from_value::<ProcessDefinition>(raw)
            .unwrap()
            .validate()
            .is_err()
    );
}
#[test]
fn timeout_parser_rejects_empty_zero_trailing_junk_and_overflow() {
    for value in [
        None,
        Some(""),
        Some("PT0S"),
        Some("PT1Sjunk"),
        Some("P4294967295Y"),
        Some("-PT1S"),
        Some("P"),
    ] {
        let timer = NodeTimeout {
            after: value.map(str::to_string),
            at: None,
            on_timeout: IdField::new("done").unwrap(),
        };
        assert!(timer.after_now().is_err(), "accepted {value:?}");
    }
    let timer = NodeTimeout {
        after: Some("PT0.5S".into()),
        at: None,
        on_timeout: IdField::new("done").unwrap(),
    };
    assert!(timer.after_now().is_ok());
}
#[test]
fn validation_checks_timers_and_empty_signal_lists() {
    let mut raw = sample();
    raw["nodes"][0]["next"] = json!("wait");
    raw["nodes"].as_array_mut().unwrap().push(json!({"id":"wait","type":"Wait","wait_for":{"signals":[]},"timeout":{"after":"PT1Sbad","on_timeout":"done"}}));
    let error = serde_json::from_value::<ProcessDefinition>(raw)
        .unwrap()
        .validate()
        .unwrap_err()
        .to_string();
    assert!(error.contains("wait_for.signals"));
    assert!(error.contains("invalid timeout"));
}
#[test]
fn reachability_reports_auxiliary_nodes_without_rejecting_external_entry() {
    let mut raw = sample();
    raw["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"auxiliary","type":"End"}));
    let d = serde_json::from_value::<ProcessDefinition>(raw).unwrap();
    assert_eq!(
        d.unreachable_nodes(),
        vec![IdField::new("auxiliary").unwrap()]
    );
    d.validate().unwrap();
}
#[cfg(feature = "runtime")]
#[test]
fn conditions_distinguish_false_from_invalid_missing_and_non_boolean() {
    use fluxpro_engine::models::context_map::context_map::ContextMap;
    use fluxpro_engine::service::expressions::{evaluate_condition, validate_condition};
    let ctx = ContextMap::default();
    assert_eq!(evaluate_condition("false", &ctx).unwrap(), false);
    for expression in [
        "ctx.missing > 0",
        "ctx[\"missing\"] == 1",
        "1 / 0 > 0",
        "42",
        "true &&",
    ] {
        assert!(
            evaluate_condition(expression, &ctx).is_err(),
            "accepted {expression}"
        );
    }
    assert!(validate_condition("ctx._last_signal == 'done'").is_ok());
    assert!(validate_condition("ctx[\"_last_signal\"] == 'done'").is_ok());
    assert!(validate_condition("let answer = true; answer").is_err());
    assert!(!evaluate_condition("ctx.contains(\"optional\") && ctx.optional", &ctx).unwrap());
}

#[cfg(feature = "runtime")]
#[test]
fn legacy_expression_normalization_preserves_literals_and_escapes() {
    use fluxpro_engine::models::context_map::context_map::{ContextMap, ContextValue};
    use fluxpro_engine::service::expressions::evaluate_condition;
    let context = ContextMap(
        [(
            IdField::new("_last_signal").unwrap(),
            ContextValue::string("done".into()),
        )]
        .into(),
    );
    for expression in [
        "ctx._last_signal == 'done'",
        "ctx . _last_signal == \"done\"",
        r#""ctx._last_signal" == 'ctx._last_signal'"#,
        r#""can't don't" == "can't don't""#,
        r#"'can\'t' == "can't""#,
        r#"'say "hi"' == "say \"hi\"""#,
    ] {
        assert!(
            evaluate_condition(expression, &context).unwrap(),
            "{expression}"
        );
    }
    assert!(evaluate_condition("ctx._last_signal == 'done", &context).is_err());
}
