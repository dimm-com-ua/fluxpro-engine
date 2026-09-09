//! Typed process context with JSON serialization and exact-type accessors.

use crate::models::context_map::context_patcher::{ContextPatcher, ContextPatcherOp};
use crate::models::id_field::IdField;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Typed values keyed by validated identifiers.
///
/// Wire values retain their type wrapper, for example
/// `{"amount":{"number":100},"approved":{"boolean":true}}`.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::context_map::context_map::{ContextMap, ContextMapBuilder, ContextValue};
/// use fluxpro_engine::models::id_field::IdField;
///
/// let amount = IdField::new("amount").unwrap();
/// let context = ContextMapBuilder::new()
///     .set(amount.clone(), ContextValue::number(100))
///     .build();
/// assert_eq!(context.as_number(&amount), Some(100));
/// assert_eq!(context.as_string(&amount), None);
/// let json = serde_json::to_value(&context).unwrap();
/// assert_eq!(json["amount"]["number"], 100);
/// let decoded: ContextMap = serde_json::from_value(json).unwrap();
/// assert_eq!(decoded, context);
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct ContextMap(pub HashMap<IdField, ContextValue>);

impl ContextMap {
    /// Borrows a value by its exact context key.
    pub fn get(&self, key: &IdField) -> Option<&ContextValue> {
        self.0.get(key)
    }
}

/// A typed context value serialized as a single-key object.
///
/// For example, a string is `{ "string": "approved" }`, not a bare string.
/// The runtime unwraps primitive values when exposing context to Rhai.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
#[serde(rename_all = "lowercase")]
pub enum ContextValue {
    /// A wrapped UTF-8 string.
    String {
        /// UTF-8 string value.
        string: String,
    },
    /// A wrapped validated identifier.
    IdField {
        /// Validated identifier value.
        #[serde(alias = "id_field")]
        id_field: IdField,
    },
    /// A wrapped signed integer.
    Number {
        /// Signed 64-bit integer value.
        number: i64,
    },
    /// A wrapped floating-point number.
    Float {
        /// Double-precision floating-point value.
        float: f64,
    },
    /// A wrapped calendar date.
    Date {
        /// Calendar date without a time zone.
        date: NaiveDate,
    },
    /// A sequence of typed values.
    Array {
        /// Ordered sequence of typed context values.
        array: Vec<ContextValue>,
    },
    /// A wrapped UTC timestamp.
    DateTime {
        /// UTC timestamp value.
        #[serde(alias = "date_time")]
        datetime: DateTime<Utc>,
    },
    /// A wrapped boolean.
    Boolean {
        /// Boolean value.
        boolean: bool,
    },
    /// A wrapped arbitrary JSON value.
    Object {
        /// Arbitrary JSON value stored under the object wrapper.
        object: Value,
    },
}

impl ContextValue {
    /// Wraps a string value for typed context serialization.
    pub fn string(value: String) -> Self {
        Self::String { string: value }
    }

    /// Wraps a id field value for typed context serialization.
    pub fn id_field(value: IdField) -> Self {
        Self::IdField { id_field: value }
    }

    /// Wraps a number value for typed context serialization.
    pub fn number(value: i64) -> Self {
        Self::Number { number: value }
    }

    /// Wraps a float value for typed context serialization.
    pub fn float(value: f64) -> Self {
        Self::Float { float: value }
    }

    /// Wraps a date value for typed context serialization.
    pub fn date(value: NaiveDate) -> Self {
        Self::Date { date: value }
    }

    /// Wraps a array value for typed context serialization.
    pub fn array(value: Vec<ContextValue>) -> Self {
        Self::Array { array: value }
    }

    /// Wraps a date time value for typed context serialization.
    pub fn date_time(value: DateTime<Utc>) -> Self {
        Self::DateTime { datetime: value }
    }

    /// Wraps a boolean value for typed context serialization.
    pub fn boolean(value: bool) -> Self {
        Self::Boolean { boolean: value }
    }

    /// Wraps a object value for typed context serialization.
    pub fn object(value: Value) -> Self {
        Self::Object { object: value }
    }
}

impl ContextMap {
    /// Applies patch operations in order to this in-memory context.
    pub fn apply_patcher(&mut self, patcher: &ContextPatcher) {
        patcher.ops().iter().for_each(|op| match op {
            ContextPatcherOp::Set { key, value } => {
                if self.0.contains_key(&key) {
                    self.0.remove(&key);
                }
                self.0.insert(key.clone(), value.clone());
            }
            ContextPatcherOp::Remove { key } => {
                self.0.remove(&key);
            }
        })
    }

    /// Returns the string value, or `None` for an absent key or different type.
    pub fn as_string(&self, key: &IdField) -> Option<String> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::String { string } => Some(string.clone()),
            _ => None,
        })
    }

    /// Returns the number value, or `None` for an absent key or different type.
    pub fn as_number(&self, key: &IdField) -> Option<i64> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Number { number } => Some(number.clone()),
            _ => None,
        })
    }

    /// Returns the id field value, or `None` for an absent key or different type.
    pub fn as_id_field(&self, key: &IdField) -> Option<IdField> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::IdField { id_field } => Some(id_field.clone()),
            _ => None,
        })
    }

    /// Returns the float value, or `None` for an absent key or different type.
    pub fn as_float(&self, key: &IdField) -> Option<f64> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Float { float } => Some(float.clone()),
            _ => None,
        })
    }

    /// Returns the date value, or `None` for an absent key or different type.
    pub fn as_date(&self, key: &IdField) -> Option<NaiveDate> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Date { date } => Some(date.clone()),
            _ => None,
        })
    }

    /// Returns the date time value, or `None` for an absent key or different type.
    pub fn as_date_time(&self, key: &IdField) -> Option<DateTime<Utc>> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::DateTime { datetime } => Some(datetime.clone()),
            _ => None,
        })
    }

    /// Returns the bool value, or `None` for an absent key or different type.
    pub fn as_bool(&self, key: &IdField) -> Option<bool> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Boolean { boolean } => Some(boolean.clone()),
            _ => None,
        })
    }

    /// Returns the object value, or `None` for an absent key or different type.
    pub fn as_object(&self, key: &IdField) -> Option<Value> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Object { object } => Some(object.clone()),
            _ => None,
        })
    }

    /// Returns the array value, or `None` for an absent key or different type.
    pub fn as_array(&self, key: &IdField) -> Option<Vec<ContextValue>> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Array { array } => Some(array.clone()),
            _ => None,
        })
    }
}

/// Fluent builder that inserts or replaces typed context values.
pub struct ContextMapBuilder {
    context_map: ContextMap,
}

impl ContextMapBuilder {
    /// Creates an empty context builder.
    pub fn new() -> Self {
        Self {
            context_map: ContextMap::default(),
        }
    }

    /// Starts a builder from an existing context map.
    pub fn from(context_map: ContextMap) -> Self {
        Self { context_map }
    }

    /// Adds a typed value assignment; a later assignment to the same key wins.
    pub fn set(mut self, key: IdField, value: ContextValue) -> Self {
        self.context_map.0.insert(key, value);
        self
    }

    /// Consumes the builder and returns its accumulated value.
    pub fn build(self) -> ContextMap {
        self.context_map
    }
}
