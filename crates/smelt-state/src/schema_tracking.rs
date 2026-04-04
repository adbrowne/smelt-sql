use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smelt_types::{parse_type, DataType};
use std::collections::HashMap;

/// Persisted schema for a deployed model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployedSchema {
    pub model: String,
    pub version: u32,
    pub deployed_at: DateTime<Utc>,
    pub model_hash: String,
    pub columns: Vec<DeployedColumn>,
}

/// A column in a deployed schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeployedColumn {
    pub name: String,
    /// SQL type string (e.g., "INTEGER", "VARCHAR", "DECIMAL(10,2)")
    #[serde(rename = "type")]
    pub data_type: String,
    pub nullable: bool,
}

/// What kind of schema change was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaChange {
    AddColumn {
        name: String,
        data_type: String,
        nullable: bool,
    },
    RemoveColumn {
        name: String,
    },
    ChangeType {
        name: String,
        from: String,
        to: String,
    },
    ChangeNullability {
        name: String,
        from_nullable: bool,
        to_nullable: bool,
    },
}

/// Result of comparing deployed schema to inferred schema.
#[derive(Debug, Clone)]
pub struct SchemaDiff {
    pub changes: Vec<SchemaChange>,
}

impl SchemaDiff {
    /// No schema changes detected.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Whether the diff requires a full refresh (data-lossy changes).
    pub fn requires_full_refresh(&self) -> bool {
        self.changes.iter().any(|c| match c {
            // Adding a NOT NULL column requires full refresh (can't have NULL default)
            SchemaChange::AddColumn { nullable, .. } => !nullable,
            // Removing columns is allowed with flag, doesn't require refresh
            SchemaChange::RemoveColumn { .. } => false,
            // Type changes: check if it's a safe widening
            SchemaChange::ChangeType { from, to, .. } => !is_safe_type_widening_str(from, to),
            // Making nullable -> NOT NULL requires full refresh
            SchemaChange::ChangeNullability {
                to_nullable: false, ..
            } => true,
            // Making NOT NULL -> nullable is safe
            SchemaChange::ChangeNullability {
                to_nullable: true, ..
            } => false,
        })
    }

    /// Whether the diff includes column removals (requires --allow-column-removal).
    pub fn has_column_removals(&self) -> bool {
        self.changes
            .iter()
            .any(|c| matches!(c, SchemaChange::RemoveColumn { .. }))
    }

    /// Get a human-readable summary of changes.
    pub fn summary(&self) -> Vec<String> {
        self.changes
            .iter()
            .map(|c| match c {
                SchemaChange::AddColumn {
                    name,
                    data_type,
                    nullable,
                } => {
                    let null_str = if *nullable { "NULL" } else { "NOT NULL" };
                    format!("ADD COLUMN {} {} {}", name, data_type, null_str)
                }
                SchemaChange::RemoveColumn { name } => format!("DROP COLUMN {}", name),
                SchemaChange::ChangeType { name, from, to } => {
                    format!("ALTER COLUMN {} TYPE {} -> {}", name, from, to)
                }
                SchemaChange::ChangeNullability {
                    name,
                    from_nullable,
                    to_nullable,
                } => {
                    let from_str = if *from_nullable { "NULL" } else { "NOT NULL" };
                    let to_str = if *to_nullable { "NULL" } else { "NOT NULL" };
                    format!("ALTER COLUMN {} {} -> {}", name, from_str, to_str)
                }
            })
            .collect()
    }
}

