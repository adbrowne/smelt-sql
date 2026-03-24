use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub sources: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub materialization: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub has_errors: bool,
    pub node_type: NodeType,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Model,
    Source,
}

#[derive(Clone, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Clone, Serialize)]
pub struct ModelDetailResponse {
    pub name: String,
    pub path: String,
    pub sql: String,
    pub materialization: Option<String>,
    pub tags: Vec<String>,
    pub owner: Option<String>,
    pub description: Option<String>,
    pub refs: Vec<String>,
    pub columns: Vec<ColumnInfo>,
    /// Incremental configuration (if model is incremental).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<IncrementalInfo>,
    /// Batch safety classification for backfill operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_safety: Option<BatchSafetyInfo>,
    /// Parse errors and type diagnostics.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticInfo>,
}

#[derive(Clone, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: Option<String>,
    pub nullable: Option<bool>,
    pub source: ColumnSourceInfo,
    pub expression: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub enum ColumnSourceInfo {
    #[serde(rename = "computed")]
    Computed,
    #[serde(rename = "from_model")]
    FromModel { model: String, column: String },
    #[serde(rename = "wildcard")]
    Wildcard { model: String },
    #[serde(rename = "external_table")]
    ExternalTable { table: String },
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Clone, Serialize)]
pub struct ProjectResponse {
    pub name: String,
    pub version: u32,
    pub model_count: usize,
    pub source_count: usize,
}

#[derive(Clone, Serialize)]
pub struct IncrementalInfo {
    pub granularity: String,
    pub partition_column: String,
    pub event_time_column: String,
    pub unique_key: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct BatchSafetyInfo {
    pub level: String,
    pub max_chunk_days: Option<u32>,
    pub context_days: Option<u32>,
    pub reason: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct DiagnosticInfo {
    pub severity: String,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

// --- Run Planner Types ---

#[derive(Clone, Deserialize)]
pub struct RunPlanRequest {
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub batch_size_days: Option<u32>,
    #[serde(default)]
    pub per_partition: bool,
    #[serde(default)]
    pub select: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct RunPlanResponse {
    pub models: Vec<PlanModel>,
    pub execution_order: Vec<String>,
    pub total_batches: usize,
}

#[derive(Clone, Serialize)]
pub struct PlanModel {
    pub name: String,
    pub is_incremental: bool,
    pub batch_safety: Option<BatchSafetyInfo>,
    pub partition_range: Option<PlanTimeRange>,
    pub filter_range: Option<PlanTimeRange>,
    pub batches: Vec<PlanBatch>,
}

#[derive(Clone, Serialize)]
pub struct PlanTimeRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Serialize)]
pub struct PlanBatch {
    pub partition_start: String,
    pub partition_end: String,
    pub filter_start: String,
    pub filter_end: String,
}
