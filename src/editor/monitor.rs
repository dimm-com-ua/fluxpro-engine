//! Transport-independent models for the live process monitor.
use crate::editor::EditorDocument;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Identity of the exact definition version being inspected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorScope {
    /// Logical process key.
    pub key: String,
    /// Normalized version string.
    pub version: String,
    /// Persisted definition UUID, when known; distinguishes separate deployments.
    pub definition_uuid: Option<String>,
}
impl MonitorScope {
    /// Creates the scope from an editor document without changing its layout.
    pub fn from_document(document: &EditorDocument) -> Self {
        Self {
            key: document.definition.key.to_string(),
            version: document.definition.version.to_string(),
            definition_uuid: document.definition.uuid.map(|v| v.to_string()),
        }
    }
    /// Checks both version and database identity when the latter is supplied.
    pub fn contains(&self, instance: &MonitorInstance) -> bool {
        self.key == instance.process_key
            && self.version == instance.process_version
            && self
                .definition_uuid
                .as_ref()
                .is_none_or(|uuid| uuid == &instance.process_definition_uuid)
    }
}
/// Server-side filtering and pagination. Counts must remain unfiltered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorQuery {
    /// Case-insensitive search over business ID, runtime token and process key.
    pub search: String,
    /// Optional exact current node filter.
    pub node_id: Option<String>,
    /// Optional exact runtime state, including suspended/failed states.
    pub state: Option<String>,
    /// Number of matching instances to skip.
    pub offset: u64,
    /// Maximum instances to return; the UI uses 25 per page.
    pub limit: u32,
}
impl Default for MonitorQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            node_id: None,
            state: None,
            offset: 0,
            limit: 25,
        }
    }
}
impl MonitorQuery {
    /// Local equivalent of the requested filters, useful for snapshot transports.
    pub fn matches(&self, instance: &MonitorInstance) -> bool {
        let search = self.search.trim().to_lowercase();
        (search.is_empty()
            || [
                &instance.process_id,
                &instance.token,
                &instance.process_key,
                &instance.uuid,
            ]
            .iter()
            .any(|v| v.to_lowercase().contains(&search)))
            && self
                .node_id
                .as_ref()
                .is_none_or(|id| instance.current_node_id.as_ref() == Some(id))
            && self
                .state
                .as_ref()
                .is_none_or(|state| &instance.state == state)
    }
}
/// Detail history requested for an open instance tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInstanceRequest {
    /// Database instance UUID, never its business ID or runtime token.
    pub uuid: String,
    /// Execution history prefix size, starting at offset zero, newest first.
    pub log_limit: u32,
    /// Signal history prefix size, starting at offset zero, newest first.
    pub signal_limit: u32,
}
/// Complete read request emitted on navigation, search, polling or refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorRequest {
    /// Count only unfinished (including suspended) instances on the graph.
    /// Independent of list filters and pagination; false preserves all-instance counts.
    #[serde(default)]
    pub active_counts_only: bool,
    /// Exact version whose data may be rendered.
    pub scope: MonitorScope,
    /// Monotonically increasing revision within this mounted scope.
    pub revision: u64,
    /// Instance-list query. Does not apply to node counts or open details.
    pub query: MonitorQuery,
    /// Details to load for open tabs; closed tabs disappear from future requests.
    /// Form transports omit empty lists when all detail tabs are closed.
    #[serde(default)]
    pub instances: Vec<MonitorInstanceRequest>,
}
/// A paginated response; history pages use offset zero and a growing prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorPage<T> {
    /// Rows returned by the host.
    pub items: Vec<T>,
    /// Total matching rows across all pages.
    pub total: u64,
}
impl<T> Default for MonitorPage<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: 0,
        }
    }
}
/// Current operational issue. Resolved issues belong in history, not this list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorIssue {
    /// Stable incident or support-ticket identity.
    pub id: String,
    /// `error` or `escalation`; other categories remain visible as warnings.
    pub kind: String,
    /// Human-readable failure or escalation description.
    pub message: String,
    /// Associated node, when known.
    pub node_id: Option<String>,
    /// Escalation topic or error category.
    pub topic: Option<String>,
    /// ISO/RFC3339 occurrence time.
    pub created_at: Option<String>,
    /// Additional incident or ticket data retained for inspection.
    #[serde(default)]
    pub details: Value,
}
/// Counts for the entire definition version, independent of list pagination.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorNodeCount {
    /// Definition-local block identifier.
    pub node_id: String,
    /// Instances currently assigned to this node (including terminal nodes).
    pub instance_count: u64,
    /// Assigned instances with an unresolved execution error.
    #[serde(default)]
    pub errors: u64,
    /// Assigned instances with an active escalation.
    #[serde(default)]
    pub escalations: u64,
}
/// JSON-compatible projection of the engine's administrative instance summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorInstance {
    /// Database instance UUID; used as the tab identity.
    pub uuid: String,
    /// ISO/RFC3339 creation time.
    pub created_at: String,
    /// Persisted definition UUID.
    pub process_definition_uuid: String,
    /// Logical definition key.
    pub process_key: String,
    /// Exact normalized version.
    pub process_version: String,
    /// Application business key (engine `process_id`).
    pub process_id: String,
    /// Runtime token used to address this instance.
    pub token: String,
    /// Current runtime state, preserved verbatim.
    pub state: String,
    /// Current definition-local node identifier.
    pub current_node_id: Option<String>,
    /// Current stage identifier.
    pub current_stage_id: Option<String>,
    /// Display name of the current stage.
    pub current_stage_name: Option<String>,
    /// Explanation attached to the current stage.
    pub current_stage_reason: Option<String>,
    /// Active incidents and escalations supplied by the host.
    #[serde(default)]
    pub issues: Vec<MonitorIssue>,
    /// Other engine summary fields, including visit ID and wait completion.
    #[serde(flatten)]
    pub metadata: std::collections::BTreeMap<String, Value>,
}
/// Persisted stage transition with its context snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorStageEntry {
    /// Database history identity.
    pub uuid: String,
    /// ISO/RFC3339 transition time.
    pub created_at: String,
    /// Stage identifier when still available.
    pub stage_id: Option<String>,
    /// Stage display name when available.
    pub stage_name: Option<String>,
    /// Stage transition reason.
    pub reason: Option<String>,
    /// Context recorded at that transition, distinct from current context.
    #[serde(default)]
    pub context: Value,
}
/// Execution event, compatible with `ExecutionLogEntry` JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorLogEntry {
    /// Database log identity.
    pub uuid: String,
    /// ISO/RFC3339 event time.
    pub created_at: String,
    /// Severity, including error and critical.
    pub level: String,
    /// Engine event category such as `node.entered` or `instance.suspended`.
    pub event_type: String,
    /// Human-readable event text.
    pub message: String,
    /// Related node identifier.
    pub node_id: Option<String>,
    /// Other engine log fields: source, handler, attempt, queue ID, error and details.
    #[serde(flatten)]
    pub data: std::collections::BTreeMap<String, Value>,
}
/// Signal admission record; does not imply that queued delivery succeeded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorSignalEntry {
    /// Database signal history identity.
    pub uuid: String,
    /// ISO/RFC3339 admission time.
    pub created_at: String,
    /// Signal identifier.
    pub signal_name: String,
    /// Context payload recorded at admission.
    #[serde(default)]
    pub payload: Value,
}
/// Detail response for one instance tab. Histories may arrive incrementally.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorInstanceDetails {
    /// Identity and current state of the instance.
    pub instance: MonitorInstance,
    /// Current typed context, as serialized by the engine.
    #[serde(default)]
    pub context: Value,
    /// Scoped variable rows; retained because names can repeat across scopes.
    #[serde(default)]
    pub context_variables: Vec<Value>,
    /// Current node configuration.
    #[serde(default)]
    pub current_node: Option<Value>,
    /// Stage transitions with their historical context snapshots.
    #[serde(default)]
    pub stage_history: Vec<MonitorStageEntry>,
    /// Newest execution events first; the host supplies total for Load more.
    #[serde(default)]
    pub logs: MonitorPage<MonitorLogEntry>,
    /// Newest admitted signals first; the host supplies total for Load more.
    #[serde(default)]
    pub signals: MonitorPage<MonitorSignalEntry>,
    /// Per-instance loading error; does not hide the instance's existing data.
    #[serde(default)]
    pub error: Option<String>,
}
/// Consistent version-scoped snapshot supplied by polling, SSE or WebSocket.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    /// Echo the exact request. Responses to old queries/revisions are ignored.
    pub request: MonitorRequest,
    /// RFC3339 server observation time; absent until a response is available.
    pub observed_at: Option<String>,
    /// Version-wide counts. An omitted node is unknown, not assumed to be zero.
    pub node_counts: Vec<MonitorNodeCount>,
    /// Filtered, paginated instance list for the echoed query.
    pub instances: MonitorPage<MonitorInstance>,
    /// Requested open-instance details, each checked against the version scope.
    pub details: Vec<MonitorInstanceDetails>,
    /// Whether the host supplies authoritative support-escalation counts.
    #[serde(default)]
    pub escalation_counts_available: bool,
    /// Fetch error. Previously accepted data remain visible with a stale label.
    pub error: Option<String>,
}

