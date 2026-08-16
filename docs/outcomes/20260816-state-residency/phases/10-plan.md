# Phase 10 plan — Docs sweep: state modes, residency, and closed Known Divergences

## Objective

Bring the user-facing docs and the spec §Known Divergences sections into line with what phases
2–9 actually shipped, so criterion 6's "every Known Divergences bullet this outcome claims is
actually removed from the owning spec" holds and `/smelt:validate state` reports no drift. This
is the documentation half of criteria 1–4: `state.mode` is now consulted, the reconciliation
ledger is engine-resident, availability resolution exists and is explain-visible, and the
absent-state behaviours are implemented — none of that is reflected in `docs-site/` or in
`state.md` §Known Divergences today.

## Spec delta

Spec edits ARE the phase (no runtime behaviour changes). Verify each claim against the code
before removing a bullet — a bullet whose claim is still partly true gets **narrowed**, not
deleted, and names precisely what remains.

- **`docs/specs/state.md` §Known Divergences**
  - Delete bullet 1 ("The runtime ignores `state.mode` entirely") — phase 2 threaded
    `StateMode` through `execute_project`.
  - Delete bullet 3 ("No availability-resolution step exists in derivation") — phases 5/6/9
    landed the two-step derivation, `MaintenanceStateDowngraded` (`smelt-db`
    `diagnostics_types.rs`, explain text + `--json`), and `DeclaredContractRequiresState`.
  - Rewrite bullet 4 ("Structure-level degradation behaviours … not yet honoured"): phase 3
    landed the absent-baseline behaviour (`ProbeBaselineUnavailable` in `smelt-runtime`'s
    source/contract probes and the reporter). Confirm whether it is surfaced as a
    `DiagnosticCode` variant or only as a run-time advisory and state exactly that; keep only
    the residue that is genuinely still open.
  - Bullet 2 ("No ledger builder exists outside DuckDB") stays, but drop its trailing
    "fails loudly today rather than downgrading" clause, which phase 5 falsified.
  - §References "User docs" line: it claims `smelt-yml.md` documents `state.mode`; it does not
    (`deployment.md` does). Fix after task 4 lands the reference entry.
- **`docs/specs/incremental_models.md` §Known Divergences** — narrow the "ledger/frontier
  warehouse substrate is DuckDB-only" bullet: the "fails loudly / skips its frontier reset with
  a warning" half is closed (the downgrade path is what happens now); only the
  dialect-coverage question and the Spark-builder open question remain.
- **`docs/specs/run_state.md` §Known Divergences** — the forward-propagation bullet's closing
  "until forward propagation and the reconciliation ledger land" is stale (the ledger landed).
  Narrow to the forward-propagation half only.
- Per phase 9's summary, do **not** describe `resolve_live_delta_restriction_facts` or
  `propagation.rs`'s edge walk as availability residue — both are intentional and commented.

## Tests

New doc-sync gate `crates/smelt-cli/tests/state_docs.rs` (pattern: `smelt-logical`'s
`backbuild_docs.rs`):

1. `every_state_mode_variant_is_documented` — every `StateMode` variant in
   `smelt-core/src/config.rs` appears in `docs-site/docs/reference/smelt-yml.md`'s `state` key
   documentation. Red today (the file documents no `state.mode` at all); a future variant
   added without docs fails.
2. `reconciliation_ledger_is_not_a_documented_smelt_dir_artifact` — neither
   `docs-site/docs/reference/state.md`'s inventory table nor
   `docs-site/docs/guide/deployment.md`'s layout block lists `reconciliation.json` as a live
   `.smelt/` artifact (a legacy-import mention in prose is allowed and must be matched
   explicitly, not by accident). Red today.
3. `state_reference_documents_the_downgrade_and_engine_resident_ledger` — the reference page
   names `_smelt_ledger`/`_smelt_frontier` and the downgrade behaviour, so the residency claim
   the spec makes has a user-facing home. Red today.

## Tasks

1. Read `docs/specs/state.md` §Semantics ("residency rule", "optionality rule", "degradation
   contract") and confirm each Known-Divergence claim against the code before editing.
2. Land the spec §Known Divergences edits above (spec-first, before docs-site).
3. Write the three failing tests in `crates/smelt-cli/tests/state_docs.rs`.
4. `docs-site/docs/reference/smelt-yml.md`: add the `state:` key section — `mode`
   (`stateless` default | `intervals` | `environments`), one line per mode on what it persists
   and what degrades without it, linking `guide/deployment.md#smelt-state-layout` and
   `reference/state.md`.
5. `docs-site/docs/reference/state.md`: drop the `reconciliation.json` inventory row; replace
   with a short "The reconciliation ledger lives in the warehouse" subsection
   (`_smelt_ledger`/`_smelt_frontier`, transactional with the fold, survives `.smelt/`
   deletion, legacy `reconciliation.json` imported once then removed); fix the "independent of
   `state.mode`" sentence and the "`.smelt/` is lost" recovery paragraph so neither implies a
   keyed fold's correctness rides on `.smelt/`.
6. `docs-site/docs/guide/deployment.md`: remove `reconciliation.json` from the layout listing;
   state that a keyed model's correctness state is engine-resident and not part of the
   `.smelt/` backup surface.
7. Add one short user-facing paragraph (best home: `docs-site/docs/guide/incremental-models.md`
   or `reference/smelt-explain.md`) on the downgrade: on a backend with no ledger builder the
   cell downgrades to a recompute and `smelt explain` prints the `state downgrade:` line
   (reuse phase 9's real output).
8. Run `/smelt:validate state`; fix any drift it reports that is in this outcome's scope, and
   record the report verbatim in `phases/10-summary.md`. Anything out of scope goes in the
   summary's "For the next planner", not silently dropped.
9. Green the three tests; re-check every `§"…"` cross-reference introduced still resolves.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full)
- `cargo test -p smelt-cli --test state_docs`
- `cargo test -p smelt-cli --test tutorial_freshness --test example_diagnostics`
- `/smelt:validate state` — report captured in the summary
- Timeless-oracle check: no `Phase [A-Z0-9]`, plan, or outcome vocabulary in the docs-site
  prose or spec bodies added by this phase (tracking links in §Known Divergences excepted).

## Commit message

`docs(state): sync docs-site and spec Known Divergences with shipped state residency`
