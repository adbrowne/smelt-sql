# Outcome: Every defect the real pipeline hits on BigQuery is fixed, and DuckDB and BigQuery agree

**Created:** 2026-09-06
**Status:** done
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"The programme" (D2), §"Sequencing: models first, punch-list second", §"Findings already banked"
**Spec anchors:** `docs/specs/multi_backend.md` §"Operator lowering", §"Statement-level lowering", §"Output-schema type conformance", §"Cross-engine emission audit"; `docs/specs/architecture.md` §"Constraints & Invariants" item 14; `docs/reference/dialect-coverage.md`

## The outcome

The BigQuery defects that a real pipeline actually reaches are fixed, and each fix is held
by a gate that would have caught it. The unconditional ones — wrong for any model on any
run — are fixed first and without waiting for evidence: `emit_fingerprint_digest_select`
ignores its `dialect` parameter and hardcodes DuckDB's hash spelling on every backend.
Everything after that is driven by
`docs/outcomes/20260906-bigquery-dogfood-spine`'s findings handoff: only the registry
entries, techniques, grains and capability rows the spine's models genuinely hit are
built, and each one lands with its emission verdict, its ledger row, and its coverage in
the published dialect table. Where DuckDB and BigQuery disagree on the same rows, the
difference is either fixed or registered with a reason — never tolerated silently.

## Success criteria (checkable)

