# Outcome: Every defect the real pipeline hits on BigQuery is fixed, and DuckDB and BigQuery agree

**Created:** 2026-09-06
**Status:** active
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
| 5 | Punch-list 2b — make that cell live on the run path: thread model edges into `resolve_live_column_scoped_cell`/`maintenance_availability::derive_resolved` and the mutation gate, so `gold.events_enriched`'s already-written `current_repo_name` heals and the `github_activity` stale-row count reaches zero | planned |
| 6 | Punch-list 3 — `compute_calendar_windows`' interior-chunk-boundary forward-reach loss for Form-B models, which makes the full-refresh oracle itself undercount a cross-midnight session | pending |
| 7 | Punch-list 4 — the missing repair edge from a Form-B model's own self-rebase to a Form-A downstream aggregate that reads it verbatim; first check whether phases 4-5's mechanism already covers it | pending |
| 8 | Resolve every divergence the spine registered (`github_activity_oracle.rs`'s `DIVERGENCE_REGISTRY`): each entry fixed, or promoted to a reasoned permanent entry naming the engines and the construct; unexplained count zero | pending |
| 9 | Characterise or fix the two known live conformance failures (`diamond_propagation_suffices`, `composed_keyed_pool_upholds_equivalence`) | pending |
| 10 | Close: regenerate `docs/reference/dialect-coverage.md`, move the gap ratchets down, update issue #179 with what was verified, all standing gates green | pending |

## Decision log

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
