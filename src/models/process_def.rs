use crate::models::context_map::context_map::ContextMap;
use crate::models::id_field::IdField;
use crate::models::process_def::escalation_def::EscalationDef;
use crate::models::process_def_error::CreateProcessError;
use crate::models::version_id::VersionId;
use anyhow::Error;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_json::json;
use std::collections::HashSet;
use std::fmt::Display;
use uuid::Uuid;

pub mod escalation_def;

/// Корневой объект процесса
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDefinition {
    #[serde(skip)]
    pub uuid: Option<Uuid>,
    pub key: IdField,
    pub name: String,
    pub version: VersionId,

    pub status: ProcessStatus,

    pub effective_from: Option<DateTime<Utc>>,

    #[serde(default)]
    pub deprecated_at: Option<DateTime<Utc>>,

    #[serde(default)]
    pub metadata: ProcessMetadata,

    #[serde(default)]
    pub signals: Vec<SignalDef>,

    #[serde(default)]
    pub forms: Vec<FormDef>,

    #[serde(default)]
    pub stages: Vec<StageDef>,

    pub special_handlers: Option<SpecialHandlers>,

    #[serde(default)]
    pub escalations: Vec<EscalationDef>,

    pub nodes: Vec<Node>,
}

impl ProcessDefinition {
    pub fn compile(&self) -> Result<JsonValue, CreateProcessError> {
        self.validate()?;
        Ok(json!(self))
    }

