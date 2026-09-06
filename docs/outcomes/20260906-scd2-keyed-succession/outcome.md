# Outcome: SCD2 — the keyed-succession grain, maintained

**Created:** 2026-09-06
**Status:** active
**Source:** spec diff `git diff 1e5e0675..HEAD -- docs/specs/` (commits `1dacc2d2`..`91cec5e2`, all `spec(scd2): ...`); `docs/research/20260723-scd2-succession-pattern.md`
**Spec anchors:** `docs/specs/incremental_shapes.md` §"Succession-grain admission (no declaration)", §"Diagnostics" (succession-grain codes), §"The succession grain" (§"The tombstone ledger (hidden state)", §"The maintenance theorem (bounded footprint)", §"Delete events", §"Run shape and late events", §"What stays out of this grain"), §"Succession-grain design", §"Succession-grain constraints", §Known Divergences "The succession grain"; `docs/specs/model_properties.md` §"Keyed-succession classification", §"Event-time monotonicity trace", §"The composition walk"; `docs/specs/model_transforms.md` (Succession-patch keyed `MERGE` row); `docs/specs/diagnostics.md` §"Succession grain"; `docs/specs/sources.md` §Known Divergences (append-only probe fingerprint leg); `docs/specs/state.md` (Tombstone ledger row, §"The degradation contract"); `docs/specs/incremental_models.md` §Limitations "SCD2 recognition is bounded to the keyed-succession pattern"

## The outcome

A `refresh: incremental` model with no declared grain whose SQL is the keyed-succession shape —
row-local columns plus `LEAD(t)`/`LAG(t) OVER (PARTITION BY k ORDER BY t)` over one
`append_only`, clocked source, with an optional pre-window lateness clamp and an optional
`QUALIFY NOT <flag>` delete filter — is recognised by a leaf classifier the composition walk
invokes, and is maintained by the succession-patch technique: each window's events are
projected row-locally, non-delete events are inserted, delete events are recorded in a
backend-resident tombstone ledger written in the same transaction, and each event's immediate
neighbours are patched over the union of presented rows, ledger, and batch. Late events reach
the grain on the run axis (arrival partitioning makes them free; event-time partitioning
re-presents the closed window), re-running or reordering windows is a no-op, and a `(k, t)`
collision rolls the run back with `SuccessionClockTie`. Every clause outside the grammar refuses
with its named code; nothing falls back silently. The generative conformance gate proves
`incremental_state(S) == full_refresh(model SQL over S)` for a succession recipe family that
exercises splices, deletes, delete-then-late-insert, delete-only keys, `LAG` projections,
out-of-order and repeated windows, and the clamp.

## Success criteria (checkable)

1. **Classifier.** `smelt_logical::analysis::succession::classify_keyed_succession` is a pure
   leaf returning exactly `Recognized{source, pre_filter, key_cols, clock_col, lead_cols,
   lag_cols, delete_flag}` / `NotSuccession{reason}` per `model_properties.md` §"Keyed-succession
   classification" rules 1, 1a, 1b, 2–6, invoked only from the walk in
   `crates/smelt-logical/src/analysis/walk.rs` and classified as a leaf in its doc comment
   (`cargo test -p smelt-logical --test walk_coverage` green). One unit test per rule and per
   named refusal: non-`LEAD`/`LAG`, `LEAD(other_col)`, explicit offset/default, mixed partition
   keys, nullable key, non-strict clock (`CAST(... AS DATE)`), clock tracing to a non-
   `event_time_column` column, descending/second sort key, unprojected `k`/`t`, aggregate
   sibling, join/CTE/subquery/set-op `FROM`, non-`append_only` or unclocked source,
   non-row-local or nondeterministic or second pre-filter, `WHERE NOT <flag>` advisory,
   `QUALIFY` misplacement/nullable flag, `DISTINCT`/`GROUP BY`/`HAVING`/`ORDER BY`/`LIMIT`.
