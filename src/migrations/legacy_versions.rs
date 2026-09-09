//! Opt-in maintenance before migration 7. Never deletes or merges definitions.
//!
//! Plans contain original rows (including YAML) for review and backup. Apply
//! verifies the complete affected-key snapshot under a write lock. Runners and
//! definition writers must be stopped for the entire repair/upgrade window.

use crate::models::version_id::VersionId;
use anyhow::{Context, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Reviewable reassignment. The retained member of each group is not changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyVersionChange {
    /// Definition identity; instance and declaration bindings keep this UUID.
    pub uuid: Uuid,
    /// Logical process key.
    pub key: String,
    /// Previously ambiguous version.
    pub old_version: String,
    /// Newly reserved, normalized numeric version.
    pub new_version: String,
    /// Definition retaining the original version in this group.
    pub retained_uuid: Uuid,
}

/// Environment-specific plan and original-row backup. Store with restricted access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyVersionRepairPlan {
    /// Plan schema version; unknown versions are refused.
    pub format_version: u32,
    /// Database name, checked on apply as well as the row snapshots.
    pub database: String,
    /// Time used when choosing the currently eligible member of a duplicate group.
    pub observed_at: DateTime<Utc>,
    /// All definitions of affected keys, including non-duplicates, in UUID order.
    /// Exact original JSON/YAML and metadata are retained for audit/recovery.
    pub original_rows: Vec<Value>,
    /// Deterministic changes, recomputed and verified before apply.
    pub changes: Vec<LegacyVersionChange>,
}

/// Idempotent apply result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyVersionRepairOutcome {
    /// The transaction changed this many definitions.
    Applied(usize),
    /// The exact requested changes were already applied, or the plan was empty.
    AlreadyApplied,
}

#[derive(Deserialize)]
struct DefinitionRow {
    uuid: Uuid,
    key_: String,
    version: String,
    index_id: i32,
    created_at: DateTime<Utc>,
    status: String,
    effective_from: DateTime<Utc>,
    deprecated_at: Option<DateTime<Utc>>,
    definition: Value,
    source_definition: Option<String>,
}

impl DefinitionRow {
    fn eligible(&self, at: DateTime<Utc>) -> bool {
        self.status.eq_ignore_ascii_case("active")
            && self.effective_from <= at
            && self.deprecated_at.is_none_or(|end| end > at)
    }
}

