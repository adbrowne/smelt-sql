# Phase 1 summary — Spec: output-delta types, transfer rules, typed edges, the narrowed keyed refusal

**Shipped:**
- `docs/specs/model_properties.md`: new §Surface → *Derived proofs* row **Output-delta shape**
  (maturity `not-yet`); new §Semantics section `### Output-delta shape` (after §"Affected-key
  discovery") stating the lattice `AppendOnlyWindow{axis} ⊑ KeyedUpsert{keys} ⊑ General{reason}`,
  the per-column-group scoping, the addressing component, and a transfer-rule table (selection /
  projection / `UNION ALL` preserve or meet; keyed aggregation over `AppendOnlyWindow` emits
  `KeyedUpsert{k}`; join emits the meet degraded to `General` on proven `OneToMany`; window
  functions and unregistered operators fail closed to `General`); composition-walk lattice bullet
  renamed to cite the new section instead of the old informal spelling; new Known Divergences
  gap entry.
- `docs/specs/incremental_models.md` §"The graph layer": new **Typed edges** paragraph
  ((shape × addressing × column set) vector, day intervals restated as the `AppendOnlyWindow`/
  window-addressed case, adjoint property preserved); new **Keyed dirt-sets and the narrowed
  refusal** paragraph; the Refusals paragraph now scopes the keyed-node refusal to a `General`
  verdict instead of "no admitted time axis" categorically; new Known Divergences gap entry
  under §"The contract, plan, and graph layer".
- `crates/smelt-logical/tests/output_delta_spec.rs`: 5 new tests gating the above (lattice order,
  transfer-table well-formedness, fail-closed-row presence, §Surface row, graph-layer wording).

**Decisions:**
- Table rows that *preserve* the input shape spell out all three lattice names explicitly
  (`"AppendOnlyWindow"/"KeyedUpsert"/"General", whichever the input already is`) rather than just
  saying "preserves the input shape" — makes the transfer-rule table machine-checkable per-row
  without weakening the spec prose. Appended to outcome.md decision log.

**For the next planner:**
- Phase 2 (walk transfer rules) has a concrete table to implement against; the transfer function
  signature sketched in prose is `(operator, child verdicts) → verdict` per the existing walk
  convention (`crates/smelt-logical/src/analysis/walk.rs`).
- No code changes landed here beyond the test — `OutputDelta`, the transfer functions, and edge
  typing are all still `not-yet`/unbuilt; phase 2 is unblocked to start on that.
- Nothing out of scope was discovered; the plan's phase boundaries (spec here, walk in phase 2,
  edge typing in phase 3, consumer fold in phase 4, keyed dirt-sets in phase 5, conformance in
  phase 6, surface in phase 7) still look right after writing the spec text.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test output_delta_spec` — 5/5 passed.
- `cargo test -p smelt-logical --test probe_obligation --test walk_coverage` — 6/6 + 4/4 passed
  (unchanged spec gates still parse the edited files).
- `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]' docs/specs/model_properties.md docs/specs/incremental_models.md`
  — 2 hits, both pre-existing (unchanged from baseline), no new hits.
