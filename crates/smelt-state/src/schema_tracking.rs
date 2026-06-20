use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smelt_dialect::BackendCapabilities;
use smelt_types::{parse_type, DataType};
use std::collections::HashMap;

use crate::ddl_spark::{self, MigrationExecution, SparkTableFormat};

/// SQL keywords that must be quoted when used as identifiers.
const SQL_KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASE",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CURRENT",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXISTS",
    "FALSE",
    "FETCH",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "GROUP",
    "HAVING",
    "IF",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "PRIMARY",
    "REFERENCES",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TIME",
    "TO",
    "TRUE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "WHEN",
    "WHERE",
    "WITH",
];

/// Quote a SQL identifier if it needs quoting.
///
/// Returns the identifier double-quoted if it:
/// - Contains spaces or special characters (anything other than `[a-zA-Z0-9_]`)
/// - Is a SQL keyword (case-insensitive)
/// - Starts with a digit
///
/// Otherwise returns the identifier as-is.
pub fn quote_identifier(ident: &str) -> String {
    let needs_quoting = ident.is_empty()
        || ident.starts_with(|c: char| c.is_ascii_digit())
        || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || SQL_KEYWORDS.iter().any(|kw| kw.eq_ignore_ascii_case(ident));

    if needs_quoting {
        format!("\"{}\"", ident.replace('"', "\"\""))
    } else {
        ident.to_string()
    }
}

/// Quote a dot-separated path of identifiers, quoting each segment individually.
pub fn quote_dot_path(path: &str) -> String {
    path.split('.')
        .map(quote_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

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
    /// A field was added to a struct column (possibly nested).
    StructFieldAdded {
        column: String,
        path: Vec<String>,
        field_name: String,
        field_type: String,
        nullable: bool,
    },
    /// A field was removed from a struct column (possibly nested).
    StructFieldRemoved {
        column: String,
        path: Vec<String>,
        field_name: String,
    },
    /// A type changed inside a nested struct field.
    NestedTypeChange {
        column: String,
        path: Vec<String>,
        from: String,
        to: String,
    },
    /// The element type of an array column changed.
    ArrayElementTypeChange {
        column: String,
        path: Vec<String>,
        from: String,
        to: String,
    },
    /// The key type of a map column changed (always unsafe).
    MapKeyTypeChange {
        column: String,
        from: String,
        to: String,
    },
    /// The value type of a map column changed.
    MapValueTypeChange {
        column: String,
        path: Vec<String>,
        from: String,
        to: String,
    },
    /// Incompatible structural change (e.g., struct→scalar, field reorder).
    IncompatibleTypeChange {
        column: String,
        from: String,
        to: String,
        reason: String,
    },
}

/// Result of comparing deployed schema to inferred schema.
#[derive(Debug, Clone)]
pub struct SchemaDiff {
    pub changes: Vec<SchemaChange>,
    /// Warnings generated during diffing (e.g., unparseable type strings).
    pub warnings: Vec<String>,
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
            // Struct field added: nullable is safe, not nullable requires refresh
            SchemaChange::StructFieldAdded { nullable, .. } => !nullable,
            // Struct field removed: doesn't require refresh (like column removal, needs flag)
            SchemaChange::StructFieldRemoved { .. } => false,
            // Nested type change: safe if widening
            SchemaChange::NestedTypeChange { from, to, .. } => !is_safe_type_widening_str(from, to),
            // Array element type change: safe if widening
            SchemaChange::ArrayElementTypeChange { from, to, .. } => {
                !is_safe_type_widening_str(from, to)
            }
            // Map key type change: always unsafe
            SchemaChange::MapKeyTypeChange { .. } => true,
            // Map value type change: safe if widening
            SchemaChange::MapValueTypeChange { from, to, .. } => {
                !is_safe_type_widening_str(from, to)
            }
            // Incompatible type change: always unsafe
            SchemaChange::IncompatibleTypeChange { .. } => true,
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
                SchemaChange::StructFieldAdded {
                    column,
                    path,
                    field_name,
                    field_type,
                    ..
                } => {
                    let full_path = format_nested_path(column, path, field_name);
                    format!("ADD STRUCT FIELD {} {}", full_path, field_type)
                }
                SchemaChange::StructFieldRemoved {
                    column,
                    path,
                    field_name,
                } => {
                    let full_path = format_nested_path(column, path, field_name);
                    format!("DROP STRUCT FIELD {}", full_path)
                }
                SchemaChange::NestedTypeChange {
                    column,
                    path,
                    from,
                    to,
                } => {
                    let full_path = format_nested_path_no_leaf(column, path);
                    format!("ALTER NESTED TYPE {} {} -> {}", full_path, from, to)
                }
                SchemaChange::ArrayElementTypeChange {
                    column,
                    path,
                    from,
                    to,
                } => {
                    let full_path = format_nested_path_no_leaf(column, path);
                    format!("ALTER ARRAY ELEMENT TYPE {} {} -> {}", full_path, from, to)
                }
                SchemaChange::MapKeyTypeChange {
                    column, from, to, ..
                } => {
                    format!("ALTER MAP KEY TYPE {} {} -> {}", column, from, to)
                }
                SchemaChange::MapValueTypeChange {
                    column,
                    path,
                    from,
                    to,
                } => {
                    let full_path = format_nested_path_no_leaf(column, path);
                    format!("ALTER MAP VALUE TYPE {} {} -> {}", full_path, from, to)
                }
                SchemaChange::IncompatibleTypeChange {
                    column,
                    from,
                    to,
                    reason,
                } => {
                    format!(
                        "INCOMPATIBLE TYPE CHANGE {} {} -> {} ({})",
                        column, from, to, reason
                    )
                }
            })
            .collect()
    }
}

/// Format a nested path like "column.path1.path2.field_name"
fn format_nested_path(column: &str, path: &[String], field_name: &str) -> String {
    let mut parts = vec![column.to_string()];
    parts.extend(path.iter().cloned());
    parts.push(field_name.to_string());
    parts.join(".")
}

/// Format a nested path without a leaf field: "column.path1.path2"
fn format_nested_path_no_leaf(column: &str, path: &[String]) -> String {
    let mut parts = vec![column.to_string()];
    parts.extend(path.iter().cloned());
    parts.join(".")
}

/// Produce fine-grained `SchemaChange` variants by recursively comparing two `DataType` values.
///
/// - Struct vs Struct: detect field additions/removals at end, type changes per field, reordering.
/// - Array vs Array: compare element types recursively.
/// - Map vs Map: compare key types (unsafe if changed) and value types recursively.
/// - Different kinds (struct vs scalar, etc.): `IncompatibleTypeChange`.
/// - Scalar vs Scalar with same normalized type: no changes.
/// - Scalar vs Scalar with different types: `NestedTypeChange` if inside a struct, or the caller handles it.
pub fn diff_types(
    column: &str,
    path: &[String],
    old: &DataType,
    new: &DataType,
) -> Vec<SchemaChange> {
    let old_n = old.normalize();
    let new_n = new.normalize();

    // If normalized types are equal, no change
    if old_n == new_n {
        return vec![];
    }

    match (&old_n, &new_n) {
        (DataType::Struct(old_fields), DataType::Struct(new_fields)) => {
            diff_struct_fields(column, path, old_fields, new_fields, old, new)
        }
        (DataType::Array(old_elem), DataType::Array(new_elem)) => {
            // Recurse into array element
            let inner = diff_types(column, path, old_elem, new_elem);
            if inner.is_empty() {
                vec![]
            } else {
                // If the inner diff is just scalar changes, emit ArrayElementTypeChange
                // For nested struct changes inside arrays, pass them through
                let mut result = vec![];
                for change in inner {
                    match change {
                        // A scalar-level change at the element level → ArrayElementTypeChange
                        SchemaChange::NestedTypeChange {
                            column: c,
                            path: p,
                            from,
                            to,
                        } if p == path => {
                            result.push(SchemaChange::ArrayElementTypeChange {
                                column: c,
                                path: path.to_vec(),
                                from,
                                to,
                            });
                        }
                        // Nested struct changes inside array elements pass through
                        other => result.push(other),
                    }
                }
                result
            }
        }
        (DataType::Map(old_k, old_v), DataType::Map(new_k, new_v)) => {
            let mut changes = vec![];
            let old_k_n = old_k.normalize();
            let new_k_n = new_k.normalize();
            if old_k_n != new_k_n {
                changes.push(SchemaChange::MapKeyTypeChange {
                    column: column.to_string(),
                    from: old_k.to_sql(),
                    to: new_k.to_sql(),
                });
            }
            let old_v_n = old_v.normalize();
            let new_v_n = new_v.normalize();
            if old_v_n != new_v_n {
                // If value is a complex type, recurse structurally
                if old_v.is_complex() || new_v.is_complex() {
                    let mut value_path = path.to_vec();
                    value_path.push("value".to_string());
                    let inner = diff_types(column, &value_path, old_v, new_v);
                    changes.extend(inner);
                } else {
                    // Simple scalar value type change
                    changes.push(SchemaChange::MapValueTypeChange {
                        column: column.to_string(),
                        path: path.to_vec(),
                        from: old_v.to_sql(),
                        to: new_v.to_sql(),
                    });
                }
            }
            changes
        }
        // Different structural kinds → incompatible
        _ => {
            // Check if one is complex and the other isn't, or different complex kinds
            let old_is_struct = matches!(old_n, DataType::Struct(_));
            let new_is_struct = matches!(new_n, DataType::Struct(_));
            let old_is_array = matches!(old_n, DataType::Array(_));
            let new_is_array = matches!(new_n, DataType::Array(_));
            let old_is_map = matches!(old_n, DataType::Map(_, _));
            let new_is_map = matches!(new_n, DataType::Map(_, _));

            let old_is_complex = old_is_struct || old_is_array || old_is_map;
            let new_is_complex = new_is_struct || new_is_array || new_is_map;

            if old_is_complex || new_is_complex {
                let reason = if old_is_struct && !new_is_struct {
                    "struct to non-struct".to_string()
                } else if !old_is_struct && new_is_struct {
                    "non-struct to struct".to_string()
                } else if old_is_array && !new_is_array {
                    "array to non-array".to_string()
                } else if !old_is_array && new_is_array {
                    "non-array to array".to_string()
                } else {
                    format!("{} to {}", old.to_sql(), new.to_sql())
                };
                vec![SchemaChange::IncompatibleTypeChange {
                    column: column.to_string(),
                    from: old.to_sql(),
                    to: new.to_sql(),
                    reason,
                }]
            } else {
                // Both are scalars with different types → NestedTypeChange
                vec![SchemaChange::NestedTypeChange {
                    column: column.to_string(),
                    path: path.to_vec(),
                    from: old.to_sql(),
                    to: new.to_sql(),
                }]
            }
        }
    }
}

