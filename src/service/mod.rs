//! Workflow execution and conversion of typed context into Rhai values.

use crate::models::context_map::context_map::{ContextMap, ContextValue};
use rhai::{Array, Dynamic, Map};

pub mod expressions;
pub mod process_service;

// Expose top-level keys directly under `ctx` rather than retaining wire wrappers.
fn ctx_to_rhai_map(ctx: &ContextMap) -> Map {
    let mut map = Map::new();
    for (k, v) in &ctx.0 {
        map.insert(k.get_id().into(), cv_to_dynamic(v));
    }
    map
}

// Arrays are converted recursively; Object values remain opaque JSON values.
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

// Normalize legacy strings and the reserved property only outside literals/comments.
fn normalize_quotes(expr: &str) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
        } else if ch == '/' && chars.get(i + 1) == Some(&'*') {
            out.push_str("/*");
            i += 2;
            while i < chars.len() {
                let ch = chars[i];
                out.push(ch);
                i += 1;
                if ch == '*' && chars.get(i) == Some(&'/') {
                    out.push('/');
                    i += 1;
                    break;
                }
            }
        } else if ch == '\'' || ch == '"' || ch == '`' {
            let quote = ch;
            out.push(if quote == '\'' { '"' } else { quote });
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if ch == quote {
                    out.push(if quote == '\'' { '"' } else { quote });
                    break;
                }
                if ch == '\\' && i < chars.len() {
                    let escaped = chars[i];
                    i += 1;
                    if quote == '\'' && escaped == '\'' {
                        out.push('\'');
                    } else {
                        out.push('\\');
                        out.push(escaped);
                    }
                } else {
                    if quote == '\'' && ch == '"' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push_str(&word);
            if word == "ctx" {
                let mut j = i;
                while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                    j += 1;
                }
                if chars.get(j) == Some(&'.') {
                    j += 1;
                    while chars.get(j).is_some_and(|c| c.is_whitespace()) {
                        j += 1;
                    }
                    let start = j;
                    while chars
                        .get(j)
                        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                    {
                        j += 1;
                    }
                    if chars[start..j].iter().collect::<String>() == "_last_signal" {
                        out.push_str("[\"_last_signal\"]");
                        i = j;
                    }
                }
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}
