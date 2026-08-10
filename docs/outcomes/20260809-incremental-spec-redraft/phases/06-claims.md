# Phase 6 claim inventory

Source: `docs/specs/incremental_models.md` `## Known Divergences / Open Questions` (pre-redraft
lines 2050–2389, three `###` subsections) and `docs/specs/model_properties.md`
`## Known Divergences / Open Questions` (pre-redraft lines 318–350, flat). One row per distinct
claim: a bundled bullet that names several unrelated gaps is split into one row per gap; a bullet
that is pure landed-work narrative with no residual gap is one `drop` row. `verdict` is `keep`
(live gap — survives as a gap bullet), `drop` (landed-work narrative, no gap inside it), or
`merge:<id>` (duplicate of another row, kept once). `status` is filled in by the adversarial-verify
pass (`preserved` / `weakened` / `lost`, only for `keep` rows).

## `incremental_models.md` — "The contract, plan, and graph layer"

| id | claim | rg anchor | verdict | status |
|---|---|---|---|---|
| IC-01 | Landed-work recital: both `frozen_horizon`/`deferral` triples (loader, `smelt-logical` admissibility, write-range narrowing, both probes, licensing, `maintenance_conformance`, `smelt explain`) are landed | `Both triples are landed` | drop | |
| IC-02 | Per-cell `deferral` still parses/validates/prints but is not yet scheduled — needs per-cell frontier addressing (frontier record is per-region today) | `Per-cell \`deferral\`` | keep | preserved |
| IC-03 | `diff_patch` over a live `PerGroupRecompute` cell executes via its emitter, `statement_parity` leg proven | `over a live \`PerGroupRecompute\`` | drop | |
| IC-04 | `diff_patch` over the region `DeleteInsert` default has no runtime lowering; resolver fails loud by name but no caller reaches it, so the pin is unenforced not refused | `no caller today reaches it` | keep | preserved |
| IC-05 | Frontmatter-time grain checking gap: a `grain: key` model with no top-level `unique_key:` is checked against the derived key only at plan derivation, not frontmatter validation | `Frontmatter-time grain checking has one narrow gap` | keep | preserved |
| IC-06 | Write-pin equivalence factor is structural only; per-cell equivalence hook always accepts | `equivalence factor is structural only` | keep | preserved |
| IC-07 | Inadmissible write-*variant* pin (`technique: suppress`) has no pre-execution gate; resolver silently falls back instead of refusing; `smelt explain` misses the case | `has no pre-execution gate` | keep | preserved |
| IC-08 | Observed-delta consumption partial: `--since-upstream` doesn't read recorded delta live; backward resolution consumes none; keyed-fold/staged-candidate families record nothing; settle-bound × observed-delta composition has no live leg | `Observed-delta consumption is partial` | keep | preserved |
| IC-09 | No execution technique keys off a maintained-model creation cell; propagated region uses ordinary run loop not a per-cell technique | `No execution technique keys off a maintained-model creation cell` | keep | preserved |
| IC-10 | `KeyedUpsert` upstream feeding `grain: partition` downstream has no live key-addressed dispatch (wired only inside `grain: key` run branch); correctness-preserving, not soundness gap | `has no live key-addressed` | keep | preserved |
| IC-11 | Definition-change backfill non-atomic when `schema_evolution: strategy: full_refresh` skips the migration gate (standalone `UPDATE` for `PureBackfill`) | `frontmatter skips the migration gate entirely` | keep | preserved |
| IC-12 | Definition-change backfill non-atomic on non-transactional-DDL backends (`supports_transactional_ddl == false`); no repair path for either case | `non-transactional-DDL backend` | keep | preserved |
| IC-13 | Keyed dirt-set typed-edge routing landed (`propagate`/`required_inputs` route `Addressing::Keyed` through the channel instead of refusing) | `keyed dirt-set channel instead of refusing` | drop | |
| IC-14 | Keyed dirt-set records key columns + provenance, not affected key **values**; value-level discovery stays a run-time mechanism | `not affected key values` | keep | merge:IC-42(MP dup) |
| IC-15 | Horizon-clamped partition-local mutation corner unreachable from any real workspace (trigger builder only emits `UpstreamMutation` for unclocked sources) | `unreachable from any real workspace` | keep | preserved |
| IC-16 | Dispatch cannot distinguish "a mutation genuinely happened" from re-derivation | `distinguish "a mutation genuinely happened"` | keep | preserved |
| IC-17 | `prefer` soft-bias ladder and `scan_bounds.on_violation: warn` parse but are not consumed; every refusal is an Error | `parse but are not consumed` | keep | preserved |
| IC-18 | Cost model between two admissible techniques is unbuilt | `cost model between two admissible techniques is unbuilt` | keep | preserved |
| IC-19 | `AppendOnly` sources get no `UpstreamMutation` cell | `AppendOnly sources get no UpstreamMutation cell` | keep | preserved |
| IC-20 | Additive fold's MERGE-inside-ledger-transaction interior not observable at the statement-group seam; parity leg uses idempotent fixture | `not observable at the statement-group seam` | keep | preserved |
| IC-21 | `Backend::delete_partitions`/`insert_overwrite` hand-author SQL for production-unreachable `InsertOverwrite` strategy (dead code, allowlisted) | `still hand-author SQL for the` | drop (landed: phase 8 deleted `IncrementalStrategy::InsertOverwrite`, the code the bullet described) | |
| IC-22 | Landed-work recital: "All seven maintenance-plan proofs are derived" | `All seven maintenance-plan proofs are derived` | drop | |
| IC-23 | Keyed-grain output poses no partition-locality question; locality-admitted keyed model's clamps carry an assumed (underived) write-footprint mirror into propagation | `a keyed-grain output poses no` | keep | preserved |
| IC-24 | `MaintenanceSkeletonColumnAdded` reachable via unit coverage but not surfaced as an LSP/CLI diagnostic ahead of a run; a skeleton-position add doesn't block the run today | `not yet surfaced as an LSP/CLI` | keep | preserved |
| IC-25 | Column-group-scoped dirt coarsens to whole-partition (safe, over-running) | `column-group-scoped dirt coarsens` | keep | preserved |
| IC-26 | Hour granularity is declared surface but propagation is day-ordinal | `propagation is day-ordinal` | keep | preserved |
| IC-27 | Grain-alignment check validates only the declaration (widen-never-narrow); graph edges take the declaration directly | `graph edges still take the` | keep | restored (widen-never-narrow + MaintenanceGranularityMismatch re-added) |
| IC-28 | Ledger's warehouse substrate is DuckDB-only; additive-graded cell on another backend fails loud; Spark-dialect ledger builder unbuilt | `warehouse substrate is DuckDB-only` | keep | preserved |
| IC-29 | Bare `grain: key` nodes with no admitted locality refuse (`MaintenanceGraphUnsupportedNode`) | `MaintenanceGraphUnsupportedNode` | keep | preserved |
| IC-30 | Time-unrolled self-edges designed but unbuilt | `time-unrolled self-edges are designed but unbuilt` | keep | preserved |
| IC-31 | No key-level dirt representation exists — intervals are the graph's only currency | `intervals are the graph's only currency` | keep | preserved |
| IC-32 | `examples/web_analytics` not fully `--since-upstream`-compatible end to end (self-referential model + bare-keyed model with readers refuse whole-workspace graph); no `--select` scoping | `is not fully \`--since-upstream\`-compatible` | keep | restored (self-referential + bare-keyed-with-readers blockers re-added) |
| IC-33 | Delta detection for `--since-upstream` explicit-only in v1; no persisted watermark or automatic diffing | `explicit-only in v1` | keep | preserved |
| IC-34 | Straddle attribution without locality scoped out of ledger's v1 | `Straddle attribution without locality` | keep | preserved |
| IC-35 | No out-of-band-edit tripwire (Open Question) — external mutation between runs undetected; whether a digest tripwire is worth it is open | `No out-of-band-edit tripwire` | keep | preserved |
| IC-36 | `on_column_add: backfill\|leave_null\|recompute` policy knob proposed but not surfaced | `A proposed \`on_column_add` | keep | preserved |
| IC-37 | Derived model-wide horizon under construction | `derived model-wide horizon is under construction` | keep | preserved |
| IC-38 | Data-quality check for model-author lateness-flag pattern under construction | `data-quality check for the` | keep | preserved |
| IC-39 | Per-input scope-map explain surface specified but unbuilt | `per-input scope-map explain surface is specified but unbuilt` | keep | preserved |
| IC-40 | Locality route 2's declared-FD sub-route unreachable for arbitrary non-clock-derived dimension column; runnable route-2 fixture missing | `route to that state is the` / `unreachable for an arbitrary non-clock-derived` | keep | preserved |
| IC-41 | Locality route 2's `IN (SELECT DISTINCT …)` slice predicate unexercised against real backend (DuckDB MERGE binder limitation, v1.4.4/v1.5.4) | `DuckDB MERGE binder limitation` | keep | preserved |
| IC-42 | Plan derivation admits routes only where it can determine driving source's granularity | `admits routes only where it can determine` | keep | preserved |
| IC-43 | Declared-vs-derived recurrence precedence and order-independent key-set comparison are implementation choices the spec underdetermines | `spec text underdetermines` | keep | preserved |
| IC-44 | `grain: key_per_partition` derives no plan — parses/validates but refuses at plan derivation (`MaintenanceUnsupportedGrain`) | `derives no plan` | keep | preserved |
| IC-45 | `smelt explain --show-sql` renders unconditional matched arm, never the suppressed form a live run executes | `never the suppressed form a live run executes` | keep | preserved |
| IC-46 | Region DELETE+INSERT family has no conditional variant | `has no conditional variant` | keep | preserved |
| IC-47 | Whole-row (keyless) staged-candidate realisation does not exist | `staged-candidate realisation does not exist` | keep | preserved |
| IC-48 | No `write:` pin selects between keyed MERGE and staged-candidate | `no \`write:\` pin selects between` | keep | preserved |
| IC-49 | Delta-restriction admission doesn't yet consume an external `mutable_snapshot` source's fingerprint-sidecar delta as a driving-source delta | `fingerprint-sidecar` | keep | preserved |
| IC-50 | Non-DuckDB targets keep the widened-scan recompute (conditional maintenance) | `non-DuckDB targets keep the widened-scan recompute` | keep | preserved |
| IC-51 | Keyed-fold suppression consumer honours `Suppressed` unconditionally; first-build-vs-steady-state rule doesn't reach it | `doesn't reach it` | keep | preserved |
| IC-52 | No real fixture derives a column-scoped/keyed-fold cell under a first-build/backfill trigger; branch proven only at resolver level | `proven only at resolver level` | keep | preserved |
| IC-53 | `smelt bakeoff` measures technique-family cost only, not write-suppression dimension; open whether a cost model needs region-level change-ratio stats | `measures technique-family cost only` | keep | preserved |
| IC-54 | docs-site coverage of the plan's CLI surface is partial, residue not enumerated | `docs-site coverage of the plan's CLI surface is partial` | keep | preserved |
| IC-55 | Group merged across two mutable inputs has no group-merge-provenance policy (Open Question) | `no group-merge-provenance policy` | keep | preserved |
| IC-56 | `change_feed` sources never get an `UpstreamMutation` cell; even when threaded through, only full-input re-derivation is admitted | `\`change_feed\` sources never get` | keep | preserved |
| IC-57 | `INTERSECT`/`EXCEPT` unclassified set operations collapse to whole-model mutation-sensitivity; future distribution proof needs per-arm-cardinality reasoning | `unclassified set operations` | keep | merge with MP dup |

