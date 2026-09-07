use crate::models::context_map::context_map::{ContextMap, ContextValue};
use regex::Regex;
use rhai::{Array, Dynamic, Map};

pub mod process_service;

fn ctx_to_rhai_map(ctx: &ContextMap) -> Map {
    let mut map = Map::new();
    for (k, v) in &ctx.0 {
        map.insert(k.get_id().into(), cv_to_dynamic(v));
    }
    map
}

fn cv_to_dynamic(cv: &ContextValue) -> Dynamic {
    match cv {
        ContextValue::String { string } => Dynamic::from(string.clone()),
        ContextValue::Number { number } => Dynamic::from(*number),
        ContextValue::Float { float } => Dynamic::from(*float),
        ContextValue::Date { date } => Dynamic::from(date.to_string()),
        ContextValue::Array { array } => {
            let arr: Array = array.iter().map(|v| cv_to_dynamic(v)).collect();
            Dynamic::from_array(arr)
        }
        ContextValue::DateTime { datetime } => Dynamic::from(datetime.to_string()),
        ContextValue::Boolean { boolean } => Dynamic::from(*boolean),
        ContextValue::Object { object } => Dynamic::from(object.clone()),
        ContextValue::IdField { id_field } => Dynamic::from(id_field.get_id().to_string()),
    }
}

fn normalize_quotes(expr: &str) -> String {
    lazy_static::lazy_static! {
        static ref RE_SQ: Regex = Regex::new(r"'([^']*)'").unwrap();
    }
    RE_SQ.replace_all(expr, "\"$1\"").to_string()
}
