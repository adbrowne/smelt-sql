use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub sources: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct GraphNode {
    /// Canonical dot-path identifier, e.g. `silver.events_parsed`.
    pub id: String,
    /// Human-readable label — identical to `id` (the canonical dot-path).
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
    Test,
}

#[derive(Clone, Serialize)]
pub struct GraphEdge {
    /// Canonical dot-path of the upstream (dependency) node.
    pub source: String,
    /// Canonical dot-path of the downstream (dependent) node.
    pub target: String,
}

#[derive(Clone, Serialize)]
pub struct ModelDetailResponse {
    /// Canonical dot-path identifier, e.g. `silver.events_parsed`.
    pub name: String,
    pub path: String,
    pub sql: String,
    pub materialization: Option<String>,
    pub tags: Vec<String>,
    pub owner: Option<String>,
    pub description: Option<String>,
    /// Canonical dot-path identifiers of upstream models this model depends on.
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
    /// Function type signature: (inputs) -> outputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_type: Option<String>,
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

// --- Selector Resolution Types ---

#[derive(Clone, Deserialize)]
pub struct ResolveRequest {
    #[serde(default)]
    pub select: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ResolveResponse {
    pub selected: Vec<String>,
    pub excluded: Vec<String>,
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
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct RunPlanResponse {
    pub models: Vec<PlanModel>,
    pub execution_order: Vec<String>,
    pub total_batches: usize,
    pub cli_command: String,
}

#[derive(Clone, Serialize)]
pub struct PlanModel {
    /// Canonical dot-path identifier for this model in the execution plan.
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

// --- Run Execution Types ---

/// Request body for POST /api/run/execute (same shape as RunPlanRequest).
#[derive(Clone, Deserialize)]
pub struct RunExecuteRequest {
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub batch_size_days: Option<u32>,
    #[serde(default)]
    pub per_partition: bool,
    #[serde(default)]
    pub select: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "default_target")]
    pub target: String,
}

fn default_target() -> String {
    "dev".to_string()
}

/// Response for GET /api/run/status.
#[derive(Clone, Serialize)]
pub struct RunStatusResponse {
    pub state: RunState,
    /// Present when state is Running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batches_completed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batches_total: Option<usize>,
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Idle,
    Running,
}

/// Events streamed via WebSocket during run execution.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum RunProgressEvent {
    #[serde(rename = "run_started")]
    RunStarted {
        run_id: String,
        models: Vec<String>,
        total_batches: usize,
    },
    #[serde(rename = "model_started")]
    ModelStarted {
        run_id: String,
        model: String,
        model_index: usize,
        models_total: usize,
    },
    #[serde(rename = "batch_completed")]
    BatchCompleted {
        run_id: String,
        model: String,
        batch_index: usize,
        batches_total: usize,
        row_count: usize,
        duration_ms: u64,
    },
    #[serde(rename = "model_completed")]
    ModelCompleted {
        run_id: String,
        model: String,
        row_count: usize,
        duration_ms: u64,
    },
    #[serde(rename = "run_completed")]
    RunCompleted {
        run_id: String,
        models_executed: usize,
        duration_ms: u64,
    },
    #[serde(rename = "run_failed")]
    RunFailed {
        run_id: String,
        error: String,
        model: Option<String>,
    },
    #[serde(rename = "run_cancelled")]
    RunCancelled { run_id: String },
}

/// Response for GET /api/runs (run history list).
#[derive(Clone, Serialize)]
pub struct RunHistoryEntry {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub model_count: usize,
    pub models: std::collections::HashMap<String, RunModelSummary>,
}

/// Per-model summary in run history.
#[derive(Clone, Serialize)]
pub struct RunModelSummary {
    pub strategy: String,
    pub row_count: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<PlanTimeRange>,
}
