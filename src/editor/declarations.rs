//! Editing process-level declarations without changing executable node types.
use crate::editor::EditorDocument;
use crate::models::{
    context_map::context_map::ContextValue,
    id_field::IdField,
    process_def::{FormDef, Node, SignalDef, WaitFor, escalation_def::EscalationDef},
};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclarationKind {
    Form,
    Signal,
    Escalation,
}

#[derive(Clone, Debug)]
pub(crate) struct DeclarationUse {
    pub label: String,
    pub node: Option<String>,
    pub escalation: Option<String>,
}

impl DeclarationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Form => "Forms",
            Self::Signal => "Signals",
            Self::Escalation => "Escalations",
        }
    }
    pub fn singular(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Signal => "signal",
            Self::Escalation => "escalation",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Form => "▤",
            Self::Signal => "⌁",
            Self::Escalation => "⚑",
        }
    }
    pub fn id_field(self) -> &'static str {
        match self {
            Self::Form => "id",
            Self::Signal => "name",
            Self::Escalation => "topic",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Form => {
                "Declare the forms and roles used by your user tasks. Your application renders the form itself."
            }
            Self::Signal => {
                "Declare the events that resume waiting blocks or follow an operator action."
            }
            Self::Escalation => {
                "Define topics and the actions available to an operator when a process needs attention."
            }
        }
    }
}

impl EditorDocument {
    pub(crate) fn declaration_ids(&self, kind: DeclarationKind) -> Vec<String> {
        match kind {
            DeclarationKind::Form => self
                .definition
                .forms
                .iter()
                .map(|v| v.id.to_string())
                .collect(),
            DeclarationKind::Signal => self
                .definition
                .signals
                .iter()
                .map(|v| v.name().to_owned())
                .collect(),
            DeclarationKind::Escalation => self
                .definition
                .escalations
                .iter()
                .map(|v| v.topic.to_string())
                .collect(),
        }
    }

    pub(crate) fn declaration(&self, kind: DeclarationKind, id: &str) -> Option<Value> {
        match kind {
            DeclarationKind::Form => self
                .definition
                .forms
                .iter()
                .find(|v| v.id.get_id() == id)
                .map(|v| json!(v)),
            DeclarationKind::Signal => self
                .definition
                .signals
                .iter()
                .find(|v| v.name() == id)
                .map(|v| json!({"name": v.name()})),
            DeclarationKind::Escalation => self
                .definition
                .escalations
                .iter()
                .find(|v| v.topic.get_id() == id)
                .map(|v| json!(v)),
        }
    }

    pub(crate) fn new_declaration(&self, kind: DeclarationKind) -> Value {
        let ids = self.declaration_ids(kind);
        let id = (1..)
            .map(|n| format!("{}_{n}", kind.singular()))
            .find(|id| !ids.contains(id))
            .unwrap();
        match kind {
            DeclarationKind::Form => json!({"id": id, "roles": []}),
            DeclarationKind::Signal => json!({"name": id}),
            DeclarationKind::Escalation => json!({"topic": id, "actions": []}),
        }
    }

    pub(crate) fn declaration_uses(&self, kind: DeclarationKind, id: &str) -> Vec<DeclarationUse> {
        let mut uses = Vec::new();
        for node in &self.definition.nodes {
            let typed = match (kind, node) {
                (DeclarationKind::Form, Node::UserTask { form, .. }) => form.get_id() == id,
                (
                    DeclarationKind::Signal,
                    Node::UserTask {
                        wait_for: Some(wait),
                        ..
                    }
                    | Node::Wait { wait_for: wait, .. },
                ) => wait.signals().iter().any(|s| s.get_id() == id),
                (
                    DeclarationKind::Escalation,
                    Node::ServiceTask {
                        handler,
                        args: Some(args),
                        ..
                    },
                ) if handler.get_id() == "create_support_ticket" => args
                    .as_id_field(&IdField::new("topic").unwrap())
                    .is_some_and(|topic| topic.get_id() == id),
                _ => false,
            };
            // Rhai is not rewritten: include possible textual references so the
            // operator sees why a rename/delete needs a condition edit first.
            let in_condition = kind == DeclarationKind::Signal && node_conditions_contain(node, id);
            if typed || in_condition {
                uses.push(DeclarationUse {
                    label: format!(
                        "{}{}",
                        node.id(),
                        if in_condition { " · condition" } else { "" }
                    ),
                    node: Some(node.id().to_string()),
                    escalation: None,
                });
            }
        }
        if kind == DeclarationKind::Signal {
            for topic in &self.definition.escalations {
                for action in &topic.actions {
                    if action
                        .emit_signal
                        .as_ref()
                        .is_some_and(|s| s.get_id() == id)
                    {
                        uses.push(DeclarationUse {
                            label: format!("{} / {}", topic.topic, action.label),
                            node: None,
                            escalation: Some(topic.topic.to_string()),
                        });
                    }
                }
            }
        }
        uses
    }

