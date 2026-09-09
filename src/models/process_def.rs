//! Workflow schema and structural validation for JSON and YAML definitions.

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

/// A versioned workflow definition with declarations and executable nodes.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::process_def::ProcessDefinition;
///
/// let definition: ProcessDefinition = serde_json::from_value(serde_json::json!({
///     "key": "demo", "name": "Demo", "version": "1.0.0", "status": "active",
///     "effective_from": "2026-01-01T00:00:00Z",
///     "stages": [{ "id": "created", "name": "Created", "is_initial": true }],
///     "nodes": [
///         { "id": "start", "type": "Start", "next": "done" },
///         { "id": "done", "type": "End" }
///     ]
/// })).unwrap();
/// assert!(definition.validate().is_ok());
/// assert_eq!(definition.compile().unwrap()["nodes"][0]["type"], "Start");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDefinition {
    /// Database identity; skipped in JSON/YAML and assigned after persistence.
    #[serde(skip)]
    pub uuid: Option<Uuid>,
    /// Logical process key shared by all versions of this workflow.
    pub key: IdField,
    /// Human-readable process name.
    pub name: String,
    /// Normalized process definition version.
    pub version: VersionId,

    /// Only active definitions within their effective dates may start new instances.
    pub status: ProcessStatus,

    /// UTC time from which this definition can be selected.
    pub effective_from: Option<DateTime<Utc>>,

    /// UTC time after which this definition is excluded from runtime selection.
    #[serde(default)]
    pub deprecated_at: Option<DateTime<Utc>>,

    /// Optional ownership, SLA, and version notes.
    #[serde(default)]
    pub metadata: ProcessMetadata,

    /// Signal declarations available to the workflow.
    #[serde(default)]
    pub signals: Vec<SignalDef>,

    /// Forms declared for user-task lifecycle hooks.
    #[serde(default)]
    pub forms: Vec<FormDef>,

    /// Business stages; runtime startup requires an initial stage.
    #[serde(default)]
    pub stages: Vec<StageDef>,

    /// Optional callbacks invoked during stage, form, and completion events.
    pub special_handlers: Option<SpecialHandlers>,

    /// Operator escalation topics and available actions.
    #[serde(default)]
    pub escalations: Vec<EscalationDef>,

    /// Workflow graph; exactly one Start and at least one End are required.
    pub nodes: Vec<Node>,
}

impl ProcessDefinition {
    /// Validates references and serializes this definition to JSON.
    ///
    /// # Errors
    ///
    /// Returns structural failures and, with `runtime`, condition compilation errors.
    pub fn compile(&self) -> Result<JsonValue, CreateProcessError> {
        self.validate()?;
        Ok(json!(self))
    }

    /// Checks unique IDs, start/end presence, references, and service retry settings.
    ///
    /// # Errors
    ///
    /// Returns accumulated validation messages, including initial stages and timers.
    /// With `runtime`, also compiles conditions. Handler registration is host-owned.
    /// Use `unreachable_nodes` for reachability diagnostics.
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

        let initial = self
            .stages
            .iter()
            .filter(|stage| stage.is_initial())
            .count();
        if initial != 1 {
            errors.push(format!(
                "process must contain exactly one initial stage; found {initial}"
            ));
        }
        if let (Some(from), Some(until)) = (self.effective_from, self.deprecated_at) {
            if until <= from {
                errors.push("deprecated_at must be later than effective_from".into());
            }
        }
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
            if let Node::Wait { timeout, .. } | Node::UserTask { timeout, .. } = node {
                if let Some(timer) = timeout {
                    if let Err(error) = timer.after_now() {
                        errors.push(format!("node '{source}' has invalid timeout: {error}"));
                    }
                }
            }
            let wait = match node {
                Node::Wait { wait_for, .. } => Some(wait_for),
                Node::UserTask { wait_for, .. } => wait_for.as_ref(),
                _ => None,
            };
            if wait.is_some_and(|wait| wait.signals().is_empty()) {
                errors.push(format!(
                    "node '{source}' wait_for.signals must not be empty"
                ));
            }
            #[cfg(feature = "runtime")]
            for (index, branch) in node_branches(node).iter().enumerate() {
                if let Err(error) = crate::service::expressions::validate_condition(&branch.when) {
                    errors.push(format!("node '{source}' branch {index}: {error}"));
                }
            }
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

