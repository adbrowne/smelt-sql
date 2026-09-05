# Phase 10 summary — close the keyed-grain residue outcome; refresh docs-site state pages

**Shipped:**
- `crates/smelt-cli/tests/state_docs_freshness.rs` (new, 3 tests): no docs-site page claims a
  `.smelt/`-resident `reconciliation.json`; `smelt-yml.md` documents the `state` block; `state.md`
  states the per-posture write set and the "deleting `.smelt/` never changes what a maintained
  model computes" invariant.
- `docs-site/docs/reference/state.md`: dropped the `reconciliation.json` inventory row and its
  locking/lazy-creation mentions; added a `state.mode` and what is written section (per-posture
  table); rewrote §"The reconciliation ledger" to state engine residency and the
  `MaintenanceStateDowngraded` downgrade path (replacing the old "says so on the run's progress
  output" sentence); led the recovery playbook's "`.smelt/` is lost" entry with the residency
  invariant.
- `docs-site/docs/reference/smelt-yml.md`: added the `state` top-level-field row and a new
  §"State Configuration" section (`mode`, `warehouse_tables`, both diagnostics); dropped
  "reconciliation ledgers" from the State-isolation-per-target paragraph (it's backend-schema
  isolated now, not `.smelt/`-isolated).
- `docs-site/docs/guide/deployment.md`, `docs-site/docs/reference/cli.md`: same
  reconciliation-ledger removal from the layout tree / per-target artifact list.
- `docs-site/docs/guide/targets.md`: Spark `Additive`-combiner row rewritten from "fails loud" to
  the recorded, explain-visible downgrade.
- `docs-site/docs/reference/smelt-explain.md`, `docs-site/docs/guide/incremental-models.md`:
  added the residency + downgrade-printing statements to the per-model maintenance-plan sections.
- `docs/specs/state.md` §Known Divergences: all three bullets deleted (state.mode honoured,
  ledger engine-resident, warehouse_tables parsed — all landed in phases 2-9); replaced with a
  one-line "none currently open."
- `docs/specs/run_state.md`: deleted the stale "shipped store still writes reconciliation ledger
  under `.smelt/`" divergence bullet and repointed the `reconciliation.rs` References entry.
- `docs/outcomes/20260815-keyed-grain-residue/outcome.md`: criterion 3 amended to the decided
  downgrade wording, phase 3 row flipped to `done`, Status flipped to `done`, decision-log entry
  and a Blocked-section resolution line added.

**Decisions:**
- The plan's own Verification gate (`rg reconciliation\.json docs-site/ docs/specs/`) covers
  `docs/specs/`, not just `docs-site/`, despite the plan's "Spec delta: None" header — read as
  "no new normative content", not "no gap-list hygiene". Two stale Known-Divergences bullets
  (`state.md`, `run_state.md`) that the rg check would otherwise fail on were deleted after
  confirming each criterion they tracked (1, 2, 5) is actually landed in the code
  (`crates/smelt-runtime/src/execute.rs` consults `StateMode`; `crates/smelt-core/src/config.rs`
  parses `warehouse_tables`; `ddl_duckdb.rs`/`file_store.rs` confirm the ledger is
  `_smelt_ledger`-resident, not file-resident).

**For the next planner:**
- Phase 11 (validate + close out) is next: `/smelt:validate state`, confirm no drift, and note
  that this phase already emptied `state.md` §Known Divergences to a one-line "none currently
  open" — phase 11 should treat that as done rather than redoing it, and focus on gate green
  confirmation plus any drift `/smelt:validate` itself surfaces.
- No new gaps discovered outside this phase's task list.

**Gates:**
- `cargo test -p smelt-cli --test state_docs_freshness` — 3 passed (red before edits, green
  after).
- `cargo test -p smelt-cli --test tutorial_freshness --test cli_docs_coverage --test
  rebuild_dry_run` — all passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `rg -n "reconciliation\.json" docs-site/ docs/specs/` — one hit, in `run_state.md`'s legacy-
  migration sentence describing correct non-migration behaviour (not a divergence).