    /// All validation happens on a copy; direct callers get atomic edits too.
    pub(crate) fn save_declaration(
        &mut self,
        kind: DeclarationKind,
        original: Option<&str>,
        value: Value,
    ) -> Result<String, String> {
        let id = value[kind.id_field()]
            .as_str()
            .ok_or("Enter an identifier")?
            .to_string();
        IdField::new(&id).map_err(|e| e.to_string())?;
        let ids = self.declaration_ids(kind);
        if original.is_some_and(|id| !ids.iter().any(|existing| existing == id)) {
            return Err("This declaration no longer exists. Select it again.".into());
        }
        if original != Some(id.as_str()) && ids.contains(&id) {
            return Err(format!(
                "A {} named '{id}' already exists.",
                kind.singular()
            ));
        }
        let mut next = self.clone();
        match kind {
            DeclarationKind::Form => {
                let form: FormDef = serde_json::from_value(value).map_err(|e| e.to_string())?;
                let mut roles = HashSet::new();
                if form.roles.iter().any(|r| !roles.insert(r.get_id())) {
                    return Err("Role IDs must be unique.".into());
                }
                if let Some(index) = original.and_then(|id| {
                    next.definition
                        .forms
                        .iter()
                        .position(|f| f.id.get_id() == id)
                }) {
                    next.definition.forms[index] = form;
                } else {
                    next.definition.forms.push(form);
                }
            }
            DeclarationKind::Signal => {
                let signal: SignalDef = serde_json::from_value(value).map_err(|e| e.to_string())?;
                if let Some(index) = original
                    .and_then(|id| next.definition.signals.iter().position(|s| s.name() == id))
                {
                    // Preserve the original compact/object representation.
                    next.definition.signals[index] = match next.definition.signals[index] {
                        SignalDef::Name(_) => SignalDef::Name(signal.id().clone()),
                        _ => signal,
                    };
                } else {
                    next.definition.signals.push(signal);
                }
            }
            DeclarationKind::Escalation => {
                let topic: EscalationDef =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
                let mut actions = HashSet::new();
                for action in &topic.actions {
                    if !actions.insert(action.id.get_id()) {
                        return Err(format!("Duplicate action ID: {}", action.id));
                    }
                    if action.label.trim().is_empty() {
                        return Err("Every action needs a label.".into());
                    }
                    if let Some(signal) = &action.emit_signal {
                        if !next
                            .definition
                            .signals
                            .iter()
                            .any(|s| s.name() == signal.get_id())
                        {
                            return Err(format!("Declare signal '{signal}' in Signals first."));
                        }
                    }
                }
                if let Some(index) = original.and_then(|id| {
                    next.definition
                        .escalations
                        .iter()
                        .position(|v| v.topic.get_id() == id)
                }) {
                    next.definition.escalations[index] = topic;
                } else {
                    next.definition.escalations.push(topic);
                }
            }
        }
        if let Some(old) = original.filter(|old| *old != id) {
            next.rename_declaration_references(kind, old, &id)?;
        }
        *self = next;
        Ok(id)
    }