    /// Lists nodes with no structural path from Start, in definition order.
    ///
    /// This is diagnostic rather than a registration error because hosts may
    /// explicitly enqueue auxiliary nodes. Conditions are not evaluated.
    pub fn unreachable_nodes(&self) -> Vec<IdField> {
        let mut reached = HashSet::new();
        let mut pending: Vec<&Node> = self.nodes.iter().filter(|node| node.is_start()).collect();
        while let Some(node) = pending.pop() {
            if !reached.insert(node.id()) {
                continue;
            }
            for target in node_targets(node) {
                if let Some(next) = self.nodes.iter().find(|node| node.id() == target) {
                    pending.push(next);
                }
            }
        }
        self.nodes
            .iter()
            .filter(|node| !reached.contains(node.id()))
            .map(|node| node.id().clone())
            .collect()
    }
}

fn node_branches(node: &Node) -> &[Branch] {
    match node {
        Node::Gateway { branches, .. } => branches,
        Node::ServiceTask {
            next: Some(Next::Routes(routes)),
            ..
        }
        | Node::UserTask {
            next: Some(Next::Routes(routes)),
            ..
        }
        | Node::Wait {
            next: Some(Next::Routes(routes)),
            ..
        } => &routes.branches,
        _ => &[],
    }
}

fn node_targets(node: &Node) -> Vec<&IdField> {
    let mut targets: Vec<&IdField> = node_branches(node).iter().map(|b| &b.next).collect();
    match node {
        Node::Start { next, .. } => targets.push(next),
        Node::Gateway { next, .. } => targets.extend(next),
        Node::ServiceTask { next, .. } | Node::Wait { next, .. } | Node::UserTask { next, .. } => {
            match next {
                Some(Next::To(target)) => targets.push(target),
                Some(Next::Routes(routes)) => targets.extend(&routes.default),
                None => {}
            }
        }
        Node::End { .. } => {}
    }
    match node {
        Node::Wait {
            timeout: Some(timer),
            ..
        }
        | Node::UserTask {
            timeout: Some(timer),
            ..
        } => targets.push(&timer.on_timeout),
        _ => {}
    }
    if let Node::ServiceTask {
        on_error: Some(error),
        ..
    } = node
    {
        targets.extend(&error.next);
    }
    targets
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
            r#"ctx["_last_signal"] == "calculator_interacted""#
        );
        assert_eq!(branches[0].next.get_id(), "select_terms");
        assert_eq!(
            branches[1].when,
            r#"ctx["_last_signal"] == "terms_selected""#
        );
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

/// Lifecycle metadata serialized as `draft`, `active`, or `deprecated`.
///
/// Runtime starts only active versions within their effective dates. Existing
/// instances remain bound to their original version after deprecation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    /// Definition marked as a draft.
    Draft,
    /// Definition marked as active.
    Active,
    /// Definition marked as deprecated.
    Deprecated,
}

impl Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Optional ownership, service-level target, and version notes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessMetadata {
    /// Optional team or person responsible for the definition.
    pub owner: Option<String>,
    /// Descriptive ISO 8601 service-level target, such as `PT30M`; not enforced by the runtime.
    pub sla: Option<String>,
    /// Human-readable description of the changes introduced by this version.
    pub comment: Option<String>,
}

/// A signal declaration serialized as a name or an object with a `name` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SignalDef {
    /// Compact declaration containing only the identifier.
    Name(IdField),
    /// Expanded declaration with named fields.
    Obj {
        /// Signal identifier declared by this object.
        name: IdField,
    },
}

/// A host-rendered form and the roles supplied to its display hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormDef {
    /// Unique identifier within this declaration category.
    pub id: IdField,
    /// Role identifiers passed to the form-display hook.
    pub roles: Vec<IdField>,
}

