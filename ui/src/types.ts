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
  node_type: 'model' | 'source';
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