2. **Diagnostics.** The eleven analysis-time codes (`SuccessionWindowFunctionNotLead`,
   `SuccessionPartitionKeyMismatch`, `SuccessionOrderNotMonotoneClock`,
   `SuccessionRowLocalColumnViolation`, `SuccessionIdentityNotProjected`,
   `SuccessionSingleSourceOnly`, `SuccessionDrivingSourceNotAppendOnly`,
   `SuccessionPreFilterNotRowLocal`, `SuccessionDeleteFilterMisplaced`,
   `SuccessionPreFilterNegatesFlag` as Warning, `SuccessionPatternUnrecognized`) are
   `DiagnosticCode` variants in `crates/smelt-db/src/diagnostics_types.rs`, emitted from the
   pure `maintenance_plan_diagnostics` owner into `check_file_diagnostics` (LSP and CLI see
   the same set), each exercised by a fixture under `examples/broken/`; `diagnostics_catalogue`
   green; the advisory never changes admission (a test asserts the plan is identical with and
   without it).
3. **Plan purity.** `Grain::Succession { key_cols, clock_col }` and
   `Technique::SuccessionPatch` exist in `crates/smelt-logical/src/maintenance/mod.rs`;
   `StateStructure::TombstoneLedger` is in `maintenance/availability.rs` and a target that
   cannot realise it (Spark, BigQuery, or `state.warehouse_tables: none`) downgrades the cell to
   full refresh with `MaintenanceStateDowngraded`, never a ledger-less patch (test per case).
   `crates/smelt-db`'s `derive_model_maintenance_plan` produces the one succession cell for
   the running example; `crates/smelt-logical/tests/maintenance_plan_conformance.rs`'s
   "SCD2 / versioned intervals" rows are updated from REFUSED/UNSUPPORTED to the succession
   verdict, with the snapshot-derived SCD2 row still refused. A succession model declaring
   `contract.frozen_horizon` or `contract.retain_departed` is refused by the existing
   `ContractFrozenHorizonInvalid` / `ContractRetainDepartedInvalid` rules naming the succession
   grain (test per code); `contract.deferral` is admitted with unchanged frontier-lag semantics.
4. **Emitters.** The event-delta `SELECT`, the succession-patch `MERGE` (neighbour domain =
   presented ∪ ledger ∪ batch, folded as if serial in `t` order, idempotent on `(k, t)`), the
   tombstone-ledger rebuild `SELECT`, and the clock-tie probe are pure functions in
   `crates/smelt-logical/src/maintenance/emit.rs`; the ledger table DDL lives in `smelt-state`
   as bookkeeping. `cargo test -p smelt-runtime --test statement_parity` gains a succession
   family leg (executed == emitted) and its no-authoring leg finds no succession SQL in
   `smelt-runtime` or any backend crate. Each emitter is proven against a real DuckDB in a
   unit test.
5. **Runtime.** On DuckDB the window-forward driver dispatches succession cells with the
   ledger write and presented `MERGE` in one transaction (a rollback-under-failure test leaves
   both untouched); re-folding a window leaves table and ledger byte-identical; two windows
   applied in either order converge; a non-identical `(k, t)` collision, a delete-vs-insert at
   one `(k, t)`, and two identical rows under `redelivery: none` each roll back with
   `SuccessionClockTie` naming key, clock value, and a sample; an identical re-presented row is
   a no-op; `--full-refresh` and `smelt repair` rebuild the ledger from the rebuild `SELECT`
   in the same transaction as the presented rebuild; the per-model frontier is kept at the
   re-run-tolerant grade and no `KeyedReprocessedWindow` fires. `execute_parity` green.
6. **Conformance.** `cargo test -p smelt-cli --test maintenance_conformance` covers the
   succession shape: `crates/smelt-maintenance-testkit` gains an arrival-partitioned
   `SourceRecipe` (`partition_column` ≠ `event_time_column`, lateness schedules landing old
   event times in new arrival windows, `is_deleted` flag) and a `SuccessionRecipe` family whose
   oracle is the model's own SQL (`QUALIFY NOT is_deleted`) at full refresh, with legs for:
   late-arriving splice, delete then later insert, late insert before a folded delete, a key
   whose only events are deletes, `LAG`-projecting models under each, out-of-order and
   repeated window application, the pre-window clamp (clamped rows absent from oracle and
   state), an event-time-partitioned source, and an equal-`(k, t)` collision expecting
   `SuccessionClockTie`. The `state_deletion.rs` and `repair.rs` legs include succession
   recipes, and the contract-lattice `deferral` leg includes one. Seeded sample green.
