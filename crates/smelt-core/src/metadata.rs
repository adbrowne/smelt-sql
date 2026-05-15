//! Metadata extraction from SQL files with YAML frontmatter.
//!
//! Supports two formats:
//! 1. Single-model files with YAML frontmatter:
//!    ```sql
//!    ---
//!    name: daily_revenue
//!    materialization: table
//!    ---
//!    SELECT ...
//!    ```
//!
//! 2. Multi-model files with section delimiters:
//!    ```sql
//!    --- name: model1 ---
//!    materialization: table
//!    ---
//!    SELECT ...
//!
//!    --- name: model2 ---
//!    materialization: view
//!    ---
//!    SELECT ...
//!    ```

use crate::config::{DataLatency, IncrementalConfig, Materialization};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use thiserror::Error;

/// A test constraint on a column.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum ColumnTest {
    /// Simple test like "not_null" or "unique"
    Simple(String),
    /// Parameterized test like {accepted_values: [a, b]} or {min: 0}
    Parameterized(BTreeMap<String, serde_yaml::Value>),
}

/// Configuration for a test model
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct TestConfig {
    /// Name of the model being tested
    pub model: String,
    /// Optional CTE name to test in isolation (if absent, tests the whole model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_cte: Option<String>,
    /// Mock input data: maps dependency name → list of row objects
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Vec<BTreeMap<String, serde_yaml::Value>>>,
    /// Expected output rows
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<BTreeMap<String, serde_yaml::Value>>,
    /// Number of property-based test cases (default 10)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cases: Option<u32>,
    /// Whether to check row order (default false = set comparison)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_order: Option<bool>,
}

/// Schema evolution configuration for a model.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SchemaEvolutionConfig {
    /// Strategy for handling schema changes.
    /// `alter_and_backfill` (default): use ALTER TABLE + UPDATE when possible.
    /// `full_refresh`: always DROP + CREATE on schema changes.
    #[serde(default)]
    pub strategy: SchemaEvolutionStrategy,
}

/// How to handle schema changes during incremental runs.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub enum SchemaEvolutionStrategy {
    /// Use ALTER TABLE with DEFAULT values and UPDATE backfill when possible.
    #[default]
    #[serde(rename = "alter_and_backfill")]
    AlterAndBackfill,
    /// Always fall back to full refresh on any schema change.
    #[serde(rename = "full_refresh")]
    FullRefresh,
}

/// Per-column metadata declared in model frontmatter.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ColumnMetadata {
    /// How late data can arrive for this column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_latency: Option<DataLatency>,

    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Column-level test constraints (not_null, unique, accepted_values, min, max)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<ColumnTest>,

    /// Default SQL expression for schema evolution (used when adding NOT NULL columns
    /// via ALTER TABLE instead of full refresh).
    ///
    /// This is a raw SQL expression string, e.g. `"0"`, `"'unknown'"`, `"NULL"`,
    /// or complex type expressions like `"STRUCT_PACK(a := 0, b := '')"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// SQL expression for backfilling existing rows during schema evolution.
    /// Used in UPDATE statements after ALTER TABLE ADD COLUMN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backfill: Option<String>,
}

/// Metadata for a single model extracted from frontmatter
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelMetadata {
    /// Model name (optional in single-model files, required in multi-model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Generator directive: `"models"` marks the file as a multi-model
    /// generator file whose body is a `List<ModelDef>` meta expression.
    /// Any value other than `"models"` is rejected at parse time with
    /// `MetadataError::GeneratesUnknownValue`. Mutually exclusive with
    /// `name:` and with Layer-1 `--- name: foo ---` section delimiters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generates: Option<String>,

    /// Materialization strategy (table or view)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization: Option<Materialization>,

    /// Incremental configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<IncrementalConfig>,

    /// Target to execute this model on (overrides smelt.yml and CLI --target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,

    /// Tags for organization/filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Model owner (team/person)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Per-column metadata (latency, description, etc.)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub columns: HashMap<String, ColumnMetadata>,

    /// Backend-specific hints (forward compatibility)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub backend_hints: HashMap<String, serde_yaml::Value>,

    /// Test configuration (only for materialization: test)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestConfig>,

    /// Schema evolution configuration (opt out with strategy: full_refresh)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_evolution: Option<SchemaEvolutionConfig>,

    /// Override table format for this model (e.g., parquet for a specific model on a Delta target)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<crate::config::TableFormat>,
}

