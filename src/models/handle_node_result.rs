use crate::models::context_map::context_patcher::ContextPatcher;
use chrono::{DateTime, Utc};

pub struct HandleNodeResult {
    pub status: HandleResultStatus,
}

impl HandleNodeResult {
    pub fn success() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Success(Default::default()),
        }
    }
    pub fn success_with_patcher(patcher: ContextPatcher) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Success(patcher),
        }
    }
    pub fn failure() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Failure,
        }
    }

    pub fn repeat(repeat_at: DateTime<Utc>) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::Repeat(repeat_at),
        }
    }

    pub fn handler_not_exists() -> Self {
        HandleNodeResult {
            status: HandleResultStatus::HandlerNotExists,
        }
    }

    pub fn illegal_state(state: &str) -> Self {
        HandleNodeResult {
            status: HandleResultStatus::IllegalState(state.to_string()),
        }
    }
}

pub enum HandleResultStatus {
    Success(ContextPatcher),
    Failure,
    Repeat(DateTime<Utc>),
    HandlerNotExists,
    IllegalState(String),
}

impl HandleResultStatus {
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
