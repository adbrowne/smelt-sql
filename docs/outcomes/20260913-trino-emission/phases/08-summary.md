# Phase 8 summary — Seams widened to Trino

**Shipped:**
- `crates/smelt-runtime/tests/dialect_seam/fixtures.rs` — `trino_target()`, registered in `registry()`.
- `crates/smelt-runtime/tests/dialect_seam/refusals.rs` — five new tests: `one_argument_log_is_refused_for_trino`, `two_argument_log_compiles_for_trino`, `unpivot_is_refused_for_trino_on_the_compile_path`, `a_refused_construct_inside_a_function_body_is_refused_for_trino` (LOG arity-1 and UNPIVOT, each inside a `smelt.define` body), `floor_divide_compiles_for_trino`.
- `crates/smelt-runtime/src/compile.rs` — `trino_compile_refuses_unpivot` deleted; superseded by the public seam test above (plan's "move", not duplicate).
- `crates/smelt-dialect/tests/emission_ownership.rs` — `the_printer_branches_on_no_dialect_variant` rewritten to parse `enum SqlDialect` out of `src/dialect.rs` (mirrors `declared_rewrite_ids`/`declared_restructure_ids`) instead of a hardcoded three-name list; asserts the parsed list contains `Trino` so an empty/stale parse can't make the gate vacuous.
- `crates/smelt-runtime/tests/projection_dialect_invariance.rs` — `trino_target()` added and registered; both standing tests (`output_columns_and_cast_wrap_names_are_byte_identical_across_backends`, `decorrelated_model_output_columns_are_identical`) widened to four backends; the `window_to_cte` case additionally asserts no `__smelt_` synthesis appears in Trino's compiled SQL.
- `docs/specs/multi_backend.md` — one sentence in §"Output-schema type conformance" naming the widened gate and its four-dialect set; one sentence in §"Refusal covers function bodies" naming Trino's two concrete instances (`LOG` arity-1, `UNPIVOT`).
- `CLAUDE.md` §"Source-derived projection" — "DuckDB, Spark and BigQuery … across all three" → four dialects including Trino.

**Decisions:**
- No production code changed. Every new/widened test passed on the first run: the LOG conditional, the `UNPIVOT` clause refusal, the `//` template, and `emission_ownership`'s printer-purity checks were all already correct from phases 3/4/7 — this phase only proves it mechanically instead of by construction alone.
- `EVERY_LOWERED_CONSTRUCT_SQL` (which includes `MEDIAN`, a registered live-audit `Gap` for Trino per `ledger.rs`) compiles cleanly for Trino: a `Gap` ledger row is an audit-time finding about live correctness, not a compile-time `Emission::Unsupported` verdict, so it does not block this offline seam. No finding to settle as a registry verdict — flagged per the plan's instruction, but nothing to record beyond this note.
- `window_to_cte`'s Trino SQL carries no `__smelt_` synthesis, confirming phase 7's measured negative result (Trino accepts the ordered-set aggregate as a window function directly, unlike DuckDB/Spark) now holds through the real compile path, not just the isolated planner unit tests.

**For the next planner:**
- Phase 9 (type-oracle question) and phase 10 (close: doc regen) are next; nothing from this phase needs to feed either beyond what's already in the phase table.
- No out-of-scope discoveries.

**Gates:**
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --quiet` — pass (25 + 4 tests).
- `cargo test -p smelt-dialect --test emission_ownership --quiet` — pass (11 tests).
- `cargo test -p smelt-types --test registry_coverage --quiet` — pass (110 tests), untouched.
- `cargo test -p smelt-runtime --lib compile::` — pass (42 tests), confirms the moved-not-duplicated UNPIVOT assertion.
- `bash .claude/scripts/verify-phase.sh` (Trino tier unexported) — ALL GREEN (fmt, clippy both feature sets, shellcheck, full workspace `cargo test`, `example_diagnostics`).
- No ratchet files touched: `dialect-gaps-baseline.txt`, `registry-migration-baseline.txt`, `parser-gaps-baseline.txt`, `hardening-baseline.txt` all unchanged (`git status --short` confirms).