## `incremental_models.md` — "The partition grain"

| id | claim | rg anchor | verdict | status |
|---|---|---|---|---|
| IP-01 | Row-shaped MERGE-dedup key has no `.sql` frontmatter home; only `smelt.yml`'s `batched.unique_key` override | `no \`.sql\` frontmatter home` | keep | preserved |
| IP-02 | Two spellings of plausible-contract mechanism coexist (`columns.<c>.contract` vs `batched.nondeterministic_columns`) | `Two spellings of the plausible-contract mechanism coexist` | keep | preserved |
| IP-03 | One classification call site reads the outer SQL body only (bound-`NotDerivable` refusal gate) — lookback inside a function body with no outer filter would diverge | `reads the outer SQL body` | keep | preserved |
| IP-04 | Window-function batch-safety check runs on unexpanded outer SQL; `OVER` inside `smelt.define` body invisible to it | `runs on unexpanded outer SQL` | keep | preserved |
| IP-05 | Per-source clamp observability partly emitted: `--json` doesn't resolve run-relative scan window; editor-hover readout unimplemented | `Per-source clamp observability is partly emitted` | keep | preserved |
| IP-06 | Per-column `data_latency` unimplemented; only the two interim mitigations exist | `Per-column \`data_latency\` is unimplemented` | keep | preserved |
| IP-07 | Non-deterministic row-set-membership/grouping out of scope — always rejected; needs frozen-per-window-membership design | `Non-deterministic row-set-membership or grouping is out of scope` | keep | preserved |
| IP-08 | CTE-only `event_time_column` references not yet detected; escapes outer-visibility check, fails at execution | `CTE-only \`event_time_column\` references are not yet detected` | keep | preserved |
| IP-09 | Schema evolution unspecified for `partition_column` rename or output schema change | `Schema evolution is unspecified` | keep | preserved |
| IP-10 | `smelt.metric()` interaction unspecified for partition-grain models | `The \`smelt.metric()\` interaction is unspecified` | keep | preserved |
| IP-11 | Per-`ModelDef` overrides for generator-emitted models not part of closed field set in v1 | `not part of the closed field set` | keep | preserved |
| IP-12 | `g_run >= g_part` auto-coarsening not implemented; sub-`g_part` run windows hard-reject | `auto-coarsening is not implemented` | keep | preserved |
| IP-13 | Monotone-integer `partition_column` has no end-to-end run (run windows/backfill/scan-filter/explain all date-typed) | `has no end-to-end run` | keep | preserved |

