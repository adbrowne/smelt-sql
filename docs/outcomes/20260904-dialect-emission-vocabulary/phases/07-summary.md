# Phase 7 summary — Spark's conditional and template verdicts

**Shipped:**
- `LOG` on Spark: `Emission::Conditional` (`arity=1 -> Rename("LOG10")`, `otherwise -> Native`);
  `LOG`'s own signature widened from one fixed `Double` param to `variadic(Double)` so a
  two-argument call is admitted at all (`crates/smelt-types/src/signatures.rs`).
- `DAYOFWEEK` on Spark: `Emission::Template("DAYOFWEEK({0}) - 1")`; its signature moved from
  `any_args()` (rejected — templates can't name a variadic tail) to a fixed one-arg form.
- `TRUNC`: `Emission::Conditional` on **both** SparkSql and DuckDb — `(Temporal, String)` arity-2
  -> `Native`/`Unsupported` respectively (DuckDB's `TRUNC` turned out to have no temporal form at
  any arity either, discovered live; without a DuckDB-side row the audit's own DuckDB-as-reference
  run had nothing to settle the Spark arm's probe against).
- `TO_JSON` on Spark: `Emission::Conditional` (`Composite -> Native`, `otherwise -> Unsupported`).
- `//` on Spark: `Emission::Unsupported` wholesale replaced with `Emission::Conditional`
  (`Integral,Integral -> "{0} DIV {1}"`; `Floating,Floating`/`Decimal,Decimal -> "{0} / {1}"`;
  `otherwise -> Unsupported`, same reason text).
- Four ledger rows deleted (`LOG`, `DAYOFWEEK`, `TRUNC`, `TO_JSON` on Spark);
  `dialect_gaps_spark` 27 → 23, dated sign-off entry in `.claude/dialect-gaps-baseline.txt`.
- Coverage doc regenerated; spec delta landed (`docs/specs/multi_backend.md` §Known Divergences
  collapsed to the one open BigQuery `%` gap).
- New tests: `registry_coverage::log_settles_by_arity_on_spark`,
  `::intdiv_settles_per_operand_class_on_spark`; `operand_conditional::dayofweek_prints_...`,
  `::trunc_and_to_json_settle_by_first_argument_class`;
  `dialect_seam::intdiv_over_typed_integer_columns_compiles_on_spark`;
  `dialect_audit::every_conditional_arm_is_covered_by_a_probe` now exercises real data (was
  green-but-vacuous).

**Decisions:**
- LOG's `otherwise` arm (not a third explicit `arity: Some(2)` arm) covers the two-argument form —
  simpler, and the harness's arity-candidate search (below) can reach it directly.
- `TRUNC`'s Spark arm is arity-scoped (`Some(2)`, `(Temporal, String)`) rather than class-only,
  because Spark's `TRUNC` requires exactly two arguments in both its temporal and (refused)
  numeric forms — a single-arg probe isn't valid Spark SQL at all, discovered live.
- Gave DuckDB its own `TRUNC` conditional entry (Temporal+String → `Unsupported`) instead of
  leaving it unconditional `Native`: DuckDB genuinely has no temporal `TRUNC` at any arity
  (measured live), so an audit probe for Spark's temporal arm needs a real DuckDB-side verdict
  to settle against — DuckDB being the reference, "no verdict" was the actual root cause of the
  first `schema_leg_duckdb`/`value_leg_duckdb_is_self_consistent` failures below.

**For the next planner:**
- **Harness bug, fixed in this phase, worth a spec/CLAUDE.md mention**: `probe::print_for`
  (used by every leg to print a probe) built `PrintContext.settled_emissions` as `&[]` — every
  `Conditional` entry's probe printed via the *arity-only* fallback, never the operand-type-aware
  path a real compile does. Invisible before this phase (no production `Conditional` row existed
  to print), it silently broke *every* probe once one did — fixed by threading a `type_of`
  callback built from `fixture::column_types()` into `settle_emissions` inside `print_for`.
- **A second, subtler instance of the same bug**: the *ordinary* (non-arm) probe `probe_or_reason`
  derives for a `Conditional` entry carried `CallFacts::unresolved(arity)` — correct for every
  other entry (facts don't affect a non-`Conditional` verdict), but for `//`'s plain scalar probe
  this made `is_declared_unsupported` (which decides whether to skip execution) disagree with what
  `print_for` actually printed (the real `Floating,Floating` arm, valid SQL) — the engine "accepted"
  a probe the harness expected to refuse. Fixed by deriving facts from the real fixture-column
  classes for bare-column arguments. **Any future entry that goes `Conditional` needs no further
  harness work** — both fixes are structural, not per-entry.
- **Fixture bug, fixed in this phase**: `iv_interval`'s `'<n> day'` string literal
  (`CAST('1 day' AS INTERVAL DAY)`) is invalid on Spark (`INVALID_INTERVAL_FORMAT`) though valid on
  DuckDB/BigQuery — Spark's day-time interval cast wants a bare integer string, so it gets its own
  `INTERVAL '<n>' DAY` literal form instead of the shared `CAST(...)` path. This is why the schema
  leg failed *wholesale* (every probe, not just this phase's rows) the first time Spark was run
  live against phase 6's fixture columns — nobody had actually run `schema_leg_spark`/
  `value_leg_spark` live since phase 6 landed them.
- **Harness enumeration gap, fixed**: `conditional_arm_probes` derived a probe's *position* from
  the emission table's own `(dialect, position)` key, which is routinely `Position::Any` (a lookup
  wildcard `suffix()` explicitly cannot render) — panicked the instant a real `Conditional` entry
  existed. Fixed to derive the concrete position(s) from `sig.kind`, same as an ordinary probe.
  Also widened the arity search for an arity-less arm shadowed at its "natural" default arity
  (`sig.params.len()`) — bounded to `default..=default+2`, deliberately not further, so it can't
  wander into an arity the signature has no real support for.
- **Recommend**: fix the pre-existing `smelt-runtime` python-model-discovery test flakiness
  (temp-file race — confirmed here across three separate `verify-phase.sh` runs, a different
  subset of `python::tests::*` fails each time, 216/216 green with `--test-threads=1`). Not
  touched by this phase; already flagged in phase 6's summary for one test, now confirmed to hit
  at least four different tests in the same module.
- Phase 8 (the bulk `#178` paydown) can lean on all of the above without further harness changes.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics green; `cargo test
  (workspace)` red *only* on the pre-existing python-discovery flake above (confirmed unrelated:
  216/216 green single-threaded, zero touched files in `smelt-runtime/src/python.rs`).
- `SPARK_CONTAINER_ID=<live> cargo test -p smelt-db --test dialect_audit` — 61/61 green, including
  live `schema_leg_spark`/`value_leg_spark`/`gap_count_ratchet` (`dialect_gaps_spark` = 23).
- `cargo test -p smelt-types --test registry_coverage` — 102/102 green.
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional
  --test unsupported_emission --test power_lowering` — all green.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance
  --test restructure_multiplicity` — all green.
- `cargo test -p smelt-db --test integration -- registry_consistency` — 6/6 green.
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended;
  `dialect_gaps_duckdb` (6) and `dialect_gaps_bigquery` (42) unchanged.