/// Compare struct fields in order, detecting additions, removals, reordering, and type changes.
fn diff_struct_fields(
    column: &str,
    path: &[String],
    old_fields: &[(String, DataType)],
    new_fields: &[(String, DataType)],
    old_type: &DataType,
    new_type: &DataType,
) -> Vec<SchemaChange> {
    // Check for field reordering: the common fields must appear in the same order
    let old_names: Vec<&str> = old_fields.iter().map(|(n, _)| n.as_str()).collect();
    let new_names: Vec<&str> = new_fields.iter().map(|(n, _)| n.as_str()).collect();

    // Find common field names preserving order from old
    let old_common: Vec<&str> = old_names
        .iter()
        .filter(|n| new_names.contains(n))
        .copied()
        .collect();
    let new_common: Vec<&str> = new_names
        .iter()
        .filter(|n| old_names.contains(n))
        .copied()
        .collect();

    if old_common != new_common {
        return vec![SchemaChange::IncompatibleTypeChange {
            column: column.to_string(),
            from: old_type.to_sql(),
            to: new_type.to_sql(),
            reason: "struct field order changed".to_string(),
        }];
    }

    let mut changes = vec![];
    let old_map: HashMap<&str, &DataType> =
        old_fields.iter().map(|(n, t)| (n.as_str(), t)).collect();
    let new_map: HashMap<&str, &DataType> =
        new_fields.iter().map(|(n, t)| (n.as_str(), t)).collect();

    // Detect removed fields (in old but not in new)
    for (name, _) in old_fields {
        if !new_map.contains_key(name.as_str()) {
            changes.push(SchemaChange::StructFieldRemoved {
                column: column.to_string(),
                path: path.to_vec(),
                field_name: name.clone(),
            });
        }
    }

    // Detect added fields (in new but not in old) — must be at end after all common fields
    for (name, dt) in new_fields {
        if !old_map.contains_key(name.as_str()) {
            changes.push(SchemaChange::StructFieldAdded {
                column: column.to_string(),
                path: path.to_vec(),
                field_name: name.clone(),
                field_type: dt.to_sql(),
                nullable: true, // struct fields added to existing data are always nullable
            });
        }
    }

    // Detect type changes on common fields
    for (name, old_dt) in old_fields {
        if let Some(new_dt) = new_map.get(name.as_str()) {
            let mut field_path = path.to_vec();
            field_path.push(name.clone());
            let field_changes = diff_types(column, &field_path, old_dt, new_dt);
            changes.extend(field_changes);
        }
    }

    changes
}

/// Compare a deployed schema against a new (inferred) schema to detect changes.
pub fn diff_schemas(deployed: &[DeployedColumn], inferred: &[DeployedColumn]) -> SchemaDiff {
    let mut changes = Vec::new();
    let mut warnings = Vec::new();

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

            // Emit warnings for unparseable types
            if let NormalizedType::Unparsed(_) = &from_normalized {
                warnings.push(format!(
                    "Column '{}': deployed type '{}' could not be parsed; falling back to string comparison",
                    col.name, deployed_col.data_type
                ));
            }
            if let NormalizedType::Unparsed(_) = &to_normalized {
                warnings.push(format!(
                    "Column '{}': inferred type '{}' could not be parsed; falling back to string comparison",
                    col.name, col.data_type
                ));
            }

            if !normalized_types_equal(&from_normalized, &to_normalized) {
                // If both types parse and at least one is complex, use structural diff
                if let (NormalizedType::Parsed(old_dt), NormalizedType::Parsed(new_dt)) =
                    (&from_normalized, &to_normalized)
                {
                    if old_dt.is_complex() || new_dt.is_complex() {
                        // Use the already-parsed normalized types directly (no re-parsing)
                        let structural_changes = diff_types(&col.name, &[], old_dt, new_dt);
                        changes.extend(structural_changes);
                    } else {
                        changes.push(SchemaChange::ChangeType {
                            name: col.name.clone(),
                            from: deployed_col.data_type.clone(),
                            to: col.data_type.clone(),
                        });
                    }
                } else {
                    changes.push(SchemaChange::ChangeType {
                        name: col.name.clone(),
                        from: deployed_col.data_type.clone(),
                        to: col.data_type.clone(),
                    });
                }
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

    SchemaDiff { changes, warnings }
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

        // Array: safe if element type widening is safe
        (DataType::Array(from_elem), DataType::Array(to_elem)) => {
            is_safe_type_widening(from_elem, to_elem)
        }

        // Map: safe if value type widening is safe AND key type is unchanged
        (DataType::Map(from_key, from_val), DataType::Map(to_key, to_val)) => {
            from_key == to_key && is_safe_type_widening(from_val, to_val)
        }

        // Struct changes are NOT widening — they go through diff_types/field-level diff
        (DataType::Struct(_), DataType::Struct(_)) => false,

        _ => false,
    }
}

/// Parse a type string and check if it's a safe widening.
///
/// This is a convenience wrapper for callers that have type strings
/// (e.g., from `SchemaChange::ChangeType`). Falls back to `false`
/// if either type fails to parse.
fn is_safe_type_widening_str(from: &str, to: &str) -> bool {
    // Do NOT normalize here: normalize() collapses VARCHAR(n)/TEXT/CHAR into
    // one canonical type for equality, losing the length information needed to
    // distinguish safe widenings (VARCHAR(50)→VARCHAR(100)) from narrowings.
    // is_safe_type_widening has explicit Text/Char arms that handle raw types.
    match (parse_type(from), parse_type(to)) {
        (Ok(from_dt), Ok(to_dt)) => is_safe_type_widening(&from_dt, &to_dt),
        _ => false,
    }
}

/// A backend-agnostic schema operation that describes "what needs to change"
/// without specifying "how to execute it" (DDL generation is backend-specific).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaOperation {
    /// Add a column to a table (top-level)
    AddColumn {
        name: String,
        data_type: DataType,
        nullable: bool,
        default_expr: Option<String>,
    },
    /// Remove a column from a table (top-level)
    RemoveColumn { name: String },
    /// Widen a top-level column's type
    WidenColumnType {
        name: String,
        from: DataType,
        to: DataType,
    },
    /// Change a column's nullability
    ChangeNullability {
        name: String,
        to_nullable: bool,
        default_expr: Option<String>,
    },
    /// Add a field to a struct column (possibly nested)
    AddStructField {
        column: String,
        path: Vec<String>,
        field_name: String,
        field_type: DataType,
        default_expr: Option<String>,
    },
    /// Remove a field from a struct column
    RemoveStructField {
        column: String,
        path: Vec<String>,
        field_name: String,
    },
    /// Widen a type nested inside a struct/array/map
    WidenNestedType {
        column: String,
        path: Vec<String>,
        from: DataType,
        to: DataType,
    },
    /// Backfill expression for a column
    BackfillColumn { name: String, expression: String },
    /// Full column rewrite using an expression (struct_pack or cast)
    RewriteColumn {
        column: String,
        target_type: DataType,
        using_expr: String,
    },
}