    pub fn validate(&self) -> Result<(), CreateProcessError> {
        let mut errors = Vec::new();
        let node_ids = collect_unique_ids(
            "node",
            self.nodes.iter().map(|node| node.id().get_id()),
            &mut errors,
        );
        let signals = collect_unique_ids(
            "signal",
            self.signals.iter().map(SignalDef::name),
            &mut errors,
        );
        let forms = collect_unique_ids(
            "form",
            self.forms.iter().map(|form| form.id.get_id()),
            &mut errors,
        );
        let stages = collect_unique_ids(
            "stage",
            self.stages.iter().map(|stage| stage.id().get_id()),
            &mut errors,
        );
        let escalations = collect_unique_ids(
            "escalation topic",
            self.escalations
                .iter()
                .map(|escalation| escalation.topic.get_id()),
            &mut errors,
        );

        let starts = self.nodes.iter().filter(|node| node.is_start()).count();
        if starts != 1 {
            errors.push(format!(
                "process must contain exactly one Start node; found {starts}"
            ));
        }
        if !self.nodes.iter().any(|node| node.is_end()) {
            errors.push("process must contain at least one End node".into());
        }

        let check_node = |source: &str, field: &str, target: &IdField, errors: &mut Vec<String>| {
            if !node_ids.contains(target.get_id()) {
                errors.push(format!(
                    "node '{source}' field '{field}' references missing node '{}'",
                    target.get_id()
                ));
            }
        };
        let check_next = |source: &str, next: &Next, errors: &mut Vec<String>| match next {
            Next::To(target) => check_node(source, "next", target, errors),
            Next::Routes(routes) => {
                if let Some(target) = &routes.default {
                    check_node(source, "next.default", target, errors);
                }
                for (index, branch) in routes.branches.iter().enumerate() {
                    check_node(
                        source,
                        &format!("next.branches[{index}].next"),
                        &branch.next,
                        errors,
                    );
                }
            }
        };
        let check_stage = |node: &Node, errors: &mut Vec<String>| {
            if let Some(stage) = node.set_stage() {
                let stage = stage.stage_with_reason().stage;
                if !stages.contains(stage.get_id()) {
                    errors.push(format!(
                        "node '{}' references undeclared stage '{}'",
                        node.id(),
                        stage
                    ));
                }
            }
        };

        for node in &self.nodes {
            let source = node.id().get_id();
            check_stage(node, &mut errors);
            match node {
                Node::Start { next, .. } => check_node(source, "next", next, &mut errors),
                Node::End { .. } => {}
                Node::ServiceTask {
                    handler,
                    args,
                    next,
                    on_error,
                    retries,
                    ..
                } => {
                    if let Some(next) = next {
                        check_next(source, next, &mut errors);
                    }
                    check_on_error(source, on_error.as_ref(), &check_node, &mut errors);
                    if let Some(retries) = retries {
                        if retries.max == 0 {
                            errors.push(format!(
                                "node '{source}' retries.max must be greater than zero"
                            ));
                        }
                        if retries.next_attempt_at().is_err() {
                            errors.push(format!(
                                "node '{source}' has invalid retries.backoff '{}'",
                                retries.backoff
                            ));
                        }
                    }
                    if handler.get_id() == "create_support_ticket" {
                        let topic_key = IdField::new("topic").expect("static topic ID is valid");
                        let topic = args
                            .as_ref()
                            .and_then(|args| args.as_id_field(&topic_key))
                            .map(|topic| topic.get_id().to_owned());
                        match topic {
                            Some(topic) if escalations.contains(topic.as_str()) => {}
                            Some(topic) => errors.push(format!(
                                "node '{source}' references undeclared escalation topic '{topic}'"
                            )),
                            None => errors.push(format!(
                                "node '{source}' handler 'create_support_ticket' requires args.topic"
                            )),
                        }
                    }
                }
                Node::UserTask {
                    form,
                    wait_for,
                    timeout,
                    next,
                    on_error,
                    ..
                } => {
                    if !forms.contains(form.get_id()) {
                        errors.push(format!(
                            "node '{source}' references undeclared form '{form}'"
                        ));
                    }
                    if let Some(wait_for) = wait_for {
                        for signal in wait_for.signals() {
                            if !signals.contains(signal.get_id()) {
                                errors.push(format!(
                                    "node '{source}' waits for undeclared signal '{signal}'"
                                ));
                            }
                        }
                    }
                    if let Some(timeout) = timeout {
                        check_node(
                            source,
                            "timeout.on_timeout",
                            &timeout.on_timeout,
                            &mut errors,
                        );
                    }
                    if let Some(next) = next {
                        check_next(source, next, &mut errors);
                    }
                    check_on_error(source, on_error.as_ref(), &check_node, &mut errors);
                }
                Node::Wait {
                    wait_for,
                    timeout,
                    next,
                    ..
                } => {
                    for signal in wait_for.signals() {
                        if !signals.contains(signal.get_id()) {
                            errors.push(format!(
                                "node '{source}' waits for undeclared signal '{signal}'"
                            ));
                        }
                    }
                    if let Some(timeout) = timeout {
                        check_node(
                            source,
                            "timeout.on_timeout",
                            &timeout.on_timeout,
                            &mut errors,
                        );
                    }
                    if let Some(next) = next {
                        check_next(source, next, &mut errors);
                    }
                }
                Node::Gateway { branches, next, .. } => {
                    for (index, branch) in branches.iter().enumerate() {
                        check_node(
                            source,
                            &format!("branches[{index}].next"),
                            &branch.next,
                            &mut errors,
                        );
                    }
                    if let Some(next) = next {
                        check_node(source, "next", next, &mut errors);
                    }
                }
            }
        }

        for escalation in &self.escalations {
            let mut action_ids = HashSet::new();
            for action in &escalation.actions {
                if !action_ids.insert(action.id.get_id()) {
                    errors.push(format!(
                        "escalation '{}' contains duplicate action id '{}'",
                        escalation.topic, action.id
                    ));
                }
                if let Some(signal) = &action.emit_signal
                    && !signals.contains(signal.get_id())
                {
                    errors.push(format!(
                        "escalation '{}' action '{}' emits undeclared signal '{}'",
                        escalation.topic, action.id, signal
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(CreateProcessError::ValidationError(errors))
        }
    }
}

fn collect_unique_ids<'a>(
    kind: &str,
    values: impl Iterator<Item = &'a str>,
    errors: &mut Vec<String>,
) -> HashSet<&'a str> {
    let mut result = HashSet::new();
    for value in values {
        if !result.insert(value) {
            errors.push(format!("duplicate {kind} id '{value}'"));
        }
    }
    result
}

fn check_on_error(
    source: &str,
    on_error: Option<&OnError>,
    check_node: &impl Fn(&str, &str, &IdField, &mut Vec<String>),
    errors: &mut Vec<String>,
) {
    if let Some(on_error) = on_error {
        if let Some(next) = &on_error.next {
            check_node(source, "on_error.next", next, errors);
        }
        if let Some(compensate) = &on_error.compensate {
            check_node(source, "on_error.compensate", compensate, errors);
        }
    }
}

#[cfg(all(test, feature = "api"))]
mod validation_tests {
    use super::*;

    const TK_ONLINE_DEFINITION: &str = include_str!("../../tests/fixtures/tk_online.yaml");
    const CASH_LOAN_DEFINITION: &str = include_str!("../../tests/fixtures/cash_loan.yaml");

    #[test]
    fn a_process_definition_is_valid() {
        let definition: ProcessDefinition = serde_yaml::from_str(TK_ONLINE_DEFINITION).unwrap();

        definition.validate().unwrap();
    }

    #[test]
    fn a_user_task_can_start_a_timeout_after_a_previous_signal() {
        let definition: ProcessDefinition = serde_yaml::from_str(TK_ONLINE_DEFINITION).unwrap();

        let open_node = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "credit_info_open")
            .unwrap();
        let Node::UserTask {
            wait_for: Some(wait_for),
            timeout: None,
            next: Some(Next::To(next)),
            ..
        } = open_node
        else {
            panic!("credit_info_open must wait for page opening without a timeout");
        };
        assert_eq!(wait_for.signals()[0].get_id(), "credit_info_opened");
        assert_eq!(next.get_id(), "credit_info");

        let timer_node = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "credit_info")
            .unwrap();
        let Node::UserTask {
            wait_for: Some(wait_for),
            timeout: Some(timeout),
            ..
        } = timer_node
        else {
            panic!("credit_info must be the timed credit-info task");
        };
        assert_eq!(wait_for.signals()[0].get_id(), "terms_viewed");
        assert_eq!(timeout.after.as_deref(), Some("PT3M"));
    }