/// Complete file metadata (single or multi-model)
#[derive(Debug, Clone, PartialEq)]
pub enum FileMetadata {
    /// File has no frontmatter
    Empty,

    /// Single model with frontmatter
    Single {
        metadata: Box<ModelMetadata>,
        /// Byte offset where SQL starts (after closing ---)
        sql_offset: usize,
    },

    /// Multiple models in one file (Layer-1 section delimiter format)
    Multi { models: Vec<ModelSection> },

    /// Generator file: `generates: models` frontmatter marks the body as a
    /// meta-evaluable `List<ModelDef>` expression. Each emitted `ModelDef`
    /// value becomes a model in the workspace.
    Generator {
        /// The parsed frontmatter (includes `generates: "models"` and any
        /// other allowed keys such as `tags`, `owner`, etc.).
        metadata: Box<ModelMetadata>,
        /// Byte offset of the first character of the body (the byte
        /// immediately after the closing `---\n` line of the frontmatter).
        body_offset: usize,
    },
}

/// One model section in a multi-model file
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSection {
    pub metadata: ModelMetadata,
    /// Byte range of SQL in file
    pub sql_range: Range<usize>,
}

/// Which mutual-exclusivity rule was violated in a `generates: models` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedKind {
    /// The frontmatter contained a `name:` field alongside `generates: models`.
    NameField,
    /// The body contained at least one Layer-1 `--- name: foo ---` section
    /// delimiter alongside `generates: models` frontmatter.
    SectionDelimiter,
}

/// A 1-based line + column position within the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}

/// Errors that can occur during metadata extraction
#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("YAML parse error: {0}")]
    YamlParseError(#[from] serde_yaml::Error),

    #[error("Missing model name in multi-model file at section {0}")]
    MissingModelName(usize),

    #[error("Malformed section delimiter at line {0}: expected '--- name: model_name ---'")]
    MalformedDelimiter(usize),

    #[error("Frontmatter not closed: missing closing '---' after line {0}")]
    UnclosedFrontmatter(usize),

    /// `generates:` value other than `"models"` was supplied.
    /// `value_span` is the 1-based line/column of the value token in the YAML.
    #[error("generates must be `models`; found {value}")]
    GeneratesUnknownValue {
        value: String,
        value_span: SourceSpan,
    },

    /// `generates: models` combined with a `name:` frontmatter field or
    /// with Layer-1 `--- name: foo ---` section delimiters.
    #[error("generates: models cannot coexist with bare-model identity (name field or section delimiter)")]
    GeneratesMixedWithBareModel {
        offending: MixedKind,
        span: SourceSpan,
    },
}

/// Check whether a `---\n`-prefixed source contains a `generates:` key in its
/// frontmatter block (the text between the opening `---` and the first closing
/// `---`). Returns `true` only when the key appears inside the frontmatter —
/// not inside the body.
fn frontmatter_has_generates(source: &str) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    // Skip first line ("---"), scan until closing "---" or EOF.
    for line in lines.iter().skip(1) {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("generates:") || trimmed == "generates:" {
            return true;
        }
    }
    false
}

/// Extract metadata from SQL source text
///
/// Returns `FileMetadata::Empty` if no frontmatter is present (backward compatible).
pub fn extract_file_metadata(source: &str) -> Result<FileMetadata, MetadataError> {
    let trimmed = source.trim_start();

    // Check for single-model frontmatter
    if trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") {
        // Generator files: `generates:` in frontmatter takes priority over
        // `--- name:` section-delimiter detection. We must route to
        // `extract_single_model` (which then dispatches to `extract_generator`)
        // so the mutual-exclusivity guard fires before `extract_multi_model`
        // swallows the section delimiters in the body.
        if frontmatter_has_generates(trimmed) {
            return extract_single_model(trimmed);
        }
        // Check if this is actually a multi-model file (Layer-1 section delimiter format)
        if trimmed.contains("--- name:") {
            extract_multi_model(trimmed)
        } else {
            extract_single_model(trimmed)
        }
    }
    // Check for multi-model sections or malformed delimiters
    else if trimmed.contains("--- name:") {
        extract_multi_model(trimmed)
    }
    // Check for malformed delimiters that look like section markers
    else if let Some(line_num) = has_malformed_delimiter(source) {
        Err(MetadataError::MalformedDelimiter(line_num))
    }
    // No frontmatter - vanilla SQL
    else {
        Ok(FileMetadata::Empty)
    }
}

