# Phase 3 summary — CTE-only `event_time_column` detection in the outer-visibility check

**Shipped:**
- `crates/smelt-logical/src/rules/rule_diagnostics.rs`: new Case 3 in `check_event_time_injectable`
  (`check_cte_from_injectable` + `resolve_cte_projects_column`) — a batched model's outer `FROM`
  naming a CTE that does not project `event_time_column` is now rejected with
  `EventTimeColumnNotVisibleAtOuterSelect` before execution, resolved through a chain of CTEs
  (visited-set + depth-16 cap against cycles), with conservative (accepting) fallback for a
  wildcard projection, a set-operation body, a `WITH RECURSIVE` clause, or a joined (multi-table)
  outer `FROM`.
- 8 new unit tests in `rule_diagnostics.rs` covering: direct rejection, direct acceptance,
  wildcard conservatism, chained-CTE rejection (message names the outer-bound alias, not the
  root), declared column-list-as-projection precedence over the body's select list, join
  conservatism, recursive-CTE conservatism, and a plain-table-FROM regression guard.
- `crates/smelt-logical/tests/partition_residue_probes.rs::probe_cte_only_event_time_column`
  inverted to assert the diagnostic now fires; doc comment updated to record phase 3 landing.
- Spec: `docs/specs/incremental_shapes.md` §"Event-time outer-visibility" extended to name the
  CTE case (chain resolution + the three conservative-accept conditions); the matching Known
  Divergences bullet deleted; both diagnostic-code table entries (§Surface and
  `docs/specs/diagnostics.md`) widened from "subquery" to "subquery or CTE"/"subquery, or CTE".

**Decisions:**
- Declared CTE column list (`recent(user_id, event_ts) AS (...)`) is authoritative over the
  body's own select-list aliases when present — matches DuckDB/PostgreSQL CTE column-aliasing
  semantics, and the plan's test 5 pins it.
- `WITH RECURSIVE` is checked once at the `WithClause` level (skip Case 3 entirely) rather than
  relying solely on the visited-set cycle guard — belt-and-suspenders, and matches the spec
  language ("recursive, ... left accepted").
- Reused the already-parsed `SelectStmt`/`Cte` AST (`Cte::query().select_stmt()`) rather than
  round-tripping through `is_column_projected_in_sql`'s text-based re-parse — avoids a second
  parse of the CTE body text and keeps wildcard/set-operation/chain detection in one place.

**For the next planner:**
- Swept `examples/` for batched models (`grain: partition`/`refresh: incremental`) with a
  CTE-shaped outer `FROM`: `web_analytics/models/silver/sessions.sql` (+ its tutorial-stage
  copy) and `sessions_chained.sql`. Both project `session_time_column`
  (`session_start_date`) directly in the CTE the outer `FROM` binds (`sessionized` /
  `aggregated`) — true negatives, no example changes needed, no classifier over-reach found.
- No new forward-looking gaps surfaced beyond phase 2's already-recorded, out-of-scope
  `CASE`-nested-window finding.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test partition_residue_probes` — PASS (2/2, including inverted probe).
- `cargo test -p smelt-logical --test walk_coverage` — PASS (4/4).
- `cargo test -p smelt-lsp --test example_workspaces` — PASS (35/35).
