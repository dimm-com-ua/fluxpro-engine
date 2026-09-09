use crate::document::{BlockKind, EditorDocument};
use fluxpro_engine::models::process_def::{Node, ProcessDefinition};
use serde_json::{Value, json};
use std::collections::HashSet;

const GLOBAL_FIELDS: &[&str] = &[
    "key",
    "name",
    "version",
    "status",
    "effective_from",
    "deprecated_at",
    "metadata",
    "stages",
    "special_handlers",
];

pub(crate) fn process_settings(doc: &EditorDocument) -> Value {
    let full = serde_json::to_value(&doc.definition).expect("serializable definition");
    Value::Object(
        GLOBAL_FIELDS
            .iter()
            .map(|key| (key.to_string(), full[*key].clone()))
            .collect(),
    )
}

impl EditorDocument {
    pub(crate) fn save_process_settings(
        &mut self,
        baseline: &Value,
        draft: Value,
    ) -> Result<(), String> {
        if process_settings(self) != *baseline {
            return Err("Process settings changed. Reload before applying your draft.".into());
        }
        let mut full = serde_json::to_value(&self.definition).map_err(|e| e.to_string())?;
        for key in GLOBAL_FIELDS {
            full[*key] = draft[*key].clone();
        }
        let mut replacement: ProcessDefinition =
            serde_json::from_value(full).map_err(|e| format!("Process settings: {e}"))?;
        let mut ids = HashSet::new();
        for stage in &replacement.stages {
            if !ids.insert(stage.id().to_string()) {
                return Err(format!("Duplicate stage: {}", stage.id()));
            }
        }
        // Removing a used stage must never silently break assignments. Rename by adding
        // the new stage, choosing it on affected nodes, then removing the old one.
        for node in &replacement.nodes {
            if let Some(stage) = node.set_stage() {
                let id = stage.stage_with_reason().stage.to_string();
                if !ids.contains(&id) {
                    return Err(format!(
                        "Stage {id} is used by {}. Update its stage assignment before removing or renaming it.",
                        node.id()
                    ));
                }
            }
        }
        replacement.uuid = self.definition.uuid;
        self.definition = replacement;
        Ok(())
    }

    pub(crate) fn save_node_settings(
        &mut self,
        id: &str,
        baseline: &Value,
        draft: Value,
    ) -> Result<String, String> {
        let current = self.node(id).ok_or("Block no longer exists")?;
        if serde_json::to_value(current).map_err(|e| e.to_string())? != *baseline {
            return Err("Block changed. Reload before applying your draft.".into());
        }
        let replacement: Node =
            serde_json::from_value(draft).map_err(|e| format!("Block settings: {e}"))?;
        if BlockKind::of(current) != BlockKind::of(&replacement) {
            return Err("Add a new block to change its type.".into());
        }
        let new_id = replacement.id().to_string();
        if new_id != id && self.node(&new_id).is_some() {
            return Err(format!("Block {new_id} already exists."));
        }
        let mut candidate = self.clone();
        *candidate
            .definition
            .nodes
            .iter_mut()
            .find(|n| n.id().get_id() == id)
            .unwrap() = replacement;
        if new_id != id {
            // Includes self loops and all conditional, timeout and error destinations.
            for edge in candidate
                .connections()
                .into_iter()
                .filter(|e| e.target == id)
            {
                let node = candidate
                    .definition
                    .nodes
                    .iter_mut()
                    .find(|n| n.id().get_id() == edge.source)
                    .unwrap();
                let mut value = serde_json::to_value(&*node).map_err(|e| e.to_string())?;
                *value
                    .pointer_mut(&edge.pointer)
                    .ok_or("Route no longer exists")? = json!(new_id);
                *node = serde_json::from_value(value).map_err(|e| e.to_string())?;
            }
            if let Some(position) = candidate.positions.remove(id) {
                candidate.positions.insert(new_id.clone(), position);
            }
        }
        *self = candidate;
        Ok(new_id)
    }
}

