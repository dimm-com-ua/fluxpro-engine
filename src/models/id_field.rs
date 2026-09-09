//! Validated identifiers for definitions, nodes, signals, and instance tokens.

use rand::Rng;
use rand::distributions::Alphanumeric;
use regex::Regex;
use serde::ser::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use uuid::Uuid;

/// A string identifier serialized as a plain JSON or YAML string.
///
/// [`Self::new`] and deserialization enforce `[a-zA-Z_][a-zA-Z0-9_]*`.
/// The derived default is empty and bypasses validation; use `new` for input.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::id_field::IdField;
///
/// let id = IdField::new("example_id").unwrap();
/// assert_eq!(serde_json::to_string(&id).unwrap(), "\"example_id\"");
/// assert!(IdField::new("invalid-id").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Default)]
#[serde(transparent)]
pub struct IdField(String);
type Err = fmt::Error;

fn random_string(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .filter(|c| c.is_ascii_alphabetic())
        .map(char::from)
        .take(len)
        .collect()
}

impl IdField {
    /// Validates a string identifier.
    ///
    /// # Errors
    ///
    /// Returns a formatting error when the input does not match the identifier grammar.
    pub fn new(id: impl Into<String>) -> Result<Self, Err> {
        let re = Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
        let id = id.into();
        if re.is_match(&id) {
            Ok(Self(id))
        } else {
            let error = format!(
                "Invalid id_field '{}': must match [a-zA-Z_][a-zA-Z0-9_]*",
                id
            );
            Err(Error::custom(error))
        }
    }

    /// Generates an identifier from six random letters and a UUID suffix.
    pub fn generate() -> Self {
        let random_chars = random_string(6);
        Self(format!(
            "{}_{}",
            random_chars,
            Uuid::new_v4().simple().to_string()
        ))
    }

    /// Generates an alphabetic identifier of the requested length.
    ///
    /// A zero length produces an empty value and bypasses identifier validation.
    pub fn with_length(length: usize) -> Self {
        Self(random_string(length))
    }
    /// Generates a six-letter identifier without a uniqueness guarantee.
    pub fn short() -> Self {
        Self(random_string(6))
    }

    /// Borrows the underlying identifier string.
    pub fn get_id(&self) -> &str {
        &self.0
    }
}

impl TryFrom<Box<dyn ToString>> for IdField {
    type Error = ();

    fn try_from(value: Box<dyn ToString>) -> Result<Self, Self::Error> {
        let s = value.to_string();
        IdField::new(s.as_str()).map_err(|_| ())
    }
}

impl<'de> Deserialize<'de> for IdField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        IdField::new(s.as_str()).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for IdField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
