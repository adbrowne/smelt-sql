# Phase 7 summary — Lowering + execution of a key-addressed model-edge cell

## Shipped

- `docs/specs/incremental_models.md` §"Upstream model edges": documents what a key-addressed
  cell executes — group-grain fingerprint sidecar diff over the upstream's output at its
  `KeyedUpsert` key grain, projected to the downstream's key columns; candidate = the
  downstream's full SQL semi-joined to that key relation; write = the repair family's targeted
  `DELETE`+`INSERT`; both fail-loud legs named. §"Known Divergences" narrowed: lowering is no
  longer a gap, only the `smelt explain` rendering (phase 9's own scope).
- `smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select` — the new emitter:
  `SELECT DISTINCT <downstream_key_expr> FROM <upstream_table> WHERE <upstream_key_expr> IN
  (<changed_key_literals>)`, well-typed-empty (`WHERE FALSE`) for no changed keys, mirroring
  `emit_repair_group_digest_select`'s dialect-handling shape (`crates/smelt-logical/src/
  maintenance/emit.rs`).
- `smelt-runtime::maintenance_driver`:
  - `resolve_live_key_addressed_model_edge_cell` — derives the plan via
    `derive_model_maintenance_plan_with_edges`, selects the `PerGroupRecompute` cell whose
    `key_scope.is_some()`, resolves the cell's write leg via `resolve_repair_write`. Two fail-loud
    legs before any backend call: non-DuckDB dialect, and a `key_scope.keys` column not carried
    by the upstream's own proven `KeyedUpsert` key set (`edge.output_shape`, never
    `edge.unique_key` — a separate, often-undeclared field). Also derives the sidecar's digest
    column set from the downstream's own **clean** SQL (`fingerprint::fingerprint_projection`),
    threaded through the returned tuple rather than recomputed later against compiled SQL (whose
    `smelt.*` refs are already rewritten to physical names).
  - `resolve_key_addressed_affected_keys` / `execute_key_addressed_model_edge_cell` — the
    execution path: `diff_repair_group_sidecar_changed_keys` over the upstream's output →
    `emit_key_addressed_affected_keys_select`'s key relation → `repair_candidate_select` over the
    downstream's compiled full SQL → `execute_per_group_recompute`/`execute_diff_patch` with
    `RepairSidecarRefresh` (upstream sidecar advances transactionally with the write). Empty
    changed-key set → `Ok(None)`, reported as a no-op.
- `smelt-runtime::execute.rs`:
  - `model_edges_for` now derives each edge's `output_shape` from the same
    `workspace_output_delta_verdicts`/`upstream_output_delta_groups` fold `propagation.rs` uses
    (`workspace_output_delta_verdicts`/`upstream_output_delta_groups`/`model_output_delta_sources`/
    `model_skeleton_columns`/`bare_name` promoted to `pub(crate)` in `propagation.rs` for reuse) —
    no second, independent derivation.
  - The keyed run loop resolves a `key_addressed_edge_cell` alongside the existing
    `per_group_recompute_cell`, checked **before** the `(start_date, end_date)` dispatch (not
    nested inside its window-forward arm): a key-addressed cell has no run-window axis at all, and
    a clockless upstream typically drives its downstream into the snapshot-reconcile run shape,
    which has no window to match on. Never dispatched on the creation run.
- New tests: `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` (6: live
  resolution, missing-key-scope refusal, non-DuckDB refusal, restricted/well-typed-empty emitter
  unit tests, and a real-DuckDB two-model chain — `agg` (clockless `grain: key`, `SUM` over an
  append-only clocked source) → `downstream` (clockless `grain: key`, folds `agg` via
  `ANY_VALUE`/`GROUP BY`) — proving only the mutated key's group is rewritten and the untouched
  key is bit-identical, cross-checked against a full-refresh oracle).
- `statement_parity.rs` gained `key_addressed_model_edge_statements_come_from_the_emitter`: the
  same executed-vs-emitted byte-identity + result-equivalence proof `per_group_recompute_
  statements_come_from_the_emitter` runs for the ordinary repair route, over the key-addressed
  route instead — including asserting the affected-keys relation is the direct upstream-table
  projection (`SELECT DISTINCT`), never the ordinary sidecar-literal-`VALUES` shape. The existing
  structural no-authoring scan (`no_maintenance_statement_authoring_outside_the_emitter`) covers
  the new emitter for free — it lives inside the already-allowlisted `emit.rs` module.

## Decisions

- Task 8 (the `skeleton_closure.rs` conditional fix) was **not** needed: no fixture in this
  phase's scope tripped the `sources.`-only breadcrumb gap phase 6 flagged (the chain fixture's
  enrichment closure never reaches `find_enrichment_join`/`model_edge_enrichment_closure` for a
  `smelt.models.<addr>`-style ref — the downstream reads `agg` with no subdirectory). Left
  untouched per the plan's own instruction not to fix speculatively.
- The sidecar's digest column set is derived once, in the resolver, from the downstream's clean
  SQL — not recomputed at execution time from compiled SQL. `fingerprint_projection` matches
  `smelt.*` ref syntax; compiled SQL has already rewritten refs to physical table names, so
  recomputing there would silently classify every projection as `FullRow` (or worse, match
  nothing). This was caught by getting the phase's own real-DuckDB test green, not by design
  review alone — worth flagging for future phases touching this seam.
- The digest column set falls back to the upstream's own key columns alone when
  `fingerprint_projection` returns `FullRow` (unclassifiable) — narrower than the ideal (misses a
  payload-only mutation the downstream's SQL does not itself read), never a widening. Matches the
  existing repair family's own posture for an unclassified projection.
- `used_per_group_recompute`/`used_diff_patch` (the keyed run loop's manifest strategy labels) are
  reused verbatim for a dispatched key-addressed cell rather than adding a new label — the
  technique and write mechanism are genuinely identical to the ordinary repair route; only the
  affected-key discovery source differs, which is not manifest-visible today for the ordinary
  route either (`RepairDiscovery` has no manifest label).

## For the next planner

- **Scope note on where the live dispatch reaches.** This phase wired the key-addressed cell into
  the **keyed run loop** (`plan_is_keyed` — a `grain: key` downstream), the same branch
  `per_group_recompute_cell` already dispatches from. A `KeyedUpsert` upstream feeding a
  **`grain: partition`** downstream (the shape `append_model_edge_cells` also admits — the route
  applies whenever `edge.clock_col.is_none() || output_partition_col.is_none()`, not only for a
  keyed-grain downstream) is derived and plan-visible but has no live dispatch: the window-forward
  `DeleteInsert` branch (`execute.rs`'s other major branch, around `model_edges_for`'s T3
  delta-restriction wiring) never consults `resolve_live_key_addressed_model_edge_cell`. Not a
  regression — that combination had no live dispatch before this phase either — but success
  criterion 3's "a two-model chain … maintained incrementally end-to-end" is proven here only for
  a keyed-grain downstream. Flagging as real follow-up scope, not silently covered.
- **`smelt explain` doesn't render a key-addressed model-edge cell's execution route yet** —
  unaffected by this phase (plan-derivation rendering already existed from phase 6); the
  discovery-route/sidecar naming is phase 9's own surface scope per the outcome's phase-7 planning
  note.
- The `skeleton_closure.rs` `sources.`-only breadcrumb gap (flagged phase 6, re-flagged phase 7
  plan) remains untouched — still latent, still no fixture exercising it.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt-check + clippy zero-warnings + full
  `cargo test` + `example_diagnostics`; ~9 min full-workspace run).
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --test statement_parity --test execute_parity --test repair_lowering --test typed_edge_graph` — 6 + 23 + 4 + 17 + 5 = 55, all green.
- `cargo test -p smelt-logical --test keyed_model_edge --test walk_coverage` — 5 + 4, all green.
- `cargo test -p smelt-cli --test maintenance_conformance --test explain_maintenance` — 63 + 11, all green.
- `cargo clippy --all-targets -p smelt-logical -p smelt-runtime -p smelt-db -p smelt-cli` — zero warnings.
