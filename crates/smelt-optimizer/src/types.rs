use serde::{Deserialize, Serialize};

/// A transformation the optimizer wants to apply to a model.
#[derive(Debug, Clone, Serialize)]
pub enum Transformation {
    /// Replace a model's execution with a multi-step plan (e.g., cube split).
    ReplaceWithPlan {
        model: String,
        steps: Vec<ExecutionStep>,
    },
    /// Mark a model for incremental execution.
    SetIncremental {
        model: String,
        /// The source time column used for filtering (e.g., `event_time`).
        event_time_column: String,
        /// The partition column alias in SELECT (e.g., `event_date`).
        partition_column: String,
        /// Partition granularity.
        granularity: Granularity,
    },
}

/// A single step in a multi-step execution plan.
#[derive(Debug, Clone, Serialize)]
pub enum ExecutionStep {
    /// Create a temporary table from a query.
    CreateTemp { name: String, sql: String },
    /// Append query results to an existing temp table.
    AppendToTemp { name: String, sql: String },
    /// Run the final query that produces the model's output.
    FinalQuery { sql: String },
    /// Drop a temporary table.
    DropTemp { name: String },
}

/// An optimization opportunity detected by a rule.
#[derive(Debug, Clone, Serialize)]
pub struct Opportunity {
    pub rule_name: String,
    pub model: String,
    pub description: String,
    pub data: OpportunityData,
}

/// Per-rule data attached to an opportunity.
#[derive(Debug, Clone, Serialize)]
pub enum OpportunityData {
    CubeSplit {
        count_distinct_count: usize,
        group_by_keys: Vec<String>,
    },
    Incremental {
        event_time_column: String,
        partition_column: String,
        granularity: Granularity,
    },
}

/// Granularity for incremental partition generation.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Hour,
    Day,
    Month,
}

/// Safety overrides for incremental materialization checks.
///
/// Each flag allows a specific pattern that is normally rejected
/// because it can produce different results on partial vs full data.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IncrementalSafetyOverrides {
    #[serde(default)]
    pub allow_window_functions: bool,
    #[serde(default)]
    pub allow_having: bool,
    #[serde(default)]
    pub allow_limit: bool,
    #[serde(default)]
    pub allow_subqueries: bool,
    #[serde(default)]
    pub allow_nondeterministic: bool,
    #[serde(default)]
    pub allow_distinct: bool,
}

/// Incremental configuration from YAML frontmatter.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IncrementalConfig {
    pub partition_column: String,
    pub event_time_column: String,
    pub granularity: Granularity,
    #[serde(default)]
    pub safety_overrides: IncrementalSafetyOverrides,
}

/// Frontmatter configuration parsed from model SQL files.
#[derive(Debug, Clone, Deserialize)]
pub struct Frontmatter {
    pub materialized: Option<String>,
    pub incremental: Option<IncrementalConfig>,
}

impl Frontmatter {
    /// Parse frontmatter from a SQL file that starts with `---`.
    pub fn parse(sql: &str) -> Option<Self> {
        let trimmed = sql.trim_start();
        if !trimmed.starts_with("---") {
            return None;
        }
        let after_first = &trimmed[3..];
        let end = after_first.find("---")?;
        let yaml_str = &after_first[..end];
        serde_yaml::from_str(yaml_str).ok()
    }

    /// Strip frontmatter from SQL, returning just the SQL portion.
    pub fn strip(sql: &str) -> &str {
        let trimmed = sql.trim_start();
        if !trimmed.starts_with("---") {
            return sql;
        }
        let after_first = &trimmed[3..];
        match after_first.find("---") {
            Some(end) => {
                let rest = &after_first[end + 3..];
                rest.trim_start_matches(['\n', '\r'])
            }
            None => sql,
        }
    }
}
