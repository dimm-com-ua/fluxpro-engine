use serde::Serialize;

#[derive(Serialize)]
pub struct ProcessId {
    pub id: String,
}

impl ProcessId {
    pub fn new(id: String) -> Self {
        Self { id }
    }
}