/// Reads a consistent snapshot without changing the schema or any process data.
pub async fn plan_legacy_versions(pool: &PgPool) -> anyhow::Result<LegacyVersionRepairPlan> {
    let mut tx = pool.begin().await?;
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await?;
    let database: String = sqlx::query_scalar("select current_database()")
        .fetch_one(&mut *tx)
        .await?;
    let observed_at = sqlx::query_scalar("select transaction_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let exists: bool =
        sqlx::query_scalar("select to_regclass('fluxpro.process_definition') is not null")
            .fetch_one(&mut *tx)
            .await?;
    let original_rows = if exists {
        sqlx::query_scalar::<_, Value>(
            "select to_jsonb(d) from fluxpro.process_definition d
             where key_ in (select key_ from fluxpro.process_definition
                            group by key_, version having count(*) > 1)
             order by d.uuid",
        )
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };
    let changes = assignments(&original_rows, observed_at)?;
    // Validate YAML and identity consistency during preview, before any writes.
    repaired_rows(&original_rows, &changes)?;
    tx.commit().await?;
    Ok(LegacyVersionRepairPlan {
        format_version: 1,
        database,
        observed_at,
        original_rows,
        changes,
    })
}

/// Applies only the reviewed version fields in a single transaction.
///
/// Rejects a stale/tampered plan and a changed currently eligible group member.
/// Repeating the same plan is safe, including after migration 7 has succeeded.
/// This does not run migrations or disable published-definition protections.
pub async fn apply_legacy_version_plan(
    pool: &PgPool,
    plan: &LegacyVersionRepairPlan,
) -> anyhow::Result<LegacyVersionRepairOutcome> {
    ensure!(plan.format_version == 1, "Unsupported repair plan format");
    ensure!(
        assignments(&plan.original_rows, plan.observed_at)? == plan.changes,
        "Repair plan assignments were modified; generate a new plan"
    );
    let expected = repaired_rows(&plan.original_rows, &plan.changes)?;
    let mut tx = pool.begin().await?;
    let database: String = sqlx::query_scalar("select current_database()")
        .fetch_one(&mut *tx)
        .await?;
    ensure!(
        database == plan.database,
        "Repair plan belongs to another database"
    );
    if plan.changes.is_empty() {
        let exists: bool =
            sqlx::query_scalar("select to_regclass('fluxpro.process_definition') is not null")
                .fetch_one(&mut *tx)
                .await?;
        if exists {
            ensure!(
                !has_duplicates(&mut tx).await?,
                "New duplicates exist; generate a new plan"
            );
        }
        tx.commit().await?;
        return Ok(LegacyVersionRepairOutcome::AlreadyApplied);
    }
    sqlx::query("set local lock_timeout = '10s'")
        .execute(&mut *tx)
        .await?;
    // Conflicts with legacy writers too; advisory locks alone cannot protect
    // this upgrade because old releases did not acquire them.
    sqlx::query("lock table fluxpro.process_definition in share row exclusive mode")
        .execute(&mut *tx)
        .await?;
    let keys: Vec<_> = plan
        .original_rows
        .iter()
        .filter_map(|row| row["key_"].as_str())
        .collect();
    let current: Vec<Value> = sqlx::query_scalar(
        "select to_jsonb(d) from fluxpro.process_definition d where key_=any($1) order by d.uuid",
    )
    .bind(keys)
    .fetch_all(&mut *tx)
    .await?;
    if current == expected {
        ensure!(
            !has_duplicates(&mut tx).await?,
            "Other duplicates exist; generate a new plan"
        );
        tx.commit().await?;
        return Ok(LegacyVersionRepairOutcome::AlreadyApplied);
    }
    ensure!(
        current == plan.original_rows,
        "Definitions changed since preview; generate a new plan. Nothing was changed"
    );
    let now = sqlx::query_scalar("select clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    ensure!(
        assignments(&current, now)? == plan.changes,
        "The eligible definition changed since preview; generate a new plan"
    );
    for change in &plan.changes {
        let row = expected
            .iter()
            .find(|row| row["uuid"].as_str() == Some(&change.uuid.to_string()))
            .context("Missing repaired row")?;
        let result = sqlx::query(
            "update fluxpro.process_definition set version=$2, definition=$3, source_definition=$4 where uuid=$1",
        ).bind(change.uuid).bind(&change.new_version).bind(&row["definition"])
            .bind(row["source_definition"].as_str()).execute(&mut *tx).await?;
        ensure!(
            result.rows_affected() == 1,
            "Definition disappeared during repair"
        );
    }
    ensure!(
        !has_duplicates(&mut tx).await?,
        "Other duplicates exist; transaction rolled back. Generate a new plan"
    );
    tx.commit().await?;
    Ok(LegacyVersionRepairOutcome::Applied(plan.changes.len()))
}

async fn has_duplicates(connection: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("select exists(select 1 from fluxpro.process_definition group by key_, version having count(*) > 1)")
        .fetch_one(connection).await
}

fn successor(version: &VersionId) -> anyhow::Result<VersionId> {
    let (major, minor, patch) = (version.major(), version.minor(), version.patch());
    let next = if let Some(patch) = patch.checked_add(1) {
        format!("{major}.{minor}.{patch}")
    } else if let Some(minor) = minor.checked_add(1) {
        format!("{major}.{minor}.0")
    } else if let Some(major) = major.checked_add(1) {
        format!("{major}.0.0")
    } else {
        bail!("No free numeric version above {version}");
    };
    VersionId::new(next).map_err(anyhow::Error::msg)
}

fn assignments(rows: &[Value], at: DateTime<Utc>) -> anyhow::Result<Vec<LegacyVersionChange>> {
    let mut groups: BTreeMap<(String, String), Vec<DefinitionRow>> = BTreeMap::new();
    let mut maxima: BTreeMap<String, VersionId> = BTreeMap::new();
    let mut identities = std::collections::BTreeSet::new();
    for value in rows {
        let row: DefinitionRow =
            serde_json::from_value(value.clone()).context("Invalid original definition row")?;
        ensure!(identities.insert(row.uuid), "Repeated UUID in repair plan");
        let version = VersionId::new(&row.version)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("Invalid stored version for {}", row.uuid))?;
        maxima
            .entry(row.key_.clone())
            .and_modify(|max| *max = max.clone().max(version.clone()))
            .or_insert(version);
        groups
            .entry((row.key_.clone(), row.version.clone()))
            .or_default()
            .push(row);
    }
    let mut result = Vec::new();
    for ((key, old_version), mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        // Preserve explicit-version selection at preview time. When no member
        // is eligible, retain the latest registered one. Resolve legacy index
        // ties deterministically using creation time and UUID.
        group.sort_by_key(|row| {
            std::cmp::Reverse((row.eligible(at), row.index_id, row.created_at, row.uuid))
        });
        let retained_uuid = group[0].uuid;
        for row in group.into_iter().skip(1) {
            let max = maxima.get_mut(&key).expect("key maximum exists");
            *max = successor(max)?;
            result.push(LegacyVersionChange {
                uuid: row.uuid,
                key: key.clone(),
                old_version: old_version.clone(),
                new_version: max.to_string(),
                retained_uuid,
            });
        }
    }
    Ok(result)
}

