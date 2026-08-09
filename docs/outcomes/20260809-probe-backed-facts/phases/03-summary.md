# Phase 3 summary — `referential_integrity` tripwire wired into the runs that consume the closure narrowing

**Shipped:**
- `SkeletonSourceClosure::Closed` now carries `row_preservation: RowPreservation` (`JoinShape` /
  `DeclaredReferentialIntegrity { source }`) — `crates/smelt-logical/src/analysis/skeleton_closure.rs`.
  Every construction/match site across `smelt-logical`/`smelt-runtime`/`smelt-cli` fixed (~15 sites).
- `emit_count_preservation_probe_from_body(body_sql, enrichment_source)` — `crates/smelt-logical/src/maintenance/emit.rs`.
  Splices the named join out of a compiled model body by text range; builds both `SELECT 1 …`
  sides sharing the body's own `WHERE`; delegates to `emit_count_preservation_probe`. Matches the
  join by `TableRef::bare_path_text` (new method, `crates/smelt-parser/src/ast.rs`) against the
  *physical* compiled table name, not `resolve_table_ref_source_name` (which only resolves
  unresolved `smelt.<path>` refs and would never match an already-compiled body).
- `execute_delete_insert_with_delta_restriction` (`crates/smelt-runtime/src/maintenance_driver.rs`)
  dispatches the probe before trusting a `Closed { DeclaredReferentialIntegrity }` restriction:
  fails the run (`BackendError::ExecutionFailed`, message-prefixed `SourceCountPreservationViolated`,
  naming source/counts/remedy) on a violation before any write; falls back to the widened scan
  (dropping the restriction) if the probe can't be built from the body.
- `derive_model_maintenance_plan`/`_with_edges` (`smelt-db`) gained a `source_referential_integrity`
  parameter; the production Salsa callers (`smelt-db/src/lib.rs`'s `maintenance_plan`/`maintenance_
  plan_report`, `maintenance_plan_diagnostics`, `smelt-runtime`'s propagation walk) now build the
  real map via new `build_source_referential_integrity` (mirrors `build_key_recurrences`); every
  other caller (test fixtures, resolvers that don't need it) passes an empty map unchanged.
- Spec: `model_properties.md` §"Skeleton-source closure" states the route-naming + probe
  obligation; the `referential_integrity` probe-registry row flips `built (unwired)` → `built`;
  Known Divergences updated in `model_properties.md`/`sources.md`/`diagnostics.md`.

**Decisions:**
- `body_sql`'s join is matched by *physical* table identifier (`bare_path_text`, exact-or-last-
  segment), not by `smelt.<path>` ref resolution — the runtime's compiled body never carries
  unresolved refs; the first-draft implementation using `resolve_table_ref_source_name` would
  never have matched a real call and was caught by `probe_execution.rs`'s real-DuckDB tests.
- The `source_referential_integrity` param was threaded to `derive_model_maintenance_plan` itself
  (not a separate `_with_referential_integrity` wrapper) — the many source-only test callers just
  pass an empty map, mechanical and bounded (~25 call sites), versus adding a field to `SourceFacts`
  (101 literal-construction sites across the workspace).
- Runtime *dispatch* reachability for the declared route stays scoped to `execute_delete_insert_
  with_delta_restriction`'s existing model-edge call site, per the outcome's own "Out of scope"
  note — model edges never carry RI (`model_edge_enrichment_closure` hardcodes `None`), so this
  phase's runtime wiring is exercised directly by new unit tests, not (yet) through a live
  UpstreamMutation dispatch path. Widening *which* cells consult a declared-RI closure remains
  tracked separately.

**For the next planner:**
- Phase 4 (runtime wiring: `probes:` in `Config`, cadence, firing → diagnostic + remedy marking)
  is unblocked structurally by this phase's `BackendError::ExecutionFailed` shape, but no `probes:`
  cadence field exists yet on any of the four still-unwired probes (monotonicity, FD, bounded-domain,
  append-only) — this phase only wired the fifth (RI), whose obligation predates the cadence
  design. Phase 4 should decide whether RI's dispatch should also respect `probes: off/periodic`
  (today it always fires unconditionally whenever the restriction is taken).
- `docs/specs/sources.md` §Known Divergences still correctly notes: only the source-enrichment
  `UpstreamMutation` route can ever derive a `DeclaredReferentialIntegrity` closure; a model-edge
  creation cell's own closure is always derived against an empty RI map. If a future phase wants
  the wired probe to actually gate a *live* run (not just be probe-ready), it needs a new resolver
  analogous to `resolve_live_delta_restriction_facts` but keyed on `Trigger::UpstreamMutation`,
  reading the fingerprint-sidecar-derived observed delta for the *source* rather than the model-edge
  observed-delta table — out of this phase's scope, not started.
- Discovered while wiring: `JoinClause` had no `syntax()` accessor in `smelt-parser`; added one
  (trivial, matches every other AST wrapper's convention). `TableRef::bare_path_text` is a new
  sibling of the existing `quoted_identifier_path_text` for the unquoted case — general-purpose,
  not RI-specific, and may be useful to any future leaf classifier that needs a physical-SQL join
  target's full dotted path.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test emit_statements --test probe_execution --test probe_obligation --test skeleton_closure --test skeleton_closure_pinned --test maintenance_referential_integrity` — 74 passed.
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity` — 51 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 59 passed.
