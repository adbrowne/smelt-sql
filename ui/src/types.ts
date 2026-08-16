export interface GraphResponse {
  nodes: GraphNode[];
  edges: GraphEdge[];
  sources: string[];
}

export interface GraphNode {
  id: string;
  label: string;
  materialization: string | null;
  tags: string[];
  description: string | null;
  has_errors: boolean;
  node_type: 'model' | 'source' | 'test';
}

export interface GraphEdge {
  source: string;
  target: string;
}

export interface ModelDetailResponse {
  name: string;
  path: string;
  sql: string;
  materialization: string | null;
  tags: string[];
  owner: string | null;
  description: string | null;
  refs: string[];
  columns: ColumnInfo[];
  incremental?: IncrementalInfo;
  batch_safety?: BatchSafetyInfo;
  diagnostics?: DiagnosticInfo[];
  function_type?: string;
}

export interface ColumnInfo {
  name: string;
  data_type: string | null;
  nullable: boolean | null;
  source: ColumnSourceInfo;
  expression: string;
}

export type ColumnSourceInfo =
  | { type: 'computed' }
  | { type: 'from_model'; model: string; column: string }
  | { type: 'wildcard'; model: string }
  | { type: 'external_table'; table: string }
  | { type: 'unknown' };

export interface ProjectResponse {
  name: string;
  version: number;
  model_count: number;
  source_count: number;
}

export interface IncrementalInfo {
  granularity: string;
  partition_column: string;
  event_time_column: string;
  unique_key: string[];
}

export interface BatchSafetyInfo {
  level: string;
  max_chunk_days?: number;
  context_days?: number;
  reason?: string;
}

export interface DiagnosticInfo {
  severity: string;
  message: string;
  line?: number;
  column?: number;
}

// --- Selector Resolution Types ---

export interface ResolveRequest {
  select?: string[];
  exclude?: string[];
}

export interface ResolveResponse {
  selected: string[];
  excluded: string[];
}

// --- Run Planner Types ---

export interface RunPlanRequest {
  start: string;
  end: string;
  batch_size_days?: number;
  per_partition?: boolean;
  select?: string[];
  exclude?: string[];
}

export interface RunPlanResponse {
  models: PlanModel[];
  execution_order: string[];
  total_batches: number;
  cli_command: string;
}

export interface PlanModel {
  name: string;
  is_incremental: boolean;
  batch_safety?: BatchSafetyInfo;
  partition_range?: { start: string; end: string };
  filter_range?: { start: string; end: string };
  batches: PlanBatch[];
}

export interface PlanBatch {
  partition_start: string;
  partition_end: string;
  filter_start: string;
  filter_end: string;
}

// --- Model Diagnostics Types ---
//
// Mirrors `smelt_runtime::diagnostics::ModelDiagnostics`
// (`crates/smelt-runtime/src/diagnostics.rs`), the shared builder both
// `smelt explain` and `GET /api/models/:name/diagnostics` render verbatim
// (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder").

export interface ModelDiagnosticsResponse {
  model: string;
  properties: PropertySet;
  contract: RelationContractView;
  inbound_edges: InboundEdgeContract[];
  cells: PlanCellDiagnostics[];
}

export interface PropertySet {
  columns: string[];
  grain: Grain;
  functional_dependencies: DerivedFd[];
  determinism: ColumnDeterminism[];
  comparability: ColumnComparability[];
  discriminants: ColumnDiscriminant[];
  literal_columns: [string, string][];
  has_set_op_barrier: boolean;
  has_fan_out_join: boolean;
  row_identity: RowIdentityVerdict;
  source_bounds: Record<string, BoundResult>;
}

export interface Grain {
  keys: string[][];
}

export interface DerivedFd {
  key: string[];
  determines: string;
}

export interface ColumnDeterminism {
  output: string;
  level: 'Clean' | 'Run' | 'Row';
}

export interface ColumnComparability {
  output: string;
  comparability: 'Comparable' | 'Incomparable';
}

export interface ColumnDiscriminant {
  output: string;
  discriminants: Discriminants;
}

export interface Discriminants {
  is_monoid: boolean;
  needs_inverse: boolean;
  decomposable: boolean;
  monotone: 'Value' | 'Order' | 'None';
}

export type RowIdentity = { Key: string[] } | 'WholeRow';

export interface RowIdentityVerdict {
  identity: RowIdentity;
  proven_mismatch: string[] | null;
}

export type BoundResult =
  | { type: 'bounded'; source_partition_col: string; before: string; after: string }
  | { type: 'unbounded' }
  | { type: 'not_derivable' };

export interface RelationContractClock {
  event_time_column: string;
  partition_column: string;
  granularity: string;
}

export interface RelationContractView {
  clock?: RelationContractClock;
  identity?: string[];
  derived_grain?: 'partition' | 'key' | 'key_per_partition';
}

export interface InboundEdgeContract {
  name: string;
  provider: 'source' | 'model';
  contract: RelationContractView;
}

export type Technique = 'DeleteInsert' | 'KeyedFold' | 'ColumnScopedMerge' | 'InPlaceUpdate';

export interface PreviewStatement {
  sql: string;
}

export type Admissibility =
  | { verdict: 'admitted' }
  | { verdict: 'interchangeable_alternative' }
  | { verdict: 'not_applicable'; reason: string };

export interface TechniquePreview {
  technique: Technique;
  transactional: boolean;
  statements: PreviewStatement[];
  admissibility: Admissibility;
}

export interface PlanCellDiagnostics {
  group: string;
  trigger: string;
  corner: string;
  admitted_technique: Technique;
  row_identity: RowIdentity;
  technique_previews: TechniquePreview[];
}

// --- Run Execution Types ---

export interface RunExecuteRequest {
  start: string;
  end: string;
  batch_size_days?: number;
  per_partition?: boolean;
  select?: string[];
  exclude?: string[];
  target?: string;
}

export interface RunStatusResponse {
  state: 'idle' | 'running';
  run_id?: string;
  current_model?: string;
  models_completed?: number;
  models_total?: number;
  batches_completed?: number;
  batches_total?: number;
}

export type RunProgressEvent =
  | { type: 'run_started'; run_id: string; models: string[]; total_batches: number }
  | { type: 'model_started'; run_id: string; model: string; model_index: number; models_total: number }
  | { type: 'batch_completed'; run_id: string; model: string; batch_index: number; batches_total: number; row_count: number; duration_ms: number }
  | { type: 'model_completed'; run_id: string; model: string; row_count: number; duration_ms: number }
  | { type: 'run_completed'; run_id: string; models_executed: number; duration_ms: number }
  | { type: 'run_failed'; run_id: string; error: string; model?: string }
  | { type: 'run_cancelled'; run_id: string };

export interface RunHistoryEntry {
  run_id: string;
  started_at: string;
  completed_at?: string;
  model_count: number;
  models: Record<string, RunModelSummary>;
}

export interface RunModelSummary {
  strategy: string;
  row_count: number;
  duration_ms: number;
  time_range?: { start: string; end: string };
}