    fn rename_declaration_references(
        &mut self,
        kind: DeclarationKind,
        old: &str,
        new: &str,
    ) -> Result<(), String> {
        if kind == DeclarationKind::Signal
            && self
                .definition
                .nodes
                .iter()
                .any(|node| node_conditions_contain(node, old))
        {
            return Err(format!(
                "'{old}' appears in a condition. Update that condition first; expressions are not rewritten automatically."
            ));
        }
        let new = IdField::new(new).map_err(|e| e.to_string())?;
        for node in &mut self.definition.nodes {
            match (kind, node) {
                (DeclarationKind::Form, Node::UserTask { form, .. }) if form.get_id() == old => {
                    *form = new.clone()
                }
                (
                    DeclarationKind::Signal,
                    Node::UserTask {
                        wait_for: Some(wait),
                        ..
                    }
                    | Node::Wait { wait_for: wait, .. },
                ) => match wait {
                    WaitFor::Single { signal } => {
                        if signal.get_id() == old {
                            *signal = new.clone();
                        }
                    }
                    WaitFor::Multi { signals } => {
                        for signal in signals {
                            if signal.get_id() == old {
                                *signal = new.clone();
                            }
                        }
                    }
                },
                (
                    DeclarationKind::Escalation,
                    Node::ServiceTask {
                        handler,
                        args: Some(args),
                        ..
                    },
                ) if handler.get_id() == "create_support_ticket" => {
                    if let Some(ContextValue::IdField { id_field }) =
                        args.0.get_mut(&IdField::new("topic").unwrap())
                    {
                        if id_field.get_id() == old {
                            *id_field = new.clone();
                        }
                    }
                }
                _ => {}
            }
        }
        if kind == DeclarationKind::Signal {
            for topic in &mut self.definition.escalations {
                for action in &mut topic.actions {
                    if action
                        .emit_signal
                        .as_ref()
                        .is_some_and(|s| s.get_id() == old)
                    {
                        action.emit_signal = Some(new.clone());
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn remove_declaration(
        &mut self,
        kind: DeclarationKind,
        id: &str,
    ) -> Result<(), String> {
        if !self
            .declaration_ids(kind)
            .iter()
            .any(|existing| existing == id)
        {
            return Err("Declaration not found.".into());
        }
        let uses = self.declaration_uses(kind, id);
        if !uses.is_empty() {
            return Err(format!(
                "'{id}' is used by {}. Update these references before deleting it.",
                uses.iter()
                    .map(|u| u.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        match kind {
            DeclarationKind::Form => self.definition.forms.retain(|v| v.id.get_id() != id),
            DeclarationKind::Signal => self.definition.signals.retain(|v| v.name() != id),
            DeclarationKind::Escalation => self
                .definition
                .escalations
                .retain(|v| v.topic.get_id() != id),
        }
        Ok(())
    }
}

fn node_conditions_contain(node: &Node, id: &str) -> bool {
    let value = json!(node);
    ["/branches", "/next/branches"].into_iter().any(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_array)
            .is_some_and(|branches| {
                branches.iter().any(|b| {
                    b["when"]
                        .as_str()
                        .is_some_and(|condition| condition.contains(id))
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{BlockKind, Position, document::EditorState};

    fn with_user_task() -> (EditorDocument, String) {
        let mut doc = EditorDocument::default();
        let id = doc
            .add_block(BlockKind::UserTask, Position::new(300.0, 200.0))
            .unwrap();
        doc.connect("start", &id, None).unwrap();
        doc.connect(&id, "finish", None).unwrap();
        (doc, id)
    }

    #[test]
    fn forms_rename_together_with_user_task_references_and_roles() {
        let (mut doc, node) = with_user_task();
        let old = doc.definition.forms[0].id.to_string();
        let positions = doc.positions.clone();
        doc.save_declaration(
            DeclarationKind::Form,
            Some(&old),
            json!({"id":"review_form","roles":["reviewer","manager"]}),
        )
        .unwrap();
        assert_eq!(json!(doc.node(&node).unwrap())["form"], "review_form");
        assert_eq!(
            doc.declaration_uses(DeclarationKind::Form, "review_form")[0]
                .node
                .as_deref(),
            Some(node.as_str())
        );
        assert!(
            doc.remove_declaration(DeclarationKind::Form, "review_form")
                .is_err()
        );
        assert_eq!(doc.positions, positions);
        doc.definition.validate().unwrap();
    }

    #[test]
    fn signals_rename_wait_lists_and_escalation_actions_atomically() {
        let (mut doc, node) = with_user_task();
        let old = doc.definition.signals[0].name().to_string();
        doc.save_declaration(DeclarationKind::Escalation, None, topic("support", &old))
            .unwrap();
        doc.save_declaration(
            DeclarationKind::Signal,
            Some(&old),
            json!({"name":"reviewed"}),
        )
        .unwrap();
        let value = json!(doc.node(&node).unwrap());
        assert_eq!(value["wait_for"]["signal"], "reviewed");
        assert_eq!(
            doc.definition.escalations[0].actions[0]
                .emit_signal
                .as_ref()
                .unwrap()
                .get_id(),
            "reviewed"
        );
        assert_eq!(
            doc.declaration_uses(DeclarationKind::Signal, "reviewed")
                .len(),
            2
        );
        assert!(
            doc.remove_declaration(DeclarationKind::Signal, "reviewed")
                .is_err()
        );
        doc.definition.validate().unwrap();
    }

    #[test]
    fn conditions_are_reported_and_never_silently_rewritten() {
        let mut doc =
            EditorDocument::from_yaml(include_str!("../../examples/definitions/approval.yaml"))
                .unwrap();
        let before = doc.to_project_yaml().unwrap();
        let error = doc
            .save_declaration(
                DeclarationKind::Signal,
                Some("approved"),
                json!({"name":"accepted"}),
            )
            .unwrap_err();
        assert!(error.contains("condition"));
        assert_eq!(doc.to_project_yaml().unwrap(), before);
        assert!(
            doc.declaration_uses(DeclarationKind::Signal, "approved")
                .iter()
                .any(|u| u.label.contains("condition"))
        );
        assert!(
            doc.remove_declaration(DeclarationKind::Signal, "approved")
                .is_err()
        );
    }

    #[test]
    fn escalation_topics_rename_typed_handler_arguments() {
        let mut doc = EditorDocument::default();
        doc.save_declaration(DeclarationKind::Signal, None, json!({"name":"resolved"}))
            .unwrap();
        doc.save_declaration(
            DeclarationKind::Escalation,
            None,
            topic("support", "resolved"),
        )
        .unwrap();
        let node = doc
            .add_block(BlockKind::ServiceTask, Position::default())
            .unwrap();
        doc.update_node_yaml(&node, &format!("id: {node}\ntype: ServiceTask\nhandler: create_support_ticket\nargs:\n  topic: {{id_field: support}}\nnext: finish")).unwrap();
        assert!(
            doc.remove_declaration(DeclarationKind::Escalation, "support")
                .is_err()
        );
        doc.save_declaration(
            DeclarationKind::Escalation,
            Some("support"),
            topic("manual_review", "resolved"),
        )
        .unwrap();
        assert_eq!(
            json!(doc.node(&node).unwrap())["args"]["topic"]["id_field"],
            "manual_review"
        );
        doc.definition.validate().unwrap();
    }

    #[test]
    fn invalid_actions_duplicates_and_invalid_identifiers_do_not_mutate_documents() {
        let mut doc = EditorDocument::default();
        doc.save_declaration(DeclarationKind::Signal, None, json!({"name":"resolved"}))
            .unwrap();
        let before = doc.to_project_yaml().unwrap();
        assert!(
            doc.save_declaration(DeclarationKind::Signal, None, json!({"name":"resolved"}))
                .is_err()
        );
        assert!(
            doc.save_declaration(DeclarationKind::Signal, None, json!({"name":"not valid"}))
                .is_err()
        );
        assert!(
            doc.save_declaration(
                DeclarationKind::Escalation,
                None,
                topic("support", "missing")
            )
            .is_err()
        );
        let mut duplicate = topic("support", "resolved");
        let action = duplicate["actions"][0].clone();
        duplicate["actions"].as_array_mut().unwrap().push(action);
        assert!(
            doc.save_declaration(DeclarationKind::Escalation, None, duplicate)
                .is_err()
        );
        assert_eq!(doc.to_project_yaml().unwrap(), before);
    }

    #[test]
    fn declaration_edits_roundtrip_and_share_undo_redo_history() {
        let mut state = EditorState::default();
        let before = state.document.to_project_yaml().unwrap();
        state
            .edit(|doc| {
                doc.save_declaration(
                    DeclarationKind::Form,
                    None,
                    json!({"id":"details","roles":["operator"]}),
                )
            })
            .unwrap();
        let changed = state.document.to_project_yaml().unwrap();
        state.undo();
        assert_eq!(state.document.to_project_yaml().unwrap(), before);
        state.redo();
        assert_eq!(state.document.to_project_yaml().unwrap(), changed);
        let restored = EditorDocument::from_yaml(&changed).unwrap();
        assert_eq!(json!(restored.definition), json!(state.document.definition));
        state
            .edit(|doc| doc.remove_declaration(DeclarationKind::Form, "details"))
            .unwrap();
        assert!(state.document.definition.forms.is_empty());
        state.undo();
        assert_eq!(state.document.definition.forms.len(), 1);
    }

    #[test]
    fn imported_duplicate_declarations_are_rejected_for_stable_card_identity() {
        let mut value = json!(EditorDocument::default().definition);
        value["signals"] = json!(["duplicated", {"name":"duplicated"}]);
        let error = EditorDocument::from_yaml(&serde_yaml::to_string(&value).unwrap()).unwrap_err();
        assert!(error.contains("Duplicate signal"));
    }

    fn topic(id: &str, signal: &str) -> Value {
        json!({"topic":id,"actions":[{"id":"resolve","operator_action":"resolve","label":"Resolve issue","hint":"Resume processing","kind":"Primary","emit_signal":signal}]})
    }
}
