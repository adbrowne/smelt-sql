/// Kind of an entity referenced by a `smelt.<path>` value or call.
/// Determined by the file format/content at the resolved path per
/// architecture Surface §"Resolution".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `.sql` file with a bare SELECT.
    Model,
    /// `.csv` file under a `paths` directory.
    Seed,
    /// `.yml` file declaring an external table.
    Source,
    /// `.sql` file declaring `smelt.define <name>`.
    Function,
    /// `.sql` file containing `smelt.test` declarations. Cannot be used in
    /// `TableExpr` positions per the spec.
    Test,
}

/// Schema representation for sqt models
///
/// This module defines the schema types used throughout the project for:
/// - Tracking column names and lineage
/// - LSP features (hover, autocomplete)
/// - Future refactoring API
/// - Row polymorphism (wildcard expansion, input constraints)
use rowan::TextRange;
use smelt_types::TypedColumn;
use std::collections::BTreeMap;

/// Represents a column in a model's output schema
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The output column name (either the alias or the base name)
    pub name: String,

    /// The alias if explicitly provided (e.g., "total_revenue" in "revenue AS total_revenue")
    pub alias: Option<String>,

    /// Source/lineage of this column
    pub source: ColumnSource,

    /// The SQL expression text that produces this column
    pub expression: String,

    /// Text range in the source file (for LSP navigation)
    pub range: TextRange,

    /// The inferred or declared type of this column (None if unknown)
    pub data_type: Option<TypedColumn>,
}

/// Tracks where a column comes from (lineage)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnSource {
    /// Computed from an expression (can't be traced to upstream)
    /// Example: `SUM(revenue)`, `user_id * 2`
    Computed,

    /// Direct reference to a column from an upstream model
    /// Example: `user_id` from `{{ ref('raw_events') }}`
    FromModel {
        model_name: String,
        column_name: String,
    },

    /// Wildcard select (*) - expanded to all columns from source
    /// Example: `SELECT * FROM {{ ref('users') }}`
    Wildcard { model_name: String },

    /// Column from a non-ref table (e.g., source.events)
    /// External tables that aren't sqt models
    ExternalTable { table_name: String },

    /// Unable to determine source (error recovery)
    Unknown,
}

/// Represents a row extension: "...plus all other columns from this ref"
/// Replaces wildcard Column entries with a structured reference that can be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowExtension {
    pub ref_name: String,
    /// Columns already explicitly selected (excluded from expansion to avoid duplicates)
    pub excluded_columns: Vec<String>,
    pub range: TextRange,
}

/// Constraint: this model requires a ref to have specific columns
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputConstraint {
    pub ref_name: String,
    pub required_columns: BTreeMap<String, ColumnConstraint>,
}

/// A single column constraint within an InputConstraint
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnConstraint {
    /// Type the column must be compatible with (None = any type)
    pub expected_type: Option<TypedColumn>,
    /// Where in the SQL this constraint originates
    pub usage_sites: Vec<TextRange>,
}

/// Schema with all row extensions expanded where possible
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSchema {
    pub columns: Vec<Column>,
    pub is_fully_resolved: bool,
    pub unresolved_extensions: Vec<RowExtension>,
}

/// Schema for a model (list of output columns)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSchema {
    pub columns: Vec<Column>,
    /// Row extensions from SELECT * (resolved lazily via resolved_model_schema)
    pub row_extensions: Vec<RowExtension>,
    /// Constraints this model places on its input refs
    pub input_constraints: Vec<InputConstraint>,
}

impl ModelSchema {
    /// Create an empty schema
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
            row_extensions: Vec::new(),
            input_constraints: Vec::new(),
        }
    }

    /// Find a column by name
    pub fn find_column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Get all column names
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// Represents a model as a function: inputs (required columns from refs) -> outputs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFunctionType {
    pub model_name: String,
    pub inputs: Vec<FunctionInput>,
    pub outputs: Vec<FunctionOutput>,
    pub has_wildcard_output: bool,
}

/// An input ref with its required columns
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInput {
    pub ref_name: String,
    pub columns: Vec<TypedField>,
}

/// A typed field in an input constraint
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedField {
    pub name: String,
    pub constraint: Option<TypedColumn>,
}

/// An output column with its inferred type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionOutput {
    pub name: String,
    pub data_type: Option<TypedColumn>,
}

impl std::fmt::Display for ModelFunctionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}:", self.model_name)?;

        // Inputs
        if self.inputs.is_empty() {
            write!(f, "  () -> ")?;
        } else {
            write!(f, "  (")?;
            for (i, input) in self.inputs.iter().enumerate() {
                if i > 0 {
                    write!(f, ",\n   ")?;
                }
                write!(f, "{}: {{", input.ref_name)?;
                for (j, col) in input.columns.iter().enumerate() {
                    if j > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", col.name)?;
                    if let Some(constraint) = &col.constraint {
                        write!(f, ": {}", constraint.data_type)?;
                    }
                }
                write!(f, "}}")?;
            }
            write!(f, ")\n  -> ")?;
        }

        // Outputs
        write!(f, "{{")?;
        for (i, output) in self.outputs.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", output.name)?;
            if let Some(dt) = &output.data_type {
                write!(f, ": {}", dt.data_type)?;
            }
        }
        if self.has_wildcard_output {
            if !self.outputs.is_empty() {
                write!(f, ", ")?;
            }
            write!(f, "...")?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_schema() {
        let schema = ModelSchema::empty();
        assert_eq!(schema.columns.len(), 0);
        assert!(schema.find_column("foo").is_none());
    }

    #[test]
    fn test_find_column() {
        let schema = ModelSchema {
            columns: vec![
                Column {
                    name: "user_id".to_string(),
                    alias: None,
                    source: ColumnSource::Computed,
                    expression: "user_id".to_string(),
                    range: TextRange::new(0.into(), 7.into()),
                    data_type: None,
                },
                Column {
                    name: "total".to_string(),
                    alias: Some("total".to_string()),
                    source: ColumnSource::Computed,
                    expression: "COUNT(*)".to_string(),
                    range: TextRange::new(9.into(), 24.into()),
                    data_type: None,
                },
            ],
            row_extensions: vec![],
            input_constraints: vec![],
        };

        assert!(schema.find_column("user_id").is_some());
        assert!(schema.find_column("total").is_some());
        assert!(schema.find_column("nonexistent").is_none());
    }

    #[test]
    fn test_column_names() {
        let schema = ModelSchema {
            columns: vec![
                Column {
                    name: "a".to_string(),
                    alias: None,
                    source: ColumnSource::Computed,
                    expression: "a".to_string(),
                    range: TextRange::new(0.into(), 1.into()),
                    data_type: None,
                },
                Column {
                    name: "b".to_string(),
                    alias: None,
                    source: ColumnSource::Computed,
                    expression: "b".to_string(),
                    range: TextRange::new(3.into(), 4.into()),
                    data_type: None,
                },
            ],
            row_extensions: vec![],
            input_constraints: vec![],
        };

        assert_eq!(schema.column_names(), vec!["a", "b"]);
    }
}
