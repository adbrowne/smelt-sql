use serde::Serialize;

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
