use crate::models::process_def::{Node, ProcessDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet, VecDeque};

pub(crate) const NODE_WIDTH: f64 = 224.0;
pub(crate) const NODE_HEIGHT: f64 = 88.0;
const BRANCH_ROW_STEP: f64 = 44.0;

pub(crate) fn branch_color(index: usize) -> &'static str {
    [
        "#0ea5e9", "#10b981", "#8b5cf6", "#f59e0b", "#ec4899", "#64748b",
    ][index % 6]
}

/// Canvas coordinates, independent of scrolling and the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// Horizontal offset in CSS pixels.
    pub x: f64,
    /// Vertical offset in CSS pixels.
    pub y: f64,
}

impl Position {
    /// Creates a position, clamped to the finite, nonnegative canvas.
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: finite(x),
            y: finite(y),
        }
    }
}

fn finite(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1_000_000.0)
    } else {
        0.0
    }
}

/// All node types supported by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Process entry.
    Start,
    /// Host service invocation.
    ServiceTask,
    /// A form waiting for a signal.
    UserTask,
    /// Ordered XOR conditions.
    Gateway,
    /// Wait for a signal without a form.
    Wait,
    /// Process exit.
    End,
}

impl BlockKind {
    /// Palette order.
    pub const ALL: [Self; 6] = [
        Self::Start,
        Self::ServiceTask,
        Self::UserTask,
        Self::Gateway,
        Self::Wait,
        Self::End,
    ];

    /// Engine schema discriminator.
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::ServiceTask => "ServiceTask",
            Self::UserTask => "UserTask",
            Self::Gateway => "Gateway",
            Self::Wait => "Wait",
            Self::End => "End",
        }
    }

    /// Human-readable palette label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::ServiceTask => "Service task",
            Self::UserTask => "User task",
            Self::Gateway => "Condition",
            Self::Wait => "Wait for signal",
            Self::End => "Finish",
        }
    }

    /// Short explanation displayed in the palette.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Start => "Where it begins",
            Self::ServiceTask => "Run a handler",
            Self::UserTask => "Ask for input",
            Self::Gateway => "Choose a path",
            Self::Wait => "Pause until an event",
            Self::End => "Complete this path",
        }
    }

    /// Compact visual symbol; labels remain available to assistive technology.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Start => "▷",
            Self::ServiceTask => "⚙",
            Self::UserTask => "▤",
            Self::Gateway => "◇",
            Self::Wait => "◷",
            Self::End => "◉",
        }
    }

    /// Identifies a typed engine node.
    pub fn of(node: &Node) -> Self {
        match node {
            Node::Start { .. } => Self::Start,
            Node::ServiceTask { .. } => Self::ServiceTask,
            Node::UserTask { .. } => Self::UserTask,
            Node::Gateway { .. } => Self::Gateway,
            Node::Wait { .. } => Self::Wait,
            Node::End { .. } => Self::End,
        }
    }
}

/// A directed edge, including conditional, error, compensation, and timeout routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// Source node ID.
    pub source: String,
    /// Destination node ID.
    pub target: String,
    /// Condition or route label.
    pub label: String,
    /// JSON pointer into the source node identifying the route target.
    pub pointer: String,
}

impl Connection {
    /// Distinct footer outlet (horizontal offset, color, icon) for exceptional routes.
    pub fn special_outlet(&self) -> Option<(f64, &'static str, &'static str)> {
        match self.pointer.as_str() {
            "/on_error/next" => Some((40.0, "#e11d48", "!")),
            "/on_error/compensate" => Some((72.0, "#a855f7", "↶")),
            "/timeout/on_timeout" => Some((184.0, "#d97706", "◷")),
            _ => None,
        }
    }

    /// Cubic Bézier path from the bottom of a source to the top of a target.
    /// Back edges loop to the side so a cycle remains visible.
    pub fn path(&self, positions: &BTreeMap<String, Position>) -> Option<String> {
        self.path_with_source(positions, NODE_HEIGHT, None, false)
    }