pub(crate) fn valid_snapshot(snapshot: &MonitorSnapshot, request: &MonitorRequest) -> bool {
    snapshot.request == *request
        && snapshot
            .instances
            .items
            .iter()
            .all(|instance| request.scope.contains(instance))
        && snapshot
            .details
            .iter()
            .all(|detail| request.scope.contains(&detail.instance))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instance(version: &str) -> MonitorInstance {
        MonitorInstance {
            uuid: "db-1".into(),
            process_key: "loan".into(),
            process_version: version.into(),
            process_definition_uuid: "definition-a".into(),
            process_id: "ORDER-482".into(),
            token: "runtime_Secret482".into(),
            state: "suspended".into(),
            current_node_id: Some("review".into()),
            ..Default::default()
        }
    }
    #[test]
    fn searches_business_keys_tokens_and_exact_current_state_without_crossing_versions() {
        let i = instance("1.0.0");
        let scope = MonitorScope {
            key: "loan".into(),
            version: "1.0.0".into(),
            definition_uuid: Some("definition-a".into()),
        };
        assert!(scope.contains(&i));
        assert!(!scope.contains(&instance("2.0.0")));
        let mut duplicate = i.clone();
        duplicate.process_definition_uuid = "definition-b".into();
        assert!(!scope.contains(&duplicate));
        for search in ["order-482", "SECRET482", " loan ", "db-1"] {
            assert!(
                MonitorQuery {
                    search: search.into(),
                    ..Default::default()
                }
                .matches(&i)
            );
        }
        assert!(
            !MonitorQuery {
                search: "absent".into(),
                ..Default::default()
            }
            .matches(&i)
        );
        assert!(
            MonitorQuery {
                node_id: Some("review".into()),
                state: Some("suspended".into()),
                ..Default::default()
            }
            .matches(&i)
        );
        assert!(
            !MonitorQuery {
                node_id: Some("start".into()),
                ..Default::default()
            }
            .matches(&i)
        );
    }
    #[test]
    fn stale_queries_revisions_and_foreign_instance_details_are_rejected() {
        let request = MonitorRequest {
            scope: MonitorScope {
                key: "loan".into(),
                version: "1.0.0".into(),
                definition_uuid: None,
            },
            revision: 12,
            ..Default::default()
        };
        let mut data = MonitorSnapshot {
            request: request.clone(),
            instances: MonitorPage {
                items: vec![instance("1.0.0")],
                total: 1,
            },
            ..Default::default()
        };
        assert!(valid_snapshot(&data, &request));
        data.request.revision = 11;
        assert!(!valid_snapshot(&data, &request));
        data.request = request.clone();
        data.request.query.search = "different".into();
        assert!(!valid_snapshot(&data, &request));
        data.request = request.clone();
        data.details.push(MonitorInstanceDetails {
            instance: instance("2.0.0"),
            ..Default::default()
        });
        assert!(!valid_snapshot(&data, &request));
    }
    #[test]
    fn administrative_json_preserves_visit_and_diagnostic_fields() {
        let mut value = serde_json::to_value(instance("1.0.0")).unwrap();
        value["node_visit_id"] = serde_json::json!("visit-123");
        value["wait_completed"] = serde_json::json!(true);
        let instance: MonitorInstance = serde_json::from_value(value).unwrap();
        assert_eq!(instance.metadata["node_visit_id"], "visit-123");
        assert_eq!(instance.metadata["wait_completed"], true);
        let entry:MonitorLogEntry=serde_json::from_value(serde_json::json!({"uuid":"log-1","created_at":"2026-09-09T10:00:00Z","level":"error","event_type":"instance.suspended","message":"Stopped","node_id":"review","error_kind":"handler","error_message":"Provider unavailable","attempt":3,"details":{"nested":[1,true]}})).unwrap();
        assert_eq!(entry.data["attempt"], 3);
        assert_eq!(entry.data["details"]["nested"][1], true);
    }
}
