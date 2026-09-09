//! Signal-to-timeout cancellation associations for a waiting node.

use crate::models::id_field::IdField;
use crate::models::process_def::Node;

/// Association between a signal and a timeout belonging to one node instance.
pub struct QueueTaskCancel<'a> {
    event_id: IdField,
    process_token: &'a IdField,
    node: &'a Node,
}

impl<'a> QueueTaskCancel<'a> {
    /// Associates a cancellation signal with a borrowed token and waiting node.
    pub fn new(event_id: IdField, process_token: &'a IdField, node: &'a Node) -> Self {
        Self {
            event_id,
            process_token,
            node,
        }
    }

    /// Borrows the signal identifier that cancels the timeout.
    pub fn event_id(&self) -> &IdField {
        &self.event_id
    }

    /// Borrows the runtime instance token associated with this task.
    pub fn process_token(&self) -> &'a IdField {
        self.process_token
    }

    /// Borrows the node whose timeout is canceled by this association.
    pub fn node(&self) -> &'a Node {
        self.node
    }
}