/// A named business stage, optionally carrying initial and final markers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StageDef {
    /// Expanded declaration with named fields.
    Obj {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Human-readable business-stage name.
        name: String,
        /// Marks the stage selected during startup; omitted means `false`.
        #[serde(default)]
        is_initial: Option<bool>,
        /// Marks a business-final stage; omitted means `false`.
        #[serde(default)]
        is_final: Option<bool>,
    },
    /// Compact declaration containing only the identifier.
    Name(IdField),
}

impl StageDef {
    /// Returns the declared identifier.
    pub fn id(&self) -> &IdField {
        match self {
            StageDef::Name(s) => s,
            StageDef::Obj { id, .. } => id,
        }
    }

    /// Returns the stage display name, falling back to its ID in compact declarations.
    pub fn name(&self) -> &str {
        match self {
            StageDef::Name(s) => s.get_id(),
            StageDef::Obj { name, .. } => name.as_str(),
        }
    }

    /// Returns the initial-stage marker, defaulting to `false` for a plain stage ID.
    pub fn is_initial(&self) -> bool {
        match self {
            StageDef::Obj { is_initial, .. } => is_initial.unwrap_or(false),
            StageDef::Name(_) => false,
        }
    }
}

impl SignalDef {
    /// Returns the declared signal name from either representation.
    pub fn name(&self) -> &str {
        match self {
            SignalDef::Name(s) => s.get_id(),
            SignalDef::Obj { name } => name.get_id(),
        }
    }

    /// Returns the declared identifier.
    pub fn id(&self) -> &IdField {
        match self {
            SignalDef::Name(s) => s,
            SignalDef::Obj { name } => name,
        }
    }
}

/// A named string argument model; node arguments use `ContextMap` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArgument {
    /// Argument identifier.
    pub name: IdField,
    /// String payload of the argument.
    pub value: String,
}

/// An executable workflow node selected by the case-sensitive `type` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Node {
    /// The unique process entry node; immediately enqueues its successor.
    Start {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Direct successor node ID.
        next: IdField,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
    },

    /// Terminal node; invokes the completion hook and enqueues no successor.
    End {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
    },

    /// Invokes a registered backend handler, then applies its outcome.
    ServiceTask {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Registry key of the host handler to invoke.
        handler: IdField,
        /// Typed arguments passed unchanged to the registered handler.
        #[serde(default)]
        args: Option<ContextMap>,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
        /// Optional fixed-delay retry policy for failed service-task attempts.
        #[serde(default)]
        retries: Option<Retries>,
        /// Optional successor ID or ordered conditional routes after success.
        #[serde(default)]
        next: Option<Next>,
        /// Error routing metadata; only service-task `next` is currently executed.
        #[serde(default)]
        on_error: Option<OnError>,
    },

    /// Exposes a form through lifecycle hooks and waits for an accepted signal or timeout.
    UserTask {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Declared form identifier passed to the form lifecycle hooks.
        form: IdField,
        /// Signals accepted by this node; any one listed signal can complete the wait.
        #[serde(default)]
        wait_for: Option<WaitFor>,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
        /// Optional deadline whose event routes to `on_timeout` while this node is current.
        #[serde(rename = "timeout")]
        #[serde(default)]
        timeout: Option<NodeTimeout>,

        /// Successor selected after an accepted signal updates the context.
        #[serde(default)]
        next: Option<Next>,

        /// Reserved error routing metadata; parsed and validated but not executed.
        #[serde(default)]
        on_error: Option<OnError>,
    },

    /// Technical wait state. Unlike `UserTask`, it does not expose a form and
    /// does not invoke user-interface lifecycle handlers.
    Wait {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Signal names accepted by this wait; any one completes it.
        wait_for: WaitFor,
        /// Optional node deadline and the target to enqueue on expiration.
        #[serde(default)]
        timeout: Option<NodeTimeout>,
        /// Optional direct successor or XOR routes after an accepted signal.
        #[serde(default)]
        next: Option<Next>,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
    },

    /// Selects one successor according to the configured gateway rule.
    Gateway {
        /// Unique identifier within this declaration category.
        id: IdField,
        /// Optional business-stage assignment applied before this node executes.
        set_stage: Option<SetStage>,
        /// Branch selection rule; retain explicit `gateway: XOR` in definitions.
        gateway: GatewayKind,
        /// Conditions evaluated in declaration order.
        #[serde(default)]
        branches: Vec<Branch>,
        /// Fallback target when no branch matches the gateway rule.
        #[serde(default)]
        next: Option<IdField>,
    },
}

