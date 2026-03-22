use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
            SchemaChange::ChangeType { from, to, .. } => !is_safe_type_widening(from, to),
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
            let from_type = normalize_type(&deployed_col.data_type);
            let to_type = normalize_type(&col.data_type);

            if from_type != to_type {
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

/// Normalize type strings for comparison (case-insensitive, strip aliases).
fn normalize_type(type_str: &str) -> String {
    type_str.to_uppercase().trim().to_string()
}

/// Check if a type change is a safe widening (no data loss).
fn is_safe_type_widening(from: &str, to: &str) -> bool {
    let from_upper = from.to_uppercase();
    let to_upper = to.to_uppercase();

    matches!(
        (from_upper.as_str(), to_upper.as_str()),
        // Integer widenings
        ("SMALLINT", "INTEGER")
            | ("SMALLINT", "BIGINT")
            | ("INTEGER", "BIGINT")
            // Float widenings
            | ("FLOAT", "DOUBLE")
            // String widenings (VARCHAR to TEXT, smaller to larger VARCHAR)
            | ("VARCHAR", "TEXT")
            | ("CHAR", "VARCHAR")
            | ("CHAR", "TEXT")
    ) || is_varchar_widening(&from_upper, &to_upper)
        || is_decimal_widening(&from_upper, &to_upper)
}

/// Check if a VARCHAR(N) -> VARCHAR(M) where M > N.
fn is_varchar_widening(from: &str, to: &str) -> bool {
    let from_len = parse_varchar_length(from);
    let to_len = parse_varchar_length(to);

    match (from_len, to_len) {
        // VARCHAR(N) -> VARCHAR (unbounded) is always safe
        (Some(_), None) if to.starts_with("VARCHAR") => true,
        // VARCHAR(N) -> VARCHAR(M) where M > N
        (Some(n), Some(m)) => m > n,
        _ => false,
    }
}

fn parse_varchar_length(type_str: &str) -> Option<u32> {
    let s = type_str.trim();
    if let Some(inner) = s.strip_prefix("VARCHAR(").and_then(|s| s.strip_suffix(')')) {
        inner.trim().parse().ok()
    } else {
        None
    }
}

/// Check if DECIMAL(P1,S1) -> DECIMAL(P2,S2) where P2>=P1 and S2>=S1.
fn is_decimal_widening(from: &str, to: &str) -> bool {
    let from_params = parse_decimal_params(from);
    let to_params = parse_decimal_params(to);

    match (from_params, to_params) {
        (Some((p1, s1)), Some((p2, s2))) => p2 >= p1 && s2 >= s1,
        _ => false,
    }
}

fn parse_decimal_params(type_str: &str) -> Option<(u8, u8)> {
    let s = type_str.trim();
    let inner = s
        .strip_prefix("DECIMAL(")
        .and_then(|s| s.strip_suffix(')'))?;
    let parts: Vec<&str> = inner.split(',').collect();
    match parts.len() {
        1 => {
            let p = parts[0].trim().parse().ok()?;
            Some((p, 0))
        }
        2 => {
            let p = parts[0].trim().parse().ok()?;
            let s = parts[1].trim().parse().ok()?;
            Some((p, s))
        }
        _ => None,
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
pub fn plan_migration(
    schema: &str,
    table: &str,
    diff: &SchemaDiff,
    allow_column_removal: bool,
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

    // Check if full refresh is needed
    if diff.requires_full_refresh() {
        let reasons: Vec<String> = diff
            .changes
            .iter()
            .filter_map(|c| match c {
                SchemaChange::AddColumn {
                    name,
                    nullable: false,
                    ..
                } => Some(format!("NOT NULL column '{}' added", name)),
                SchemaChange::ChangeType { name, from, to, .. } => {
                    if !is_safe_type_widening(from, to) {
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
                } => Some(format!("column '{}' changed to NOT NULL", name)),
                _ => None,
            })
            .collect();

        return MigrationAction::FullRefresh {
            reason: reasons.join("; "),
        };
    }

    // Generate ALTER TABLE statements for safe changes
    let qualified_table = format!("{}.{}", schema, table);
    let mut statements = Vec::new();

    for change in &diff.changes {
        match change {
            SchemaChange::AddColumn {
                name, data_type, ..
            } => {
                statements.push(format!(
                    "ALTER TABLE {} ADD COLUMN {} {}",
                    qualified_table, name, data_type
                ));
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
                to_nullable: false, ..
            } => {
                // Handled by full refresh above
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
        assert!(is_safe_type_widening("INTEGER", "BIGINT"));
        assert!(is_safe_type_widening("SMALLINT", "INTEGER"));
        assert!(is_safe_type_widening("FLOAT", "DOUBLE"));
        assert!(is_safe_type_widening("VARCHAR", "TEXT"));
        assert!(is_safe_type_widening("VARCHAR(50)", "VARCHAR(100)"));
        assert!(is_safe_type_widening("VARCHAR(50)", "VARCHAR"));
        assert!(is_safe_type_widening("DECIMAL(10,2)", "DECIMAL(12,2)"));
        assert!(is_safe_type_widening("DECIMAL(10,2)", "DECIMAL(10,4)"));
    }

    #[test]
    fn test_unsafe_type_change() {
        assert!(!is_safe_type_widening("BIGINT", "INTEGER")); // narrowing
        assert!(!is_safe_type_widening("VARCHAR(100)", "VARCHAR(50)")); // narrowing
        assert!(!is_safe_type_widening("INTEGER", "VARCHAR")); // incompatible
        assert!(!is_safe_type_widening("DECIMAL(12,2)", "DECIMAL(10,2)")); // narrowing
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
        let action = plan_migration("main", "my_table", &diff, false);
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
        let action = plan_migration("main", "my_table", &diff, false);
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
        let action = plan_migration("main", "my_table", &diff, false);
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
        let action = plan_migration("main", "my_table", &diff, true);
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
        let action = plan_migration("main", "my_table", &diff, false);
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
        let action = plan_migration("main", "my_table", &diff, false);
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
}