fn repaired_rows(rows: &[Value], changes: &[LegacyVersionChange]) -> anyhow::Result<Vec<Value>> {
    let mut result = rows.to_vec();
    for value in &mut result {
        let row: DefinitionRow = serde_json::from_value(value.clone())?;
        let Some(change) = changes.iter().find(|change| change.uuid == row.uuid) else {
            continue;
        };
        ensure!(
            row.definition.is_object(),
            "Definition {} is not a JSON object",
            row.uuid
        );
        ensure!(
            row.definition["key"].as_str() == Some(&row.key_),
            "JSON key mismatch for {}",
            row.uuid
        );
        let json_version = row.definition["version"]
            .as_str()
            .context("Missing JSON version")?;
        ensure!(
            VersionId::new(json_version).map_err(anyhow::Error::msg)?
                == VersionId::new(&row.version).map_err(anyhow::Error::msg)?,
            "JSON version mismatch for {}",
            row.uuid
        );
        value["version"] = change.new_version.clone().into();
        value["definition"]["version"] = change.new_version.clone().into();
        if let Some(source) = row.source_definition {
            // Parse as a generic YAML value: old node syntax and unknown editor
            // metadata must survive without current ProcessDefinition validation.
            let mut yaml: serde_yaml::Value = serde_yaml::from_str(&source)
                .with_context(|| format!("Invalid source YAML for {}", row.uuid))?;
            let mapping = yaml
                .as_mapping_mut()
                .with_context(|| format!("Source YAML for {} is not a mapping", row.uuid))?;
            ensure!(
                mapping
                    .get(serde_yaml::Value::String("key".into()))
                    .and_then(|v| v.as_str())
                    == Some(&row.key_),
                "YAML key mismatch for {}",
                row.uuid
            );
            let old = mapping
                .get(serde_yaml::Value::String("version".into()))
                .and_then(|v| v.as_str())
                .with_context(|| format!("Missing YAML version for {}", row.uuid))?;
            ensure!(
                VersionId::new(old).map_err(anyhow::Error::msg)?
                    == VersionId::new(&row.version).map_err(anyhow::Error::msg)?,
                "YAML version mismatch for {}",
                row.uuid
            );
            mapping.insert(
                serde_yaml::Value::String("version".into()),
                serde_yaml::Value::String(change.new_version.clone()),
            );
            value["source_definition"] = serde_yaml::to_string(&yaml)?.into();
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numeric_successor_carries_and_rejects_exhaustion() {
        for (from, to) in [
            ("1.9.9", "1.9.10"),
            ("1.9.4294967295", "1.10.0"),
            ("1.4294967295.4294967295", "2.0.0"),
        ] {
            assert_eq!(
                successor(&VersionId::new(from).unwrap()).unwrap().as_str(),
                to
            );
        }
        assert!(successor(&VersionId::new("4294967295.4294967295.4294967295").unwrap()).is_err());
    }

    #[test]
    fn tied_legacy_indices_use_deterministic_identity_and_keep_null_source() {
        let at = DateTime::parse_from_rfc3339("2026-09-09T00:00:00Z")
            .unwrap()
            .to_utc();
        let rows: Vec<_> = (1..=3).map(|id| json!({
            "uuid": Uuid::from_u128(id), "key_": "flow", "version": "1.0.0", "index_id": 1,
            "created_at": at, "status": "Active", "effective_from": "2020-01-01T00:00:00Z",
            "deprecated_at": null, "definition": {"key":"flow", "version":"1.0.0", "nodes":[]},
            "source_definition": null
        })).collect();
        let changes = assignments(&rows, at).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(
            changes
                .iter()
                .all(|c| c.retained_uuid == Uuid::from_u128(3))
        );
        let mut reversed = rows.clone();
        reversed.reverse();
        assert_eq!(changes, assignments(&reversed, at).unwrap());
        assert!(
            repaired_rows(&rows, &changes)
                .unwrap()
                .iter()
                .all(|r| r["source_definition"].is_null())
        );
        let mut mismatch = rows;
        mismatch[0]["definition"]["version"] = "2.0.0".into();
        assert!(
            repaired_rows(&mismatch, &changes)
                .unwrap_err()
                .to_string()
                .contains("JSON version mismatch")
        );
    }
}