## `incremental_models.md` — "The key grain"

| id | claim | rg anchor | verdict | status |
|---|---|---|---|---|
| IK-01 | Window-forward keyed run with no event-time window silently full-refreshes instead of refusing; no test asserts refusal; user docs describe the fallback | `silently full-refreshes instead of refusing` | keep | preserved |
| IK-02 | Once-write classifier has no nullability route around the fallback case — only FD-backed proof reaches decomposed state; NOT-NULL derivation proves only partition/driving-clock-derived columns | `has no nullability route around the fallback case` | keep | preserved |
| IK-03 | Key-derived route requires a bare `unique_key` column reference, not an arbitrary key-derived expression | `not an arbitrary key-derived \*expression\*` | keep | preserved |
| IK-04 | Admission reads whole-scope fan-out/set-operation facts rather than per-column join trace; any fan-out/undiscriminated set op anywhere refuses every candidate | `reads whole-scope fan-out/set-operation facts` | keep | preserved |
| IK-05 | Re-run-tolerant keyed model keeps no ledger at all unless additive-graded; idempotent families have no reprocessing-detection/`--auto` bookkeeping substrate | `keeps no ledger at all` | keep | preserved |
| IK-06 | Snapshot-reconcile admits at most one unclocked source in FROM; a join of ≥2 unclocked candidates refuses rather than picking one | `admits at most one unclocked source in the FROM clause` | keep | preserved |
| IK-07 | `KeyedRetractableContribution` has no implementation — code specified but no classifier/diagnostic/test produces it | `has no implementation` | keep | preserved |
| IK-08 | `safety_overrides:` on a key-addressed model is not a hard error — parses and is ignored (§Surface says it should be one) | `is not a hard error` | keep | preserved |
| IK-09 | Reconciliation ledger's fold is transactional on DuckDB only; default `fold_ledger_delta` is best-effort check-then-act | `is transactional on DuckDB only` | keep | preserved |
| IK-10 | `smelt explain` prints neither the per-column guarantee ledger nor the derivable forward reach | `prints neither the per-column guarantee ledger` | keep | preserved |
| IK-11 | Key temporal locality route 2 admits only a declared FD; key-derived-expression sub-route never consulted | `admits only a declared functional dependency` | keep | preserved |
| IK-12 | Derived execution postures internal; order-independence not derived as a named verdict anywhere; windows always applied sequentially; neither run shape nor postures printed by `smelt explain` | `is not derived as a named verdict anywhere` | keep | preserved |
| IK-13 | Generative conformance pool cannot stage NULL payloads (`GenRow::val` non-nullable `i64`); once-write NULL direction covered only by a targeted test case | `cannot stage NULL payloads` | keep | preserved |
| IK-14 | Locality open questions: recurrence bound licensing slice pruning under snapshot-reconcile; relaxing granularity-equality precondition; slice-scoped deletion interacting with key-deletion | `Locality open questions` | keep | preserved |
| IK-15 | Pattern functions (`smelt.latest`/`smelt.once`/`smelt.current`) unshipped; reachable only via hand-written SQL spelling; ship-as-builtin-vs-template open | `are unshipped` | keep | preserved |
| IK-16 | Driver granularity is `day`/`week` only | `Driver granularity is \`day\`/\`week\` only` | keep | preserved |
| IK-17 | `--auto` staleness fidelity for all-invertible models conservative in v1; needs group rung's delta-history mechanism | `is conservative in v1` | keep | preserved |
| IK-18 | Self-referential keyed models rejected; admitting needs explicit input/state distinction design | `Self-referential keyed models are rejected` | keep | preserved |
| IK-19 | Run-pinning alignment deferred — `NOW()`/`CURRENT_*` rejected outright in keyed models | `Run-pinning alignment is deferred` | keep | preserved |
| IK-20 | Key deletion unresolved beyond retention; no explicit delete mechanism; tombstones/hard-delete/observer contract deferred | `Key deletion is unresolved beyond retention` | keep | preserved |
| IK-21 | Ladder rungs 3–4 (group retraction, bounded-domain multiset) out of scope for rung-2 work; rung 3 additionally depends on unbuilt change-feed consumption | `Ladder rungs 3–4 remain specified` | keep | preserved |