    fn path_with_source(
        &self,
        positions: &BTreeMap<String, Position>,
        height: f64,
        port: Option<Position>,
        right_facing: bool,
    ) -> Option<String> {
        let a = positions.get(&self.source)?;
        let b = positions.get(&self.target)?;
        let (x1, y1, x2, y2) = (
            port.map_or(a.x + NODE_WIDTH / 2.0, |p| p.x),
            port.map_or(a.y + height, |p| p.y),
            b.x + NODE_WIDTH / 2.0,
            b.y,
        );
        if right_facing {
            // Leave each condition horizontally through its own right-hand port.
            // Keep a separate side lane for backward edges and self-loops.
            let side = x1.max(b.x + NODE_WIDTH) + 48.0;
            return Some(if y2 > y1 + 32.0 {
                format!(
                    "M {x1} {y1} C {} {y1} {x2} {} {x2} {y2}",
                    x1 + 64.0,
                    y2 - 40.0
                )
            } else {
                format!(
                    "M {x1} {y1} C {side} {y1} {side} {y1} {side} {} L {side} {} C {side} {} {x2} {} {x2} {y2}",
                    y1 - 24.0,
                    y2 - 40.0,
                    y2 - 64.0,
                    y2 - 48.0
                )
            });
        }
        if y2 > y1 {
            let bend = ((y2 - y1) / 2.0).max(32.0);
            Some(format!(
                "M {x1} {y1} C {x1} {} {x2} {} {x2} {y2}",
                y1 + bend,
                y2 - bend
            ))
        } else {
            let side = a.x.max(b.x) + NODE_WIDTH + 64.0;
            Some(format!(
                "M {x1} {y1} C {x1} {} {side} {} {side} {} L {side} {} C {side} {} {x2} {} {x2} {y2}",
                y1 + 48.0,
                y1 + 48.0,
                y1 + 24.0,
                y2 - 24.0,
                y2 - 48.0,
                y2 - 48.0
            ))
        }
    }
}

/// An engine definition and an independent, optional visual layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorDocument {
    /// Complete typed definition, including declarations and runtime settings.
    /// Editor transport preserves database identity; exported process YAML does not.
    #[serde(with = "document_definition")]
    pub definition: ProcessDefinition,
    /// Top-left coordinates indexed by stable node ID.
    pub positions: BTreeMap<String, Position>,
}

// ProcessDefinition intentionally skips UUID in portable workflow files. The
// editor/monitor transport also needs that identity to request the exact version.
mod document_definition {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        definition: &ProcessDefinition,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(definition).map_err(serde::ser::Error::custom)?;
        if let Some(uuid) = definition.uuid {
            value["uuid"] = Value::String(uuid.to_string());
        }
        value.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ProcessDefinition, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let uuid = value
            .get("uuid")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .flatten();
        let mut definition: ProcessDefinition =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        definition.uuid = uuid;
        Ok(definition)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Layout {
    #[serde(default)]
    positions: BTreeMap<String, Position>,
}

impl Default for EditorDocument {
    fn default() -> Self {
        Self::from_yaml("key: new_process\nname: Untitled process\nversion: 1.0.0\nstatus: draft\nstages:\n  - id: created\n    name: Created\n    is_initial: true\nnodes:\n  - { id: start, type: Start, next: finish }\n  - { id: finish, type: End }\n").expect("built-in process is valid")
    }
}

impl EditorDocument {
    /// Wraps an existing typed engine definition and arranges it vertically.
    pub fn new(definition: ProcessDefinition) -> Result<Self, String> {
        let mut ids = HashSet::new();
        for node in &definition.nodes {
            if !ids.insert(node.id().get_id()) {
                return Err(format!("Duplicate node ID: {}", node.id()));
            }
        }
        let mut document = Self {
            definition,
            positions: BTreeMap::new(),
        };
        for kind in [
            crate::editor::declarations::DeclarationKind::Form,
            crate::editor::declarations::DeclarationKind::Signal,
            crate::editor::declarations::DeclarationKind::Escalation,
        ] {
            let mut ids = HashSet::new();
            for id in document.declaration_ids(kind) {
                if !ids.insert(id.clone()) {
                    return Err(format!("Duplicate {} ID: {id}", kind.singular()));
                }
            }
        }
        document.auto_layout();
        Ok(document)
    }