    #[test]
    fn a_wait_node_is_valid() {
        let definition: ProcessDefinition = serde_yaml::from_str(CASH_LOAN_DEFINITION).unwrap();

        definition.validate().unwrap();
        let wait = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "final_terms_after_reminder")
            .unwrap();
        assert!(matches!(wait, Node::Wait { .. }));
    }

    #[test]
    fn a_user_task_can_route_on_multiple_signals_before_a_timeout() {
        let definition: ProcessDefinition = serde_yaml::from_str(CASH_LOAN_DEFINITION).unwrap();

        let open_node = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "select_terms_open")
            .unwrap();
        let Node::UserTask {
            wait_for: Some(wait_for),
            timeout: None,
            next: Some(Next::To(next)),
            ..
        } = open_node
        else {
            panic!("select_terms_open must wait for calculator signals without a timeout");
        };
        assert_eq!(
            wait_for
                .signals()
                .iter()
                .map(|signal| signal.get_id())
                .collect::<Vec<_>>(),
            vec!["calculator_interacted", "terms_selected"]
        );
        assert_eq!(next.get_id(), "select_terms_open_route");

        let route_node = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "select_terms_open_route")
            .unwrap();
        let Node::Gateway { branches, next, .. } = route_node else {
            panic!("select_terms_open_route must route calculator signals");
        };
        assert_eq!(branches.len(), 2);
        assert_eq!(
            branches[0].when,
            "ctx._last_signal == 'calculator_interacted'"
        );
        assert_eq!(branches[0].next.get_id(), "select_terms");
        assert_eq!(branches[1].when, "ctx._last_signal == 'terms_selected'");
        assert_eq!(branches[1].next.get_id(), "close_ticket_terms");
        assert_eq!(next.as_ref().map(IdField::get_id), Some("finalize_reject"));

        let timer_node = definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == "select_terms")
            .unwrap();
        let Node::UserTask {
            wait_for: Some(wait_for),
            timeout: Some(timeout),
            ..
        } = timer_node
        else {
            panic!("select_terms must be the timed calculator task");
        };
        assert_eq!(wait_for.signals()[0].get_id(), "terms_selected");
        assert_eq!(timeout.after.as_deref(), Some("PT1M"));
    }

    #[test]
    fn validation_reports_all_broken_references() {
        let definition: ProcessDefinition = serde_yaml::from_str(
            r#"
key: invalid_process
name: Invalid process
version: 1.0.0
status: active
effective_from: 2026-01-01T00:00:00Z
signals: [known_signal]
forms:
  - id: known_form
    roles: [client]
stages: [known_stage]
nodes:
  - id: start
    type: Start
    next: missing_node
    set_stage: missing_stage
  - id: task
    type: UserTask
    form: missing_form
    wait_for:
      signals: [missing_signal]
    next: also_missing
    set_stage: null
    timeout: null
    on_error: null
"#,
        )
        .unwrap();

        let CreateProcessError::ValidationError(errors) = definition.validate().unwrap_err() else {
            panic!("expected validation errors");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.contains("at least one End"))
        );
        assert!(errors.iter().any(|error| error.contains("missing_node")));
        assert!(errors.iter().any(|error| error.contains("missing_stage")));
        assert!(errors.iter().any(|error| error.contains("missing_form")));
        assert!(errors.iter().any(|error| error.contains("missing_signal")));
        assert!(errors.iter().any(|error| error.contains("also_missing")));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    Draft,
    Active,
    Deprecated,
}

