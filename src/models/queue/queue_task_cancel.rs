use crate::models::id_field::IdField;
use crate::models::process_def::Node;

pub struct QueueTaskCancel<'a> {
    event_id: IdField,
    process_token: &'a IdField,
    node: &'a Node,
}

impl<'a> QueueTaskCancel<'a> {
    pub fn new(event_id: IdField, process_token: &'a IdField, node: &'a Node) -> Self {
        Self {
            event_id,
            process_token,
            node,
        }
    }

    pub fn event_id(&self) -> &IdField {
        &self.event_id
    }

    pub fn process_token(&self) -> &'a IdField {
        self.process_token
    }

    pub fn node(&self) -> &'a Node {
        self.node
    }
}
