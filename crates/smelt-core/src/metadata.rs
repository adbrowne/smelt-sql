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
use std::collections::HashMap;
use std::ops::Range;
use thiserror::Error;

/// Per-column metadata declared in model frontmatter.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ColumnMetadata {
    /// How late data can arrive for this column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_latency: Option<DataLatency>,

    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Metadata for a single model extracted from frontmatter
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ModelMetadata {
    /// Model name (optional in single-model files, required in multi-model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Materialization strategy (table or view)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization: Option<Materialization>,

    /// Incremental configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<IncrementalConfig>,

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

    /// Custom fields (forward compatibility)
    #[serde(flatten)]
    pub custom: HashMap<String, serde_yaml::Value>,
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

    /// Multiple models in one file
    Multi { models: Vec<ModelSection> },
}

/// One model section in a multi-model file
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSection {
    pub metadata: ModelMetadata,
    /// Byte range of SQL in file
    pub sql_range: Range<usize>,
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
}

/// Extract metadata from SQL source text
///
/// Returns `FileMetadata::Empty` if no frontmatter is present (backward compatible).
pub fn extract_file_metadata(source: &str) -> Result<FileMetadata, MetadataError> {
    let trimmed = source.trim_start();

    // Check for single-model frontmatter
    if trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") {
        // Check if this is actually a multi-model file
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

/// Extract metadata from a single-model file
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
}