    /// Imports an existing engine YAML definition, optionally with `editor.positions`.
    /// Missing positions receive a deterministic vertical layout. Syntax/schema and
    /// duplicate-ID errors are rejected; incomplete drafts remain editable.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let mut value: Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        let layout = value
            .as_object_mut()
            .and_then(|map| map.remove("editor"))
            .map(serde_json::from_value::<Layout>)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let definition: ProcessDefinition =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        let mut document = Self::new(definition)?;
        for (id, position) in layout.positions {
            if document.positions.contains_key(&id) {
                document
                    .positions
                    .insert(id, Position::new(position.x, position.y));
            }
        }
        Ok(document)
    }

    /// Serializes the engine definition without editor-specific metadata.
    /// Drafts may be exported; call [`Self::diagnostics`] before deployment.
    pub fn to_process_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(&self.definition).map_err(|e| e.to_string())
    }

    /// Serializes a YAML project including manually placed node coordinates.
    pub fn to_project_yaml(&self) -> Result<String, String> {
        let mut value = serde_json::to_value(&self.definition).map_err(|e| e.to_string())?;
        value["editor"] = json!({ "positions": self.positions });
        serde_yaml::to_string(&value).map_err(|e| e.to_string())
    }

    /// Finds a node by stable ID.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.definition
            .nodes
            .iter()
            .find(|node| node.id().get_id() == id)
    }

    /// Enumerates every node reference supported by the definition schema.
    pub fn connections(&self) -> Vec<Connection> {
        let mut edges = Vec::new();
        for node in &self.definition.nodes {
            let value = serde_json::to_value(node).expect("node serialization");
            let mut push = |pointer: String, label: String| {
                if let Some(target) = value.pointer(&pointer).and_then(Value::as_str) {
                    edges.push(Connection {
                        source: node.id().to_string(),
                        target: target.into(),
                        label,
                        pointer,
                    });
                }
            };
            push(
                "/next".into(),
                if matches!(node, Node::Gateway { .. }) {
                    "Otherwise"
                } else {
                    ""
                }
                .into(),
            );
            push("/next/default".into(), "Otherwise".into());
            for prefix in ["/branches", "/next/branches"] {
                if let Some(branches) = value.pointer(prefix).and_then(Value::as_array) {
                    for (i, branch) in branches.iter().enumerate() {
                        push(
                            format!("{prefix}/{i}/next"),
                            branch["when"].as_str().unwrap_or("Condition").into(),
                        );
                    }
                }
            }
            for (pointer, label) in [
                ("/on_error/next", "On error"),
                ("/on_error/compensate", "Compensate"),
                ("/timeout/on_timeout", "Timeout"),
            ] {
                push(pointer.into(), label.into());
            }
        }
        edges
    }

    /// Ordered condition rows followed by the fallback route, when this node branches.
    pub fn branch_routes(&self, id: &str) -> Vec<Connection> {
        let edges: Vec<_> = self
            .connections()
            .into_iter()
            .filter(|e| e.source == id)
            .collect();
        let mut branches: Vec<_> = edges
            .iter()
            .filter(|e| e.pointer.contains("/branches/"))
            .cloned()
            .collect();
        if !branches.is_empty() {
            branches.extend(
                edges
                    .into_iter()
                    .filter(|e| matches!(e.pointer.as_str(), "/next" | "/next/default")),
            );
        }
        branches
    }

    /// Timeout, error and compensation routes shown as separate footer icons.
    pub fn special_routes(&self, id: &str) -> Vec<Connection> {
        self.connections()
            .into_iter()
            .filter(|edge| edge.source == id && edge.special_outlet().is_some())
            .collect()
    }

    /// Rendered card height, including the visible condition rows.
    pub fn node_height(&self, id: &str) -> f64 {
        let count = self.branch_routes(id).len();
        NODE_HEIGHT
            + if count == 0 {
                0.0
            } else {
                count as f64 * BRANCH_ROW_STEP + 8.0
            }
    }

    /// Exact condition-row outlet in canvas coordinates and its matching edge color.
    pub fn branch_port(&self, edge: &Connection) -> Option<(Position, &'static str)> {
        let index = self
            .branch_routes(&edge.source)
            .iter()
            .position(|r| r.pointer == edge.pointer)?;
        let source = self.positions.get(&edge.source)?;
        Some((
            Position::new(
                source.x + NODE_WIDTH,
                source.y + NODE_HEIGHT + 20.0 + index as f64 * BRANCH_ROW_STEP,
            ),
            branch_color(index),
        ))
    }

    /// Connection geometry shared by the editor and monitor, including branch outlets.
    pub fn connection_path(&self, edge: &Connection) -> Option<String> {
        let branch = self.branch_port(edge);
        let port = branch.map(|(p, _)| p).or_else(|| {
            let (x, _, _) = edge.special_outlet()?;
            let source = self.positions.get(&edge.source)?;
            Some(Position::new(
                source.x + x,
                source.y + self.node_height(&edge.source),
            ))
        });
        edge.path_with_source(
            &self.positions,
            self.node_height(&edge.source),
            port,
            branch.is_some(),
        )
    }

    /// Arranges reachable nodes in breadth-first layers, keeps cycles finite,
    /// places disconnected nodes after them, and aligns every End at the bottom.
    pub fn auto_layout(&mut self) {
        let edges = self.connections();
        let mut ranks = BTreeMap::new();
        let mut queue = VecDeque::new();
        for node in &self.definition.nodes {
            if node.is_start() {
                ranks.insert(node.id().to_string(), 0usize);
                queue.push_back(node.id().to_string());
            }
        }
        while let Some(id) = queue.pop_front() {
            let next_rank = ranks[&id] + 1;
            for edge in edges.iter().filter(|edge| edge.source == id) {
                if self.node(&edge.target).is_some() && !ranks.contains_key(&edge.target) {
                    ranks.insert(edge.target.clone(), next_rank);
                    queue.push_back(edge.target.clone());
                }
            }
        }
        let mut tail = ranks.values().copied().max().unwrap_or(0) + 1;
        for node in &self.definition.nodes {
            if !ranks.contains_key(node.id().get_id()) && !node.is_end() {
                ranks.insert(node.id().to_string(), tail);
                tail += 1;
            }
        }
        let end_rank = ranks.values().copied().max().unwrap_or(0) + 1;
        for node in &self.definition.nodes {
            if node.is_end() {
                ranks.insert(node.id().to_string(), end_rank);
            }
        }
        let mut rows: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for node in &self.definition.nodes {
            rows.entry(ranks[node.id().get_id()])
                .or_default()
                .push(node.id().to_string());
        }
        let columns = rows.values().map(Vec::len).max().unwrap_or(1);
        self.positions.clear();
        let mut y = 64.0;
        for (_, nodes) in rows {
            let offset = (columns - nodes.len()) as f64 * 144.0;
            let height = nodes
                .iter()
                .map(|id| self.node_height(id))
                .fold(NODE_HEIGHT, f64::max);
            for (column, id) in nodes.into_iter().enumerate() {
                self.positions
                    .insert(id, Position::new(80.0 + offset + column as f64 * 288.0, y));
            }
            y += height + 96.0;
        }
    }

    /// Canvas bounds that grow with manually positioned nodes.
    pub fn canvas_size(&self) -> (f64, f64) {
        self.positions
            .iter()
            .fold((1000.0_f64, 900.0_f64), |(w, h), (id, p)| {
                (
                    w.max(p.x + NODE_WIDTH + 220.0),
                    h.max(p.y + self.node_height(id) + 200.0),
                )
            })
    }

    /// Adds a block with a collision-free ID. Form/signal declarations are added
    /// for user tasks and waits; service handler IDs are editable placeholders.
    pub fn add_block(&mut self, kind: BlockKind, position: Position) -> Result<String, String> {
        if kind == BlockKind::Start && self.definition.nodes.iter().any(Node::is_start) {
            return Err("A process already has a Start block.".into());
        }
        let prefix = match kind {
            BlockKind::ServiceTask => "service",
            BlockKind::UserTask => "user_task",
            BlockKind::Gateway => "condition",
            BlockKind::Wait => "wait",
            BlockKind::Start => "start",
            BlockKind::End => "finish",
        };
        let id = (1..)
            .map(|i| format!("{prefix}_{i}"))
            .find(|id| self.node(id).is_none())
            .unwrap();
        let mut value = json!({"id": id, "type": kind.name()});
        match kind {
            BlockKind::Start => {
                value["next"] = json!(
                    self.definition
                        .nodes
                        .iter()
                        .find(|n| n.is_end())
                        .map(|n| n.id().to_string())
                        .unwrap_or_else(|| "finish".into())
                )
            }
            BlockKind::ServiceTask => value["handler"] = json!("configure_handler"),
            BlockKind::Gateway => {
                value["gateway"] = json!("XOR");
                value["branches"] = json!([]);
            }
            BlockKind::UserTask | BlockKind::Wait => {
                let signal = (1..)
                    .map(|i| format!("{id}_completed_{i}"))
                    .find(|id| !self.definition.signals.iter().any(|s| s.name() == id))
                    .unwrap();
                value["wait_for"] = json!({ "signal": signal });
                self.definition
                    .signals
                    .push(serde_json::from_value(json!(signal)).map_err(|e| e.to_string())?);
                if kind == BlockKind::UserTask {
                    let form = (1..)
                        .map(|i| format!("{id}_form_{i}"))
                        .find(|id| !self.definition.forms.iter().any(|f| f.id.get_id() == id))
                        .unwrap();
                    value["form"] = json!(form);
                    self.definition.forms.push(
                        serde_json::from_value(json!({ "id": form, "roles": [] }))
                            .map_err(|e| e.to_string())?,
                    );
                }
            }
            BlockKind::End => {}
        }
        self.definition
            .nodes
            .push(serde_json::from_value(value).map_err(|e| e.to_string())?);
        self.positions
            .insert(id.clone(), Position::new(position.x, position.y));
        Ok(id)
    }

    /// Moves a node without changing its definition or routes.
    pub fn move_node(&mut self, id: &str, position: Position) {
        if self.node(id).is_some() {
            self.positions
                .insert(id.into(), Position::new(position.x, position.y));
        }
    }

    /// Connects two blocks. A condition appends an ordered branch; no condition
    /// sets the default successor while preserving all existing conditional routes.
    pub fn connect(
        &mut self,
        source: &str,
        target: &str,
        condition: Option<&str>,
    ) -> Result<(), String> {
        let destination = self.node(target).ok_or("Destination block not found")?;
        if destination.is_start() {
            return Err("Start cannot have incoming connections.".into());
        }
        let node = self.node(source).ok_or("Source block not found")?;
        if node.is_end() {
            return Err("Finish cannot have outgoing connections.".into());
        }
        let condition = condition.map(str::trim).filter(|s| !s.is_empty());
        let mut value = serde_json::to_value(node).map_err(|e| e.to_string())?;
        if let Some(condition) = condition {
            if node.is_start() {
                return Err("Use a Condition block after Start to branch.".into());
            }
            let branches = if matches!(node, Node::Gateway { .. }) {
                &mut value["branches"]
            } else {
                if !value["next"].is_object() {
                    value["next"] = json!({"default": value["next"], "branches": []});
                }
                &mut value["next"]["branches"]
            };
            branches
                .as_array_mut()
                .ok_or("Invalid route branches")?
                .push(json!({"when": condition, "next": target}));
        } else if value["next"].is_object() {
            value["next"]["default"] = json!(target);
        } else {
            value["next"] = json!(target);
        }
        self.replace_value(source, value)
    }

    /// Removes a route. Start's required successor must be reconnected instead.
    /// Removing a timeout route removes its whole timeout configuration.
    pub fn disconnect(&mut self, connection: &Connection) -> Result<(), String> {
        let node = self
            .node(&connection.source)
            .ok_or("Source block not found")?;
        if node.is_start() {
            return Err("Reconnect Start to another block; its successor is required.".into());
        }
        if !self.connections().contains(connection) {
            return Err("Connection changed; select it again.".into());
        }
        let mut value = serde_json::to_value(node).map_err(|e| e.to_string())?;
        let pointer = &connection.pointer;
        if pointer.contains("/branches/") {
            let branch = pointer.strip_suffix("/next").ok_or("Invalid branch")?;
            let (parent, index) = branch.rsplit_once('/').ok_or("Invalid branch")?;
            let index: usize = index.parse().map_err(|_| "Invalid branch index")?;
            value
                .pointer_mut(parent)
                .and_then(Value::as_array_mut)
                .ok_or("Missing branches")?
                .remove(index);
        } else if pointer == "/timeout/on_timeout" {
            value["timeout"] = Value::Null;
        } else {
            *value.pointer_mut(pointer).ok_or("Missing route")? = Value::Null;
        }
        self.replace_value(&connection.source, value)
    }

    /// Replaces the selected node's settings from YAML. IDs stay stable so that
    /// references and saved coordinates cannot silently break.
    pub fn update_node_yaml(&mut self, id: &str, yaml: &str) -> Result<(), String> {
        let value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        self.replace_value(id, value)
    }

    fn replace_value(&mut self, id: &str, value: Value) -> Result<(), String> {
        let replacement: Node = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if replacement.id().get_id() != id {
            return Err("Keep the node ID unchanged; edit IDs in the process YAML.".into());
        }
        let node = self
            .definition
            .nodes
            .iter_mut()
            .find(|node| node.id().get_id() == id)
            .ok_or("Block not found")?;
        if BlockKind::of(node) != BlockKind::of(&replacement) {
            return Err(
                "Keep the block type unchanged; add a new block to change its type.".into(),
            );
        }
        *node = replacement;
        Ok(())
    }

    /// Deletes an unreferenced node. Incoming routes must be removed or
    /// reconnected first; declarations remain available to other nodes.
    pub fn remove_node(&mut self, id: &str) -> Result<(), String> {
        if self.node(id).is_none() {
            return Err("Block not found".into());
        }
        if self
            .connections()
            .iter()
            .any(|edge| edge.target == id && edge.source != id)
        {
            return Err(
                "Reconnect or remove incoming connections before deleting this block.".into(),
            );
        }
        self.definition
            .nodes
            .retain(|node| node.id().get_id() != id);
        self.positions.remove(id);
        Ok(())
    }

    /// Structural engine errors plus actionable graph authoring hints.
    /// This does not validate host handlers or execute expressions.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut messages = match self.definition.validate() {
            Ok(()) => Vec::new(),
            Err(crate::models::process_def_error::CreateProcessError::ValidationError(errors)) => {
                errors
            }
            Err(error) => vec![error.to_string()],
        };
        let edges = self.connections();
        let mut reached: HashSet<String> = self
            .definition
            .nodes
            .iter()
            .filter(|n| n.is_start())
            .map(|n| n.id().to_string())
            .collect();
        let mut queue: VecDeque<String> = reached.iter().cloned().collect();
        while let Some(id) = queue.pop_front() {
            for edge in edges.iter().filter(|edge| edge.source == id) {
                if reached.insert(edge.target.clone()) {
                    queue.push_back(edge.target.clone());
                }
            }
        }
        for node in &self.definition.nodes {
            let id = node.id().get_id();
            if !reached.contains(id) {
                messages.push(format!("{id}: not reachable from Start"));
            }
            if !node.is_end() && !edges.iter().any(|edge| edge.source == id) {
                messages.push(format!("{id}: add an outgoing connection"));
            }
            if let Node::ServiceTask { handler, .. } = node {
                if handler.get_id() == "configure_handler" {
                    messages.push(format!("{id}: choose a registered service handler"));
                }
            }
        }
        messages
    }
}