/// Compare a deployed schema against a new (inferred) schema to detect changes.
pub fn diff_schemas(deployed: &[DeployedColumn], inferred: &[DeployedColumn]) -> SchemaDiff {
    let mut changes = Vec::new();

    // Build lookup maps
    let deployed_map: std::collections::HashMap<&str, &DeployedColumn> =
        deployed.iter().map(|c| (c.name.as_str(), c)).collect();
    let inferred_map: std::collections::HashMap<&str, &DeployedColumn> =
        inferred.iter().map(|c| (c.name.as_str(), c)).collect();

    // Check for added columns (in inferred but not in deployed)
    for col in inferred {
        if !deployed_map.contains_key(col.name.as_str()) {
            changes.push(SchemaChange::AddColumn {
                name: col.name.clone(),
                data_type: col.data_type.clone(),
                nullable: col.nullable,
            });
        }
    }

    // Check for removed columns (in deployed but not in inferred)
    for col in deployed {
        if !inferred_map.contains_key(col.name.as_str()) {
            changes.push(SchemaChange::RemoveColumn {
                name: col.name.clone(),
            });
        }
    }

    // Check for type/nullability changes on existing columns
    for col in inferred {
        if let Some(deployed_col) = deployed_map.get(col.name.as_str()) {
            let from_normalized = normalize_type(&deployed_col.data_type);
            let to_normalized = normalize_type(&col.data_type);

            if !normalized_types_equal(&from_normalized, &to_normalized) {
                changes.push(SchemaChange::ChangeType {
                    name: col.name.clone(),
                    from: deployed_col.data_type.clone(),
                    to: col.data_type.clone(),
                });
            }

            if deployed_col.nullable != col.nullable {
                changes.push(SchemaChange::ChangeNullability {
                    name: col.name.clone(),
                    from_nullable: deployed_col.nullable,
                    to_nullable: col.nullable,
                });
            }
        }
    }

    SchemaDiff { changes }
}

/// Result of attempting to parse and normalize a type string.
///
/// If parsing succeeds, we get a normalized `DataType` for structural comparison.
/// If parsing fails, we fall back to uppercase string comparison (forward compat).
enum NormalizedType {
    Parsed(DataType),
    Unparsed(String),
}

/// Parse and normalize a SQL type string for comparison.
///
/// Tries to parse the string into a `DataType` via `parse_type()`, then
/// normalizes aliases (e.g., Text → Varchar). Falls back to uppercase string
/// comparison for types that can't be parsed (forward compatibility).
fn normalize_type(type_str: &str) -> NormalizedType {
    match parse_type(type_str) {
        Ok(dt) => NormalizedType::Parsed(dt.normalize()),
        Err(_) => NormalizedType::Unparsed(type_str.to_uppercase().trim().to_string()),
    }
}

/// Compare two normalized types for equality.
fn normalized_types_equal(a: &NormalizedType, b: &NormalizedType) -> bool {
    match (a, b) {
        (NormalizedType::Parsed(da), NormalizedType::Parsed(db)) => da == db,
        (NormalizedType::Unparsed(sa), NormalizedType::Unparsed(sb)) => sa == sb,
        // One parsed, one didn't — they're different
        _ => false,
    }
}

/// Check if a type change is a safe widening (no data loss).
///
/// Accepts `DataType` values for structural comparison. For Phase 2, this handles
/// scalar types only. Phase 4 will add recursive rules for Array/Map/Struct.
fn is_safe_type_widening(from: &DataType, to: &DataType) -> bool {
    match (from, to) {
        // Integer widenings
        (DataType::SmallInt, DataType::Integer) => true,
        (DataType::SmallInt, DataType::BigInt) => true,
        (DataType::Integer, DataType::BigInt) => true,

        // Float widenings
        (DataType::Float, DataType::Double) => true,

        // String widenings
        (DataType::Varchar { .. } | DataType::Text, DataType::Text) => true,
        (DataType::Char { .. }, DataType::Varchar { .. } | DataType::Text) => true,
        (
            DataType::Varchar {
                max_length: Some(_),
            },
            DataType::Varchar { max_length: None },
        ) => true,
        (
            DataType::Varchar {
                max_length: Some(from_len),
            },
            DataType::Varchar {
                max_length: Some(to_len),
            },
        ) => to_len > from_len,

        // Decimal widenings: precision and scale must not decrease
        (
            DataType::Decimal {
                precision: p1,
                scale: s1,
            },
            DataType::Decimal {
                precision: p2,
                scale: s2,
            },
        ) => p2 >= p1 && s2 >= s1 && (p2 > p1 || s2 > s1),

        _ => false,
    }
}

/// Parse a type string and check if it's a safe widening.
///
/// This is a convenience wrapper for callers that have type strings
/// (e.g., from `SchemaChange::ChangeType`). Falls back to `false`
/// if either type fails to parse.
fn is_safe_type_widening_str(from: &str, to: &str) -> bool {
    match (parse_type(from), parse_type(to)) {
        (Ok(from_dt), Ok(to_dt)) => is_safe_type_widening(&from_dt.normalize(), &to_dt.normalize()),
        _ => false,
    }
}

