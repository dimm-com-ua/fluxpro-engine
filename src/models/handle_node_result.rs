//! Handler outcomes that control context updates, transitions, and repetition.

use crate::models::context_map::context_patcher::ContextPatcher;
use chrono::{DateTime, Utc};

/// Outcome returned by a host handler to the workflow dispatcher.
pub struct HandleNodeResult {
    /// Outcome determining service-task continuation or failure handling.
    pub status: HandleResultStatus,
}

impl HandleNodeResult {
    /// Returns success with an empty context patch.
    pub fn success() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Success(Default::default()),
        }
    }
    /// Returns success with the context updates to apply before routing onward.
    pub fn success_with_patcher(patcher: ContextPatcher) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Success(patcher),
        }
    }
    /// Returns a failed outcome handled by service-task retry and error routing.
    pub fn failure() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Failure,
        }
    }

    /// Requests a new queue item for the same node at the supplied UTC timestamp.
    pub fn repeat(repeat_at: DateTime<Utc>) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Repeat(repeat_at),
        }
    }

    /// Reports a missing handler; queued execution treats this as failure.
    ///
    /// An actually unregistered handler instead produces an execution error.
    pub fn handler_not_exists() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::HandlerNotExists,
        }
    }

    /// Reports an invalid application state as a service-task failure.
    pub fn illegal_state(state: &str) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::IllegalState(state.to_string()),
        }
    }
}

/// Controls service-task continuation, context patching, failure, or repetition.
pub enum HandleResultStatus {
    /// Successful completion carrying its result or context patch.
    Success(ContextPatcher),
    /// Handler failure eligible for service-task retry handling.
    Failure,
    /// Requests another execution of this node at the supplied UTC timestamp.
    Repeat(DateTime<Utc>),
    /// Missing-handler outcome treated as failure by queued execution.
    HandlerNotExists,
    /// Handler failure with an application-specific state description.
    IllegalState(String),
}

impl HandleResultStatus {
    /// Returns the stable lowercase outcome label used in execution logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Success(_) => "success",
            Self::Failure => "failure",
            Self::Repeat(_) => "repeat",
            Self::HandlerNotExists => "handler_not_exists",
            Self::IllegalState(_) => "illegal_state",
        }
    }
}