impl Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessMetadata {
    pub owner: Option<String>,
    /// ISO‑8601 duration, например "PT30M"
    pub sla: Option<String>,
    /// Human-readable description of the changes introduced by this version.
    pub comment: Option<String>,
}

/// Сигнал — либо строка с именем, либо объект { name }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SignalDef {
    Name(IdField),
    Obj { name: IdField },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormDef {
    pub id: IdField,
    pub roles: Vec<IdField>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StageDef {
    Obj {
        id: IdField,
        name: String,
        #[serde(default)]
        is_initial: Option<bool>,
        #[serde(default)]
        is_final: Option<bool>,
    },
    Name(IdField),
}

impl StageDef {
    pub fn id(&self) -> &IdField {
        match self {
            StageDef::Name(s) => s,
            StageDef::Obj { id, .. } => id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            StageDef::Name(s) => s.get_id(),
            StageDef::Obj { name, .. } => name.as_str(),
        }
    }

    pub fn is_initial(&self) -> bool {
        match self {
            StageDef::Obj { is_initial, .. } => is_initial.unwrap_or(false),
            StageDef::Name(_) => false,
        }
    }
}

impl SignalDef {
    pub fn name(&self) -> &str {
        match self {
            SignalDef::Name(s) => s.get_id(),
            SignalDef::Obj { name } => name.get_id(),
        }
    }

    pub fn id(&self) -> &IdField {
        match self {
            SignalDef::Name(s) => s,
            SignalDef::Obj { name } => name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArgument {
    pub name: IdField,
    pub value: String,
}

/// Узлы процесса — полиморфизм по полю `type`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    Start {
        id: IdField,
        next: IdField,
        set_stage: Option<SetStage>,
    },

    End {
        id: IdField,
        set_stage: Option<SetStage>,
    },

    /// Задача, выполняемая сервисом/бэкендом
    ServiceTask {
        id: IdField,
        handler: IdField,
        #[serde(default)]
        args: Option<ContextMap>,
        set_stage: Option<SetStage>,
        #[serde(default)]
        retries: Option<Retries>,
        /// Переход может быть строкой (id узла) или структурой с ветками
        #[serde(default)]
        next: Option<Next>,
        #[serde(default)]
        on_error: Option<OnError>,
    },

    /// Пользовательская задача: ждём сигнал(ы)
    UserTask {
        id: IdField,
        form: IdField,
        /// Поддерживаем как один сигнал, так и несколько
        #[serde(default)]
        wait_for: Option<WaitFor>,
        set_stage: Option<SetStage>,
        /// Таймаут узла (после которого переходим в on_timeout)
        #[serde(rename = "timeout")]
        #[serde(default)]
        timeout: Option<NodeTimeout>,

        /// Куда идём ПОСЛЕ завершения узла (обычно роутер/gateway)
        #[serde(default)]
        next: Option<Next>,

        #[serde(default)]
        on_error: Option<OnError>,
    },

    /// Technical wait state. Unlike `UserTask`, it does not expose a form and
    /// does not invoke user-interface lifecycle handlers.
    Wait {
        id: IdField,
        wait_for: WaitFor,
        #[serde(default)]
        timeout: Option<NodeTimeout>,
        #[serde(default)]
        next: Option<Next>,
        set_stage: Option<SetStage>,
    },

    /// Ветвление
    Gateway {
        id: IdField,
        set_stage: Option<SetStage>,
        gateway: GatewayKind,
        #[serde(default)]
        branches: Vec<Branch>,
        /// Для XOR — запасной путь
        #[serde(default)]
        next: Option<IdField>,
    },
}

impl Node {
    pub fn id(&self) -> &IdField {
        match self {
            Node::Start { id, .. } => id,
            Node::End { id, .. } => id,
            Node::ServiceTask { id, .. } => id,
            Node::UserTask { id, .. } => id,
            Node::Wait { id, .. } => id,
            Node::Gateway { id, .. } => id,
        }
    }
    pub fn is_start(&self) -> bool {
        matches!(self, Node::Start { .. })
    }
    pub fn is_end(&self) -> bool {
        matches!(self, Node::End { .. })
    }
    pub fn is_service_task(&self) -> bool {
        matches!(self, Node::ServiceTask { .. })
    }
    pub fn is_user_task(&self) -> bool {
        matches!(self, Node::UserTask { .. })
    }
    pub fn set_stage(&self) -> Option<&SetStage> {
        match self {
            Node::Start { set_stage, .. } => Option::from(set_stage),
            Node::End { set_stage, .. } => Option::from(set_stage),
            Node::ServiceTask { set_stage, .. } => Option::from(set_stage),
            Node::UserTask { set_stage, .. } => Option::from(set_stage),
            Node::Wait { set_stage, .. } => Option::from(set_stage),
            Node::Gateway { set_stage, .. } => Option::from(set_stage),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SetStage {
    Stage(IdField),
    StageWithReason(SetStageWithReason),
}

impl SetStage {
    pub fn stage_with_reason(&self) -> SetStageWithReason {
        match self {
            SetStage::Stage(s) => SetStageWithReason {
                stage: s.clone(),
                reason: None,
            },
            SetStage::StageWithReason(s) => SetStageWithReason {
                stage: s.stage.clone(),
                reason: s.reason.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetStageWithReason {
    pub stage: IdField,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GatewayKind {
    XOR,
    AND,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// CEL‑выражение
    pub when: String,
    pub next: IdField,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retries {
    pub max: u32,
    /// ISO‑8601 duration: "PT10S"
    pub backoff: String,
}

impl Retries {
    pub fn next_attempt_at(&self) -> anyhow::Result<DateTime<Utc>> {
        let parsed = self
            .backoff
            .parse::<iso8601::Duration>()
            .map_err(|_| Error::msg(format!("Couldn't parse retry backoff {}", self.backoff)))?;
        let duration = std::time::Duration::from(parsed);
        Ok(Utc::now() + Duration::from_std(duration)?)
    }
}

/// Переход для ServiceTask: строка или объект с ветками
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Next {
    /// Простое «далее»
    To(IdField),
    /// Ветвящийся «далее»
    Routes(NextRoutes),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextRoutes {
    #[serde(default)]
    pub default: Option<IdField>,
    #[serde(default)]
    pub branches: Vec<Branch>,
}

/// Ожидание сигнала: один или несколько
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WaitFor {
    Single { signal: IdField },
    Multi { signals: Vec<IdField> },
}

impl WaitFor {
    pub fn signals(&self) -> Vec<IdField> {
        match self {
            WaitFor::Single { signal } => {
                vec![signal.clone()]
            }
            WaitFor::Multi { signals } => signals.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTimeout {
    /// Любая из спецификаций таймера
    #[serde(default)]
    pub after: Option<String>, // "PT15M"
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    // #[serde(default)]
    // pub cron: Option<String>,
    /// Куда перейти при срабатывании таймаута
    pub on_timeout: IdField,
}

impl NodeTimeout {
    pub fn after_now(&self) -> anyhow::Result<DateTime<Utc>> {
        if let Some(at) = self.at {
            return Ok(at);
        };
        if let Some(after) = &self.after {
            if let Ok(parsed) = after.parse::<iso8601::Duration>() {
                let duration = std::time::Duration::from(parsed);
                let second = duration.as_secs();
                return Ok(Utc::now() + Duration::seconds(second as i64));
            }
        }
        Err(Error::msg("Couldn't parse after"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnError {
    /// Простой переход на узел‑обработчик
    #[serde(default)]
    pub next: Option<IdField>,
    /// Или компенсирующее действие (на будущее, можно не использовать)
    #[serde(default)]
    pub compensate: Option<IdField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskHandler {
    Simple(IdField),
    Extended(TaskHandlerDescription),
}

impl TaskHandler {
    pub fn handler(&self) -> IdField {
        match self {
            TaskHandler::Simple(h) => h.clone(),
            TaskHandler::Extended(h) => h.handler.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHandlerDescription {
    pub handler: IdField,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialHandlers {
    pub on_stage_change: Option<TaskHandler>,
    pub on_show_form: Option<TaskHandler>,
    pub on_hide_form: Option<TaskHandler>,
    pub on_process_complete: Option<TaskHandler>,
}
