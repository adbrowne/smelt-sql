/// Schema representation for sqt models
///
/// This module defines the schema types used throughout the project for:
/// - Tracking column names and lineage
/// - LSP features (hover, autocomplete)
/// - Future refactoring API

use rowan::TextRange;

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
    Wildcard {
        model_name: String,
    },

    /// Column from a non-ref table (e.g., source.events)
    /// External tables that aren't sqt models
    ExternalTable {
        table_name: String,
    },

    /// Unable to determine source (error recovery)
    Unknown,
}

/// Schema for a model (list of output columns)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSchema {
    pub columns: Vec<Column>,
}

impl ModelSchema {
    /// Create an empty schema
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
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
                },
                Column {
                    name: "total".to_string(),
                    alias: Some("total".to_string()),
                    source: ColumnSource::Computed,
                    expression: "COUNT(*)".to_string(),
                    range: TextRange::new(9.into(), 24.into()),
                },
            ],
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
                },
                Column {
                    name: "b".to_string(),
                    alias: None,
                    source: ColumnSource::Computed,
                    expression: "b".to_string(),
                    range: TextRange::new(3.into(), 4.into()),
                },
            ],
        };

        assert_eq!(schema.column_names(), vec!["a", "b"]);
    }
}
