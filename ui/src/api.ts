import type { GraphResponse, ModelDetailResponse, ProjectResponse, ResolveRequest, ResolveResponse, RunPlanRequest, RunPlanResponse, RunExecuteRequest, RunStatusResponse, RunHistoryEntry } from './types';

const BASE = '';

export async function fetchProject(): Promise<ProjectResponse> {
  const res = await fetch(`${BASE}/api/project`);
  if (!res.ok) throw new Error(`Failed to fetch project: ${res.statusText}`);
  return res.json();
}

export async function fetchGraph(): Promise<GraphResponse> {
  const res = await fetch(`${BASE}/api/graph`);
  if (!res.ok) throw new Error(`Failed to fetch graph: ${res.statusText}`);
  return res.json();
}

export async function fetchModel(name: string): Promise<ModelDetailResponse> {
  const res = await fetch(`${BASE}/api/models/${encodeURIComponent(name)}`);
  if (!res.ok) throw new Error(`Failed to fetch model ${name}: ${res.statusText}`);
  return res.json();
}

export async function resolveSelectors(request: ResolveRequest): Promise<ResolveResponse> {
  const res = await fetch(`${BASE}/api/resolve`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to resolve selectors: ${res.statusText}`);
  }
  return res.json();
}

export async function fetchRunPlan(request: RunPlanRequest): Promise<RunPlanResponse> {
  const res = await fetch(`${BASE}/api/run/plan`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to compute run plan: ${res.statusText}`);
  }
  return res.json();
}

export async function executeRun(request: RunExecuteRequest): Promise<{ run_id: string }> {
  const res = await fetch(`${BASE}/api/run/execute`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to start run: ${res.statusText}`);
  }
  return res.json();
}

export async function cancelRun(): Promise<void> {
  const res = await fetch(`${BASE}/api/run/cancel`, { method: 'POST' });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to cancel run: ${res.statusText}`);
  }
}

export async function fetchRunStatus(): Promise<RunStatusResponse> {
  const res = await fetch(`${BASE}/api/run/status`);
  if (!res.ok) throw new Error(`Failed to fetch run status: ${res.statusText}`);
  return res.json();
}

export async function fetchRuns(): Promise<RunHistoryEntry[]> {
  const res = await fetch(`${BASE}/api/runs`);
  if (!res.ok) throw new Error(`Failed to fetch run history: ${res.statusText}`);
  return res.json();
}

export async function fetchRunById(id: string): Promise<RunHistoryEntry> {
  const res = await fetch(`${BASE}/api/runs/${encodeURIComponent(id)}`);
  if (!res.ok) throw new Error(`Failed to fetch run ${id}: ${res.statusText}`);
  return res.json();
}