/// A migration plan consisting of backend-agnostic operations.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub operations: Vec<SchemaOperation>,
    pub requires_full_refresh: bool,
    pub full_refresh_reason: Option<String>,
    pub requires_allow_full_refresh: bool,
    pub warnings: Vec<String>,
}

impl MigrationPlan {
    /// Create an empty plan (no changes).
    pub fn empty() -> Self {
        Self {
            operations: vec![],
            requires_full_refresh: false,
            full_refresh_reason: None,
            requires_allow_full_refresh: false,
            warnings: vec![],
        }
    }
}

/// Plan backend-agnostic schema operations from a schema diff.
///
/// Converts `SchemaChange` variants into `SchemaOperation`s. Combines related
/// operations (e.g., multiple struct field changes on the same column → single
/// `RewriteColumn` with a struct_pack expression).
///
/// `defaults` maps column names to SQL default expressions.
/// `backfills` maps column names to SQL backfill expressions.
pub fn plan_schema_operations(
    diff: &SchemaDiff,
    defaults: &HashMap<String, String>,
    backfills: &HashMap<String, String>,
) -> MigrationPlan {
    if diff.is_empty() {
        return MigrationPlan::empty();
    }

    let mut operations = Vec::new();
    let mut full_refresh_reasons = Vec::new();
    let mut warnings = Vec::new();

    // Track columns that need struct rewrites — multiple changes on the same
    // column get combined into a single RewriteColumn.
    let mut struct_rewrite_columns: HashMap<String, Vec<&SchemaChange>> = HashMap::new();

    for change in &diff.changes {
        match change {
            SchemaChange::AddColumn {
                name,
                data_type,
                nullable,
            } => {
                let parsed_type = match parse_type(data_type) {
                    Ok(t) => t,
                    Err(_) => {
                        full_refresh_reasons.push(format!(
                            "could not parse type '{}' for added column '{}' — forcing full refresh",
                            data_type, name
                        ));
                        continue;
                    }
                };
                let default_expr = defaults.get(name.as_str()).cloned();

                if !nullable && default_expr.is_none() {
                    full_refresh_reasons
                        .push(format!("NOT NULL column '{}' added without default", name));
                    continue;
                }

                operations.push(SchemaOperation::AddColumn {
                    name: name.clone(),
                    data_type: parsed_type,
                    nullable: *nullable,
                    default_expr: default_expr.clone(),
                });

                if let Some(backfill) = backfills.get(name.as_str()) {
                    operations.push(SchemaOperation::BackfillColumn {
                        name: name.clone(),
                        expression: backfill.clone(),
                    });
                }
            }
            SchemaChange::RemoveColumn { name } => {
                operations.push(SchemaOperation::RemoveColumn { name: name.clone() });
            }
            SchemaChange::ChangeType { name, from, to } => {
                let from_dt = parse_type(from).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                let to_dt = parse_type(to).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));

                if is_safe_type_widening(&from_dt.normalize(), &to_dt.normalize()) {
                    operations.push(SchemaOperation::WidenColumnType {
                        name: name.clone(),
                        from: from_dt,
                        to: to_dt,
                    });
                } else {
                    full_refresh_reasons.push(format!(
                        "unsafe type change on '{}': {} -> {}",
                        name, from, to
                    ));
                }
            }
            SchemaChange::ChangeNullability {
                name, to_nullable, ..
            } => {
                let default_expr = defaults.get(name.as_str()).cloned();
                if !to_nullable && default_expr.is_none() {
                    full_refresh_reasons.push(format!(
                        "column '{}' changed to NOT NULL without default",
                        name
                    ));
                    continue;
                }
                operations.push(SchemaOperation::ChangeNullability {
                    name: name.clone(),
                    to_nullable: *to_nullable,
                    default_expr,
                });
            }
            // Struct-level changes: collect for possible combination into RewriteColumn
            SchemaChange::StructFieldAdded { column, .. }
            | SchemaChange::StructFieldRemoved { column, .. }
            | SchemaChange::NestedTypeChange { column, .. } => {
                struct_rewrite_columns
                    .entry(column.clone())
                    .or_default()
                    .push(change);
            }
            SchemaChange::ArrayElementTypeChange {
                column, from, to, ..
            } => {
                let from_dt = parse_type(from).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                let to_dt = parse_type(to).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                let from_arr = DataType::Array(Box::new(from_dt));
                let to_arr = DataType::Array(Box::new(to_dt));

                if is_safe_type_widening(&from_arr.normalize(), &to_arr.normalize()) {
                    operations.push(SchemaOperation::WidenColumnType {
                        name: column.clone(),
                        from: from_arr,
                        to: to_arr,
                    });
                } else {
                    full_refresh_reasons.push(format!(
                        "unsafe array element type change on '{}': {} -> {}",
                        column, from, to
                    ));
                }
            }
            SchemaChange::MapKeyTypeChange {
                column, from, to, ..
            } => {
                full_refresh_reasons.push(format!(
                    "map key type changed on '{}': {} -> {}",
                    column, from, to
                ));
            }
            SchemaChange::MapValueTypeChange {
                column, from, to, ..
            } => {
                let from_dt = parse_type(from).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                let to_dt = parse_type(to).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));

                if is_safe_type_widening(&from_dt.normalize(), &to_dt.normalize()) {
                    // Map value widening operates on the whole column
                    let from_map_key = DataType::Varchar { max_length: None }; // placeholder
                    operations.push(SchemaOperation::WidenNestedType {
                        column: column.clone(),
                        path: vec!["value".to_string()],
                        from: from_dt,
                        to: to_dt,
                    });
                    let _ = from_map_key; // suppress unused warning
                } else {
                    full_refresh_reasons.push(format!(
                        "unsafe map value type change on '{}': {} -> {}",
                        column, from, to
                    ));
                }
            }
            SchemaChange::IncompatibleTypeChange {
                column,
                from,
                to,
                reason,
            } => {
                full_refresh_reasons.push(format!(
                    "incompatible type change on '{}': {} -> {} ({})",
                    column, from, to, reason
                ));
            }
        }
    }

    // Process collected struct changes — emit individual operations or combine into RewriteColumn
    for changes in struct_rewrite_columns.values() {
        let has_widen = changes
            .iter()
            .any(|c| matches!(c, SchemaChange::NestedTypeChange { .. }));
        let has_removal = changes
            .iter()
            .any(|c| matches!(c, SchemaChange::StructFieldRemoved { .. }));

        if has_widen || (changes.len() > 1 && has_removal) {
            // Multiple changes or widening on same struct → needs RewriteColumn
            // For now, emit individual operations; Phase 6 will generate the struct_pack expression
            // when translating to DDL.
            for change in changes {
                match change {
                    SchemaChange::NestedTypeChange {
                        column,
                        path,
                        from,
                        to,
                    } => {
                        let from_dt = parse_type(from).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                        let to_dt = parse_type(to).unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
                        if is_safe_type_widening(&from_dt.normalize(), &to_dt.normalize()) {
                            operations.push(SchemaOperation::WidenNestedType {
                                column: column.clone(),
                                path: path.clone(),
                                from: from_dt,
                                to: to_dt,
                            });
                        } else {
                            full_refresh_reasons.push(format!(
                                "unsafe nested type change on '{}': {} -> {}",
                                format_nested_path_no_leaf(column, path),
                                from,
                                to
                            ));
                        }
                    }
                    SchemaChange::StructFieldAdded {
                        column,
                        path,
                        field_name,
                        field_type,
                        ..
                    } => {
                        let parsed_type = match parse_type(field_type) {
                            Ok(t) => t,
                            Err(_) => {
                                full_refresh_reasons.push(format!(
                                    "could not parse type '{}' for added struct field '{}' — forcing full refresh",
                                    field_type, field_name
                                ));
                                continue;
                            }
                        };
                        operations.push(SchemaOperation::AddStructField {
                            column: column.clone(),
                            path: path.clone(),
                            field_name: field_name.clone(),
                            field_type: parsed_type,
                            default_expr: None,
                        });
                    }
                    SchemaChange::StructFieldRemoved {
                        column,
                        path,
                        field_name,
                    } => {
                        operations.push(SchemaOperation::RemoveStructField {
                            column: column.clone(),
                            path: path.clone(),
                            field_name: field_name.clone(),
                        });
                        warnings.push(format!(
                            "struct field '{}' removed from '{}'",
                            field_name,
                            format_nested_path_no_leaf(column, path)
                        ));
                    }
                    _ => {}
                }
            }
        } else {
            // Simple struct changes — emit individually
            for change in changes {
                match change {
                    SchemaChange::StructFieldAdded {
                        column,
                        path,
                        field_name,
                        field_type,
                        ..
                    } => {
                        let parsed_type = match parse_type(field_type) {
                            Ok(t) => t,
                            Err(_) => {
                                full_refresh_reasons.push(format!(
                                    "could not parse type '{}' for added struct field '{}' — forcing full refresh",
                                    field_type, field_name
                                ));
                                continue;
                            }
                        };
                        operations.push(SchemaOperation::AddStructField {
                            column: column.clone(),
                            path: path.clone(),
                            field_name: field_name.clone(),
                            field_type: parsed_type,
                            default_expr: None,
                        });
                    }
                    SchemaChange::StructFieldRemoved {
                        column,
                        path,
                        field_name,
                    } => {
                        operations.push(SchemaOperation::RemoveStructField {
                            column: column.clone(),
                            path: path.clone(),
                            field_name: field_name.clone(),
                        });
                        warnings.push(format!(
                            "struct field '{}' removed from '{}'",
                            field_name,
                            format_nested_path_no_leaf(column, path)
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    let requires_full_refresh = !full_refresh_reasons.is_empty();
    let full_refresh_reason = if requires_full_refresh {
        Some(full_refresh_reasons.join("; "))
    } else {
        None
    };

    MigrationPlan {
        operations,
        requires_full_refresh,
        full_refresh_reason,
        requires_allow_full_refresh: requires_full_refresh,
        warnings,
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
    /// Table rewrite needed (Spark: CREATE TABLE tmp AS SELECT ... FROM original).
    TableRewrite { select_expr: String },
    /// Full refresh required but `--allow-full-refresh` not set — blocked.
    FullRefreshBlocked { reason: String },
}

/// Which backend DDL generator to use for migration planning.
#[derive(Debug, Clone)]
pub enum DdlBackend {
    /// DuckDB — supports struct dot-notation, ALTER COLUMN USING, list_transform.
    DuckDb,
    /// Spark — requires catalog name; limited ALTER support depending on format.
    Spark {
        catalog: String,
        format: SparkTableFormat,
        capabilities: BackendCapabilities,
    },
}

/// Plan the migration action for a model based on the schema diff.
///
/// `column_defaults` maps column names to SQL literal default values (from frontmatter).
/// When a NOT NULL column is added and has a default, ALTER TABLE with DEFAULT is used
/// instead of triggering a full refresh.
///
/// `backfill_exprs` maps column names to SQL expressions for UPDATE backfill.
///
/// `backend` selects the DDL generator. Defaults to DuckDB if `None`.
pub fn plan_migration(
    schema: &str,
    table: &str,
    diff: &SchemaDiff,
    allow_column_removal: bool,
    column_defaults: &HashMap<String, String>,
    backfill_exprs: &HashMap<String, String>,
) -> MigrationAction {
    plan_migration_for_backend(
        schema,
        table,
        diff,
        allow_column_removal,
        column_defaults,
        backfill_exprs,
        &DdlBackend::DuckDb,
        &[],
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
/// Plan the migration action for a specific backend.
///
/// Like `plan_migration`, but allows selecting DuckDB vs Spark DDL generation.
///
/// `deployed_columns` and `inferred_columns` are the original column arrays used
/// to compute the diff. They are needed for DuckDB struct field widening, where the
/// full old/new struct types are required to generate `struct_pack` USING expressions.
pub fn plan_migration_for_backend(
    schema: &str,
    table: &str,
    diff: &SchemaDiff,
    allow_column_removal: bool,
    column_defaults: &HashMap<String, String>,
    backfill_exprs: &HashMap<String, String>,
    backend: &DdlBackend,
    _deployed_columns: &[DeployedColumn],
    inferred_columns: &[DeployedColumn],
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
            SchemaChange::IncompatibleTypeChange {
                column,
                from,
                to,
                reason,
            } => Some(format!(
                "incompatible type change on '{}': {} -> {} ({})",
                column, from, to, reason
            )),
            SchemaChange::MapKeyTypeChange {
                column, from, to, ..
            } => Some(format!(
                "map key type changed on '{}': {} -> {}",
                column, from, to
            )),
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
            // Complex type changes — delegate to backend-specific DDL module.
            SchemaChange::StructFieldAdded { .. }
            | SchemaChange::StructFieldRemoved { .. }
            | SchemaChange::NestedTypeChange { .. }
            | SchemaChange::ArrayElementTypeChange { .. }
            | SchemaChange::MapValueTypeChange { .. } => {
                // For DuckDB, certain complex type changes are best handled by ALTER COLUMN TYPE
                // on the full top-level column — DuckDB auto-casts compatible types inside
                // structs/arrays/maps. This covers:
                // 1. NestedTypeChange (e.g., struct field type widening)
                // 2. ArrayElementTypeChange with non-empty path (array inside a struct)
                // 3. StructFieldAdded with non-empty path (struct field inside an array)
                // DuckDB doesn't support dot-notation ALTER COLUMN TYPE for struct fields,
                // and can't ADD COLUMN on array sub-elements.
                if matches!(backend, DdlBackend::DuckDb) {
                    let column_name = match change {
                        SchemaChange::NestedTypeChange { column, .. } => Some(column.as_str()),
                        SchemaChange::ArrayElementTypeChange { column, path, .. }
                            if !path.is_empty() =>
                        {
                            Some(column.as_str())
                        }
                        SchemaChange::StructFieldAdded { column, path, .. } if !path.is_empty() => {
                            // Struct field add inside a nested path (e.g., struct inside a struct)
                            // DuckDB can't use dot-notation ADD COLUMN on nested sub-paths
                            Some(column.as_str())
                        }
                        SchemaChange::StructFieldAdded { column, path, .. } if path.is_empty() => {
                            // Check if the column is an array/map type — DuckDB can't use
                            // dot-notation ADD COLUMN on array/map columns, only on structs.
                            let col_type = inferred_columns
                                .iter()
                                .find(|c| c.name == *column)
                                .and_then(|c| parse_type(&c.data_type).ok());
                            if matches!(col_type, Some(DataType::Array(_) | DataType::Map(_, _))) {
                                Some(column.as_str())
                            } else {
                                None // Top-level struct: use dot-notation ADD COLUMN
                            }
                        }
                        _ => None,
                    };

                    if let Some(col_name) = column_name {
                        let new_type_str = inferred_columns
                            .iter()
                            .find(|c| c.name == col_name)
                            .map(|c| c.data_type.as_str());

                        if let Some(new_str) = new_type_str {
                            if let Ok(new_dt) = parse_type(new_str) {
                                // Deduplicate: if we already emitted ALTER COLUMN TYPE for this
                                // column, skip (multiple changes on same column → single ALTER).
                                let alter_stmt = format!(
                                    "ALTER TABLE {}.{} ALTER COLUMN {} TYPE {}",
                                    schema,
                                    table,
                                    col_name,
                                    new_dt.to_sql()
                                );
                                if !statements.contains(&alter_stmt) {
                                    statements.push(alter_stmt);
                                }
                                continue;
                            }
                        }
                    }
                    // Fallback: if we couldn't look up types, use plan_schema_operations
                }

                // Convert this single change into SchemaOperations, then generate DDL.
                let single_diff = SchemaDiff {
                    changes: vec![change.clone()],
                    warnings: vec![],
                };
                let plan = plan_schema_operations(&single_diff, column_defaults, backfill_exprs);

                match backend {
                    DdlBackend::DuckDb => {
                        let ddl_stmts =
                            crate::ddl_duckdb::generate_duckdb_ddl(schema, table, &plan.operations);
                        statements.extend(ddl_stmts);
                    }
                    DdlBackend::Spark {
                        catalog,
                        format,
                        capabilities,
                    } => {
                        let result = ddl_spark::generate_spark_ddl(
                            catalog,
                            schema,
                            table,
                            &plan.operations,
                            *format,
                            capabilities,
                        );
                        match result {
                            MigrationExecution::Statements(stmts) => {
                                statements.extend(stmts);
                            }
                            MigrationExecution::TableRewrite { select_expr } => {
                                return MigrationAction::TableRewrite { select_expr };
                            }
                            MigrationExecution::FullRefreshRequired { reason } => {
                                return MigrationAction::FullRefreshBlocked { reason };
                            }
                            MigrationExecution::MergeSchemaWrite { .. } => {
                                // MergeSchemaWrite means the write itself handles the evolution.
                                // No DDL statements needed; the caller should set mergeSchema option.
                                // For now, treat as no-op DDL (the write path handles it).
                            }
                        }
                    }
                }
            }
            SchemaChange::MapKeyTypeChange { .. } | SchemaChange::IncompatibleTypeChange { .. } => {
                // These should have been caught by requires_full_refresh() above.
                // If we reach here, it means the caller is explicitly allowing them.
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
        let diff = SchemaDiff {
            changes: vec![],
            warnings: vec![],
        };
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
            warnings: vec![],
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
        // Phase 3: now produces NestedTypeChange instead of flat ChangeType
        assert!(matches!(
            &diff.changes[0],
            SchemaChange::NestedTypeChange { .. }
        ));
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
            warnings: vec![],
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

    // === Phase 4: Safe widening rules for nested types ===

    #[test]
    fn test_array_element_widening_safe() {
        // Array(Integer) -> Array(BigInt) is safe
        assert!(is_safe_type_widening(
            &DataType::Array(Box::new(DataType::Integer)),
            &DataType::Array(Box::new(DataType::BigInt))
        ));
        // Array(Float) -> Array(Double) is safe
        assert!(is_safe_type_widening(
            &DataType::Array(Box::new(DataType::Float)),
            &DataType::Array(Box::new(DataType::Double))
        ));
        // Array(Varchar(50)) -> Array(Varchar(100)) is safe
        assert!(is_safe_type_widening(
            &DataType::Array(Box::new(DataType::Varchar {
                max_length: Some(50)
            })),
            &DataType::Array(Box::new(DataType::Varchar {
                max_length: Some(100)
            }))
        ));
    }

    #[test]
    fn test_array_element_narrowing_unsafe() {
        // Array(BigInt) -> Array(Integer) is narrowing
        assert!(!is_safe_type_widening(
            &DataType::Array(Box::new(DataType::BigInt)),
            &DataType::Array(Box::new(DataType::Integer))
        ));
    }

    #[test]
    fn test_nested_array_widening() {
        // Array(Array(Integer)) -> Array(Array(BigInt)) is safe
        assert!(is_safe_type_widening(
            &DataType::Array(Box::new(DataType::Array(Box::new(DataType::Integer)))),
            &DataType::Array(Box::new(DataType::Array(Box::new(DataType::BigInt))))
        ));
    }

    #[test]
    fn test_map_value_widening_safe() {
        // Map(Varchar, Integer) -> Map(Varchar, BigInt) is safe
        assert!(is_safe_type_widening(
            &DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::Integer)
            ),
            &DataType::Map(
                Box::new(DataType::Varchar { max_length: None }),
                Box::new(DataType::BigInt)
            )
        ));
    }

    #[test]
    fn test_map_key_change_never_safe() {
        // Map key change is NEVER safe, even if it's a widening
        assert!(!is_safe_type_widening(
            &DataType::Map(
                Box::new(DataType::Integer),
                Box::new(DataType::Varchar { max_length: None })
            ),
            &DataType::Map(
                Box::new(DataType::BigInt),
                Box::new(DataType::Varchar { max_length: None })
            )
        ));
    }

    #[test]
    fn test_struct_not_widening() {
        // Struct changes are NOT widening — they go through diff_types
        assert!(!is_safe_type_widening(
            &DataType::Struct(vec![("a".to_string(), DataType::Integer)]),
            &DataType::Struct(vec![("a".to_string(), DataType::BigInt)])
        ));
        // Adding a field is also not a widening
        assert!(!is_safe_type_widening(
            &DataType::Struct(vec![("a".to_string(), DataType::Integer)]),
            &DataType::Struct(vec![
                ("a".to_string(), DataType::Integer),
                ("b".to_string(), DataType::Varchar { max_length: None })
            ])
        ));
    }

    #[test]
    fn test_array_widening_via_str() {
        // String-based interface should also work for array widening
        assert!(is_safe_type_widening_str("INTEGER[]", "BIGINT[]"));
        assert!(!is_safe_type_widening_str("BIGINT[]", "INTEGER[]"));
        assert!(is_safe_type_widening_str(
            "MAP(VARCHAR, INTEGER)",
            "MAP(VARCHAR, BIGINT)"
        ));
        assert!(!is_safe_type_widening_str(
            "MAP(INTEGER, VARCHAR)",
            "MAP(BIGINT, VARCHAR)"
        ));
    }

    #[test]
    fn test_array_widening_no_full_refresh() {
        // Integration: diff with safe array widening should NOT require full refresh
        let diff = SchemaDiff {
            changes: vec![SchemaChange::ArrayElementTypeChange {
                column: "scores".to_string(),
                path: vec![],
                from: "INTEGER".to_string(),
                to: "BIGINT".to_string(),
            }],
            warnings: vec![],
        };
        assert!(
            !diff.requires_full_refresh(),
            "Safe array element widening should not require full refresh"
        );
    }

    #[test]
    fn test_array_narrowing_requires_full_refresh() {
        // Narrowing array element type DOES require full refresh
        let diff = SchemaDiff {
            changes: vec![SchemaChange::ArrayElementTypeChange {
                column: "scores".to_string(),
                path: vec![],
                from: "BIGINT".to_string(),
                to: "INTEGER".to_string(),
            }],
            warnings: vec![],
        };
        assert!(
            diff.requires_full_refresh(),
            "Array element narrowing should require full refresh"
        );
    }

    #[test]
    fn test_map_value_widening_no_full_refresh() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::MapValueTypeChange {
                column: "lookup".to_string(),
                path: vec![],
                from: "INTEGER".to_string(),
                to: "BIGINT".to_string(),
            }],
            warnings: vec![],
        };
        assert!(
            !diff.requires_full_refresh(),
            "Safe map value widening should not require full refresh"
        );
    }

    #[test]
    fn test_nested_type_widening_no_full_refresh() {
        let diff = SchemaDiff {
            changes: vec![SchemaChange::NestedTypeChange {
                column: "meta".to_string(),
                path: vec!["a".to_string()],
                from: "INTEGER".to_string(),
                to: "BIGINT".to_string(),
            }],
            warnings: vec![],
        };
        assert!(
            !diff.requires_full_refresh(),
            "Safe nested type widening should not require full refresh"
        );
    }

    // === Phase 3: Structural diff for complex types ===

    #[test]
    fn test_struct_field_addition() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::StructFieldAdded {
                column,
                path,
                field_name,
                field_type,
                nullable,
            } => {
                assert_eq!(column, "meta");
                assert!(path.is_empty());
                assert_eq!(field_name, "b");
                assert_eq!(field_type, "VARCHAR");
                assert!(*nullable);
            }
            other => panic!("Expected StructFieldAdded, got {:?}", other),
        }
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_struct_field_removal() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::StructFieldRemoved {
                column,
                path,
                field_name,
            } => {
                assert_eq!(column, "meta");
                assert!(path.is_empty());
                assert_eq!(field_name, "b");
            }
            other => panic!("Expected StructFieldRemoved, got {:?}", other),
        }
        // Struct field removal is like column removal — doesn't require refresh
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_nested_struct_field_addition() {
        let deployed = vec![col("data", "STRUCT(inner STRUCT(x INTEGER))", true)];
        let inferred = vec![col(
            "data",
            "STRUCT(inner STRUCT(x INTEGER, y VARCHAR))",
            true,
        )];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::StructFieldAdded {
                column,
                path,
                field_name,
                ..
            } => {
                assert_eq!(column, "data");
                assert_eq!(path, &vec!["inner".to_string()]);
                assert_eq!(field_name, "y");
            }
            other => panic!("Expected StructFieldAdded, got {:?}", other),
        }
    }

    #[test]
    fn test_array_element_type_widening() {
        let deployed = vec![col("scores", "INTEGER[]", true)];
        let inferred = vec![col("scores", "BIGINT[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::ArrayElementTypeChange {
                column, from, to, ..
            } => {
                assert_eq!(column, "scores");
                assert_eq!(from, "INTEGER");
                assert_eq!(to, "BIGINT");
            }
            other => panic!("Expected ArrayElementTypeChange, got {:?}", other),
        }
        // INTEGER → BIGINT is safe widening
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_array_element_type_unsafe() {
        let deployed = vec![col("scores", "BIGINT[]", true)];
        let inferred = vec![col("scores", "INTEGER[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(
            &diff.changes[0],
            SchemaChange::ArrayElementTypeChange { .. }
        ));
        // BIGINT → INTEGER is narrowing → requires refresh
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_map_value_struct_field_addition() {
        let deployed = vec![col("lookup", "MAP(VARCHAR, STRUCT(a INTEGER))", true)];
        let inferred = vec![col(
            "lookup",
            "MAP(VARCHAR, STRUCT(a INTEGER, b TEXT))",
            true,
        )];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::StructFieldAdded {
                column,
                path,
                field_name,
                ..
            } => {
                assert_eq!(column, "lookup");
                assert_eq!(path, &vec!["value".to_string()]);
                assert_eq!(field_name, "b");
            }
            other => panic!("Expected StructFieldAdded, got {:?}", other),
        }
    }

    #[test]
    fn test_struct_field_reorder_is_incompatible() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(b VARCHAR, a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::IncompatibleTypeChange { column, reason, .. } => {
                assert_eq!(column, "meta");
                assert!(
                    reason.contains("struct field order changed"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("Expected IncompatibleTypeChange, got {:?}", other),
        }
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_struct_to_scalar_is_incompatible() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "INTEGER", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::IncompatibleTypeChange { column, reason, .. } => {
                assert_eq!(column, "meta");
                assert!(
                    reason.contains("struct to non-struct"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("Expected IncompatibleTypeChange, got {:?}", other),
        }
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_scalar_to_struct_is_incompatible() {
        let deployed = vec![col("meta", "INTEGER", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(
            &diff.changes[0],
            SchemaChange::IncompatibleTypeChange { .. }
        ));
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_map_key_type_change_is_unsafe() {
        let deployed = vec![col("m", "MAP(INTEGER, VARCHAR)", true)];
        let inferred = vec![col("m", "MAP(BIGINT, VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(
            &diff.changes[0],
            SchemaChange::MapKeyTypeChange { .. }
        ));
        assert!(diff.requires_full_refresh());
    }

    #[test]
    fn test_map_value_type_widening() {
        let deployed = vec![col("m", "MAP(VARCHAR, INTEGER)", true)];
        let inferred = vec![col("m", "MAP(VARCHAR, BIGINT)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::MapValueTypeChange {
                column, from, to, ..
            } => {
                assert_eq!(column, "m");
                assert_eq!(from, "INTEGER");
                assert_eq!(to, "BIGINT");
            }
            other => panic!("Expected MapValueTypeChange, got {:?}", other),
        }
        // INTEGER → BIGINT is safe widening
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_nested_struct_type_widening() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            SchemaChange::NestedTypeChange {
                column,
                path,
                from,
                to,
            } => {
                assert_eq!(column, "meta");
                assert_eq!(path, &vec!["a".to_string()]);
                assert_eq!(from, "INTEGER");
                assert_eq!(to, "BIGINT");
            }
            other => panic!("Expected NestedTypeChange, got {:?}", other),
        }
        // INTEGER → BIGINT is safe widening
        assert!(!diff.requires_full_refresh());
    }

    #[test]
    fn test_multiple_struct_changes() {
        // Multiple changes in one struct: field added + type widened
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR, c BOOLEAN)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 2);
        // Should have both a StructFieldAdded and a NestedTypeChange
        let has_field_added = diff.changes.iter().any(
            |c| matches!(c, SchemaChange::StructFieldAdded { field_name, .. } if field_name == "c"),
        );
        let has_type_change = diff
            .changes
            .iter()
            .any(|c| matches!(c, SchemaChange::NestedTypeChange { path, .. } if path == &vec!["a".to_string()]));
        assert!(has_field_added, "Should have StructFieldAdded for 'c'");
        assert!(has_type_change, "Should have NestedTypeChange for 'a'");
    }

    #[test]
    fn test_summary_nested_changes() {
        let diff = SchemaDiff {
            changes: vec![
                SchemaChange::StructFieldAdded {
                    column: "meta".to_string(),
                    path: vec![],
                    field_name: "b".to_string(),
                    field_type: "VARCHAR".to_string(),
                    nullable: true,
                },
                SchemaChange::NestedTypeChange {
                    column: "meta".to_string(),
                    path: vec!["a".to_string()],
                    from: "INTEGER".to_string(),
                    to: "BIGINT".to_string(),
                },
                SchemaChange::IncompatibleTypeChange {
                    column: "data".to_string(),
                    from: "STRUCT(a INTEGER)".to_string(),
                    to: "INTEGER".to_string(),
                    reason: "struct to non-struct".to_string(),
                },
            ],
            warnings: vec![],
        };
        let summary = diff.summary();
        assert_eq!(summary.len(), 3);
        assert!(summary[0].contains("ADD STRUCT FIELD meta.b"));
        assert!(summary[1].contains("ALTER NESTED TYPE meta.a"));
        assert!(summary[2].contains("INCOMPATIBLE TYPE CHANGE data"));
    }

    #[test]
    fn test_diff_types_direct_struct() {
        // Direct call to diff_types for unit testing
        let old = DataType::Struct(vec![
            ("a".to_string(), DataType::Integer),
            ("b".to_string(), DataType::Varchar { max_length: None }),
        ]);
        let new = DataType::Struct(vec![
            ("a".to_string(), DataType::Integer),
            ("b".to_string(), DataType::Varchar { max_length: None }),
            ("c".to_string(), DataType::Boolean),
        ]);
        let changes = diff_types("col", &[], &old, &new);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            SchemaChange::StructFieldAdded { field_name, .. } if field_name == "c"
        ));
    }

    #[test]
    fn test_diff_types_deeply_nested_struct_change() {
        // STRUCT(inner STRUCT(deep STRUCT(x INTEGER))) → STRUCT(inner STRUCT(deep STRUCT(x BIGINT)))
        let old = DataType::Struct(vec![(
            "inner".to_string(),
            DataType::Struct(vec![(
                "deep".to_string(),
                DataType::Struct(vec![("x".to_string(), DataType::Integer)]),
            )]),
        )]);
        let new = DataType::Struct(vec![(
            "inner".to_string(),
            DataType::Struct(vec![(
                "deep".to_string(),
                DataType::Struct(vec![("x".to_string(), DataType::BigInt)]),
            )]),
        )]);
        let changes = diff_types("col", &[], &old, &new);
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            SchemaChange::NestedTypeChange { path, from, to, .. } => {
                assert_eq!(
                    path,
                    &vec!["inner".to_string(), "deep".to_string(), "x".to_string()]
                );
                assert_eq!(from, "INTEGER");
                assert_eq!(to, "BIGINT");
            }
            other => panic!("Expected NestedTypeChange, got {:?}", other),
        }
    }

    // ===== Phase 5: plan_schema_operations tests =====

    #[test]
    fn test_plan_ops_struct_field_add() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::AddStructField {
                column,
                path,
                field_name,
                field_type,
                ..
            } => {
                assert_eq!(column, "meta");
                assert!(path.is_empty());
                assert_eq!(field_name, "b");
                assert_eq!(*field_type, DataType::Varchar { max_length: None });
            }
            other => panic!("Expected AddStructField, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_nested_type_widen() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::WidenNestedType {
                column,
                path,
                from,
                to,
            } => {
                assert_eq!(column, "meta");
                assert_eq!(path, &vec!["a".to_string()]);
                assert_eq!(*from, DataType::Integer);
                assert_eq!(*to, DataType::BigInt);
            }
            other => panic!("Expected WidenNestedType, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_array_element_widen() {
        let deployed = vec![col("scores", "INTEGER[]", true)];
        let inferred = vec![col("scores", "BIGINT[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::WidenColumnType { name, from, to } => {
                assert_eq!(name, "scores");
                assert_eq!(*from, DataType::Array(Box::new(DataType::Integer)));
                assert_eq!(*to, DataType::Array(Box::new(DataType::BigInt)));
            }
            other => panic!("Expected WidenColumnType, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_add_column_with_backfill() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![col("id", "INTEGER", false), col("status", "VARCHAR", true)];
        let diff = diff_schemas(&deployed, &inferred);

        let mut backfills = HashMap::new();
        backfills.insert(
            "status".to_string(),
            "CASE WHEN id > 0 THEN 'active' ELSE 'inactive' END".to_string(),
        );

        let plan = plan_schema_operations(&diff, &no_defaults(), &backfills);

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 2);
        assert!(
            matches!(&plan.operations[0], SchemaOperation::AddColumn { name, .. } if name == "status")
        );
        assert!(
            matches!(&plan.operations[1], SchemaOperation::BackfillColumn { name, .. } if name == "status")
        );
    }

    #[test]
    fn test_plan_ops_not_null_without_default_requires_refresh() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![
            col("id", "INTEGER", false),
            col("required", "VARCHAR", false),
        ];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(plan.requires_full_refresh);
        assert!(plan
            .full_refresh_reason
            .as_ref()
            .unwrap()
            .contains("NOT NULL"));
    }

    #[test]
    fn test_plan_ops_not_null_with_default_ok() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![
            col("id", "INTEGER", false),
            col("required", "VARCHAR", false),
        ];
        let diff = diff_schemas(&deployed, &inferred);

        let mut defaults = HashMap::new();
        defaults.insert("required".to_string(), "'unknown'".to_string());
        let plan = plan_schema_operations(&diff, &defaults, &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::AddColumn {
                name,
                nullable,
                default_expr,
                ..
            } => {
                assert_eq!(name, "required");
                assert!(!nullable);
                assert_eq!(default_expr.as_deref(), Some("'unknown'"));
            }
            other => panic!("Expected AddColumn, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_scalar_type_widen() {
        let deployed = vec![col("count", "INTEGER", true)];
        let inferred = vec![col("count", "BIGINT", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::WidenColumnType { name, from, to } => {
                assert_eq!(name, "count");
                assert_eq!(*from, DataType::Integer);
                assert_eq!(*to, DataType::BigInt);
            }
            other => panic!("Expected WidenColumnType, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_unsafe_type_change_requires_refresh() {
        let deployed = vec![col("data", "VARCHAR", true)];
        let inferred = vec![col("data", "INTEGER", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(plan.requires_full_refresh);
        assert!(plan
            .full_refresh_reason
            .as_ref()
            .unwrap()
            .contains("unsafe type change"));
    }

    #[test]
    fn test_plan_ops_remove_column() {
        let deployed = vec![col("id", "INTEGER", false), col("old", "VARCHAR", true)];
        let inferred = vec![col("id", "INTEGER", false)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        assert!(
            matches!(&plan.operations[0], SchemaOperation::RemoveColumn { name } if name == "old")
        );
    }

    #[test]
    fn test_plan_ops_nullability_change() {
        let deployed = vec![col("name", "VARCHAR", false)];
        let inferred = vec![col("name", "VARCHAR", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::ChangeNullability {
                name,
                to_nullable,
                default_expr,
            } => {
                assert_eq!(name, "name");
                assert!(to_nullable);
                assert!(default_expr.is_none());
            }
            other => panic!("Expected ChangeNullability, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_nullability_to_not_null_without_default_requires_refresh() {
        let deployed = vec![col("name", "VARCHAR", true)];
        let inferred = vec![col("name", "VARCHAR", false)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(plan.requires_full_refresh);
        assert!(plan
            .full_refresh_reason
            .as_ref()
            .unwrap()
            .contains("NOT NULL"));
    }

    #[test]
    fn test_plan_ops_map_key_change_requires_refresh() {
        let deployed = vec![col("lookup", "MAP(VARCHAR, INTEGER)", true)];
        let inferred = vec![col("lookup", "MAP(INTEGER, INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(plan.requires_full_refresh);
        assert!(plan
            .full_refresh_reason
            .as_ref()
            .unwrap()
            .contains("map key type changed"));
    }

    #[test]
    fn test_plan_ops_map_value_widen() {
        let deployed = vec![col("lookup", "MAP(VARCHAR, INTEGER)", true)];
        let inferred = vec![col("lookup", "MAP(VARCHAR, BIGINT)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::WidenNestedType {
                column,
                path,
                from,
                to,
            } => {
                assert_eq!(column, "lookup");
                assert_eq!(path, &vec!["value".to_string()]);
                assert_eq!(*from, DataType::Integer);
                assert_eq!(*to, DataType::BigInt);
            }
            other => panic!("Expected WidenNestedType, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_ops_incompatible_type_change() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "INTEGER", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(plan.requires_full_refresh);
        assert!(plan
            .full_refresh_reason
            .as_ref()
            .unwrap()
            .contains("incompatible"));
    }

    #[test]
    fn test_plan_ops_struct_field_removed_has_warning() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert_eq!(plan.operations.len(), 1);
        assert!(
            matches!(&plan.operations[0], SchemaOperation::RemoveStructField { field_name, .. } if field_name == "b")
        );
        assert!(!plan.warnings.is_empty());
        assert!(plan.warnings[0].contains("removed"));
    }

    #[test]
    fn test_plan_ops_combined_struct_add_and_widen() {
        // Both a field addition and a type widening on the same struct column
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR, c BOOLEAN)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        // Should have both operations: WidenNestedType for 'a' and AddStructField for 'c'
        assert!(plan.operations.len() >= 2);

        let has_widen = plan.operations.iter().any(|op| {
            matches!(op, SchemaOperation::WidenNestedType { path, .. } if path == &vec!["a".to_string()])
        });
        let has_add = plan.operations.iter().any(|op| {
            matches!(op, SchemaOperation::AddStructField { field_name, .. } if field_name == "c")
        });
        assert!(has_widen, "Expected WidenNestedType for field 'a'");
        assert!(has_add, "Expected AddStructField for field 'c'");
    }

    #[test]
    fn test_plan_ops_empty_diff() {
        let deployed = vec![col("id", "INTEGER", false)];
        let inferred = vec![col("id", "INTEGER", false)];
        let diff = diff_schemas(&deployed, &inferred);
        let plan = plan_schema_operations(&diff, &no_defaults(), &HashMap::new());

        assert!(!plan.requires_full_refresh);
        assert!(plan.operations.is_empty());
    }

    // ── DDL pipeline integration tests (diff → plan_migration → DuckDB DDL) ──

    #[test]
    fn test_ddl_pipeline_struct_field_add() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration("main", "t", &diff, false, &no_defaults(), &HashMap::new());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.t ADD COLUMN meta.b VARCHAR"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_ddl_pipeline_nested_struct_field_add() {
        let deployed = vec![col("data", "STRUCT(inner STRUCT(x INTEGER))", true)];
        let inferred = vec![col(
            "data",
            "STRUCT(inner STRUCT(x INTEGER, y VARCHAR))",
            true,
        )];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration("main", "t", &diff, false, &no_defaults(), &HashMap::new());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.t ADD COLUMN data.\"inner\".y VARCHAR"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_ddl_pipeline_struct_field_removal() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        // Need allow_column_removal since struct field removal is treated like column removal
        let action = plan_migration("main", "t", &diff, true, &no_defaults(), &HashMap::new());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(statements[0], "ALTER TABLE main.t DROP COLUMN meta.b");
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_ddl_pipeline_array_element_widening() {
        let deployed = vec![col("scores", "INTEGER[]", true)];
        let inferred = vec![col("scores", "BIGINT[]", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration("main", "t", &diff, false, &no_defaults(), &HashMap::new());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.t ALTER COLUMN scores TYPE BIGINT[]"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_ddl_pipeline_nested_type_widening() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        // Use plan_migration_for_backend with deployed/inferred columns so the
        // DuckDB struct_pack USING expression can be generated correctly.
        let action = plan_migration_for_backend(
            "main",
            "t",
            &diff,
            false,
            &no_defaults(),
            &HashMap::new(),
            &DdlBackend::DuckDb,
            &deployed,
            &inferred,
        );
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                // DuckDB alters the full column type — automatic casting handles safe widenings
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.t ALTER COLUMN meta TYPE STRUCT(a BIGINT, b VARCHAR)"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_ddl_pipeline_map_value_widening() {
        let deployed = vec![col("m", "MAP(VARCHAR, INTEGER)", true)];
        let inferred = vec![col("m", "MAP(VARCHAR, BIGINT)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration("main", "t", &diff, false, &no_defaults(), &HashMap::new());
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert_eq!(
                    statements[0],
                    "ALTER TABLE main.t ALTER COLUMN m TYPE MAP(VARCHAR, BIGINT)"
                );
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    // ── Spark backend routing tests ───────────────────────────────────

    fn spark_delta_backend() -> DdlBackend {
        DdlBackend::Spark {
            catalog: "cat".into(),
            format: SparkTableFormat::Delta,
            capabilities: BackendCapabilities::spark_delta(),
        }
    }

    fn spark_parquet_backend() -> DdlBackend {
        DdlBackend::Spark {
            catalog: "cat".into(),
            format: SparkTableFormat::Parquet,
            capabilities: BackendCapabilities::spark_parquet(),
        }
    }

    #[test]
    fn test_spark_delta_struct_field_add() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration_for_backend(
            "db",
            "t",
            &diff,
            false,
            &no_defaults(),
            &HashMap::new(),
            &spark_delta_backend(),
            &deployed,
            &inferred,
        );
        match action {
            MigrationAction::AlterTable { statements } => {
                assert_eq!(statements.len(), 1);
                assert!(statements[0].contains("cat.db.t"));
                assert!(statements[0].contains("ADD COLUMNS"));
                assert!(statements[0].contains("meta.b STRING"));
            }
            other => panic!("Expected AlterTable, got {:?}", other),
        }
    }

    #[test]
    fn test_spark_delta_nested_widen_returns_table_rewrite() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration_for_backend(
            "db",
            "t",
            &diff,
            false,
            &no_defaults(),
            &HashMap::new(),
            &spark_delta_backend(),
            &deployed,
            &inferred,
        );
        match action {
            MigrationAction::TableRewrite { select_expr } => {
                assert!(select_expr.contains("meta"));
            }
            other => panic!("Expected TableRewrite, got {:?}", other),
        }
    }

    #[test]
    fn test_spark_parquet_nested_widen_blocked() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a BIGINT, b VARCHAR)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration_for_backend(
            "db",
            "t",
            &diff,
            false,
            &no_defaults(),
            &HashMap::new(),
            &spark_parquet_backend(),
            &deployed,
            &inferred,
        );
        match action {
            MigrationAction::FullRefreshBlocked { reason } => {
                assert!(reason.contains("Parquet"));
            }
            other => panic!("Expected FullRefreshBlocked, got {:?}", other),
        }
    }

    #[test]
    fn test_spark_parquet_struct_field_remove_blocked() {
        let deployed = vec![col("meta", "STRUCT(a INTEGER, b VARCHAR)", true)];
        let inferred = vec![col("meta", "STRUCT(a INTEGER)", true)];
        let diff = diff_schemas(&deployed, &inferred);
        let action = plan_migration_for_backend(
            "db",
            "t",
            &diff,
            true,
            &no_defaults(),
            &HashMap::new(),
            &spark_parquet_backend(),
            &deployed,
            &inferred,
        );
        match action {
            MigrationAction::FullRefreshBlocked { reason } => {
                assert!(reason.contains("Parquet"));
                assert!(reason.contains("column mapping"));
            }
            other => panic!("Expected FullRefreshBlocked, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_incompatible_type_change_triggers_full_refresh() {
        // IncompatibleTypeChange (e.g., struct → scalar) should trigger FullRefresh,
        // not silently pass through.
        let diff = SchemaDiff {
            changes: vec![SchemaChange::IncompatibleTypeChange {
                column: "meta".to_string(),
                from: "STRUCT(a INTEGER)".to_string(),
                to: "INTEGER".to_string(),
                reason: "struct to scalar".to_string(),
            }],
            warnings: vec![],
        };
        let action = plan_migration_for_backend(
            "main",
            "t",
            &diff,
            true,
            &no_defaults(),
            &HashMap::new(),
            &DdlBackend::DuckDb,
            &[],
            &[],
        );
        match action {
            MigrationAction::FullRefresh { reason } => {
                assert!(
                    reason.contains("incompatible") || reason.contains("struct to scalar"),
                    "Reason should explain the incompatible change, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefresh, got {:?}", other),
        }
    }

    #[test]
    fn test_plan_migration_map_key_type_change_triggers_full_refresh() {
        // MapKeyTypeChange is always unsafe — should trigger FullRefresh.
        let diff = SchemaDiff {
            changes: vec![SchemaChange::MapKeyTypeChange {
                column: "lookup".to_string(),
                from: "VARCHAR".to_string(),
                to: "INTEGER".to_string(),
            }],
            warnings: vec![],
        };
        let action = plan_migration_for_backend(
            "main",
            "t",
            &diff,
            true,
            &no_defaults(),
            &HashMap::new(),
            &DdlBackend::DuckDb,
            &[],
            &[],
        );
        match action {
            MigrationAction::FullRefresh { reason } => {
                assert!(
                    reason.contains("map key") || reason.contains("lookup"),
                    "Reason should explain the map key change, got: {}",
                    reason
                );
            }
            other => panic!("Expected FullRefresh, got {:?}", other),
        }
    }

    #[test]
    fn test_diff_schemas_unparseable_deployed_type_falls_back() {
        // If a deployed type string can't be parsed (e.g., from an older smelt version
        // or external source), diff_schemas should fall back to string comparison,
        // not panic.
        let deployed = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(), // unparseable in our type system
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(), // same unparseable type
            nullable: true,
        }];
        // Same type string → no change (should not panic)
        let diff = diff_schemas(&deployed, &inferred);
        assert!(
            diff.changes.is_empty(),
            "Same unparseable types should produce no changes"
        );
    }

    #[test]
    fn test_diff_schemas_unparseable_type_change_detected() {
        // Different unparseable types should still detect a change via string comparison
        let deployed = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(),
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSON".to_string(),
            nullable: true,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(&diff.changes[0], SchemaChange::ChangeType { .. }));
    }

    #[test]
    fn test_diff_schemas_unparseable_complex_type_falls_back() {
        // When one type is parseable but complex, and the other isn't,
        // we should fall back to ChangeType, not panic
        let deployed = vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "CUSTOM_STRUCT_TYPE".to_string(), // unparseable
            nullable: true,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(&diff.changes[0], SchemaChange::ChangeType { .. }));
    }

    #[test]
    fn test_diff_schemas_unparseable_to_complex_type_falls_back() {
        // Deployed type is unparseable, inferred is complex and parseable
        let deployed = vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "CUSTOM_STRUCT_TYPE".to_string(),
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "meta".to_string(),
            data_type: "STRUCT(a INTEGER, b VARCHAR)".to_string(),
            nullable: true,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(&diff.changes[0], SchemaChange::ChangeType { .. }));
    }

    #[test]
    fn test_diff_schemas_unparseable_type_emits_warning() {
        let deployed = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(),
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "VARCHAR".to_string(),
            nullable: true,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        // Should have a warning about the unparseable deployed type
        assert_eq!(diff.warnings.len(), 1);
        assert!(diff.warnings[0].contains("JSONB"));
        assert!(diff.warnings[0].contains("could not be parsed"));
        // Should still detect the change via fallback
        assert_eq!(diff.changes.len(), 1);
    }

    #[test]
    fn test_diff_schemas_both_unparseable_same_type_no_warning_on_match() {
        // When both sides are the same unparseable type, there's still a warning
        // but no change detected
        let deployed = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(),
            nullable: true,
        }];
        let inferred = vec![DeployedColumn {
            name: "data".to_string(),
            data_type: "JSONB".to_string(),
            nullable: true,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        // Warnings should still be emitted (they indicate parsing limitations)
        assert_eq!(diff.warnings.len(), 2); // both sides unparseable
        assert!(diff.changes.is_empty());
    }

    #[test]
    fn test_diff_schemas_parseable_types_no_warning() {
        let deployed = vec![DeployedColumn {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            nullable: false,
        }];
        let inferred = vec![DeployedColumn {
            name: "id".to_string(),
            data_type: "BIGINT".to_string(),
            nullable: false,
        }];
        let diff = diff_schemas(&deployed, &inferred);
        assert!(diff.warnings.is_empty());
        assert_eq!(diff.changes.len(), 1);
    }

    #[test]
    fn test_add_column_unparseable_type_triggers_full_refresh() {
        // An unparseable stored type string must NOT produce AddColumn{Unknown}
        // (which would generate invalid DDL). It must trigger a full refresh.
        let diff = SchemaDiff {
            changes: vec![SchemaChange::AddColumn {
                name: "mystery_col".to_string(),
                data_type: "MYSTERY_TYPE_CANNOT_BE_PARSED".to_string(),
                nullable: true,
            }],
            warnings: vec![],
        };
        let plan = plan_schema_operations(&diff, &no_defaults(), &no_defaults());
        assert!(
            plan.requires_full_refresh,
            "unparseable column type must trigger full refresh"
        );
        let has_unknown_add = plan.operations.iter().any(|op| {
            matches!(
                op,
                SchemaOperation::AddColumn {
                    data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                    ..
                }
            )
        });
        assert!(
            !has_unknown_add,
            "AddColumn with Unknown type must not be emitted for unparseable type string"
        );
    }

    #[test]
    fn test_struct_field_added_unparseable_type_triggers_full_refresh() {
        // Same guard for struct field additions: unparseable field type must
        // trigger full refresh, not AddStructField{field_type: Unknown}.
        let diff = SchemaDiff {
            changes: vec![SchemaChange::StructFieldAdded {
                column: "info".to_string(),
                path: vec![],
                field_name: "secret".to_string(),
                field_type: "MYSTERY_STRUCT_FIELD_TYPE".to_string(),
                nullable: true,
            }],
            warnings: vec![],
        };
        let plan = plan_schema_operations(&diff, &no_defaults(), &no_defaults());
        assert!(
            plan.requires_full_refresh,
            "unparseable struct field type must trigger full refresh"
        );
        let has_unknown_field = plan.operations.iter().any(|op| {
            matches!(
                op,
                SchemaOperation::AddStructField {
                    field_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
                    ..
                }
            )
        });
        assert!(
            !has_unknown_field,
            "AddStructField with Unknown type must not be emitted for unparseable field type string"
        );
    }
}