/// Compute a 1-based `SourceSpan` for the YAML `generates:` value token given
/// the raw frontmatter YAML content (the text between the two `---` delimiters).
///
/// The outer file always starts with `---\n` (line 1); the YAML block begins on
/// line 2. We scan the YAML lines looking for `generates:` and return the
/// 1-based column position of the first non-whitespace character after the colon.
fn span_for_generates_value(yaml_content: &str) -> SourceSpan {
    // Outer file line 1 is "---"; YAML starts on line 2.
    let yaml_base_line = 2usize;
    for (idx, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("generates:") {
            let colon_rel = line.find(':').unwrap_or(0);
            let rest = &line[colon_rel + 1..];
            let value_col = colon_rel + 1 + rest.len() - rest.trim_start().len() + 1;
            return SourceSpan {
                line: yaml_base_line + idx,
                column: value_col,
            };
        }
    }
    SourceSpan { line: 1, column: 1 }
}

/// Compute a 1-based `SourceSpan` for the `name:` key inside the YAML block.
fn span_for_name_key(yaml_content: &str) -> SourceSpan {
    let yaml_base_line = 2usize;
    for (idx, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("name:") {
            let col = line.find("name").unwrap_or(0) + 1;
            return SourceSpan {
                line: yaml_base_line + idx,
                column: col,
            };
        }
    }
    SourceSpan { line: 1, column: 1 }
}

/// Extract a generator file: `generates: models` frontmatter with a body that
/// is a meta-evaluable `List<ModelDef>` expression.
///
/// Returns `Err(MetadataError::GeneratesUnknownValue)` when the `generates:`
/// value is not `"models"`, `Err(MetadataError::GeneratesMixedWithBareModel)`
/// when `name:` appears alongside `generates: models`, or
/// `Ok(FileMetadata::Generator { … })` on success.
fn extract_generator(
    source: &str,
    metadata: ModelMetadata,
    yaml_content: &str,
    closing_line_index: usize,
) -> Result<FileMetadata, MetadataError> {
    // Validate the `generates:` value.
    let value = metadata.generates.as_deref().unwrap_or("");
    if value != "models" {
        let value_span = span_for_generates_value(yaml_content);
        return Err(MetadataError::GeneratesUnknownValue {
            value: value.to_string(),
            value_span,
        });
    }

    // `name:` is mutually exclusive with `generates: models`.
    if metadata.name.is_some() {
        let span = span_for_name_key(yaml_content);
        return Err(MetadataError::GeneratesMixedWithBareModel {
            offending: MixedKind::NameField,
            span,
        });
    }

    // Calculate body_offset (byte just after the closing `---\n`).
    let body_offset = source
        .lines()
        .take(closing_line_index + 1)
        .map(|line| line.len() + 1) // +1 for newline
        .sum();

    // Check if the body contains Layer-1 section delimiters (`--- name: foo ---`).
    let body = &source[body_offset..];
    for (idx, line) in body.lines().enumerate() {
        if line.trim().starts_with("--- name:") {
            // Count actual file line number: closing_line_index+1 (0-based) + 1 (1-based) + body line offset.
            let body_line = closing_line_index + 2 + idx;
            return Err(MetadataError::GeneratesMixedWithBareModel {
                offending: MixedKind::SectionDelimiter,
                span: SourceSpan {
                    line: body_line,
                    column: 1,
                },
            });
        }
    }

    Ok(FileMetadata::Generator {
        metadata: Box::new(metadata),
        body_offset,
    })
}

/// Check if source contains lines that look like malformed section delimiters
///
/// Returns the 1-based line number of the first malformed delimiter, if any.
fn has_malformed_delimiter(source: &str) -> Option<usize> {
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        // Look for lines that start and end with --- but don't have "name:"
        if trimmed.starts_with("---") && trimmed.ends_with("---") && trimmed.len() > 6 {
            // Exclude exact "---" (valid delimiter)
            if trimmed != "---" && !trimmed.starts_with("--- name:") {
                return Some(idx + 1); // 1-based line number
            }
        }
    }
    None
}

