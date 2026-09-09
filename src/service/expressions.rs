//! Shared bounded Rhai compilation and evaluation for workflow decisions.

use super::{ctx_to_rhai_map, normalize_quotes};
use crate::models::context_map::context_map::ContextMap;
use rhai::{Engine, Scope};
use std::fmt;

/// A condition failed to compile, execute, or return a boolean.
#[derive(Debug)]
pub struct ConditionError(pub String);
impl fmt::Display for ConditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ConditionError {}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_fail_on_invalid_map_property(true);
    engine.set_max_operations(50_000);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(1_000_000);
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(10_000);
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine
}

/// Compiles a condition without evaluating context-dependent values.
pub fn validate_condition(condition: &str) -> Result<(), ConditionError> {
    engine()
        .compile_expression(normalize_quotes(condition))
        .map(|_| ())
        .map_err(|error| ConditionError(format!("invalid condition: {error}")))
}

/// Evaluates a boolean condition; errors must never be treated as a false branch.
pub fn evaluate_condition(condition: &str, context: &ContextMap) -> Result<bool, ConditionError> {
    let engine = engine();
    let ast = engine
        .compile_expression(normalize_quotes(condition))
        .map_err(|error| ConditionError(format!("invalid condition: {error}")))?;
    let mut scope = Scope::new();
    scope.push("ctx", ctx_to_rhai_map(context));
    engine
        .eval_ast_with_scope::<bool>(&mut scope, &ast)
        .map_err(|error| ConditionError(format!("condition evaluation failed: {error}")))
}