## `model_properties.md` — flat section (bullets 320–350)

| id | claim | rg anchor | verdict | status |
|---|---|---|---|---|
| MP-01 | Landed-work recital: property layer framing + `assert_monotonic`/FD-declaration/`bounded_domain`/`horizon_ceiling`/discriminants "is now built" narratives | `is now built` (bullet 320, multiple occurrences) | drop | |
| MP-02 | `functional_dependency_verdict_over_vector` and the once-write *enrichment transform* remain unconsumed | `remain unconsumed` | keep | preserved |
| MP-03 | `bounded_domain:` has no consumer (multiset maintenance transform unwired) | `bounded_domain:\` has no consumer` | keep | preserved |
| MP-04 | `horizon_ceiling:` never narrows the bound used for the clamp | `never narrows the bound` | keep | preserved |
| MP-05 | Fan-out/join-contribution-monotonicity proofs have no consumer wired (F15's dimension-driven horizon MERGE) | `has no consumer (F15` | keep | preserved |
| MP-06 | Window-independence proof has no consumer; ordered-backfill chunker unwired | `the ordered-backfill` | keep | preserved |
| MP-07 | Change comparability has no consumer yet (future write-suppression compare is the intended one) | `change comparability has no consumer` | keep | preserved |
| MP-08 | Region row identity has no write emitter or admission rule consuming it yet | `region row` | keep | preserved |
| MP-09 | `EffectiveWindow` (day-granular, batch-sizing) and `BoundResult` (second-granular, pushdown) remain two separate walks answering different questions with deliberately different fail-closure; collapsing would lose one property; tracked as future work | `remain two separate walks` | keep | preserved |
| MP-10 | Landed-work recital: code-duplication unification (nondeterminism predicate, interval parsing, aggregate-name extraction) | `Remaining live code duplications. None` | drop | |
| MP-11 | Expression-position (scalar/`EXISTS`) subquery scopes are not enumerated as walk nodes; their window/`LIMIT`/reach/`DISTINCT`/`HAVING` content is judged only in the owning scope's region | `are not enumerated as walk nodes` | keep | preserved |
| MP-12 | `temporal` proof and driving-fact/anchor join resolution still run their own traversal rather than the shared walk | `still run their own traversal` | keep | preserved |
| MP-13 | Declared source lateness reaches no live scan today; feeds batch fields no execute path consumes; becomes a scan obligation only with the (unbuilt) tail-rewrite transform | `reaches no live scan today` | keep | preserved |
| MP-14 | Reach-migration residues: a redundantly-parenthesized derived table (`FROM ((SELECT …)) AS t`) falls back to legacy whole-text derivation; same-scope chained bands still max-merge; an absorbing verdict rejects every context source | `redundantly-parenthesized derived table` | keep | preserved |
| MP-15 | Whole-model property vector (`model_property_vector`) has no consumer wired yet | `property vector` | keep | preserved |
| MP-16 | Landed-work recital: AST-pure core, walk-wired admission gates, per-node classification of surviving scans | `is now uppercase-substring based` / narrative around walk rewire | drop | |
| MP-17 | `cumulative.rs`'s whole-SQL window-function admission scan (`OVER(`/`OVER (` check) is not yet classified onto the walk; remaining debt for a future property-discovery pass | `is \*\*not yet classified\*\*` | keep | preserved |
| MP-18 | `INTERSECT`/`EXCEPT` unclassified for filter distribution (their arm scopes judged by the admission walk) | `are unclassified for filter distribution` | keep | merge:IC-57 |
| MP-19 | Additive-only model-diff can't tell "existing column's semantics changed under unchanged expression"; falls to a declared migration intent whose exact surface is open | `not derivable from the` | keep | preserved |
| MP-20 | Landed-work recital: "All seven maintenance-plan proofs are built" framing + per-proof build narrative (skeleton-role, grouping, faithful-fold, footprint, locality, definition-change, cross-axis evidence) | `All seven maintenance-plan proofs are built` | drop | |
| MP-21 | Keyed-grain output: partition-locality question not posed; locality-admitted keyed model's clamps still carry the assumed mirror into propagation | `poses no partition-locality question` | keep | merge:IC-23 |
| MP-22 | `MaintenanceSkeletonColumnAdded` reachable from pure derivation (and runtime driver) but not yet surfaced as an LSP/CLI diagnostic ahead of a run | `not yet surfaced as an LSP/CLI diagnostic` | keep | merge:IC-24 |
| MP-23 | Landed-work recital: skeleton-source closure's five-conjunct composition and its runtime-obligation wiring | narrative in bullet 328 before "v1 is restricted" | drop | |
| MP-24 | Skeleton-source closure v1 restricted to non-aggregating enrichment scopes; a join feeding `GROUP BY`/window is always `Open`; widening is open future work, not scheduled | `v1 is restricted to non-aggregating enrichment scopes` | keep | preserved |
| MP-25 | Only the source-enrichment `UpstreamMutation` route derives a declared-RI closure today; a model-edge creation cell's closure is always derived with an empty referential-integrity map | `closure is always derived` | keep | preserved |
| MP-26 | Landed-work recital: fingerprint projection (P4) built as walk-composed fail-closed proof, per-consumer derivation | narrative in bullet 329 before "No consumer reads it" | drop | |
| MP-27 | Fingerprint projection (P4) has no consumer yet (sidecar build / digest compare is later-phase scope) | `has no consumer yet` | keep | preserved |
| MP-28 | Append-only posture probe does not consult declared lateness; a legitimate late append into a closed partition can fire spuriously or mask a genuine violation | `does not consult declared lateness` | keep | preserved |
| MP-29 | `SourceUniqueKeyViolated` remains the one probe-registry row with no emitter at all | `the one probe-registry row with no emitter` | keep | preserved |
| MP-30 | Landed-work recital: every other probe row built and wired, single-owner dispatch helper, cadence policy | `Every other row is now built and wired` | drop | |
| MP-31 | Landed-work recital: output-delta shape derived/typed/acted on, keyed-edge classification wired | narrative in bullet 331 before "The remaining gap" | drop | |
| MP-32 | Keyed dirt-set is a symbolic key-addressed channel (key columns + provenance), not a materialised key-value set; value-level affected-key discovery stays with the run-time mechanism | `is a \*\*symbolic\*\* key-addressed channel` | keep | merge:IC-14 |
| MP-33 | `nondeterministic_columns` list-form declaration not yet removed from the parser (fossil; removal is a separate, out-of-scope row) | `is not yet removed from the parser` | keep | preserved |
| MP-34 | Grammar boundary between `columns.<c>.contract` and a future column `tests:` block deliberately deferred (Open Question, cross-ref `models.md`) | `deliberately deferred` | keep | preserved |
