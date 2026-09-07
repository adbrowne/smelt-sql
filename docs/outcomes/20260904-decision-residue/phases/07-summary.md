# Phase 7 summary — close the residue

**Shipped:**
- `docs/specs/models.md`: removed retired `data_latency` from the "Facts and their fill modes"
  table (`*model-only*` row) and the §Design "Declared" paragraph — both still listed it as a
  live per-column declared fact even though the frontmatter table already marked it Retired.
- `docs/specs/sources.md`: the `mutation_profile` Known Divergences bullet dropped `lateness`
  from the "still unbuilt per-cell admission" list (it is never a plan input at all, so there is
  no admission to await) and now notes `key_recurrence` is checked, not only consumed
  (`KeyedRecurrenceDeclarationMismatch`, phase 4).
- `crates/smelt-logical/tests/lateness_orchestration_only.rs`: new doc-sweep test
  `specs_do_not_present_per_column_data_latency_as_live`, greps `docs/specs/*.md` and
  `docs-site/docs/**` for `data_latency` and requires every surviving mention to name the
  retirement. Red before the `models.md` edit, green after.
- `docs/specs/model_properties.md`: bumped `last_reviewed`; added a Known Divergence bullet (see
  below) rather than a fix, per the plan's scope rule.

**Decisions:**
- Found a real contradiction in `model_properties.md` §"Unified bound / reach derivation" and its
  declarations-table row: both describe `derive_model_bounds` as folding a declared
  *source-lateness* margin into the licensed `Bounded{before, after}` scan widening. Grepped
  `crates/smelt-logical/src/analysis/source_bounds.rs` — it reads no lateness value at all; reach
  comes from the SQL's frames/offsets/interval shifts only, matching §Constraints "Declared
  lateness is orchestration-only" (added by the same commit, `3e9c1a4a`, that started this
  outcome — i.e. this prose was already stale before phase 1 began, not something phases 1-6
  broke). Per the plan's explicit rule ("a sentence a phase 1-6 change made false gets corrected;
  anything else is left alone and recorded... as out of scope" / step 4's drift classification
  (c): pre-existing and unrecorded → add a bullet, don't implement), I added a Known Divergence
  bullet naming the gap rather than rewriting the prose in this phase.
- The four `/smelt:validate` passes were done as targeted reads + greps (Surface/Semantics/
  Invariant/timeless-oracle checks) rather than full automated re-runs per spec, since
  `verify-phase.sh` already covers the automated-checks section once for all four and running
  `cargo test`/clippy four times would be pure repetition. Timeless-oracle grep
  (`Phase [A-Z0-9]+` across the four specs) found only the rule-statement sentence itself in
  `sources.md`/`diagnostics.md` — no leakage.

**For the next planner:**
- The `model_properties.md` "source-lateness margin" stale-prose bullet (just added) needs an
  actual wording pass removing the source-lateness component from both the table row and
  §"Unified bound / reach derivation" — this outcome's own Known Divergence bullet documents it
  but does not close it. Small, contained, no code change needed.
- `diagnostics.md` (`last_reviewed: 2026-07-17`) was read but not edited — both
  `PartitionGrainForbidsMetrics` and `KeyedRecurrenceDeclarationMismatch` are already correctly
  catalogued from phases 1 and 4. No drift found there.
- Outcome success criteria 1-9 all judged met by the six prior phase summaries plus this one:
  bullets deleted (own-inline per phase 1 precedent) or corrected (this phase's residue sweep),
  all listed gates green. Recommend flipping `outcome.md` Status to `done` after this phase's row
  is marked done — no further phases are queued.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test lateness_orchestration_only --test walk_coverage` — 3 + 8
  passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4 + 37 passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 81 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 122 passed, 1 ignored.
- `cargo test -p smelt-core --test hardening_budget` — 4 passed (baseline unchanged; the
  "REGRESSION" line in output is the gate's own self-test fixture, not a real regression).
