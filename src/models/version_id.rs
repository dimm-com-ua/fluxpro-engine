//! Normalized three-component process versions with numeric ordering.

use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::Display;
use std::str::FromStr;

/// A normalized `major.minor.patch` version with numeric comparison.
///
/// Input may start with `v`; serialization removes that prefix. Prerelease and
/// build suffixes are not accepted.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::version_id::VersionId;
///
/// let version = VersionId::new("v1.10.0").unwrap();
/// assert_eq!(version.as_str(), "1.10.0");
/// assert!(version > VersionId::new("1.2.9").unwrap());
/// assert!(VersionId::new("1.0.0-beta").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionId {
    raw: String,
    major: u32,
    minor: u32,
    patch: u32,
}

impl VersionId {
    /// Parses an optional `v` prefix and three unsigned 32-bit version components.
    ///
    /// # Errors
    ///
    /// Rejects malformed versions and components that overflow `u32`.
    pub fn new<S: AsRef<str>>(id: S) -> Result<Self, String> {
        let re = Regex::new(r"^v?(\d+)\.(\d+)\.(\d+)$").unwrap();
        let s = id.as_ref();
        let caps = re
            .captures(s)
            .ok_or_else(|| format!("Invalid version id: {}", s))?;
        let major = caps
            .get(1)
            .unwrap()
            .as_str()
            .parse::<u32>()
            .map_err(|_| format!("Invalid major version id: {}", s))?;
        let minor = caps
            .get(2)
            .unwrap()
            .as_str()
            .parse::<u32>()
            .map_err(|_| format!("Invalid minor version id: {}", s))?;
        let patch = caps
            .get(3)
            .unwrap()
            .as_str()
            .parse::<u32>()
            .map_err(|_| format!("Invalid patch version id: {}", s))?;
        Ok(Self {
            raw: format!("{}.{}.{}", major, minor, patch),
            major,
            minor,
            patch,
        })
    }

    /// Returns the major version component.
    #[inline]
    pub fn major(&self) -> u32 {
        self.major
    }
    /// Returns the minor version component.
    #[inline]
    pub fn minor(&self) -> u32 {
        self.minor
    }
    /// Returns the patch version component.
    #[inline]
    pub fn patch(&self) -> u32 {
        self.patch
    }
    /// Returns the normalized string representation.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl Display for VersionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl FromStr for VersionId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl PartialOrd for VersionId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl Serialize for VersionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}
impl<'de> Deserialize<'de> for VersionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl AsRef<str> for VersionId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