/// Extract metadata from a single-model file (or a generator file that starts
/// with `---\n` frontmatter).
fn extract_single_model(source: &str) -> Result<FileMetadata, MetadataError> {
    let lines: Vec<&str> = source.lines().collect();

    if lines.is_empty() || lines[0] != "---" {
        return Ok(FileMetadata::Empty);
    }

    // Find closing ---
    let closing_line = lines
        .iter()
        .skip(1)
        .position(|&line| line.trim() == "---")
        .ok_or(MetadataError::UnclosedFrontmatter(1))?
        + 1; // +1 because we skipped first line

    // Extract YAML content between delimiters
    let yaml_lines = &lines[1..closing_line];
    let yaml_content = yaml_lines.join("\n");

    // Parse YAML
    let metadata: ModelMetadata = serde_yaml::from_str(&yaml_content)?;

    // Route generator files to `extract_generator`.
    if metadata.generates.is_some() {
        return extract_generator(source, metadata, &yaml_content, closing_line);
    }

    // Calculate SQL offset (after closing ---)
    let sql_offset = source
        .lines()
        .take(closing_line + 1)
        .map(|line| line.len() + 1) // +1 for newline
        .sum();

    Ok(FileMetadata::Single {
        metadata: Box::new(metadata),
        sql_offset,
    })
}

/// Extract metadata from a multi-model file
fn extract_multi_model(source: &str) -> Result<FileMetadata, MetadataError> {
    let mut models = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut current_line = 0;

    while current_line < lines.len() {
        // Skip empty lines and comments until next section
        while current_line < lines.len() {
            let line = lines[current_line].trim();
            if line.starts_with("--- name:") {
                break;
            }
            if !line.is_empty() && !line.starts_with("--") {
                // Found SQL without section delimiter - error
                return Err(MetadataError::MalformedDelimiter(current_line + 1));
            }
            current_line += 1;
        }

        if current_line >= lines.len() {
            break; // No more sections
        }

        // Parse section delimiter: "--- name: model_name ---"
        let delimiter_line = lines[current_line];
        let model_name = parse_section_delimiter(delimiter_line, current_line + 1)?;

        current_line += 1;

        // Find closing --- for this section's YAML
        let yaml_start_line = current_line;
        let closing_line = lines[current_line..]
            .iter()
            .position(|&line| line.trim() == "---")
            .ok_or(MetadataError::UnclosedFrontmatter(current_line + 1))?
            + current_line;

        // Extract YAML between delimiter and closing ---
        let yaml_lines = &lines[yaml_start_line..closing_line];
        let yaml_content = yaml_lines.join("\n");

        // Parse YAML
        let mut metadata: ModelMetadata = if yaml_content.trim().is_empty() {
            ModelMetadata::default()
        } else {
            serde_yaml::from_str(&yaml_content)?
        };

        // Set model name from delimiter
        metadata.name = Some(model_name);

        current_line = closing_line + 1;

        // Find SQL range (from after closing --- to next section or EOF)
        let sql_start_byte: usize = source
            .lines()
            .take(current_line)
            .map(|line| line.len() + 1)
            .sum();

        // Find next section delimiter or EOF
        let sql_end_line = lines[current_line..]
            .iter()
            .position(|&line| line.trim().starts_with("--- name:"))
            .map(|pos| current_line + pos)
            .unwrap_or(lines.len());

        let sql_end_byte: usize = source
            .lines()
            .take(sql_end_line)
            .map(|line| line.len() + 1)
            .sum();

        models.push(ModelSection {
            metadata,
            sql_range: sql_start_byte..sql_end_byte,
        });

        current_line = sql_end_line;
    }

    if models.is_empty() {
        Ok(FileMetadata::Empty)
    } else {
        Ok(FileMetadata::Multi { models })
    }
}

