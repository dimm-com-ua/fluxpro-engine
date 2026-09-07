use crate::models::context_map::context_patcher::{ContextPatcher, ContextPatcherOp};
use crate::models::id_field::IdField;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct ContextMap(pub HashMap<IdField, ContextValue>);

impl ContextMap {
    pub fn get(&self, key: &IdField) -> Option<&ContextValue> {
        self.0.get(key)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
#[serde(rename_all = "lowercase")]
pub enum ContextValue {
    String {
        string: String,
    },
    IdField {
        #[serde(alias = "id_field")]
        id_field: IdField,
    },
    Number {
        number: i64,
    },
    Float {
        float: f64,
    },
    Date {
        date: NaiveDate,
    },
    Array {
        array: Vec<ContextValue>,
    },
    DateTime {
        #[serde(alias = "date_time")]
        datetime: DateTime<Utc>,
    },
    Boolean {
        boolean: bool,
    },
    Object {
        object: Value,
    },
}

impl ContextValue {
    pub fn string(value: String) -> Self {
        Self::String { string: value }
    }

    pub fn id_field(value: IdField) -> Self {
        Self::IdField { id_field: value }
    }

    pub fn number(value: i64) -> Self {
        Self::Number { number: value }
    }

    pub fn float(value: f64) -> Self {
        Self::Float { float: value }
    }

    pub fn date(value: NaiveDate) -> Self {
        Self::Date { date: value }
    }

    pub fn array(value: Vec<ContextValue>) -> Self {
        Self::Array { array: value }
    }

    pub fn date_time(value: DateTime<Utc>) -> Self {
        Self::DateTime { datetime: value }
    }

    pub fn boolean(value: bool) -> Self {
        Self::Boolean { boolean: value }
    }

    pub fn object(value: Value) -> Self {
        Self::Object { object: value }
    }
}

impl ContextMap {
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

    pub fn as_string(&self, key: &IdField) -> Option<String> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::String { string } => Some(string.clone()),
            _ => None,
        })
    }

    pub fn as_number(&self, key: &IdField) -> Option<i64> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Number { number } => Some(number.clone()),
            _ => None,
        })
    }

    pub fn as_id_field(&self, key: &IdField) -> Option<IdField> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::IdField { id_field } => Some(id_field.clone()),
            _ => None,
        })
    }

    pub fn as_float(&self, key: &IdField) -> Option<f64> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Float { float } => Some(float.clone()),
            _ => None,
        })
    }

    pub fn as_date(&self, key: &IdField) -> Option<NaiveDate> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Date { date } => Some(date.clone()),
            _ => None,
        })
    }

    pub fn as_date_time(&self, key: &IdField) -> Option<DateTime<Utc>> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::DateTime { datetime } => Some(datetime.clone()),
            _ => None,
        })
    }

    pub fn as_bool(&self, key: &IdField) -> Option<bool> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Boolean { boolean } => Some(boolean.clone()),
            _ => None,
        })
    }

    pub fn as_object(&self, key: &IdField) -> Option<Value> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Object { object } => Some(object.clone()),
            _ => None,
        })
    }

    pub fn as_array(&self, key: &IdField) -> Option<Vec<ContextValue>> {
        self.0.get(key).and_then(|v| match v {
            ContextValue::Array { array } => Some(array.clone()),
            _ => None,
        })
    }
}

pub struct ContextMapBuilder {
    context_map: ContextMap,
}

impl ContextMapBuilder {
    pub fn new() -> Self {
        Self {
            context_map: ContextMap::default(),
        }
    }

    pub fn from(context_map: ContextMap) -> Self {
        Self { context_map }
    }

    pub fn set(mut self, key: IdField, value: ContextValue) -> Self {
        self.context_map.0.insert(key, value);
        self
    }

    pub fn build(self) -> ContextMap {
        self.context_map
    }
}
