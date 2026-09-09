# Legacy duplicate version repair

Migration 7 requires unique `(key_, version)` pairs. Older releases allowed
multiple different graphs under the same version. Do not delete or merge those
definitions: instances and declarations bind to their individual UUIDs.

The optional `migration-repair` feature extends the existing `migrations` module
with `legacy_versions`. It does not edit migrations or bypass published-row
protection. Hosts expose the API through their existing maintenance CLI:

1. Stop all old runners and definition writers for the repair/upgrade window.
   Back up the database before upgrading.
2. Call `plan_legacy_versions(pool)`. This is a read-only consistent snapshot.
3. Persist the returned `LegacyVersionRepairPlan` as restricted-access JSON in
   a new file. It contains `changes` and complete `original_rows` for all affected
   keys, including original JSON/YAML. Review the assignments before proceeding.
4. Call `apply_legacy_version_plan(pool, &reviewed_plan)`, then `migrations::migrate(pool)`.
5. Start the new runners only after the schema upgrade succeeds.

Build a separate plan in every environment. The apply operation verifies the
database name and complete definition snapshots under a table write lock; a
stale plan fails rather than being silently recalculated. Waiting for the lock
is bounded to 10 seconds. All row changes commit or roll back together.

For every duplicate group, the original number is retained by the currently
eligible active definition with the highest registration index. If none is
eligible, the highest index wins. Creation time and UUID break index ties
deterministically. Legacy selection with tied indices was ambiguous. Apply
rechecks the choice at its current time to catch eligibility changes since preview.

Other group members receive numeric versions above the highest version of the
same key, taking all existing versions into account. Allocation uses numeric
`VersionId` ordering, incrementing patch and carrying overflow into minor/major.
The original SQL version, JSON root version and YAML root version must agree.
Those three fields change together. Generic YAML parsing preserves unknown
fields and editor positions, but reserialization may change formatting/comments;
the exact original text remains in the plan. Invalid source data causes preview
to fail before any changes. Graphs are not revalidated against newer runtime rules.

Definition UUIDs, keys, indices, lifecycle labels/dates and all child/instance
bindings are preserved. Default runtime selection still uses registration order,
not the numerically highest version. Explicit requests for the old version now
refer only to the retained UUID; automatic switching between the old duplicates
as eligibility windows change is no longer possible. New publication must use
a version higher than every number reserved during the repair.

Repeating apply returns `AlreadyApplied` if rows still match the exact planned
result, even after migration 7. If later publications or lifecycle changes have
occurred, generate a new plan; a healthy database produces no changes. The plan
is an original-definition backup, not a replacement for a full database backup.
Never restore duplicate identities after migration 7 without a coordinated
database rollback: its uniqueness/immutability constraints remain enforced.

The API does not create tables, rewrite migration history, alter instance data,
or automatically run schema migrations. PostgreSQL integration coverage is in
`tests/legacy_version_repair.rs`:

```sh
cargo test --features runtime,migration-repair --test legacy_version_repair -- --ignored
```

Use a disposable PostgreSQL `DATABASE_URL` for that suite. It covers the failed
legacy upgrade, repair and successful upgrade, preserved bindings/graphs/YAML,
stale and modified plans, invalid YAML, rollback after a mid-repair SQL error,
concurrent apply, repeated apply after migration, and clean databases.
