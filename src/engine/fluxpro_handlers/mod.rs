//! Registry of host-provided service and lifecycle handlers.

use crate::models::id_field::IdField;
use crate::traits::node_handlers::service_node_handler::FluxproServiceHandler;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;

/// Registry keyed by each handler's declared name.
#[derive(Clone, Default)]
pub struct FluxproHandlersContainer {
    handlers: HashMap<IdField, Arc<dyn FluxproServiceHandler + Send + Sync>>,
}

impl FluxproHandlersContainer {
    /// Builds the registry; the last handler wins when names are duplicated.
    pub fn new(
        handlers: Vec<Arc<dyn FluxproServiceHandler + Send + Sync>>,
    ) -> FluxproHandlersContainer {
        let handlers = handlers
            .into_iter()
            .map(|handler| (handler.get_name(), handler))
            .collect::<HashMap<_, _>>();
        FluxproHandlersContainer { handlers }
    }

    /// Returns a shared handler by its registered ID, or `None` if absent.
    pub fn get_handler(
        &self,
        id: &IdField,
    ) -> Option<Arc<dyn FluxproServiceHandler + Send + Sync>> {
        info!("Retrieving handler with id {}", id);
        self.handlers
            .get(id)
            .map(|handler| {
                info!(" *+*+*+*+*+*+ Found handler with id {}", id);
                handler
            })
            .cloned()
    }
}