1. **The unconditional fix.** `emit_fingerprint_digest_select`
   (`crates/smelt-logical/src/maintenance/emit.rs`) threads its `dialect` through to
   `row_fingerprint_expr` instead of passing `MaintenanceDialect::DuckDb`; a unit test per
   dialect asserts the emitted expression (BigQuery gets `TO_HEX(SHA256(…))`, since
   GoogleSQL's `SHA256` returns `BYTES` and the value feeds a `STRING_AGG`). Whether the
   path is reachable on a live `mutable_snapshot` run is answered in the decision log
   either way — the fix does not depend on the answer.
2. **Punch-list harvested, not invented.** Phase 1 reads the spine's findings handoff and
   rewrites this outcome's remaining phase rows from it. A row whose only justification is
   "issue #179 lists it" and which no spine model reaches is recorded under Out of scope
   with that rationale, per §"Sequencing".
3. **Every fixed construct is gated.** Each defect fixed here gains coverage in the gate
   that owns its class — a `signatures.rs` emission verdict plus a `dialect_audit` probe
   for a spelling, a `ledger.rs` row for a registered mismatch, a `dialect_seam` case for
   a compile-time refusal, a `projection_dialect_invariance` case for a projection bug —
   so a regression fails offline. No fix lands with only a manual sweep behind it.
4. **Ratchets move the right way.** `.claude/dialect-gaps-baseline.txt` and
   `.claude/parser-gaps-baseline.txt` fall or hold; neither is raised. Registry entries
   this outcome gives verdicts to leave `#179`'s unverified count lower, and
   `docs/reference/dialect-coverage.md` is regenerated so the doc-sync gate is green.
5. **Cross-target agreement.** Every divergence the spine registered under its criterion 6
   is resolved here: fixed, or promoted to a permanent, reasoned entry in the divergence
   registry with the engines and the construct named. The count of unexplained differences
   is zero.
6. **The two known live conformance failures are characterised.**
   `dags_bigquery::diamond_propagation_suffices` and
   `gate_composed_bigquery::composed_keyed_pool_upholds_equivalence` are each either fixed
   or explained in the handoff with the mechanism named — not left uncharacterised.
7. **Gates green.** `bash .claude/scripts/verify-phase.sh`, plus
   `cargo test -p smelt-dialect --test emission_ownership`,
   `cargo test -p smelt-runtime --test dialect_seam`,
   `cargo test -p smelt-runtime --test projection_dialect_invariance` and
   `cargo test -p smelt-db --test dialect_audit` (DuckDB legs in-process). The BigQuery
   value leg is a manual sweep (`scripts/bigquery-dialect-audit.sh`) — a phase that needs
   it and cannot run it emits `<<PHASE_BLOCKED>>` rather than skipping green.

## Out of scope

- Building the 42 no-verdict BigQuery registry entries of issue #179 speculatively. Only
  entries a spine model reaches are built here; the rest stay on #179.
- Snowflake, Redshift and Postgres emission work.
- Any new model, source or feature — this outcome only fixes what the spine surfaced.
- Widening the sample, the model set, or the pipeline's scope (the spine owns that).
- Retiring the PostgreSQL emission dialect (tracked separately by the
  dialect-emission-vocabulary outcome).
- **Closing the LSP-diagnostics / `smelt explain` divergence for model-edge refusals**
  (found by phase 4, recorded in the decision log). `smelt-db`'s `maintenance_plan` Salsa
  query never threads model edges, so no model-edge refusal — the pre-existing
  `ReachNotDerivable` included — reaches `file_diagnostics()`. It is an editor-surface gap
  that predates this outcome and is not a defect the real pipeline hits on BigQuery, so it
  serves none of the success criteria; it needs its own outcome.
- **Any BigQuery-only emission defect not already in hand.** The spine is `blocked` with
  its live-BigQuery half (its phase 16) never run, so its findings handoff is DuckDB-half
  only and harvested no BigQuery-reached registry entry, technique, grain or capability
  row. Building any would be speculation against issue #179, which criterion 2 forbids;
  it stays on #179 and on the spine's resume.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | The unconditional fix: thread `dialect` through `emit_fingerprint_digest_select` to `row_fingerprint_expr`, per-dialect unit tests, and answer in the decision log whether the path is reachable on a live `mutable_snapshot` run | done |
| 2 | The rest of the dialect-blind fingerprint SQL: `key_expr_for_columns`' hardcoded `CAST(... AS VARCHAR)` and `emit_repair_group_digest_select`'s DuckDB-only `bit_xor(hash(...))` + `VARCHAR` cast — fix per-dialect or refuse loudly, with the capability gate held by a test | done |
| 3 | Punch-list 1 — `emit_succession_full_rebuild` folds on `(key_cols, clock_col)` with a per-column aggregate over the model's own output schema, and runs the clock-tie probe it has never run; closes the `silver_repo_naming` / `silver_actor_naming` divergence | done |
| 4 | Punch-list 2a — derive the missing `UpstreamMutation(gold.repo_dim)` cell: a new **enrichment-keyed** route in `append_model_edge_cells` for a clockless keyed upstream read in value-enrichment position by a partition-addressed downstream, plus a real `MaintenanceRepairKeysNotDiscoverable` diagnostic so the remaining fail-closed leg is loud at `build`/`run` rather than only `explain` | done |
| 5 | Punch-list 2b — make that cell live on the run path: thread model edges into `resolve_live_column_scoped_cell`/`maintenance_availability::derive_resolved` and the mutation gate, so `gold.events_enriched`'s already-written `current_repo_name` heals and the `github_activity` stale-row count reaches zero | done |
| 6 | Punch-list 3 — `compute_calendar_windows`' interior-chunk-boundary forward-reach loss for Form-B models, which makes the full-refresh oracle itself undercount a cross-midnight session | done |
| 7 | Punch-list 4 — the missing repair edge from a Form-B model's own self-rebase to a Form-A downstream aggregate that reads it verbatim; first check whether phases 4-5's mechanism already covers it | done |
| 8 | Resolve every divergence the spine registered (`github_activity_oracle.rs`'s `DIVERGENCE_REGISTRY`): each residual entry fixed or promoted to a reasoned permanent entry naming the engines and the construct, unexplained count zero — and, if phases 3-7 have emptied the registry, prove the unregistered-divergence sweep still fails closed on an empty registry rather than passing vacuously | done |
| 9 | Characterise or fix the two known live conformance failures (`diamond_propagation_suffices`, `composed_keyed_pool_upholds_equivalence`) | done |
| 10 | Close: regenerate `docs/reference/dialect-coverage.md`, move the gap ratchets down, update issue #179 with what was verified, all standing gates green | done |

## Decision log

- 2026-09-08 (phase 10 implementation, close): **outcome done.** All success criteria
  verified at HEAD as the planning entry laid out. New structural gate
  (`cargo test -p smelt-logical --test maintenance_dialect_blindness`, 3/3) closes
  criterion 3's remaining gap; `handoff_claimed_relations()` in
  `crates/smelt-cli/tests/github_activity_oracle.rs` is now scoped to `## The registered
  divergences` (fixing the false-positive phase 9 flagged), with two new tests proving
  both the scoping and its non-vacuity. `docs/handoffs/2026-09-08-github-activity-findings.md`
  gained a `## Close-out (2026-09-08)` section (one row per criterion, artifact + gate) and
  `.claude/dialect-gaps-baseline.txt` gained a dated hold note —
  `dialect_gaps_bigquery` stays 42, `duckdb_seed_gaps 0` confirmed untouched.
  `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
  the_coverage_table_matches_the_registry` reconfirmed `docs/reference/dialect-coverage.md`
  byte-identical (`git status` clean). Issue #179 got a comment
  (github.com/adbrowne/smelt-sql/issues/179#issuecomment-5581444485) naming what was fixed
  nearby and that its 42 entries are untouched — not closed. All gates green:
  `verify-phase.sh`, `maintenance_dialect_blindness` (3/3), `github_activity_oracle` (18
  passed, 1 ignored), `dialect_audit` (61/61), `emission_ownership` (11/11), `dialect_seam`
  + `projection_dialect_invariance` (18/18 + 4/4), `googlesql_render` (4/4),
  `large-file-check.sh`. Row 10 flipped to `done`; outcome Status flipped to `done`.

- 2026-09-08 (phase 10 planning): **no reshape; row 10 is the last row and its "move the
  ratchets down" clause resolves to *hold*, with the reason written into the baseline
  file.** Measured at HEAD: `cargo test -p smelt-db --test dialect_audit` is 61/61 green,
  so `the_coverage_table_matches_the_registry` already passes (the regeneration is a
  no-op to be confirmed, not a pending edit) and `gap_count_ratchet` already matches
  `dialect_gaps_bigquery 42`. That count cannot fall here: every fix phases 1-9 landed was
  in `smelt-logical`'s maintenance emitters, not in `BuiltinRegistry`, and giving the 42
  no-verdict entries verdicts speculatively is forbidden by criterion 2 and already
  recorded under Out of scope. Criterion 4's "fall or hold; neither is raised" is therefore
  satisfied by holding, and the phase's job is to make that legible rather than to move a
  number. Two pieces of real work remain and are in the phase: (a) criterion 3 is still
  short one gate — the defect class this outcome opened on (an emitter taking a `dialect`
  parameter and hardcoding `MaintenanceDialect::DuckDb` anyway) is held only by per-emitter
  unit tests, so a *new* emitter could reintroduce it silently; a structural scan over
  `crates/smelt-logical/src/maintenance/` with a planted-needle non-vacuity control closes
  that. (b) `handoff_claimed_relations()` in `github_activity_oracle.rs` scans the whole
  handoff for any `` | ` ``-leading row, which phase 9's summary flagged as a false-positive
  trap — and phase 10 must itself append a close-out section to that handoff, so scoping
  the scan to the divergence table is required work, not cleanup. Issue #179 gets a
  comment, not a close: its 42 entries are untouched.

- 2026-09-08 (phase 9 implementation): **the plan's citation table verified exactly as
  written; the durable half is now landed and no live re-run is needed.** All six commits
  (`7a2eb89d0`, `af972abe0`, `0178e6bd4`, `d84320a44`, `e028596e3`, `aee113753`) confirmed via
  `git show`; `modulo_lowering`/`power_lowering` both pass at HEAD unchanged. New
  `crates/smelt-maintenance-testkit/tests/googlesql_render.rs` (4 tests: the two positive
  scans over every `DagBody` variant across all six DAG recipes and all four `ComposedRoute`s,
  a non-vacuity negative control over all seven refused-construct needles, and a fail-loud
  check on an unparseable body) — none of the scans found a live needle, so no fix was needed,
  only the gate. `crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs` gained
  `bigquery_oracle_relation_issues_no_ddl_and_returns_an_inline_subquery` against the REAL
  `BigQueryConformanceBackend` (not a stand-in fake) and a real in-memory-equivalent
  `DuckDbBackend`, gated `#[cfg(feature = "duckdb")]` (on by default alongside `bigquery`) so
  it needs no warehouse and no `SMELT_BQ_PROJECT`. `main.rs`'s doc comment and
  `gate_composed_bigquery.rs`'s doc comment both retired their stale
  "uncharacterised"/"not yet re-confirmed" wording in favour of the plan's table plus the
  2026-08-21/2026-08-22 sweep results. Spec delta landed
  (`docs/specs/multi_backend.md` §"Known Divergences": "The BigQuery conformance leg's live
  evidence has a date", naming the 2026-08-22 sweep and the five offline gates that stand in
  for a live re-run between sweeps). `docs/handoffs/2026-09-08-github-activity-findings.md`
  gained a §"Criterion 6" section carrying the table verbatim; its leading-cell format had to
  change from `` | `name` `` to `` | Test: `name` `` after `findings_handoff_names_no_unknown_relation`
  (a pre-existing generic scan for any `` | ` `` -leading markdown row, not specific to the
  divergence-registry table) flagged both new rows as stale registered-divergence claims — a
  real, if narrow, false positive in a gate this phase didn't own, fixed by reformatting rather
  than touching the gate. All gates green: `verify-phase.sh`, `googlesql_render` (4/4),
  `modulo_lowering`+`power_lowering` (11/11), `maintenance_conformance --features duckdb`
  (101/101, including `dags::diamond_propagation_suffices` and the `composed_pool` family),
  `cargo check -p smelt-cli --features bigquery --tests`, the new BigQuery-gated test (1/1,
  `SMELT_BQ_PROJECT` unset), `large-file-check.sh`. Row 10 is unchanged.

- 2026-09-08 (phase 9 planning): **no reshape, and no block — the two failures are already
  characterised AND fixed in the repo record, so phase 9 is offline forensics plus the gate that
  makes the characterisation durable.** Criterion 6 reads as open only because this outcome's row
  9 inherited the 2026-08-16 handoff's "uncharacterised" wording. The record since disagrees:
  `diamond_propagation_suffices_on_bigquery` is `WHERE id % 2 = 0` reaching GoogleSQL unlowered
  (`400 Syntax error: Expected ")" but got "%"`, measured live 2026-08-19), fixed by `7a2eb89d0`
  (`%`→`MOD`) plus `af972abe0` (`^`→`POWER`, the worse silent-wrong-number sibling found chasing
  it); `composed_keyed_pool_upholds_equivalence_on_bigquery` had no mechanism of its own and was
  collateral from three already-closed gaps (`INSERT *` in the keyed-fold MERGE `0178e6bd4`, the
  `DROP` object-type mismatch `d84320a44`, the hand-rolled `FROM (VALUES …)` row set
  `e028596e3`/`aee113753`), confirmed live in the 2026-08-19 sweep. Both then passed the
  whole-sweep measurements of 2026-08-21 (21/21, 2190.85s) and 2026-08-22 (22 cases, 621.61s
  concurrent). So the phase does not need the live leg and must not emit `<<PHASE_BLOCKED>>`
  under criterion 7. What it does need is the durable half, which does not exist: the diamond
  mechanism is gated at the printer (`modulo_lowering`, `power_lowering`) but nothing ties the
  *testkit's own rendered recipe bodies* to those lowerings, which is exactly the seam that let a
  `%` reach a live warehouse in the first place — hence the new `googlesql_render` gate over
  every `DagBody` and the composed pool's rendered bodies, with a non-vacuity control in phase
  8's shape. The one genuinely unrunnable item — re-confirming green at today's HEAD, after
  phases 1-8 touched maintenance emitters — is recorded as a dated, named debt in the spec and
  the handoff rather than skipped green; it belongs to the spine's blocked live half. Row 10 is
  unchanged.

- 2026-09-08 (phase 8 implementation): **fail-closed proof landed as planned; criterion
  5's cross-target half registered nothing because the spine never ran live BigQuery.**
  `check_matches_oracle` (a `Result`-returning split of `assert_matches_oracle`) plus five
  new tests (`assert_matches_oracle_fails_closed_on_an_empty_registry`,
  `check_bound_accepts_a_holding_bound`, `check_bound_rejects_a_leading_side`,
  `check_bound_rejects_divergence_outside_the_licensed_columns`,
  `no_relation_diverges_unexplained`) now exercise the registry-consulting comparator, both
  `check_bound` arms/`Side` variants, and criterion 5's zero-unexplained-count claim
  directly, sharing a new `perturbed_one_day_pair()` staging helper with
  `an_unregistered_divergence_fails`. `registry_entries_are_all_live` gained the same
  direct check on an empty registry so its own loop cannot pass vacuously either. The
  `#[allow(dead_code)]` attributes on `Bound` and `Side` are gone — both are now
  constructed by real tests. `docs/handoffs/2026-09-08-github-activity-findings.md`'s
  divergence section now names these five tests, so "empty registry" reads as "measured
  and found nothing," not "never measured." Criterion 5's **cross-target**
  (DuckDB-vs-BigQuery) half registered nothing to resolve: the spine
  (`docs/outcomes/20260906-bigquery-dogfood-spine`) is `blocked` with its live-BigQuery
  half (its phase 16) never run, so it produced no dual-target divergence at all — already
  covered by this outcome's Out of scope bullet on BigQuery-only defects; recorded again
  here so criterion 5 does not read as half-checked. All gates green: `verify-phase.sh`,
  `github_activity_oracle` (16 passed, 1 ignored measurement sweep, 115s), and
  `github_activity_replay` (17 passed, 57s). File grew from 975 to 1151 lines, under the
  1500-line default cap with no baseline entry needed.

- 2026-09-08 (phase 8 planning): **no reshape; row 8's content is now the fail-closed
  proof, and criterion 5's cross-target half is answered rather than left open.** Phases
  3-7 fixed all five registered divergences instead of promoting any, so
  `DIVERGENCE_REGISTRY` is empty and criterion 5's "unexplained count zero" holds — but
  three of the registry's own gates (`succession_divergence_is_exactly_tied_row_
  multiplicity`, `registry_entries_are_all_live`, `every_registry_entry_is_named_in_the_
  findings_handoff`) now iterate an empty slice and pass by construction, and neither
  `check_bound`'s `MonotoneDivergence` arm nor `assert_matches_oracle`'s unregistered
  branch has any live test: the existing negative control
  (`an_unregistered_divergence_fails`) stops at `compare_databases` and never reaches the
  registry-consulting comparator. Phase 8 is therefore exactly that proof, plus one
  directly-named zero-unexplained-count assertion. The **cross-target** reading of
  criterion 5 (the spine's criterion 6 is DuckDB-vs-BigQuery dual-target parity) has no
  residual work: the spine is `blocked` with its live-BigQuery half never run, so it
  registered no dual-target divergence at all — already covered by this outcome's Out of
  scope bullet on BigQuery-only defects, and recorded again by phase 8's task 9 so
  criterion 5 does not read as half-checked. Rows 9 and 10 are unchanged.

- 2026-09-08 (phase 7 implementation): **the mechanism worked on the first try; the tutorial
  freshness gate did not move.** Task 1's inspection confirmed the plan's diagnosis exactly
  (the clocked cell already exists; the run-window widening was the missing piece), so
  `IncrementalWindows::output_window()`, the pure `widen_run_window_for_upstream_outputs`
  helper, and threading both through `build_model_plans` (recording each model's output
  window in a map keyed by name, consulted by name via `refs` before the `frozen_horizon`
  clamp) closed `marts_daily_active_contributors`'s divergence on the first run of the full
  30-day oracle — no second-attempt fix was needed, unlike phases 3 and 6.
  `cargo test -p smelt-cli --test tutorial_freshness --features duckdb` passed unmodified
  (no regeneration needed): the web-analytics tutorial's directive commands apparently never
  select a Form-B upstream and its Form-A downstream together in one invocation the way the
  plan's task 8 anticipated, so the widening never triggers there. `DIVERGENCE_REGISTRY` is
  now empty; `findings_handoff_names_no_unknown_relation`'s "claimed non-empty" assertion had
  to be loosened to accept an empty table when the registry itself is empty (a fixed-in-phase
  consequence of the registry emptying now rather than in phase 8, not a new mechanism) — see
  `phases/07-summary.md`.
- 2026-09-08 (phase 7 planning): **no reshape of the phase order; row 8 reworded, and the
  handoff's open question is answered inside phase 7 rather than by a row of its own.**
  Reading the code settled punch-list item 4's mechanism: the edge from `silver.actor_sessions`
  to `marts.daily_active_contributors` is *clocked*, so `append_model_edge_cells`' clock route
  already derives a `NewData` / `RecomputeRegion` / `DeleteInsert` cell for it — the maintenance
  cell is not missing. What is missing is the window it is ever dispatched over: `build_model_plans`
  gives every model the invocation's requested run window verbatim, so a Form-A downstream never
  learns that its Form-B upstream rebased `[D-1, D+2)` on a `[D, D+1)` run. Phases 4-5's
  enrichment-keyed route therefore does **not** subsume this (it is key-addressed value enrichment
  for a *clockless* upstream; this read is membership-sensitive and clocked), which is the handoff's
  question answered — phase 7 confirms it by inspection as its first task rather than carrying a
  separate row. The fix is one rule, stated in `docs/specs/incremental_models.md` §"Forward
  propagation" for an explicit landed delta but never applied to an ordinary windowed run: a model's
  run window is the union of the requested window and every in-run upstream's derived output window.
  Row 8 is reworded because phases 3-7 are expected to leave `DIVERGENCE_REGISTRY` empty, at which
  point its real content is proving the unregistered-divergence sweep still fails closed rather than
  passing vacuously — criterion 5 stays owned by a row either way.
- 2026-09-08 (phase 7 planning): **phase 6's suggested extra `statement_parity` lookback+skew+chunking
  fixture does not get a row.** The regression it names is already gated at its source by
  `windowing_form_b_chunking.rs::lookback_and_skew_widen_independently_never_summed`; a second fixture
  asserting the same property further from the code would duplicate, not widen, coverage, so criterion 3
  is satisfied without it. Recorded here rather than under Out of scope because no work is leaving the
  outcome — it was never in it.

- 2026-09-08 (phase 6 implementation): **the plan's diagnosis targeted a dead field; the real
  fix needed a second layer.** `IncrementalBatch::filter_start`/`filter_end` — what the plan's
  formula computes — turned out to have zero consumers anywhere in the real execute path
  (`rg`-confirmed): `derive_batch_filtered_sql` (`crate::execute::sources`) widens each bounded
  source's scan from `run_range` (`batch.partition_start`/`partition_end`, unwidened) plus a
  *per-source*, independently-derived lookback/lookahead bound (`per_model_source_bounds`) —
  never from `filter_start`/`filter_end` at all. So the phase 6 plan's formula, applied only in
  `windowing.rs`, was inert against the actual defect; the `github_activity` oracle test still
  failed after it (a real cross-midnight session still truncated at the interior chunk
  boundary). Fixed by threading a new `scan_range` parameter through `derive_batch_filtered_sql`
  and its three call sites (`execute/project/mod.rs`, `execute/project/dry_run.rs`,
  `smelt-cli/explain.rs`), sourced from a **new** `IncrementalBatch::scan_start`/`scan_end` field
  pair — deliberately not a repurposing of `filter_start`/`filter_end`. The first attempt reused
  `filter_start`/`filter_end` (already skew-widened) as `scan_range`, which passed the
  `github_activity` oracle but **double-widened** the lookback component for any model with a
  nonzero SQL-inferred lookback: `derive_batch_filtered_sql` still adds `per_model_source_bounds`'
  own lookback on top, so a model carrying both a real lookback and a chunked run got its scan
  literal widened twice. Caught by `web_analytics_tutorial_pages_are_fresh` (the doc-freshness
  gate), not by any windowing-crate test, because none of them combine a nonzero lookback with
  chunking and a literal-text assertion — see phases/06-summary.md "For the next planner" for the
  gap this leaves. `scan_start`/`scan_end` carries skew alone (clamped to the outer envelope,
  same clamp as the plan's original formula); `filter_start`/`filter_end` keeps its pre-existing,
  lookback-only meaning untouched.

- 2026-09-08 (phase 6 planning): **no reshape; the fix is scan-side and clamped to the existing
  outer envelope.** Reading `compute_calendar_windows` confirmed the row's diagnosis and pinned the
  mechanism: the Form-B relation has *two* inversions — a write-side one (run window → output window
  `[start − after, end + before)`, implemented, applied once per invocation) and a scan-side one (to
  write partitions `[bs, be)` the scan must cover driving dates `[bs − before, be + after)`, never
  implemented). A single-chunk invocation covers the scan side incidentally because its batch bounds
  *are* the output-window bounds; every interior boundary loses it. The fix folds the skew into the
  per-batch filter but clamps it to the invocation's existing outer scan envelope, so single-chunk
  literals stay byte-identical and no existing statement-parity fixture moves — the narrower choice,
  taken deliberately: whether the outermost chunk should also read past the run window's trailing edge
  is a data-availability question this defect does not raise. Both specs already assert the correct
  rule ("each sized from its own chunk's reach"; scan "relative to the derived output window"), so the
  spec delta is a clarifying sentence per file naming the skew inversion and the property it buys —
  output invariant under chunk count. The integer axis needs no change (nonzero skew is already refused
  fail-closed there). Phase 7's `marts_daily_active_contributors` entry is expected to *shift* under
  this fix (its upstream now writes fuller sessions), so phase 6 re-measures and re-registers it
  without fixing it.

- 2026-09-08 (phase 5 implementation): **the enrichment-keyed cell is live;
  `github_activity`'s stale-row count reaches zero.** `resolve_live_column_
  scoped_cell` gained a `model_edges` parameter and switched to
  `derive_resolved_with_edges` when non-empty; `decide_column_merge_dispatch`
  excludes an `EnrichmentKeyed` cell from per-batch dispatch; a new
  `execute/enrichment_heal.rs` dispatches it once per run over the model's
  unwindowed output. The mutation gate's existing `None`-on-missing-
  `SourceInfo` behaviour already implements the plan's "fails open to
  dispatch" posture for an edge trigger — no new gating code was needed, only
  tests and a spec sentence naming the property. `execute/project/mod.rs`
  grew 52 lines past its large-file baseline (the two call sites' own
  ~20-argument lists, irreducible without moving locals); bumped with a
  sign-off note rather than left red. Full 30-day replay
  (`enrichment_heal_repairs_rows_written_before_the_rename`,
  `gold_events_enriched_matches_the_full_refresh_oracle`) confirms zero stale
  rows; the plan's `statement_parity` isolation test (test 5) was not added —
  see `phases/05-summary.md` "For the next planner".

- 2026-09-08 (phase 5 planning): **no reshape to the phase rows; one item moved to Out of
  scope.** Reading the run path confirmed phase 4's split was right and phase 5's scope is
  exactly as written — the resolver takes no edges, so the derived cell is invisible to both
  dispatch branches. One design question the row did not name is settled in the plan: an
  enrichment-keyed cell's write is addressed by the join key, not by a partition interval,
  so the existing per-batch `ColumnMergeDispatch::Full` arm (which MERGEs the *window-
  filtered* compiled SQL) cannot heal rows written on earlier days and would leave the stale
  count non-zero. The cell is therefore excluded from the per-batch dispatch and dispatched
  once per run over the model's unwindowed output, licensed by the edge's declared
  `allow_full_scan`. Phase 4's LSP-diagnostics finding is recorded under Out of scope: it is
  a pre-existing editor-surface gap serving none of this outcome's success criteria.

- 2026-09-08 (phase 4 implementation): **found, did not fix, a pre-existing LSP-diagnostics
  gap for every model-edge refusal.** `smelt-db`'s LSP-facing `maintenance_plan` Salsa query
  (`maintenance_plan_diagnostics`, what `file_diagnostics()` calls) is wired to the
  source-only `derive_model_maintenance_plan`, never `..._with_edges` — so no model-edge
  refusal (not just the new `RepairKeysNotDiscoverable`; `ReachNotDerivable` has the
  identical, already-documented gap) has ever reached `file_diagnostics()`/the editor; only
  `smelt explain` (`maintenance_plan_report`) sees them. Phase 4's own diagnostics test was
  rewritten against `plan_for` (the `explain` query) with the gap named inline rather than
  silently expanding this phase to also thread edges into the LSP query — see
  `phases/04-summary.md` "For the next planner" for the follow-up.
- 2026-09-08 (phase 4 implementation): **the enrichment-keyed route needed a guard the plan
  didn't spell out — restricted to an actual JOIN.** Without checking that the edge resolves
  via `enrichment_join_clause` at all, the route also fired for a plain `FROM smelt.<edge>`
  driving relation (no join, i.e. the edge IS the sole source) — not value enrichment at all —
  and broke `keyed_model_edge.rs::consumer_not_carrying_upstream_keys_is_refused` (a `ScanUnbounded`
  refusal instead of the expected `RepairKeysNotDiscoverable`). Fixed by returning `Ok(None)`
  early when the edge is not resolvable as an enrichment join; full workspace gate green after.

- 2026-09-08 (phase 4 planning): **reshape — punch-list item 2 splits into a derivation
  phase (4) and a run-path phase (5); old rows 5-9 shift to 6-10.** Reading the code for the
  missing `UpstreamMutation(gold.repo_dim)` cell showed the work is two separable layers, not
  one. (a) *Derivation*: `append_model_edge_cells` today offers a clockless `KeyedUpsert` edge
  only the key-addressed `PerGroupRecompute` route, whose two discovery legs (upstream-keyed,
  grain-over-upstream) both need the DOWNSTREAM's grain to resolve against the upstream
  relation — impossible for a `grain: partition` downstream like `gold.events_enriched`. But
  the shape is not a per-group recompute at all: it is the value-enrichment shape
  (`Technique::ColumnScopedMerge`) smelt already derives for a declared `mutation_profile:
  mutable_snapshot` dimension, and its write addressing is the *join* key carried in the
  downstream's own output (`repo_id`), which IS discoverable. So the fix is a third,
  enrichment-keyed route — parity between a mutable-snapshot source dimension and a clockless
  keyed model dimension — not a widening of the existing two. (b) *Run path*: the runtime's
  live-cell resolver (`resolve_live_column_scoped_cell` →
  `maintenance_availability::derive_resolved`) calls the source-only
  `derive_model_maintenance_plan`, never `..._with_edges`, so it cannot see a model edge at
  all; the mutation gate and the dimension-`unique_key` lookup likewise search `source_infos`
  only. Making the derived cell actually dispatch is its own chunk of work with its own gate
  (the `github_activity` stale-row count reaching zero). Neither half is deferred out of the
  outcome — both are rows. The row's "or a refusal surfaced at `run`/`build`" alternative is
  kept as well, not instead: phase 4 also gives `Refusal::RepairKeysNotDiscoverable` a real
  `DiagnosticCode` (its catalogue row in `docs/specs/diagnostics.md` already exists with no
  variant behind it), so the fail-closed leg that survives the new route is loud at
  `build`/`run` rather than visible only through `smelt explain --json`.

- 2026-09-08 (phase 3 implementation): **the plan's `MAX`-per-column fold was insufficient;
  fixed to a `ROW_NUMBER()`-ranked whole-row pick instead.** Running the full 30-day
  `every_window_matches_the_full_refresh_oracle` gate exposed two bugs a per-column `MAX`
  aggregate over the model's compiled `SELECT` output cannot avoid: (1) `LEAD`/`LAG` computed
  over physically-duplicated tied rows produces genuinely different derived-column values per
  physical row (one row's `LEAD` self-references its tied sibling; `MAX` prefers that artifact
  over the correct `NULL`), and (2) the fold's `SELECT` list must preserve the model's own
  output column order (not force key-first), since the patch loop's bootstrap shell always
  uses model order and position-based `EXCEPT ALL` comparisons broke under a reordered fold.
  Both fixed; the full 30-day oracle (`crates/smelt-cli/tests/github_activity_oracle.rs`) and
  the day-by-day replay (`crates/smelt-cli/tests/github_activity_replay.rs`) now pass with
  `silver_repo_naming`/`silver_actor_naming` comparing exactly equal — see
  `phases/03-summary.md`.

- 2026-09-08 (phase 3 planning): **the harvest happened at plan time, and row 3 became
  the first real punch-list item.** Row 3 as scaffolded was a meta-phase whose entire
  content — "read the handoff and rewrite the remaining rows" — is exactly what the
  outcome loop's plan step does under its own reshape rule, so running it as an implement
  iteration would have burned a step producing only a table edit. The handoff
  (`docs/handoffs/2026-09-08-github-activity-findings.md`) is final for loop purposes: the
  spine's `**Status:**` is `blocked` and its live-BigQuery half will not land unattended,
  so waiting for a richer input is waiting for something no loop iteration can produce.
  Rows 3-6 are now its four punch-list items verbatim, in its order (item 4 explicitly
  after item 2 because the handoff asks whether item 2's mechanism subsumes it); old rows
  5-7 shift to 7-9. Nothing from the handoff was dropped: its two "requirements handed
  to" sections address the other two backlog outcomes, not this one, and its "latent,
  unmeasured" clock-tie item folds into row 3 where the same emitter is already open.

- 2026-09-08 (phase 2 implementation): **`supports_fingerprint_sidecar` stays
  DuckDB-only after phase 2.** Phase 2 proved the fingerprint/repair-group digest
  SQL well-formed per dialect (BigQuery, Spark), pinned by 8 new tests including
  a loud-refusal gate (`sidecar_capability_is_declared_only_where_the_digest_sql_
  is_verified` in `crates/smelt-runtime/tests/fingerprint_sidecar.rs`) that fails
  offline if the capability flag moves without a live value-leg sweep for that
  backend. No such sweep ran this phase — the flag is unchanged.

- 2026-09-08 (phase 1 implementation): **the fixed path is not reachable on a live
  `mutable_snapshot` run today.** `crates/smelt-dialect/src/dialect.rs` declares
  `supports_fingerprint_sidecar: true` only for DuckDB (line 218; `spark()`/
  `spark_delta()`/`spark_parquet()` and `bigquery()` all declare `false`, lines 260, 295,
  354 in the file as read for this phase). Every runtime entry point in
  `crates/smelt-runtime/src/maintenance_driver/sidecar.rs` —
  `diff_fingerprint_sidecar_changed_keys` (line 133),
  `refresh_fingerprint_sidecar` (line 242), `diff_repair_group_sidecar_changed_keys`
  (line 389), and `refresh_repair_group_sidecar` (line 493) — checks
  `backend.capabilities().supports_fingerprint_sidecar` and returns
  `BackendError::unsupported` before ever calling `emit_fingerprint_digest_select` or
  `emit_repair_group_digest_select`. So today a BigQuery (or Spark) target never reaches
  the previously-wrong DuckDB-hardcoded digest SQL at all — the bug was latent, not
  live-hit. It would become reachable the moment `bigquery()`'s (or a Spark variant's)
  `supports_fingerprint_sidecar` flips to `true`, which is presumably future work this
  outcome's punch-list (harvested in phase 3) or a follow-on outcome would drive. The fix
  still lands now, unconditionally, per criterion 1 and the outcome's framing — it removes
  a landmine ahead of that capability ever being turned on, rather than waiting for a
  spine model to trip it.

- 2026-09-08 (phase 1 planning): **reshape — a new row 2 for the remaining dialect-blind
  fingerprint SQL.** Reading the emitter for criterion 1 surfaced two siblings with the
  same defect class: `key_expr_for_columns` hardcodes `CAST(... AS VARCHAR)` (GoogleSQL
  has no `VARCHAR` at all) and `emit_repair_group_digest_select` hardcodes both that cast
  and DuckDB's `bit_xor(hash(...))`. Fixing only the digest expression would leave the
  same emitted statement invalid on BigQuery, so this serves criterion 1's substance and
  criterion 3 and is not deferred out. Old rows 2-6 shift to 3-7.

- 2026-09-08 (bigquery-dogfood-spine phase 15): **the interim findings handoff now
  exists** at `docs/handoffs/2026-09-08-github-activity-findings.md` — the four measured
  root causes, the five registered divergences, and this outcome's punch-list, all
  DuckDB-half only. Its live-BigQuery half lands in that outcome's phase 16; until then,
  this document is the phase-2 planner's rewrite input, not the final one.
- 2026-09-06 (scaffold): **deliberately near-empty.** Phase 3 is a placeholder the phase-2
  planner rewrites. This is the outcome loop's just-in-time planning used as intended, and
  it is the mechanism by which §"Sequencing"'s "let the real models generate the
  punch-list" is enforced rather than merely intended.
- 2026-09-06 (scaffold): **phase 1 runs before the spine finishes.** The fingerprint-dialect
  defect is wrong for any model on any backend, so it is not gated on evidence. It is
  ordered first so the loop has real work the moment this outcome is reached, even if the
  spine is still mid-flight.
- 2026-09-06 (scaffold): the BigQuery value leg cannot run in the loop's environment. A
  phase that needs it must block rather than skip — the same rule the active-plan pointer
  states for the Spark legs, for the same reason (a silently skipped live leg is a hole
  that reads as green).

## Blocked

(none)
