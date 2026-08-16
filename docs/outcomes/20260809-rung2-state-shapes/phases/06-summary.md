# Phase 6 summary — once-write: admit the fallback/multi-candidate spellings

## Shipped

- `classify_once_write` (`crates/smelt-logical/src/rules/cumulative.rs`) now parses the leading
  maximal run of single-bare-column `MAX(...)`/`MIN(...)` reductions as candidates, at most one
  further trailing argument as the fallback, proves each candidate's FD independently (first
  failure names that candidate), and — for a fallback-bearing or multi-candidate spelling — calls
  `decomposed_state::decompose_once_write` and returns `Admitted { state: Some(...) }`. A bare
  single reduction with no fallback and the key-derived route both stay `Admitted { state: None }`
  — unchanged shape, per the outcome's decision log.
- `OnceWriteAdmission::Admitted` gained the `{ state: Option<DecomposedState> }` payload; both
  call sites (`classify_cumulative`'s `GroupByKey` arm, `smelt-db`'s `derive_fold_spec`) updated —
  the former threads `state` into `AggregatorColumn`, the latter only pattern-matches.
  `classify_once_write` gained an `output_name: &str` parameter (the projection's alias) so it can
  name the derived state columns itself.
- `KeyedUnknownCombiner`'s once-write suggestion text and its doc comment no longer claim "no
  fallback argument" is required.
- Spec: rewrote both once-write/rung-2 Known Divergences bullets to the residual gap only
  (nullability route, key-derived arbitrary-expression widening, whole-scope fan-out/set-op
  facts); deleted the "not yet wired" claim. docs-site `cumulative-aggregate.md`: rewrote the
  "no fallback" paragraph to describe the decomposed-state mechanism, added the multi-candidate
  spelling to the admitted list, rewrote the `KeyedOnceWriteUnproven` row.
- Tests: 7 new/flipped tests in `keyed_families.rs` (fallback admits, multi-candidate admits with
  one state pair per candidate, candidates-then-fallback admits, second-candidate-unproven names
  that candidate, unpresentable-fallback refuses, bare-reduction-stays-stateless regression,
  state-column-collision reaches once-write); 2 in `maintenance_fold_spec_companion.rs` (flipped
  `_is_not_admitted` → `_is_admitted`, plus a multi-candidate case); 1 real-SQL-driven fold test
  in `emit_statements.rs` asserting the keyed merge folds `__value`/`__written` and recomputes the
  presented column with the fallback applied fresh.

## Decisions

- Per-candidate FD proof loop returns on the first failure, naming that candidate — matches the
  plan's "every candidate needs its own proof" design decision; structural disproofs
  (`has_fan_out_join`/`has_set_op_barrier`) stay whole-scope, applied identically per candidate
  (the residual gap the spec now names explicitly).
- The refusal `column` for a `decompose_once_write` `Err` (in practice only reachable via an
  unpresentable fallback, since every candidate shape it's called with is already proven) names
  the fallback text when present, falling back to the first candidate's column otherwise — more
  useful than always naming the first candidate for a fallback-purity failure.

## For the next planner

- No decomposed-state-shape bugs surfaced this phase — once-write's state columns never
  cross-reference each other (unlike `MAX_BY`'s `v`/`o`), and `state_augmented_projection` already
  runs pre-cast from phase 5's fix, so both phase-5 traps were re-verified clean rather than hit
  again.
- Row 7 (`AVG`/`STDDEV_*`/`VAR_*`) is the first *additive* state family reachable through this
  mechanism — `WindowedKeyedRule::ledger_grade` reads `cross_partition_combiner` alone today and
  would grade an `AVG` column `Idempotent`, silently dropping the ledger refusal a reprocessed
  window needs. `refuse()`'s monoid allowlist and `smelt-runtime/src/diagnostics.rs`'s `KeyedFold`
  preview folds also ignore `state` today — row 7 must fix all three alongside the admission
  widening, not as a follow-up (already captured in the outcome's decision log).
- Untouched, out of scope for this phase: row 7's `AVG`/variance folding, row 8's new
  decomposed-state conformance recipes with downstream `SELECT *` consumers, row 9's `smelt
  explain` state rendering.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage`
  — 35 + 32 + 4 pass.
- `cargo test -p smelt-db --test maintenance_fold_spec_companion` — 11/11 pass.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — 18 + 27 pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 47/47 pass unchanged (no
  already-admitted spelling changed shape).
