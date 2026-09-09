//! Escalation topics and operator actions consumed by host applications.

use crate::models::id_field::IdField;
use serde::{Deserialize, Serialize};

/// An escalation topic with actions interpreted by the host application.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EscalationDef {
    /// Escalation topic identifier referenced by host integrations.
    pub topic: IdField,
    /// Operator actions declared for this escalation topic.
    #[serde(default)]
    pub actions: Vec<EscalationActionDef>,
}

/// A host-rendered operator action and optional signal declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationActionDef {
    /// Action ID unique within its escalation topic, used by the UI and logs.
    pub id: IdField,

    /// Action value for the host application to place in `ctx.operator_action`.
    pub operator_action: IdField,

    /// Display label for the action button.
    pub label: String,

    /// Help text displayed with the action button.
    pub hint: String,

    /// Button presentation intent for the host UI.
    pub kind: SystemActionKind,
    /// Optional declared signal for the host to emit when the action is chosen.
    pub emit_signal: Option<IdField>,
}

/// Presentation intent for an escalation action; no engine authorization semantics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SystemActionKind {
    /// Main operator action.
    Primary,
    /// Alternative operator action.
    Secondary,
    /// Action styled by the host as destructive or sensitive.
    Dangerous,
}
