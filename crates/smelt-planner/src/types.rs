use serde::{Deserialize, Serialize};
pub use smelt_core::config::{
    Granularity, IncrementalConfig, IncrementalSafetyOverrides, IncrementalStrategy,
    Materialization, Weekday,
};

/// A transformation the optimizer wants to apply to a model.
#[derive(Debug, Clone, Serialize)]
pub enum Transformation {
    // -- Single-model transformations (existing) --
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

    // -- Graph-level transformations (new) --
    /// Create a synthetic intermediate node (e.g., shared materialization, cube split temp).
    CreateNode {
        name: String,
        sql: String,
        dependencies: Vec<String>,
        /// Which user-authored model spawned this node.
        origin: String,
        materialization: Materialization,
    },
    /// Remove a model from execution (e.g., fused into another model).
    RemoveNode { model: String },
    /// Redirect all references from one model to another (model fusion).
    RedirectRef { from: String, to: String },
    /// Override a model's materialization strategy.
    SetMaterialization {
        model: String,
        materialization: Materialization,
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
