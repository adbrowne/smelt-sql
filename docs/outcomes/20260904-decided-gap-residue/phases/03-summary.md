# Phase 3 summary — once-write fallback-case nullability route

**Shipped:**
- `column_provably_not_null(unique_key, column)` (`crates/smelt-logical/src/analysis/not_null.rs`) — leaf classifier, case-insensitive `unique_key` membership. `partition_column_provably_not_null` now delegates its `unique_key` case to it.
- `classify_once_write` (`crates/smelt-logical/src/rules/cumulative.rs`) admits `COALESCE(MAX(<col>)/MIN(<col>), <fallback>)` with `state: None` when `<col>` is a `unique_key` column and the existing FD-backed proof holds — the fallback is dead by construction, so no decomposed state is needed.
- Unit tests in `cumulative.rs`: not-null candidate admits statelessly with the FD declared; an ordinary nullable candidate still decomposes (regression guard); the not-null route never substitutes for the FD requirement (stays `Unproven` with none declared).
- `crates/smelt-db/tests/maintenance_fold_spec_companion.rs::fold_spec_admits_the_not_null_fallback_spelling` — plan-layer/runtime admission parity for the new route.
- `docs/specs/incremental_shapes.md` — column-family catalogue bullet, decomposed-state sentence, and the Known Divergences bullet under "The key grain" all updated.

**Decisions:**
- 2026-09-05 (already logged pre-implementation): scope the route to `unique_key`-derived non-nullness only, not driving-clock-derived — `derive_fold_spec` resolves no driving source.
- 2026-09-05: **did not** wire a generative-pool (`KeyedCombiner`) recipe for this route. Attempted it and hit two independent, unrelated validation walls in the real compiler surface — see below.

**For the next planner (important — success criterion 3 is only partially closed):**
- The classifier route, its unit tests, and plan-layer parity are done and green. The **generative-pool coverage** clause of success criterion 3 is NOT done — it is structurally blocked, not merely unattempted:
  1. The route needs a declared FD naming the `unique_key` candidate as `determines`, but for a single-column `unique_key` the only legal `key` is that same column, and `key -> key` is rejected as self-contradictory by `smelt_core::metadata::validate_functional_dependencies` (`crates/smelt-core/src/metadata.rs`, backed by a standing test in `crates/smelt-cli/tests/explain_model.rs`).
  2. Widening the model's `GROUP BY` to a second column to get a legal, distinct `key`/`determines` pair doesn't work either if that second column is the driving source's clock/partition column — `grain: key` models refuse that shape outright (`KeyedGroupByContainsPartitionColumn`).
  3. `KeyedRecipe`'s driving source (`SourceRecipe::events`) has exactly three physical columns (`d`, `id`, `val`) — there is no non-clock, non-key column available to serve as a legitimate second key member.
  - Net: this route is real and unit/plan-layer tested, but **currently unreachable via YAML** for the testkit's existing single-key-shaped sources, and per (1)-(2) likely unreachable for real single-column-`unique_key` models generally. A composite-`unique_key` model with a non-clock second key column could reach it in principle.
  - Candidate options for whoever picks this up: (a) widen `SourceRecipe`/`KeyedRecipe` with a fourth, non-clock identity column so a legitimate composite-key recipe becomes possible (bigger than this phase — touches `feed.rs`, `s_tracker.rs`, `schedule_gen.rs`'s `GenRow` shape); (b) reconsider whether `validate_functional_dependencies`'s self-contradiction check should special-case a trivial `key = determines` single column (a real semantics question — is `X -> X` actually a *useless* declaration worth rejecting, since it's true, or worth exempting when a caller like this route needs to force-satisfy the FD gate for a value already proven non-null another way?); (c) reconsider whether `classify_once_write`'s route-2 loop should skip the FD requirement entirely for a candidate that is already a `unique_key` member, given route 1 (bare reference) already makes exactly this argument without any FD.
  - `docs/specs/incremental_shapes.md`'s Known Divergences bullet under "The key grain" was rewritten to name this gap precisely, superseding the phase's original "delete/narrow" instruction (deleting it would have been dishonest — the gap is real, just narrower and more specific than originally described).
  - The `incremental_shapes.md` bullet was **not** deleted, contrary to the plan's phrasing — see above.

**Gates:**
- `cargo test -p smelt-logical --lib not_null::` / `cumulative::` — pass
- `cargo test -p smelt-db --test maintenance_fold_spec_companion` — pass
- `cargo test -p smelt-runtime --test statement_parity` — pass
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace test, example_diagnostics)
