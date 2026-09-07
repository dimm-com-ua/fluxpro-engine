use crate::models::id_field::IdField;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EscalationDef {
    pub topic: IdField,
    #[serde(default)]
    pub actions: Vec<EscalationActionDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationActionDef {
    /// Локальный ID экшна (для UI/логов/аналитики)
    pub id: IdField,

    /// Значение, которое нужно положить в ctx.operator.action
    pub operator_action: IdField,

    /// Лейбл кнопки
    pub label: String,

    /// Подсказка/хинт под кнопкой
    pub hint: String,

    pub kind: SystemActionKind,
    pub emit_signal: Option<IdField>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SystemActionKind {
    Primary,
    Secondary,
    Dangerous,
}
