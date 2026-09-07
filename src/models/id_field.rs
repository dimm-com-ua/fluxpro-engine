use rand::Rng;
use rand::distributions::Alphanumeric;
use regex::Regex;
use serde::ser::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use uuid::Uuid;

/// A newtype wrapper for a string, representing an identifier field.
///
/// The `IdField` struct is a lightweight wrapper around a `String` type
/// used to designate it as an identifier in the context where it's used.
/// It derives several useful traits including `Debug`, `Clone`, `PartialEq`,
/// `Eq`, `Hash`, and `Default`, making it compatible with hashing,
/// serialization, equality checks, and default initialization.
///
/// # Derivable Traits
///
/// - `Debug`: Allows formatting the `id_field` for debugging purposes.
/// - `Clone`: Enables creating duplicate instances of the `id_field`.
/// - `PartialEq` and `Eq`: Enables equality comparisons between `id_field` values.
/// - `Hash`: Makes the `id_field` usable in hashed collections like `HashMap` or `HashSet`.
/// - `Default`: Provides a default value of `IdField` which initializes its inner string as an empty string.
///
/// # Serialization
///
/// The `#[serde(transparent)]` attribute indicates that during serialization and
/// deserialization (e.g., when using Serde), the wrapped string is treated
/// transparently as if the wrapper type did not exist.
/// For example, `IdField::new("example".to_string())` would serialize just as `"example"`.
///
/// # Example Usage
///
/// ```
/// use fluxpro_engine::models::id_field::IdField;
/// use serde_json;
///
/// let id = IdField::new("example_id").unwrap();
/// let serialized = serde_json::to_string(&id).unwrap();
/// assert_eq!(serialized, "\"example_id\"");
///
/// let deserialized: IdField = serde_json::from_str(&serialized).unwrap();
/// assert_eq!(id, deserialized);
/// ```
///
/// This struct is useful for clearly indicating fields that represent
/// identifiers while still leveraging the underlying capabilities of
/// `String`.
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

    pub fn generate() -> Self {
        let random_chars = random_string(6);
        Self(format!(
            "{}_{}",
            random_chars,
            Uuid::new_v4().simple().to_string()
        ))
    }

    pub fn with_length(length: usize) -> Self {
        Self(random_string(length))
    }
    pub fn short() -> Self {
        Self(random_string(6))
    }

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