7. **Append-only probe.** `emit_append_only_posture_probe`'s fingerprint leg runs only when a
   closed partition's row count is unchanged and a count increase is classified as a late
   arrival whose covering window is re-presented (the decided reading in `model_properties.md`
   §Constraints "Declared lateness is orchestration-only"). This landed on the
   decision-residue outcome's branch on 2026-09-06 (`feat(sources): classify a late append
   into a closed partition as an observation`); what this outcome adds is a conformance
   `probes.rs` leg that lands a late append into a closed event-time partition of a
   *succession* recipe and asserts re-presentation, not `SourceMutationProfileViolated`.
8. **Explain.** `smelt explain <model>` (text and `--json`) renders for a succession model:
   the grain and `(k, t)` identity, the technique, the run axis vs the clock and whether the
   source is arrival- or event-time-partitioned, the re-run-tolerant and
   order-independent-but-serial postures, the pre-filter if any, and the tombstone ledger as
   internal state; the recorded downgrade renders as it does for other cells. Explain tests
   cover text and JSON; `cli_docs_coverage` green.
9. **Fixture and docs.** An example workspace carries `customer_changes` (arrival-partitioned,
   `append_only`, `is_deleted NOT NULL`) and `customer_history` with the delete filter, with
   zero diagnostics (`cargo test -p smelt-cli --test example_diagnostics`,
   `cargo test -p smelt-lsp --test example_workspaces`); a docs-site guide page documents the
   shape, the grammar, the two partitioning postures, and the refusal codes;
   `docs-site/docs/reference/diagnostics.md` lists the twelve codes.
10. **Spec closure.** `model_transforms.md`'s succession row reads **built**; the
    `incremental_shapes.md` §Known Divergences "The succession grain" bullets, the
    `diagnostics.md` "twelve succession-grain codes are specified and unimplemented" bullet,
    and `model_properties.md`'s `not-yet` status for the classifier are deleted or rewritten
    to a residual gap; `/smelt:validate incremental_shapes`, `model_properties`, and
    `diagnostics` clean; all standing gates green (`verify-phase.sh`, `walk_coverage`,
    `statement_parity`, `execute_parity`, `maintenance_conformance`, `diagnostics_catalogue`,
    `projection_dialect_invariance`, hardening ratchets).

## Out of scope

- Every grammar widening `incremental_shapes.md` §Future Extensions names: `LEAD`/`LAG` over a
  non-clock column, an enrichment join, a `change_feed` driving source, a dedupe-before-`LEAD`
  CTE, un-negated `QUALIFY <col>`, a succession projection nested in a CTE or `UNION` arm.
- Derived output facts (clock and identity) for a succession model's consumers.
- A Spark or BigQuery tombstone-ledger builder — those targets take the recorded downgrade
  (criterion 3), per the state-residency decision of 2026-09-04.
- SCD2 over mutable snapshots (a source-layer facility, never this grain).
- Tombstone-ledger compaction (the spec says never).
- New contract-lattice points or grain-specific contract semantics. `frozen_horizon` and
  `retain_departed` on a succession model are refused by the existing grain/posture rules;
  `deferral` is admitted with its existing frontier-lag semantics (decision of 2026-09-06,
  below) — no new lattice point is defined here.
- Concurrent window application (the grain is serial by constraint 5).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec closure delta: pin the residual unspecified surface only — the tombstone ledger as a per-model sibling table (name derived from the model, columns exactly `k ∪ {t}` in the model's own types, PK `(k, t)`, lifecycle tied to the presented table), the `smelt explain` succession rendering fields (text + `--json` keys), and the contract-lattice posture for a succession model (`frozen_horizon`/`retain_departed` refused by the existing rules naming the grain, `deferral` admitted with unchanged semantics) | done |
| 2 | Classifier leaf: `analysis/succession.rs` with the verdict type and every rule/refusal reason, wired into the walk as a leaf; `walk_coverage` classification; per-rule unit tests | done |
| 2a | Gate hygiene: de-flake `smelt-core`'s `checkout_scratch_is_deleted_when_materialization_fails` (unique scratch-dir naming / narrower listing) so `verify-phase.sh` is unambiguously green for every later phase's verification | done |
| 2b | Gate hygiene: fix the `hardening_budget` ratchet regression phase 2a's own `verify-phase.sh` run found (`smelt-logical` production `unwrap`/`expect` grew from baseline 1/1 to 2/4, all four new sites in phase 2's `analysis/succession.rs`) — classify each site as infallible or convert to `Result` per the fail-loud gate; no silent baseline bump without a reviewer sign-off note | planned |
| 3 | Plan and diagnostics: `Grain::Succession`, `Technique::SuccessionPatch`, `StateStructure::TombstoneLedger` + availability downgrade; plan derivation in `smelt-db`; the eleven `DiagnosticCode` variants from the pure owner into `check_file_diagnostics`; `examples/broken` fixtures; `maintenance_plan_conformance` rows | pending |
| 4 | Emitters: event-delta `SELECT`, succession-patch `MERGE` over the neighbour domain, ledger rebuild `SELECT`, clock-tie probe in `smelt-logical`; ledger DDL in `smelt-state`; DuckDB-proven unit tests; `statement_parity` family leg | pending |
| 5 | Runtime: window-forward driver dispatch for succession cells with transactional ledger write, re-run-tolerant frontier grade, clock-tie probe → `SuccessionClockTie` rollback, `--full-refresh`/`smelt repair` ledger rebuild; `execute_parity` | pending |
| 6 | Append-only probe: confirm the count-gated fingerprint leg is on `main` (landed via the decision-residue branch); add the succession-recipe late-append `probes.rs` leg | pending |
| 7 | Testkit and conformance: arrival-partitioned `SourceRecipe`, `SuccessionRecipe` family with the model-SQL oracle and every listed leg; state-deletion and repair legs widened; seeded sample green | pending |
| 8 | Explain surface: grain, identity, run axis vs clock and partitioning posture, execution postures, ledger as internal state, text + `--json`; explain tests | pending |
| 9 | Fixture and docs: example workspace `customer_changes`/`customer_history` with zero diagnostics; docs-site guide page and diagnostics reference | pending |
| 10 | Validate and close: divergences rewritten across the six specs, `/smelt:validate` clean, all standing gates green | pending |

## Decision log

- 2026-09-06 (scaffold): the branch already carries the full spec delta (six `spec(scd2)`
  commits), so phase 1 is a closure pass over what those commits left unspecified, not a
  fresh normative draft. Items the human should settle before or during phase 1:
  - The tombstone ledger's physical name and schema are unspecified (`state.md` classifies it;
    `incremental_shapes.md` says "sibling table" holding `(k, t)`). Suggested: follow the
    `_smelt_ledger` convention in `crates/smelt-state/src/ddl_duckdb.rs`.
  - A declared contract-lattice point on a succession model has no defined meaning. The
    contract-lattice single-ownership rule means a silent accept is not admissible; the likely
    answer is a refusal by the existing `Contract*Invalid` codes naming the grain.
  - Criterion 7 (the append-only probe's fingerprint leg) is tracked by
    `docs/outcomes/20260809-probe-backed-facts/outcome.md` but is criterion-serving here (the
    event-time-partitioned late-event leg cannot pass without it), so this outcome owns the
    fix unless that outcome lands it first.
  - `OutputSpec.skeleton_columns` is "v0: supplied, not extracted"; for the succession grain
    the skeleton is `k ∪ {t}` from the verdict — the planner decides whether to extract it
    here or keep supplying it.
- 2026-09-06 (human resolutions of the scaffold items above):
  - **Tombstone ledger: per-model sibling table**, not the shared `_smelt_ledger` convention.
    The neighbour lookup runs `LEAD`/`LAG` over the union of presented rows and ledger rows
    ordered by `t`, so the ledger must carry `k` and `t` in the model's own column types; a
    shared VARCHAR-keyed table would force casts into every neighbour lookup. A key/clock change
    is a skeleton change (new relation, ledger included), which also fits a per-model table.
    Name derived from the model's table name; columns exactly `k ∪ {t}`; PK `(k, t)`; created
    and dropped with the presented table. Phase 1 pins the exact naming.
  - **Contract-lattice points: no new point.** `frozen_horizon` is already refused on any
    non-partition grain (`smelt_logical::contract::frozen_horizon::validate_frozen_horizon`)
    and `retain_departed` is already refused unless the shape consumes a `mutable_snapshot`,
    which a succession model never does — phase 3 adds a test per refusal naming the succession
    grain. `deferral` is **admitted with unchanged semantics**: it measures frontier lag against
    the model's clock, which is grain-independent, and a succession model always carries a clock.
    Phase 7 adds a succession recipe to the conformance gate's deferral leg. Fallback if the
    existing deferral oracle transform proves grain-specific: refuse via `ContractDeferralInvalid`
    and record a divergence.
  - **Append-only probe fix: already landed elsewhere.** While this outcome was being
    scaffolded, the decision-residue outcome's loop branch landed the count-gated reading
    (`feat(sources): classify a late append into a closed partition as an observation, not an
    append-only violation`, 2026-09-06) and deleted the matching `sources.md` divergence. The
    stale divergence bullet this spec branch had added was dropped before merge. Phase 6 is
    now confirm-plus-one-leg, not a build.
  - **Skeleton columns: derive from the verdict** (`k ∪ {t}`), consistent with the direction
    `maintenance/skeleton.rs` already takes; do not hand-supply.
  - **Backlog placement: last.** Listed at the very end of `.claude/outcome-backlog`, below the
    done section, under its own header: the hygiene loop branch edits the lines around the
    done marker, and an insertion beside them conflicts on the loop's `origin/main` merge. The
    loop reads entries in file order and skips done/blocked ones, so the position is
    equivalent to "after `20260904-dialect-emission-vocabulary`". Promote by moving the line.

- 2026-09-06 (plan phase 1): no reshape — phase 1 is the first row, there is no
  prior summary, and the human resolutions of 2026-09-06 already reshaped phases 1
  and 6. Phase 1 planned as three normative pins with no new diagnostic codes: the
  ledger is a per-model sibling table `<presented table>__tombstones` holding exactly
  `k ∪ {t}` in the model's own types with PK `(k, t)`; `smelt explain` gains a
  `keyed_succession` delta-signature shape plus a `succession` JSON object and a text
  block; and the contract lattice gains no point — `frozen_horizon`/`retain_departed`
  fall to the existing refusals, `deferral` is admitted unchanged. A reserved-suffix
  relation collision is recorded as a residual divergence rather than a twelfth code,
  to hold the outcome's stated code budget.

- 2026-09-06 (phase 1 done): all four spec edits landed as planned, no reshape needed. Found a
  pre-existing, unrelated flaky test (`smelt-core`'s
  `checkout_scratch_is_deleted_when_materialization_fails`, races on shared `/tmp` scratch-dir
  listing under parallel test threads) — confirmed unrelated via a docs-only diff and an
  isolated `--test-threads=1` pass; not fixed here (out of this phase's spec-only scope, not a
  success criterion). See `phases/01-summary.md`.

- 2026-09-06 (plan phase 2): one reshape — inserted row **2a**, a gate-hygiene fix for the
  pre-existing `smelt-core` baseline flake phase 1's summary found
  (`checkout_scratch_is_deleted_when_materialization_fails` races on a shared `/tmp`
  scratch-dir listing under parallel test threads). Not deferred to "## Out of scope" because
  success criterion 10 requires `verify-phase.sh` green, and every remaining phase's
  verification is ambiguous while the workspace suite is red for an unrelated reason; placed
  after phase 2 (which is additive and self-contained) so the fix lands before the heavy code
  phases 3–7. Phase 2 itself planned with no spec delta — the classifier's rules are already
  normative — and with `NotSuccessionReason` as an eleven-variant enum 1:1 with the
  analysis-time codes, so phase 3's `DiagnosticCode` mapping is a total match rather than a
  re-derivation, and with `SuccessionPreFilterNegatesFlag` carried as an advisory on the
  `Recognized` verdict rather than as a refusal variant (the spec says it never changes
  admission).

- 2026-09-06 (phase 2 done): landed as planned, no reshape. `classify_keyed_succession`
  (`crates/smelt-logical/src/analysis/succession.rs`) implements rules 1, 1a, 1b, 2–6 with a
  ten-variant `NotSuccessionReason` (1:1 with the ten admission-changing codes) plus
  `SuccessionAdvisory::PreFilterNegatesFlag` carried on `Recognized` for the eleventh (Warning)
  code; wired into `walk.rs` as `model_keyed_succession`, its sole call site. 39 unit tests
  green (one per named refusal plus 6 recognition cases plus 2 wiring tests); `walk_coverage`
  green; `rg` confirms no call site outside `succession.rs`/`walk.rs`. One planned test
  (`refuses_where_over_window_derived_column`) dropped as unimplementable without a fuller
  source column schema than `SuccessionContext` carries today — noted as a gap, not silently
  skipped. `verify-phase.sh`'s full `cargo test` leg is red only on the pre-existing, unrelated
  `smelt-core` baseline flake phase 1 found (confirmed again via an isolated
  `--test-threads=1` rerun); phase 2a fixes it next. See `phases/02-summary.md`.

- 2026-09-06 (plan phase 2a): no reshape — the row was inserted by the phase-2 plan step and
  the phase-2 summary added nothing that changes the remaining phases. Root cause pinned before
  planning: `checkout_scratch_is_deleted_when_materialization_fails` snapshots every
  `smelt-baseline-*` entry in the shared `std::env::temp_dir()`, and
  `materialize_is_not_racing_git_archive_to_a_broken_pipe` — the one test in the file that takes
  no `lock()` guard — churns 200 such scratch dirs across 8 threads. The race is cross-process
  too (`smelt-runtime`'s `property_diff` and `smelt-cli`'s `transformer_metamorphic` also call
  `materialize`, and cargo runs test binaries in parallel), so taking the lock cannot fix it.
  Planned as a scratch-parent seam (`materialize_in(resolved, parent)`, `materialize`
  delegating to it with `std::env::temp_dir()`) with the test asserting over a private
  directory. No spec delta: `property_diff.md` §"Baseline materialisation" describes the
  unwind-on-error behaviour, which is unchanged.

- 2026-09-06 (phase 2a done): landed as planned, no reshape to phase 2a itself, but one row
  inserted. `materialize_in(resolved, scratch_parent)` (`crates/smelt-core/src/baseline/git.rs`)
  is the extracted seam; `materialize` delegates to it with `std::env::temp_dir()`; the flaky
  test now asserts against a private `TempDir` and two new tests pin the seam's contract
  (private-parent placement + the default-parent delegation). 20/20 green under the
  concurrency-reproduction loop; full `smelt-core` suite green.
  `bash .claude/scripts/verify-phase.sh` is still **not** green: the workspace `cargo test` leg
  fails on `smelt-core`'s `hardening_budget::gate_detects_regression`, but not for the flake —
  isolating each `analysis/` file's production `unwrap`/`expect` count shows all four new sites
  (`unwrap` baseline 1→2, `expect` baseline 1→4) are in `crates/smelt-logical/src/analysis/
  succession.rs`, landed by phase 2's classifier, not by anything in this phase's diff or by the
  `analysis/mod.rs`/`walk.rs` split refactors (`a411f3f6`, `5107c66b`) that looked like the likelier
  suspect. Confirmed by reverting this phase's diff via a temporary stash and re-running the gate
  in isolation — still red on the unchanged tree. Out of phase 2a's scope (a different gate, a
  different file, not the flake this phase targeted) per the plan's own instruction to record
  rather than force a fix. Inserted row **2b** ahead of phase 3 to resolve it, since criterion 10
  needs every standing gate green and every later phase's `verify-phase.sh` run is otherwise still
  ambiguous for the same reason 2a itself was inserted. See `phases/02a-summary.md`.

- 2026-09-06 (implement dispatch on row 2b): row 2b was marked `planned` in the phase table
  (inserted that way by the phase-2a plan step's write, rather than via a normal PLAN-step
  pass that writes `phases/02b-plan.md` and then flips the row) but no `phases/02b-plan.md`
  exists. The IMPLEMENT step contract requires reading that file and must not author its own
  plan — that authority belongs to the PLAN step (opus judgment, reads the prior summary,
  reshapes remaining rows). Reset the row to `pending` so the next loop iteration runs a PLAN
  step and produces the missing plan artifact before implementation proceeds. No code touched
  this iteration.

- 2026-09-06 (plan phase 2b): no reshape — the row was inserted by the phase-2a plan step and
  the 2a summary raises nothing else that changes the remaining rows. Pinned before planning
  that `.claude/scripts/hardening-budget.sh` is a **pure per-crate count ratchet** with no
  "classified as infallible" allowlist, so "classify or convert" collapses to a single
  admissible move here: eliminate all four sites so `smelt-logical` returns to exactly
  `unwrap 1` / `expect 1` with the baseline file untouched (a `--update` would be the silent
  lowering the fail-loud rule forbids). Planned as two structural rewrites in
  `analysis/succession.rs` — destructure the single-window select-list arm instead of
  asserting it, and replace the `Option`-accumulating shared-window loop with a
  `split_first()` seed so the "loop ran at least once" invariant becomes a type-level fact —
  plus two new unit tests covering the two refusal paths the rewrite touches
  (non-bare-column `ORDER BY`, two window calls in one projection), which today have none.
  The existing 39 succession tests are the behaviour-preservation oracle and must pass
  unedited.

## Blocked

(none)