/// Shared component state with bounded undo/redo history. Store in a Leptos
/// `RwSignal` and pass the same signal to the editor or its individual panels.
#[derive(Debug, Clone)]
pub struct EditorState {
    /// Current authoring document.
    pub document: EditorDocument,
    past: Vec<EditorDocument>,
    future: Vec<EditorDocument>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new(EditorDocument::default())
    }
}

impl EditorState {
    /// Starts an editing session with empty history.
    pub fn new(document: EditorDocument) -> Self {
        Self {
            document,
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    /// Applies an atomic edit and records one undo step. Failed edits leave
    /// the document and history unchanged.
    pub fn edit<T>(
        &mut self,
        edit: impl FnOnce(&mut EditorDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut next = self.document.clone();
        let result = edit(&mut next)?;
        self.checkpoint();
        self.document = next;
        Ok(result)
    }

    /// Records a single history entry before a multi-event drag gesture.
    pub fn checkpoint(&mut self) {
        if self.past.len() == 100 {
            self.past.remove(0);
        }
        self.past.push(self.document.clone());
        self.future.clear();
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Restores the previous document, including coordinates and declarations.
    pub fn undo(&mut self) {
        if let Some(previous) = self.past.pop() {
            self.future
                .push(std::mem::replace(&mut self.document, previous));
        }
    }

    /// Reapplies the last undone edit.
    pub fn redo(&mut self) {
        if let Some(next) = self.future.pop() {
            self.past.push(std::mem::replace(&mut self.document, next));
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn failed_edits_are_atomic_and_successful_edits_undo_declarations_too() {
        let mut state = EditorState::default();
        let original = state.document.to_project_yaml().unwrap();
        let result: Result<(), String> = state.edit(|doc| {
            doc.add_block(BlockKind::UserTask, Position::new(10.0, 20.0))?;
            Err("abort".into())
        });
        assert!(result.is_err());
        assert!(!state.can_undo());
        assert_eq!(state.document.to_project_yaml().unwrap(), original);
        state
            .edit(|doc| doc.add_block(BlockKind::UserTask, Position::new(10.0, 20.0)))
            .unwrap();
        let changed = state.document.to_project_yaml().unwrap();
        state.undo();
        assert_eq!(state.document.to_project_yaml().unwrap(), original);
        state.redo();
        assert_eq!(state.document.to_project_yaml().unwrap(), changed);
        state.undo();
        state
            .edit(|doc| doc.add_block(BlockKind::Gateway, Position::default()))
            .unwrap();
        assert!(!state.can_redo());
    }

    #[test]
    fn drag_gesture_uses_one_undo_entry() {
        let mut state = EditorState::default();
        let original = state.document.positions["start"];
        state.checkpoint();
        for i in 0..10 {
            state
                .document
                .move_node("start", Position::new(i as f64, 20.0));
        }
        state.undo();
        assert_eq!(state.document.positions["start"], original);
        assert!(!state.can_undo());
    }
}