/// What action the executor should take for a schema change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationAction {
    /// No changes needed.
    NoChange,
    /// Apply ALTER TABLE statements, then continue incremental.
    AlterTable { statements: Vec<String> },
    /// Full refresh required (destructive schema change).
    FullRefresh { reason: String },
    /// Column removal detected — requires explicit flag.
    RequiresColumnRemovalFlag { columns: Vec<String> },
}

/// Plan the migration action for a model based on the schema diff.
///
/// `column_defaults` maps column names to SQL literal default values (from frontmatter).
/// When a NOT NULL column is added and has a default, ALTER TABLE with DEFAULT is used
/// instead of triggering a full refresh.
///
/// `backfill_exprs` maps column names to SQL expressions for UPDATE backfill.
pub fn plan_migration(
    schema: &str,
    table: &str,
    diff: &SchemaDiff,
    allow_column_removal: bool,
    column_defaults: &HashMap<String, String>,
    backfill_exprs: &HashMap<String, String>,
) -> MigrationAction {
    if diff.is_empty() {
        return MigrationAction::NoChange;
    }

    // Check for column removals without flag
    if diff.has_column_removals() && !allow_column_removal {
        let removed: Vec<String> = diff
            .changes
            .iter()
            .filter_map(|c| match c {
                SchemaChange::RemoveColumn { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        return MigrationAction::RequiresColumnRemovalFlag { columns: removed };
    }

    // Check if full refresh is needed (considering available defaults)
    let unresolvable_reasons: Vec<String> = diff
        .changes
        .iter()
        .filter_map(|c| match c {
            SchemaChange::AddColumn {
                name,
                nullable: false,
                ..
            } => {
                // NOT NULL column addition is safe if we have a default value
                if column_defaults.contains_key(name.as_str()) {
                    None
                } else {
                    Some(format!(
                        "NOT NULL column '{}' added (no default specified)",
                        name
                    ))
                }
            }
            SchemaChange::ChangeType { name, from, to, .. } => {
                if !is_safe_type_widening_str(from, to) {
                    Some(format!(
                        "unsafe type change on '{}': {} -> {}",
                        name, from, to
                    ))
                } else {
                    None
                }
            }
            SchemaChange::ChangeNullability {
                name,
                to_nullable: false,
                ..
            } => {
                // nullable → NOT NULL is safe if we have a default to fill NULLs
                if column_defaults.contains_key(name.as_str()) {
                    None
                } else {
                    Some(format!(
                        "column '{}' changed to NOT NULL (no default specified)",
                        name
                    ))
                }
            }
            _ => None,
        })
        .collect();

    if !unresolvable_reasons.is_empty() {
        return MigrationAction::FullRefresh {
            reason: unresolvable_reasons.join("; "),
        };
    }

    // Generate ALTER TABLE statements for safe changes
    let qualified_table = format!("{}.{}", schema, table);
    let mut statements = Vec::new();

    for change in &diff.changes {
        match change {
            SchemaChange::AddColumn {
                name,
                data_type,
                nullable,
            } => {
                if *nullable {
                    statements.push(format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        qualified_table, name, data_type
                    ));
                } else if let Some(default_val) = column_defaults.get(name.as_str()) {
                    // NOT NULL with default: use DEFAULT clause
                    statements.push(format!(
                        "ALTER TABLE {} ADD COLUMN {} {} NOT NULL DEFAULT {}",
                        qualified_table, name, data_type, default_val
                    ));
                } else {
                    unreachable!(
                        "NOT NULL column '{}' without default should have triggered FullRefresh",
                        name
                    );
                }
                // Backfill expression for the newly added column
                if let Some(backfill) = backfill_exprs.get(name.as_str()) {
                    statements.push(format!(
                        "UPDATE {} SET {} = {}",
                        qualified_table, name, backfill
                    ));
                }
            }
            SchemaChange::RemoveColumn { name } => {
                statements.push(format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    qualified_table, name
                ));
            }
            SchemaChange::ChangeType { name, to, .. } => {
                statements.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                    qualified_table, name, to
                ));
            }
            SchemaChange::ChangeNullability {
                name,
                to_nullable: true,
                ..
            } => {
                statements.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                    qualified_table, name
                ));
            }
            SchemaChange::ChangeNullability {
                name,
                to_nullable: false,
                ..
            } => {
                // nullable → NOT NULL: fill NULLs with the column's default value, then
                // set NOT NULL. We use column_defaults (not backfill_exprs) here because
                // the goal is to fill NULL gaps with a safe constant — backfill expressions
                // are for recomputing column values from other columns, which is a different
                // semantic (used for new column additions, not nullability changes).
                if let Some(default_val) = column_defaults.get(name.as_str()) {
                    statements.push(format!(
                        "UPDATE {} SET {} = {} WHERE {} IS NULL",
                        qualified_table, name, default_val, name
                    ));
                    statements.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                        qualified_table, name
                    ));
                }
            }
        }
    }

    MigrationAction::AlterTable { statements }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, data_type: &str, nullable: bool) -> DeployedColumn {
        DeployedColumn {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable,
        }
    }

    fn no_defaults() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn test_type_alias_normalization() {
        // INT vs INTEGER should not be detected as a change
        let deployed = vec![col("id", "INT", false)];
        let inferred = vec![col("id", "INTEGER", false)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "INT vs INTEGER should not trigger a change"
        );

        // BOOL vs BOOLEAN
        let deployed = vec![col("active", "BOOL", true)];
        let inferred = vec![col("active", "BOOLEAN", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "BOOL vs BOOLEAN should not trigger a change"
        );

        // INT8 vs BIGINT
        let deployed = vec![col("big_id", "INT8", false)];
        let inferred = vec![col("big_id", "BIGINT", false)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "INT8 vs BIGINT should not trigger a change"
        );

        // TEXT vs VARCHAR (both normalize to VARCHAR)
        let deployed = vec![col("name", "TEXT", true)];
        let inferred = vec![col("name", "VARCHAR", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "TEXT vs VARCHAR should not trigger a change"
        );
    }

    #[test]
    fn test_no_changes() {
        let deployed = vec![col("id", "INTEGER", false), col("name", "VARCHAR", true)];
        let inferred = vec![col("id", "INTEGER", false), col("name", "VARCHAR", true)];

        let diff = diff_schemas(&deployed, &inferred);
        assert!(diff.is_empty());
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_add_nullable_column() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![col("id", "INTEGER", false), col("email", "VARCHAR", true)];

        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(
            &diff.changes[0],
            SchemaChange::AddColumn {
                name,
                nullable: true,
                ..
            } if name == "email"
        ));
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_add_not_null_column_requires_refresh() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![
            col("id", "INTEGER", false),
            col("required_field", "VARCHAR", false),
        ];

        let diff = diff_schemas(&deployed, &inferred);
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_remove_column() {
        let deployed = vec![col("id", "INTEGER", false), col("old_col", "VARCHAR", true)];
        let inferred = vec![col("id", "INTEGER", false)];

        let diff = diff_schemas(&deployed, &inferred);
        assert!(diff.has_column_removals());
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_safe_type_widening() {
        assert!(is_safe_type_widening_str("INTEGER", "BIGINT"));
        assert!(is_safe_type_widening_str("SMALLINT", "INTEGER"));
        assert!(is_safe_type_widening_str("FLOAT", "DOUBLE"));
        // VARCHAR and TEXT now normalize to the same type, so diff_schemas won't
        // produce a ChangeType for them. Test Varchar(N) → Text widening instead.
        assert!(is_safe_type_widening_str("VARCHAR(50)", "TEXT"));
        assert!(is_safe_type_widening_str("VARCHAR(50)", "VARCHAR(100)"));
        assert!(is_safe_type_widening_str("VARCHAR(50)", "VARCHAR"));
        assert!(is_safe_type_widening_str("DECIMAL(10,2)", "DECIMAL(12,2)"));
        assert!(is_safe_type_widening_str("DECIMAL(10,2)", "DECIMAL(10,4)"));
    }

    #[test]
    fn test_unsafe_type_change() {
        assert!(!is_safe_type_widening_str("BIGINT", "INTEGER")); // narrowing
        assert!(!is_safe_type_widening_str("VARCHAR(100)", "VARCHAR(50)")); // narrowing
        assert!(!is_safe_type_widening_str("INTEGER", "VARCHAR")); // incompatible
        assert!(!is_safe_type_widening_str("DECIMAL(12,2)", "DECIMAL(10,2)")); // narrowing
    }

    #[test]
    fn test_type_change_safe_widening() {
        let deployed = vec![col("amount", "INTEGER", true)];
        let inferred = vec![col("amount", "BIGINT", true)];

        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_type_change_unsafe() {
        let deployed = vec![col("amount", "BIGINT", true)];
        let inferred = vec![col("amount", "INTEGER", true)];

        let diff = diff_schemas(&deployed, &inferred);
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_plan_migration_no_change() {
        let diff = SchemaDiff { changes: vec![] };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        assert_eq!(action, MigrationAction::NoChange);
    }

    #[test]
    fn test_plan_migration_add_nullable_column() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "email".to_string(),
                data_type: "VARCHAR".to_string(),
                nullable: true,
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.my_table ADD COLUMN email VARCHAR"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_remove_column_without_flag() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::RemoveColumn {
                name: "old_col".to_string(),
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        assert!(matches!(
            action,
            MigrationAction::RequiresColumnRemovalFlag { .. }
        ));
    }

    #[test]
    fn test_plan_migration_remove_column_with_flag() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::RemoveColumn {
                name: "old_col".to_string(),
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            true,
            &no_defaults(),
            &no_defaults(),
        );
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.my_table DROP COLUMN old_col"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_full_refresh() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "required".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        assert!(matches!(action, MigrationAction::FullRefresh { .. }));
    }

    #[test]
    fn test_plan_migration_mixed_changes() {
        let diff = SchemaDiff {
            changes: vec![
                SchemaChange::AddColumn {
                    name: "email".to_string(),
                    data_type: "VARCHAR".to_string(),
                    nullable: true,
                },
                SchemaChange::ChangeType {
                    name: "amount".to_string(),
                    from: "INTEGER".to_string(),
                    to: "BIGINT".to_string(),
                },
            ],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 2);
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_deployed_schema_serialization() {
        let schema = DeployedSchema {
            model: "daily_revenue".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:abc123".to_string(),
            columns: vec![
                col("order_date", "DATE", false),
                col("total", "DECIMAL(10,2)", true),
            ],
        };

        let json = serde_json::to_string_pretty(&schema).unwrap();
        let deserialized: DeployedSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.model, "daily_revenue");
        assert_eq!(deserialized.columns.len(), 2);
        assert_eq!(deserialized.columns[0].name, "order_date");
    }

    #[test]
    fn test_diff_summary() {
        let diff = SchemaDiff {
            changes: vec![
                SchemaChange::AddColumn {
                    name: "email".to_string(),
                    data_type: "VARCHAR".to_string(),
                    nullable: true,
                },
                SchemaChange::RemoveColumn {
                    name: "old_col".to_string(),
                },
                SchemaChange::ChangeType {
                    name: "amount".to_string(),
                    from: "INTEGER".to_string(),
                    to: "BIGINT".to_string(),
                },
            ],
        };

        let summary = diff.summary();
        assert_eq!(summary.len(), 3);
        assert!(summary[0].contains("ADD COLUMN email"));
        assert!(summary[1].contains("DROP COLUMN old_col"));
        assert!(summary[2].contains("ALTER COLUMN amount"));
    }

    // --- Schema evolution with defaults ---

    #[test]
    fn test_plan_migration_not_null_column_with_default() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "status".to_string(),
                data_type: "VARCHAR".to_string(),
                nullable: false,
            }],
        };
        let mut defaults = HashMap::new();
        defaults.insert("status".to_string(), "'unknown'".to_string());

        let action = plan_migration("main", "my_table", &diff, false, &defaults, &no_defaults());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.my_table ADD COLUMN status VARCHAR NOT NULL DEFAULT 'unknown'"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_not_null_column_without_default_requires_refresh() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "status".to_string(),
                data_type: "VARCHAR".to_string(),
                nullable: false,
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        assert!(matches!(action, MigrationAction::FullRefresh { .. }));
    }

    #[test]
    fn test_plan_migration_nullable_to_not_null_with_default() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::ChangeNullability {
                name: "priority".to_string(),
                from_nullable: true,
                to_nullable: false,
            }],
        };
        let mut defaults = HashMap::new();
        defaults.insert("priority".to_string(), "0".to_string());

        let action = plan_migration("main", "my_table", &diff, false, &defaults, &no_defaults());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 2);
                assert_eq!(
                    statements[0],
                    "UPDATE main.my_table SET priority = 0 WHERE priority IS NULL"
                );
                assert_eq!(
                    statements[1],
                    "ALTER TABLE main.my_table ALTER COLUMN priority SET NOT NULL"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_nullable_to_not_null_without_default_requires_refresh() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::ChangeNullability {
                name: "priority".to_string(),
                from_nullable: true,
                to_nullable: false,
            }],
        };
        let action = plan_migration(
            "main",
            "my_table",
            &diff,
            false,
            &no_defaults(),
            &no_defaults(),
        );
        assert!(matches!(action, MigrationAction::FullRefresh { .. }));
    }

    #[test]
    fn test_plan_migration_add_column_with_backfill() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "full_name".to_string(),
                data_type: "VARCHAR".to_string(),
                nullable: true,
            }],
        };
        let mut backfills = HashMap::new();
        backfills.insert(
            "full_name".to_string(),
            "COALESCE(first_name || ' ' || last_name, '')".to_string(),
        );

        let action = plan_migration("main", "my_table", &diff, false, &no_defaults(), &backfills);
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 2);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.my_table ADD COLUMN full_name VARCHAR"
                );
                assert_eq!(
                    statements[1],
                    "UPDATE main.my_table SET full_name = COALESCE(first_name || ' ' || last_name, '')"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_not_null_with_default_and_backfill() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "status".to_string(),
                data_type: "VARCHAR".to_string(),
                nullable: false,
            }],
        };
        let mut defaults = HashMap::new();
        defaults.insert("status".to_string(), "'pending'".to_string());
        let mut backfills = HashMap::new();
        backfills.insert(
            "status".to_string(),
            "CASE WHEN completed THEN 'done' ELSE 'pending' END".to_string(),
        );

        let action = plan_migration("main", "my_table", &diff, false, &defaults, &backfills);
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 2);
                assert!(statements[0].contains("DEFAULT 'pending'"));
                assert!(statements[1].contains("UPDATE"));
                assert!(statements[1].contains("CASE WHEN"));
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    // === Phase 2: Complex type normalization in diff_schemas ===

    #[test]
    fn test_complex_type_alias_normalization_struct() {
        // STRUCT(a INT, b BOOL) vs STRUCT(a INTEGER, b BOOLEAN) — aliases, no real change
        let deployed = vec![col("meta", "STRUCT(a INT, b BOOL)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER, b BOOLEAN)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "STRUCT(a INT, b BOOL) vs STRUCT(a INTEGER, b BOOLEAN) should not trigger a change"
        );
    }

    #[test]
    fn test_complex_type_alias_normalization_array() {
        // INT[] vs INTEGER[] — alias, no real change
        let deployed = vec![col("tags", "INT[]", true)];
        let inferred = vec![col("tags", "INTEGER[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "INT[] vs INTEGER[] should not trigger a change"
        );
    }

    #[test]
    fn test_complex_type_alias_normalization_map() {
        // MAP(STRING, INT) vs MAP(VARCHAR, INTEGER) — aliases
        let deployed = vec![col("lookup", "MAP(STRING, INT)", true)];
        let inferred = vec![col("lookup", "MAP(VARCHAR, INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "MAP(STRING, INT) vs MAP(VARCHAR, INTEGER) should not trigger a change"
        );
    }

    #[test]
    fn test_complex_type_alias_normalization_text_varchar() {
        // STRUCT(a TEXT) vs STRUCT(a VARCHAR) — Text/Varchar normalization
        let deployed = vec![col("meta", "STRUCT(a TEXT)", true)];
        let inferred = vec![col("meta", "STRUCT(a VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "STRUCT(a TEXT) vs STRUCT(a VARCHAR) should not trigger a change"
        );
    }

    #[test]
    fn test_complex_type_alias_normalization_nested_struct() {
        // STRUCT(a STRUCT(x INT8)) vs STRUCT(a STRUCT(x BIGINT)) — nested alias
        let deployed = vec![col("data", "STRUCT(a STRUCT(x INT8))", true)];
        let inferred = vec![col("data", "STRUCT(a STRUCT(x BIGINT))", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.is_empty(),
            "STRUCT(a STRUCT(x INT8)) vs STRUCT(a STRUCT(x BIGINT)) should not trigger a change"
        );
    }

    #[test]
    fn test_complex_type_real_change_detected() {
        // STRUCT(a INTEGER) vs STRUCT(a BIGINT) — real widening, should detect change
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(
            diff.changes.len(),
            1,
            "STRUCT(a INTEGER) vs STRUCT(a BIGINT) should detect change"
        );
        assert!(matches!(&diff.changes[0], SchemaChange::ChangeType { .. }));
    }

    #[test]
    fn test_complex_type_array_widening_detected() {
        // INTEGER[] vs BIGINT[] — real change (widening)
        let deployed = vec![col("scores", "INTEGER[]", true)];
        let inferred = vec![col("scores", "BIGINT[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(
            diff.changes.len(),
            1,
            "INTEGER[] vs BIGINT[] should detect change"
        );
    }

    #[test]
    fn test_is_safe_type_widening_with_datatype() {
        // Basic scalar widenings
        assert!(is_safe_type_widening(
            &DataType::SmallInt,
            &DataType::Integer
        ));
        assert!(is_safe_type_widening(
            &DataType::SmallInt,
            &DataType::BigInt
        ));
        assert!(is_safe_type_widening(&DataType::Integer, &DataType::BigInt));
        assert!(is_safe_type_widening(&DataType::Float, &DataType::Double));

        // String widenings
        assert!(is_safe_type_widening(
            &DataType::Varchar { max_length: None },
            &DataType::Text
        ));
        assert!(is_safe_type_widening(
            &DataType::Char { length: 10 },
            &DataType::Varchar { max_length: None }
        ));
        assert!(is_safe_type_widening(
            &DataType::Varchar {
                max_length: Some(50)
            },
            &DataType::Varchar {
                max_length: Some(100)
            }
        ));
        assert!(is_safe_type_widening(
            &DataType::Varchar {
                max_length: Some(50)
            },
            &DataType::Varchar { max_length: None }
        ));

        // Decimal widenings
        assert!(is_safe_type_widening(
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
            &DataType::Decimal {
                precision: 12,
                scale: 2
            }
        ));
        assert!(is_safe_type_widening(
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
            &DataType::Decimal {
                precision: 10,
                scale: 4
            }
        ));

        // Unsafe
        assert!(!is_safe_type_widening(
            &DataType::BigInt,
            &DataType::Integer
        )); // narrowing
        assert!(!is_safe_type_widening(
            &DataType::Varchar {
                max_length: Some(100)
            },
            &DataType::Varchar {
                max_length: Some(50)
            }
        )); // narrowing
        assert!(!is_safe_type_widening(
            &DataType::Integer,
            &DataType::Varchar { max_length: None }
        )); // incompatible
    }

    #[test]
    fn test_plan_migration_mixed_with_some_defaults() {
        // One NOT NULL column with default + one safe type widening
        let diff = SchemaDiff {
            changes: vec![
                SchemaChange::AddColumn {
                    name: "priority".to_string(),
                    data_type: "INTEGER".to_string(),
                    nullable: false,
                },
                SchemaChange::ChangeType {
                    name: "amount".to_string(),
                    from: "INTEGER".to_string(),
                    to: "BIGINT".to_string(),
                },
            ],
        };
        let mut defaults = HashMap::new();
        defaults.insert("priority".to_string(), "0".to_string());

        let action = plan_migration("main", "my_table", &diff, false, &defaults, &no_defaults());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 2);
                assert!(statements[0].contains("NOT NULL DEFAULT 0"));
                assert!(statements[1].contains("ALTER COLUMN amount TYPE BIGINT"));
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }
}