impl Node {
    /// Returns the declared identifier.
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
    /// Returns whether this is the process entry node.
    pub fn is_start(&self) -> bool {
        matches!(self, Node::Start { .. })
    }
    /// Returns whether this is a terminal node.
    pub fn is_end(&self) -> bool {
        matches!(self, Node::End { .. })
    }
    /// Returns whether this node invokes a backend handler.
    pub fn is_service_task(&self) -> bool {
        matches!(self, Node::ServiceTask { .. })
    }
    /// Returns whether this node exposes a form.
    pub fn is_user_task(&self) -> bool {
        matches!(self, Node::UserTask { .. })
    }
    /// Returns the optional stage assignment applied on node entry.
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

/// A stage assignment serialized as an ID or `{ stage, reason }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SetStage {
    /// Assigns a stage without a reason.
    Stage(IdField),
    /// Assigns a stage with a reason for the lifecycle hook.
    StageWithReason(SetStageWithReason),
}

impl SetStage {
    /// Normalizes either stage representation into an ID and optional reason.
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

/// Stage assignment passed to the stage-change lifecycle handler.
///
/// Queued execution commits the stage and reason atomically with the transition.
/// The low-level `assign_stage` helper assigns a stage without a reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetStageWithReason {
    /// Declared business-stage identifier.
    pub stage: IdField,
    /// Optional explanation associated with this stage assignment.
    pub reason: Option<String>,
}

/// Branch selection rule for an exclusive gateway.
///
/// `XOR` selects the first true condition. The explicit `gateway` field and its
/// serialized value remain unchanged for existing process definitions.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::process_def::{GatewayKind, Node};
///
/// let node: Node = serde_json::from_value(serde_json::json!({
///     "id": "decision", "type": "Gateway", "gateway": "XOR",
///     "branches": [{ "when": "ctx.approved", "next": "accepted" }],
///     "next": "rejected"
/// })).unwrap();
/// assert!(matches!(node, Node::Gateway { gateway: GatewayKind::XOR, .. }));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GatewayKind {
    /// Selects the first branch evaluating to true, then the fallback.
    XOR,
}

/// A condition and the target selected when it matches the gateway rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Rhai boolean expression evaluated with typed context values exposed as `ctx`.
    pub when: String,
    /// Node ID selected when this branch matches the gateway rule.
    pub next: IdField,
}

/// Fixed-delay retry policy for failed service-task queue attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retries {
    /// Maximum retries after the initial attempt; must be greater than zero.
    pub max: u32,
    /// Fixed ISO 8601 delay between failed attempts, such as `PT10S`.
    pub backoff: String,
}

impl Retries {
    /// Returns the current UTC time plus the configured fixed backoff.
    ///
    /// # Errors
    ///
    /// Returns an error if the duration cannot be parsed or represented by Chrono.
    pub fn next_attempt_at(&self) -> anyhow::Result<DateTime<Utc>> {
        relative_deadline(&self.backoff)
    }
}

/// A successor for service tasks and signal waits: an ID or conditional routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Next {
    /// Routes directly to the specified node.
    To(IdField),
    /// Selects the first true branch, then falls back to the default target.
    Routes(NextRoutes),
}

/// Ordered XOR branches with an optional fallback target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextRoutes {
    /// Fallback node when no conditional branch is selected.
    #[serde(default)]
    pub default: Option<IdField>,
    /// Conditions evaluated in declaration order.
    #[serde(default)]
    pub branches: Vec<Branch>,
}

/// A wait condition accepting any one declared signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WaitFor {
    /// Accepts the single named signal.
    Single {
        /// Signal name accepted by a waiting node.
        signal: IdField,
    },
    /// Accepts any one signal in the list; does not wait for all signals.
    Multi {
        /// Accepted signal names; the first matching delivery completes the wait.
        signals: Vec<IdField>,
    },
}