// Recognize simple units without rewriting compound or calendar ISO durations.
pub(crate) fn duration_parts(value: &str) -> (String, String) {
    for (prefix, suffix, unit) in [
        ("PT", "S", "seconds"),
        ("PT", "M", "minutes"),
        ("PT", "H", "hours"),
        ("P", "D", "days"),
        ("P", "W", "weeks"),
    ] {
        if let Some(amount) = value
            .strip_prefix(prefix)
            .and_then(|v| v.strip_suffix(suffix))
        {
            if amount
                .parse::<f64>()
                .is_ok_and(|n| n.is_finite() && n >= 0.0)
            {
                return (amount.into(), unit.into());
            }
        }
    }
    (value.into(), "custom".into())
}
pub(crate) fn duration_value(amount: &str, unit: &str) -> String {
    match unit {
        "seconds" => format!("PT{amount}S"),
        "minutes" => format!("PT{amount}M"),
        "hours" => format!("PT{amount}H"),
        "days" => format!("P{amount}D"),
        "weeks" => format!("P{amount}W"),
        _ => amount.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXAMPLES: &[&str] = &[
        include_str!("../../../examples/definitions/approval.yaml"),
        include_str!("../../../tests/fixtures/cash_loan.yaml"),
        include_str!("../../../tests/fixtures/tk_online.yaml"),
        include_str!("../../../examples/definitions/reviewed/cash_loan.yaml"),
        include_str!("../../../examples/definitions/reviewed/tk_online.yaml"),
        include_str!("../../../tests/fixtures/submitted/cash_loan.yaml"),
        include_str!("../../../tests/fixtures/submitted/tk_online.yaml"),
    ];
    #[test]
    fn every_example_survives_every_settings_form_without_data_loss() {
        for yaml in EXAMPLES {
            let mut doc = EditorDocument::from_yaml(yaml).unwrap();
            let original = serde_json::to_value(&doc.definition).unwrap();
            let positions = doc.positions.clone();
            let uuid = doc.definition.uuid;
            let global = process_settings(&doc);
            doc.save_process_settings(&global, global.clone()).unwrap();
            let nodes = doc.definition.nodes.clone();
            for node in nodes {
                let value = serde_json::to_value(&node).unwrap();
                doc.save_node_settings(node.id().get_id(), &value, value.clone())
                    .unwrap();
            }
            assert_eq!(serde_json::to_value(&doc.definition).unwrap(), original);
            assert_eq!(doc.positions, positions);
            assert_eq!(doc.definition.uuid, uuid);
        }
    }
    #[test]
    fn node_rename_updates_all_routes_and_preserves_position() {
        let mut doc = EditorDocument::from_yaml(EXAMPLES[0]).unwrap();
        doc.connect("review", "review", Some("ctx.retry")).unwrap();
        let baseline = serde_json::to_value(doc.node("review").unwrap()).unwrap();
        let mut draft = baseline.clone();
        draft["id"] = json!("approval_screen");
        let position = doc.positions["review"];
        doc.save_node_settings("review", &baseline, draft).unwrap();
        assert_eq!(doc.positions["approval_screen"], position);
        assert!(
            doc.connections()
                .iter()
                .all(|e| e.target != "review" && e.source != "review")
        );
        assert!(
            doc.connections()
                .iter()
                .any(|e| e.source == "approval_screen" && e.target == "approval_screen")
        );
        doc.definition.validate().unwrap();
    }
    #[test]
    fn typed_arguments_and_timeout_edits_preserve_other_settings() {
        let mut doc = EditorDocument::from_yaml(EXAMPLES[0]).unwrap();
        let baseline = serde_json::to_value(doc.node("prepare").unwrap()).unwrap();
        let mut draft = baseline.clone();
        draft["args"] = json!({"text":{"string":"hello"},"id":{"id_field":"some_id"},"integer":{"number":42},"decimal":{"float":1.5},"date":{"date":"2026-09-09"},"timestamp":{"datetime":"2026-09-09T12:30:00.125Z"},"flag":{"boolean":true},"list":{"array":[{"string":"one"},{"array":[{"boolean":false}]}]},"record":{"object":{"nested":[true,null,1.5,{"key":"value"}]}}});
        doc.save_node_settings("prepare", &baseline, draft.clone())
            .unwrap();
        let saved = serde_json::to_value(doc.node("prepare").unwrap()).unwrap();
        assert_eq!(saved, draft);
        let baseline = serde_json::to_value(doc.node("review").unwrap()).unwrap();
        let mut draft = baseline.clone();
        draft["timeout"] = json!({"after":duration_value("2","hours"),"at":"2026-12-31T12:00:00Z","on_timeout":"expired"});
        doc.save_node_settings("review", &baseline, draft.clone())
            .unwrap();
        assert_eq!(
            serde_json::to_value(doc.node("review").unwrap()).unwrap(),
            draft
        );
    }
    #[test]
    fn stale_drafts_invalid_ids_and_referenced_stage_deletion_are_atomic() {
        let mut doc = EditorDocument::from_yaml(EXAMPLES[0]).unwrap();
        let baseline = process_settings(&doc);
        let original = doc.to_project_yaml().unwrap();
        let mut draft = baseline.clone();
        draft["stages"] = json!([]);
        assert!(doc.save_process_settings(&baseline, draft).is_err());
        assert_eq!(doc.to_project_yaml().unwrap(), original);
        let node = serde_json::to_value(doc.node("prepare").unwrap()).unwrap();
        let mut invalid = node.clone();
        invalid["id"] = json!("review");
        assert!(doc.save_node_settings("prepare", &node, invalid).is_err());
        assert_eq!(doc.to_project_yaml().unwrap(), original);
        let mut updated = baseline.clone();
        updated["name"] = json!("Edited process");
        doc.save_process_settings(&baseline, updated).unwrap();
        assert!(
            doc.save_process_settings(&baseline, baseline.clone())
                .is_err()
        );
        // Layout and node changes do not invalidate an unrelated process draft.
        let current = process_settings(&doc);
        doc.move_node("prepare", crate::Position::new(123.0, 456.0));
        doc.save_process_settings(&current, current.clone())
            .unwrap();
    }
    #[test]
    fn duration_controls_roundtrip_simple_compound_fractional_and_calendar_values() {
        for value in [
            "PT1H",
            "PT30M",
            "PT0.25S",
            "P2D",
            "P3W",
            "P1DT2H30M",
            "P1M",
            "PT1H30M",
            "PT01H",
        ] {
            let (amount, unit) = duration_parts(value);
            assert_eq!(duration_value(&amount, &unit), value);
        }
    }
}