/// Parse a section delimiter line to extract the model name
///
/// Expected format: "--- name: model_name ---"
fn parse_section_delimiter(line: &str, line_number: usize) -> Result<String, MetadataError> {
    let trimmed = line.trim();

    // Must start with "--- name:" and end with "---"
    if !trimmed.starts_with("--- name:") || !trimmed.ends_with("---") {
        return Err(MetadataError::MalformedDelimiter(line_number));
    }

    // Extract name between "--- name:" and final "---"
    let after_prefix = &trimmed[9..]; // Skip "--- name:"
    let name_part = &after_prefix[..after_prefix.len() - 3]; // Remove final "---"
    let name = name_part.trim();

    if name.is_empty() {
        return Err(MetadataError::MissingModelName(line_number));
    }

    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_frontmatter() {
        let source = "SELECT * FROM users";
        let result = extract_file_metadata(source).unwrap();
        assert_eq!(result, FileMetadata::Empty);
    }

    #[test]
    fn test_single_model_basic() {
        let source = r#"---
name: test_model
materialization: table
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("test_model".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_single_model_with_incremental() {
        let source = r#"---
name: daily_revenue
materialization: table
incremental:
  enabled: true
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: day
tags: [revenue, core]
---
SELECT DATE(transaction_timestamp) as revenue_date, SUM(amount)
FROM transactions
GROUP BY 1"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("daily_revenue".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
                assert_eq!(metadata.tags, vec!["revenue", "core"]);

                let incremental = metadata.incremental.unwrap();
                assert!(incremental.enabled);
                assert_eq!(incremental.event_time_column, "transaction_timestamp");
                assert_eq!(incremental.partition_column, "revenue_date");
                assert_eq!(incremental.granularity, crate::config::Granularity::Day);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_multi_model_file() {
        let source = r#"--- name: model1 ---
materialization: table
---
SELECT * FROM source1

--- name: model2 ---
materialization: view
---
SELECT * FROM source2"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Multi { models } => {
                assert_eq!(models.len(), 2);

                assert_eq!(models[0].metadata.name, Some("model1".to_string()));
                assert_eq!(
                    models[0].metadata.materialization,
                    Some(Materialization::Table)
                );

                assert_eq!(models[1].metadata.name, Some("model2".to_string()));
                assert_eq!(
                    models[1].metadata.materialization,
                    Some(Materialization::View)
                );
            }
            _ => panic!("Expected Multi variant"),
        }
    }

    #[test]
    fn test_invalid_yaml() {
        let source = r#"---
name: test
materialization: invalid_value
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_unclosed_frontmatter() {
        let source = r#"---
name: test
materialization: table
SELECT * FROM users"#;

        let result = extract_file_metadata(source);
        assert!(matches!(result, Err(MetadataError::UnclosedFrontmatter(_))));
    }

    #[test]
    fn test_malformed_section_delimiter() {
        let source = r#"--- model1 ---
materialization: table
---
SELECT * FROM source1"#;

        let result = extract_file_metadata(source);
        assert!(matches!(result, Err(MetadataError::MalformedDelimiter(_))));
    }

    #[test]
    fn test_section_delimiter_parsing() {
        assert_eq!(
            parse_section_delimiter("--- name: my_model ---", 1).unwrap(),
            "my_model"
        );
        assert_eq!(
            parse_section_delimiter("--- name:  spaced_name  ---", 1).unwrap(),
            "spaced_name"
        );
        assert!(parse_section_delimiter("--- name: ---", 1).is_err()); // Empty name
        assert!(parse_section_delimiter("--- model_name ---", 1).is_err()); // Missing "name:"
    }

    #[test]
    fn test_backward_compatibility() {
        // Files without frontmatter should work
        let vanilla_sql = r#"
-- This is a comment
SELECT user_id, COUNT(*) as count
FROM events
GROUP BY user_id
"#;
        let result = extract_file_metadata(vanilla_sql).unwrap();
        assert_eq!(result, FileMetadata::Empty);
    }

    #[test]
    fn test_empty_frontmatter_in_multi_model() {
        let source = r#"--- name: simple_model ---
---
SELECT * FROM users"#;

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Multi { models } => {
                assert_eq!(models.len(), 1);
                assert_eq!(models[0].metadata.name, Some("simple_model".to_string()));
                assert_eq!(models[0].metadata.materialization, None);
            }
            _ => panic!("Expected Multi variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_leading_whitespace() {
        // Python triple-quoted strings often produce leading newlines
        let source = "\n---\ntags:\n  - event_source\n---\nSELECT * FROM events";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.tags, vec!["event_source".to_string()]);
            }
            _ => panic!("Expected Single variant, got {:?}", result),
        }
    }

    #[test]
    fn test_unknown_field_rejected() {
        let source = "---\nname: test\nmaterialized: table\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unknown field"),
            "Expected 'unknown field' in error: {}",
            err_msg
        );
    }

    #[test]
    fn test_unknown_field_incremental_key_rejected() {
        let source = "---\nname: test\nincremental_key: user_id\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_frontmatter_with_leading_whitespace_and_spaces() {
        let source = "  \n\n---\nname: my_model\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.name, Some("my_model".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant, got {:?}", result),
        }
    }

    #[test]
    fn test_frontmatter_with_target() {
        let source =
            "---\nname: my_model\ntarget: spark_prod\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.target, Some("spark_prod".to_string()));
                assert_eq!(metadata.materialization, Some(Materialization::Table));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_without_target_is_none() {
        let source = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.target, None);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_ephemeral_materialization() {
        let source = "---\nname: staging\nmaterialization: ephemeral\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.materialization, Some(Materialization::Ephemeral));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_materialized_view() {
        let source = "---\nname: cached_report\nmaterialization: materialized_view\n---\nSELECT 1";

        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(
                    metadata.materialization,
                    Some(Materialization::MaterializedView)
                );
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_column_default() {
        let source = r#"---
name: test_model
materialization: table
columns:
  status:
    default: "'unknown'"
  priority:
    default: "0"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let status = metadata.columns.get("status").unwrap();
                assert_eq!(status.default.as_ref().unwrap(), "'unknown'");
                let priority = metadata.columns.get("priority").unwrap();
                assert_eq!(priority.default.as_ref().unwrap(), "0");
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_complex_type_defaults() {
        let source = r#"---
name: test_model
materialization: table
columns:
  meta:
    default: "STRUCT_PACK(a := 0, b := '')"
  tags:
    default: "[]::VARCHAR[]"
  lookup:
    default: "MAP {}"
  scores:
    default: "ARRAY[1, 2, 3]"
  flag:
    default: "TRUE"
  nothing:
    default: "NULL"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let meta = metadata.columns.get("meta").unwrap();
                assert_eq!(
                    meta.default.as_ref().unwrap(),
                    "STRUCT_PACK(a := 0, b := '')"
                );
                let tags = metadata.columns.get("tags").unwrap();
                assert_eq!(tags.default.as_ref().unwrap(), "[]::VARCHAR[]");
                let lookup = metadata.columns.get("lookup").unwrap();
                assert_eq!(lookup.default.as_ref().unwrap(), "MAP {}");
                let scores = metadata.columns.get("scores").unwrap();
                assert_eq!(scores.default.as_ref().unwrap(), "ARRAY[1, 2, 3]");
                let flag = metadata.columns.get("flag").unwrap();
                assert_eq!(flag.default.as_ref().unwrap(), "TRUE");
                let nothing = metadata.columns.get("nothing").unwrap();
                assert_eq!(nothing.default.as_ref().unwrap(), "NULL");
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_schema_evolution() {
        let source = r#"---
name: test_model
materialization: table
schema_evolution:
  strategy: full_refresh
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(
                    metadata.schema_evolution.unwrap().strategy,
                    SchemaEvolutionStrategy::FullRefresh
                );
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_with_format_override() {
        let source = "---\nname: my_model\nmaterialization: table\nformat: parquet\n---\nSELECT 1";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.format, Some(crate::config::TableFormat::Parquet));
            }
            _ => panic!("Expected Single variant"),
        }
    }

    #[test]
    fn test_frontmatter_without_format_is_none() {
        let source = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                assert_eq!(metadata.format, None);
            }
            _ => panic!("Expected Single variant"),
        }
    }

    // ── Generator file metadata tests ─────────────────────────────────────────

    /// A file with `generates: models` frontmatter routes to
    /// `FileMetadata::Generator` with correct `body_offset`.
    #[test]
    fn parse_generates_models_frontmatter_routes_to_generator_variant() {
        // Three-line frontmatter: "---\ngenerates: models\n---\n"
        // byte offsets: 0-3="---\n", 4-20="generates: models\n", 21-24="---\n"
        // body_offset should point to byte 25 (first byte after closing "---\n").
        let source = "---\ngenerates: models\n---\n[]\n";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Generator {
                metadata,
                body_offset,
            } => {
                assert_eq!(
                    metadata.generates.as_deref(),
                    Some("models"),
                    "metadata.generates must be Some(\"models\")"
                );
                // body_offset must point past the closing "---\n".
                assert_eq!(
                    &source[body_offset..],
                    "[]\n",
                    "body_offset must point to byte after closing ---\\n"
                );
            }
            other => panic!("Expected Generator variant, got: {:?}", other),
        }
    }

    /// `generates:` with a value other than `models` produces
    /// `MetadataError::GeneratesUnknownValue` carrying the offending value and a
    /// 1-based `(line, column)` span pointing at the first non-whitespace
    /// character after the `generates:` colon — the parser dispatch in Phase 2
    /// anchors the surfaced diagnostic at this span.
    #[test]
    fn parse_generates_unknown_value_emits_metadata_error() {
        let source = "---\ngenerates: views\n---\nSELECT 1";
        let result = extract_file_metadata(source);
        match result {
            Err(MetadataError::GeneratesUnknownValue { value, value_span }) => {
                assert_eq!(value, "views");
                // Outer line 1 = "---"; YAML "generates: views" is line 2.
                // "generates:" occupies columns 1..=10; " views" begins at column 12.
                assert_eq!(
                    value_span,
                    SourceSpan {
                        line: 2,
                        column: 12,
                    },
                    "value_span should anchor at the first non-whitespace char after the colon"
                );
            }
            other => panic!("Expected GeneratesUnknownValue(views), got: {:?}", other),
        }
    }

    /// `generates: models` combined with a `name:` field produces
    /// `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::NameField, .. }`.
    #[test]
    fn parse_generates_with_name_field_emits_mixed_error() {
        let source = "---\ngenerates: models\nname: foo\n---\n[]";
        let result = extract_file_metadata(source);
        assert!(
            matches!(
                result,
                Err(MetadataError::GeneratesMixedWithBareModel {
                    offending: MixedKind::NameField,
                    ..
                })
            ),
            "Expected GeneratesMixedWithBareModel(NameField), got: {:?}",
            result
        );
    }

    /// `generates: models` combined with Layer-1 `--- name: foo ---` section
    /// delimiters produces `MetadataError::GeneratesMixedWithBareModel { offending: MixedKind::SectionDelimiter, .. }`.
    #[test]
    fn parse_generates_with_section_delimiter_emits_mixed_error() {
        let source = "---\ngenerates: models\n---\n--- name: foo ---\nSELECT 1\n--- name: bar ---\nSELECT 2\n";
        let result = extract_file_metadata(source);
        assert!(
            matches!(
                result,
                Err(MetadataError::GeneratesMixedWithBareModel {
                    offending: MixedKind::SectionDelimiter,
                    ..
                })
            ),
            "Expected GeneratesMixedWithBareModel(SectionDelimiter), got: {:?}",
            result
        );
    }

    /// Files without `generates:` frontmatter continue to parse to
    /// `Single`, `Multi`, or `Empty` (regression guard).
    #[test]
    fn parse_no_generates_keeps_single_or_multi_variants() {
        // Single variant.
        let single = "---\nname: my_model\nmaterialization: table\n---\nSELECT 1";
        assert!(
            matches!(
                extract_file_metadata(single).unwrap(),
                FileMetadata::Single { .. }
            ),
            "Non-generates file must parse to Single"
        );

        // Multi variant.
        let multi = "--- name: m1 ---\n---\nSELECT 1\n--- name: m2 ---\n---\nSELECT 2";
        assert!(
            matches!(
                extract_file_metadata(multi).unwrap(),
                FileMetadata::Multi { .. }
            ),
            "Non-generates file must parse to Multi"
        );

        // Empty variant.
        let empty = "SELECT * FROM foo";
        assert!(
            matches!(extract_file_metadata(empty).unwrap(), FileMetadata::Empty),
            "Non-frontmatter file must parse to Empty"
        );
    }

    /// A generator file with extra frontmatter keys (`tags`, `owner`) admits them.
    #[test]
    fn parse_generates_models_with_other_frontmatter_keys_admits_them() {
        let source = "---\ngenerates: models\ntags: [cohort]\nowner: data-team\n---\n[]";
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Generator { metadata, .. } => {
                assert_eq!(metadata.tags, vec!["cohort".to_string()]);
                assert_eq!(metadata.owner.as_deref(), Some("data-team"));
            }
            other => panic!("Expected Generator variant, got: {:?}", other),
        }
    }

    #[test]
    fn test_frontmatter_with_backfill() {
        let source = r#"---
name: test_model
materialization: table
columns:
  full_name:
    backfill: "COALESCE(first_name || ' ' || last_name, '')"
---
SELECT * FROM users"#;
        let result = extract_file_metadata(source).unwrap();
        match result {
            FileMetadata::Single { metadata, .. } => {
                let col = metadata.columns.get("full_name").unwrap();
                assert_eq!(
                    col.backfill.as_ref().unwrap(),
                    "COALESCE(first_name || ' ' || last_name, '')"
                );
            }
            _ => panic!("Expected Single variant"),
        }
    }
}