impl WaitFor {
    /// Returns all accepted signal IDs; any one can complete the wait.
    pub fn signals(&self) -> Vec<IdField> {
        match self {
            WaitFor::Single { signal } => {
                vec![signal.clone()]
            }
            WaitFor::Multi { signals } => signals.clone(),
        }
    }
}

/// A deadline for a user task or technical wait.
///
/// An absolute `at` takes precedence over `after`. The event only routes onward
/// if its originating node is still current. Cron schedules are not supported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTimeout {
    /// Relative ISO 8601 delay from node entry, such as `PT15M`.
    #[serde(default)]
    pub after: Option<String>,
    /// Absolute UTC deadline, taking precedence over `after` when both are supplied.
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    /// Node ID to enqueue when the deadline fires; this is not a signal name.
    pub on_timeout: IdField,
}

impl NodeTimeout {
    /// Resolves the deadline, preferring `at` over a delay measured from now.
    ///
    /// # Errors
    ///
    /// Returns an error if neither a timestamp nor a parseable duration is present.
    pub fn after_now(&self) -> anyhow::Result<DateTime<Utc>> {
        if let Some(at) = self.at {
            return Ok(at);
        };
        if let Some(after) = &self.after {
            return relative_deadline(after);
        }

        Err(Error::msg("Couldn't parse after"))
    }
}

// The ISO parser accepts valid prefixes, so require a complete duration first.
fn relative_deadline(text: &str) -> anyhow::Result<DateTime<Utc>> {
    let syntax = regex::Regex::new(r"^P(?:[0-9]+W|(?:[0-9]+Y)?(?:[0-9]+M)?(?:[0-9]+D)?(?:T(?:[0-9]+H)?(?:[0-9]+M)?(?:[0-9]+(?:[.,][0-9]{1,3})?S)?)?)$").expect("duration regex");
    if !syntax.is_match(text) {
        return Err(Error::msg("invalid ISO 8601 duration"));
    }
    let parsed = text
        .parse::<iso8601::Duration>()
        .map_err(|e| Error::msg(e))?;
    let delay = Duration::from_std(std::time::Duration::from(parsed))?;
    if delay <= Duration::zero() {
        return Err(Error::msg("duration must be positive"));
    }
    Utc::now()
        .checked_add_signed(delay)
        .ok_or_else(|| Error::msg("duration is out of range"))
}

/// Error routing metadata; only `ServiceTask.on_error.next` is executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnError {
    /// Target after service-task retries are exhausted; unused for user tasks.
    #[serde(default)]
    pub next: Option<IdField>,
    /// Reserved compensation target; validated as a node reference but not executed.
    #[serde(default)]
    pub compensate: Option<IdField>,
}

/// Lifecycle handler reference serialized as an ID or `{ handler }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskHandler {
    /// Handler reference containing only its registry ID.
    Simple(IdField),
    /// Handler reference using the object representation.
    Extended(TaskHandlerDescription),
}

impl TaskHandler {
    /// Returns the handler ID from either reference representation.
    pub fn handler(&self) -> IdField {
        match self {
            TaskHandler::Simple(h) => h.clone(),
            TaskHandler::Extended(h) => h.handler.clone(),
        }
    }
}

/// Object form of a lifecycle handler reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHandlerDescription {
    /// Registry key of the host handler to invoke.
    pub handler: IdField,
}

/// Optional host callbacks for form, stage, and completion events.
///
/// Callbacks must be registered like service handlers. Their errors propagate,
/// and queued hooks require a successful status; their context patches are ignored.
/// During queued execution callbacks run before the database transition commits;
/// they may repeat after a failed attempt and must tolerate duplicate calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialHandlers {
    /// Called with the proposed typed `stage` object before the queued transition commits.
    pub on_stage_change: Option<TaskHandler>,
    /// Called on user-task entry with `form_id` and `roles` arguments.
    pub on_show_form: Option<TaskHandler>,
    /// Called before entering the next queued node when the current node is a user task.
    pub on_hide_form: Option<TaskHandler>,
    /// Called on End entry with an `end_node_id` string argument.
    pub on_process_complete: Option<TaskHandler>,
}
