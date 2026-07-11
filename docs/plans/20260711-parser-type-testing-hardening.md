# Plan: Parser & Type-Inference Testing Hardening

**Date**: 2026-07-11
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) (§Constraints & Invariants #13 "SQL dialect conformance testing", §Fail-loud discipline) and [`docs/specs/diagnostics.md`](../specs/diagnostics.md) (§Fail-loud invariants #3, `TrailingTopLevelContent`)
**Spec diff**: uncommitted working tree (2026-07-11) — adds fail-loud invariant #3 + `TrailingTopLevelContent` to `diagnostics.md`; adds invariant #13 and a Known Divergences entry to `architecture.md`
**Tracking PR / branch**: PR [#154](https://github.com/adbrowne/smelt-sql/pull/154), branch `parser-type-testing-hardening`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/architecture.md` §"Fail-loud discipline" + §Constraints #13, and `docs/specs/diagnostics.md` §"Fail-loud invariants" — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `parser-type-testing-hardening`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.
- Phase 1's trailing-token error fires on any model in `examples/` — that means real workspaces depend on the silent-absorption behavior, and the escape hatch needs a user decision.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Salsa purity, diagnostic range encoding, fail-loud gates).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to specs and `docs-site/docs/...` describe behavior as if it has always existed; gaps go in **Known Divergences** in behavioral terms.
- DuckDB oracle tests need `DUCKDB_LIB_DIR` and `LD_LIBRARY_PATH` set (see root `CLAUDE.md` §Build and Test).

---

## Context

A 2026-07-11 review found that the parser silently absorbs all top-level tokens after the first model body (`crates/smelt-parser/src/parser/smelt_ext.rs`, `parse_file` tail) — so unsupported syntax (`GLOB`, `MAP {…}`, `E'…'`) degrades into *silently wrong trees* instead of diagnostics, violating the fail-loud discipline. The same review found the type-inference property oracle structurally unable to fail on integer-width errors, decimal precision/scale errors, or any expression where inference returns `Unknown`, and found its generators never produce temporal/decimal arithmetic — the hairiest inference paths. The spec diff (architecture.md #13, diagnostics.md invariant #3) makes fail-loud parsing and two-directional dialect conformance normative; this plan implements it. The review's failing cases are enumerated in architecture.md → Known Divergences → "Dialect conformance gates".

## Scope

### In scope (spec coverage)
- diagnostics.md §Fail-loud invariants #3: trailing top-level content errors (`TrailingTopLevelContent`), lexer never splits malformed literals silently.
- architecture.md §Constraints #13 accept direction: DuckDB-executes ⇒ smelt-parses-or-registered-gap, with ratchet.
- architecture.md §Constraints #13 fidelity direction: smelt-parses-clean ⇒ printed SQL accepted/evaluated by DuckDB.
- architecture.md §Constraints #13 corpus grounding: vendored DuckDB sqllogictest + PostgreSQL regression SELECT corpus with failure ledger.
- architecture.md §Constraints #13 oracle strictness: exact integer-width/decimal(p,s) comparison, known-unknowns ledger, generator coverage of temporal/decimal arithmetic and the untested function list.
- Highest-value grammar gaps whose absence would swamp the new gates: `TRY_CAST`, `GROUP BY ALL`, `ORDER BY ALL`, `IGNORE/RESPECT NULLS`.
- Function-registry consolidation: one authoritative home (`BuiltinRegistry`) for built-in function names/signatures/types, replacing the three overlapping lists (registry, `REGISTRY_MIGRATED` + legacy match, `SqlFunction` enum), with a consistency gate and a shrink-only migration ratchet (added 2026-07-11 after a survey of parser-vs-registry function handling).

### Explicitly deferred
- Grammar support for the remaining registered gaps (dollar-quoted strings, list comprehensions, `MAP` literals, `GLOB`, SQL-standard function forms `trim(BOTH…)`/`substring(FROM FOR)`/`position(IN)`/`overlay`, `LIKE ANY`, hex/underscore numeric literals *as accepted syntax*). The Phase 3/4 ledgers make these visible; fixing them is follow-on work sized by ledger counts, not this plan.
- Spark-side differential parsing beyond the existing sqlparser-rs checks (needs the gated Spark server; same harness applies later).
- Nullability-oracle generator extension (same shape as Phase 5 but against the value oracle; do after the type-side pattern settles).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 1e09f6c0 | 2026-07-11 |
| 2     | done     | 3da7e023 | 2026-07-11 |
| 3     | done     | 2d8a80ce | 2026-07-11 |
| 4     | done     | df0cb16b | 2026-07-11 |
| 5     | done     |        | 2026-07-11 |
| 6     | pending  |        |      |
| 7     | pending  |        |      |
| 8     | pending  |        |      |

---

### Phase 1: Fail-loud trailing top-level content

**Goal.** `parse_file` never silently absorbs tokens: leftover top-level content after the model body produces a parse error and an `ERROR` node, surfaced as `TrailingTopLevelContent` in smelt-db diagnostics.

**Pre-conditions.** None (first phase). Note the existing comment in `parse_file` claiming legacy callers rely on silent absorption ("comma-separated FROM lists") — the TDD suite must characterize what actually reaches this path before removing it.

**TDD tests to write first.**
- `crates/smelt-parser/src/parser/tests.rs::trailing_tokens_after_select_error` — `SELECT a FROM t GARBAGE MORE GARBAGE` yields ≥1 parse error and the junk tokens sit inside an `ERROR` node (not loose under `FILE`).
- `crates/smelt-parser/src/parser/tests.rs::second_top_level_select_errors` — `SELECT 1 SELECT 2` errors; `SELECT a FROM t 'junk' )))` errors.
- `crates/smelt-parser/src/parser/tests.rs::trailing_content_after_smelt_define_errors` — declaration followed by junk errors.
- `crates/smelt-parser/src/parser/tests.rs::clean_file_has_no_trailing_error` — a multi-section file from `examples/timeseries` parses with zero errors (guard against over-firing).
- `crates/smelt-db/tests/` (existing diagnostics test file)::`trailing_content_surfaces_as_diagnostic` — a fixture added under `examples/broken/models/` with trailing junk produces a `TrailingTopLevelContent` diagnostic with a range covering the junk.
- Round-trip property tests (`crates/smelt-parser/tests/proptest_round_trip.rs`) stay green — lossless CST must still hold (junk is *in* the tree, wrapped, not dropped).

**Implementation shape.** In `parse_file` (`smelt_ext.rs`): replace the `seen_model → break` arms and the silent tail-consumption loop with `self.error("unexpected content after model body"); start ERROR node; consume to EOF (or next top-level sync point); finish`. Add `TrailingTopLevelContent` to the `DiagnosticCode` enum and map parse errors of this class in `smelt-db`'s parse-diagnostics query (keep range encoding as `TextRange`, converted once at the boundary per the invariant). If any `examples/` model trips the new error, stop and surface it (see execution prompt).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/parser/smelt_ext.rs` — `parse_file` tail
- `crates/smelt-parser/src/parser/tests.rs` — TDD tests
- `crates/smelt-db/src/` (diagnostics query + `DiagnosticCode`) — new code mapping
- `crates/smelt-db/tests/` — diagnostic surface test
- `examples/broken/models/` — new fixture
- `docs/specs/diagnostics.md` — already edited (spec diff); adjust only if review finds wording drift

**Docs touched.**
- `docs/specs/diagnostics.md` — catalogue row already in spec diff; confirm it matches the shipped behavior.
- `docs-site/docs/reference/language.md` — one paragraph under §SELECT statement: a model file contains at most one query body; anything after it is an error.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] diagnostics.md fail-loud invariant #3 satisfied: zero-error parse ⇒ every token accounted for
- [ ] Lossless CST preserved; ranges stay `TextRange` until the boundary
- [ ] No scope creep into lexer changes (Phase 2)
- [ ] Spec + docs-site edits are timeless

**Commit.** `fix(parser): error on trailing top-level content instead of silently absorbing it`

---

### Phase 2: Lexer fail-loud for malformed literals

**Goal.** The lexer never splits a literal-like token into number-plus-identifier or identifier-plus-string silently: `0x1F`, `1_000_000`, `E'…'`, `B'0101'` each either lex as one correct token or produce an error token that surfaces as a parse error.

**Pre-conditions.** Phase 1 (trailing-content errors) — several mis-lex cases only become *visible* failures once trailing junk errors.

**TDD tests to write first.**
- `crates/smelt-parser/src/parser/tests.rs::number_followed_by_ident_without_space_errors` — `SELECT 0x1F` and `SELECT 1_000_000` produce a parse error (not `0 AS x1F` / `1 AS _000_000`). Oracle for the *decision* (error vs. accept-as-literal): DuckDB accepts both forms as numeric literals; this phase only has to stop the silent mis-parse — accepting them as literals is the deferred grammar work unless trivially cheap in the lexer.
- `crates/smelt-parser/src/parser/tests.rs::number_then_space_then_ident_is_alias` — `SELECT 1 x` still parses as aliased literal (guard).
- `crates/smelt-parser/src/parser/tests.rs::prefixed_string_literals_not_split` — `SELECT E'\n'`, `SELECT B'0101'` do not silently become identifier + orphan string (error is acceptable; correct single-token lexing is better).
- Fuzz/proptest round-trip suites stay green.

**Implementation shape.** In `consume_number` (`lexer.rs`): after the numeric body, if the next char is an identifier continuation (letter/`_`), either extend into a recognized literal form (hex `0x…`, digit separators) or emit an `ERROR` token spanning the whole blob. For `E'`/`B'` prefixes, handle at the identifier/string boundary in the lexer. Whichever branch (accept vs. error) is chosen per form, it must match a written test asserting DuckDB's behavior for that form.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/lexer.rs` — number/string boundary handling
- `crates/smelt-parser/src/parser/tests.rs` — TDD tests

**Docs touched.**
- `docs-site/docs/reference/language.md` — §Type casting / literals note: which numeric-literal forms are accepted.

**Review checklist:**
- [ ] TDD tests exist; each lexer decision is anchored to a DuckDB-behavior assertion
- [ ] No silent number/identifier splits remain (grep-level scan of `consume_number` exits)
- [ ] No scope creep into full hex/underscore literal *support* unless the lexer change is where it naturally lands
- [ ] Docs timeless

**Commit.** `fix(lexer): never silently split malformed numeric/prefixed-string literals`

---

### Phase 3: DuckDB differential harness (both directions)

**Goal.** `smelt-parser-compat` gains a real DuckDB oracle and enforces both §13 directions: a seeded corpus of DuckDB-valid statements must parse or be registered gaps; anything smelt parses cleanly must round-trip to SQL DuckDB accepts.

**Pre-conditions.** Phases 1–2 (otherwise the fidelity direction drowns in known silent-mis-parse noise).

**TDD tests to write first.**
- `crates/smelt-parser-compat/tests/duckdb_differential.rs::duckdb_valid_seed_corpus_parses_or_registered` — a checked-in seed corpus (`tests/corpus/duckdb_seed.sql`, one statement per line, starting with every failing case from the 2026-07-11 review: `TRY_CAST`, `GROUP BY ALL`, `ORDER BY ALL`, `IGNORE NULLS`, `trim(BOTH…)`, `substring(FROM FOR)`, `position(IN)`, `overlay`, `LIKE ANY`, dollar-quoted, list comprehension, `MAP` literal, `GLOB`, `0x1F`, `1_000_000`, `E'…'`, `B'…'`, `INTERVAL 3 MONTH`): each statement is executed against in-memory DuckDB (with a canned schema prelude); if DuckDB accepts and smelt errors, the statement must match a `gaps.rs` entry, else fail.
- `::parsed_clean_sql_round_trips_on_duckdb` — for every seed statement smelt parses with zero errors, print the CST back to SQL and `PREPARE`/execute it on DuckDB; a DuckDB rejection fails the test unless registered. This is the gate that would have caught `SELECT a GLOB 'x*' FROM t` → `SELECT a AS GLOB`.
- `::gap_count_ratchet` — total `KNOWN_GAPS`-matched seed statements is asserted `<=` a checked-in baseline count (`.claude/parser-gaps-baseline.txt`); raising it requires editing the baseline file (reviewer-visible).
- Property variant: `proptest` wrapper feeding the existing `generators.rs` output through the fidelity direction (any generated statement that smelt parses cleanly must execute on DuckDB).

**Implementation shape.** Add `duckdb` as a dev-dependency of `smelt-parser-compat` (system-lib mode, same as smelt-db tests). New module `src/duckdb_oracle.rs`: `fn duckdb_accepts(sql: &str, schema_prelude: &str) -> Result<(), String>` using `PREPARE`, plus an execute variant. Extend `gaps.rs` with entries for every seed statement smelt cannot yet parse (dialect: `duckdb`, category: `duckdb_fails_to_parse` / `roundtrip_mismatch`). Baseline ratchet file mirrors `.claude/hardening-baseline.txt` mechanics.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser-compat/Cargo.toml`, `src/duckdb_oracle.rs`, `src/gaps.rs`, `src/lib.rs`
- `crates/smelt-parser-compat/tests/duckdb_differential.rs`, `tests/corpus/duckdb_seed.sql`
- `.claude/parser-gaps-baseline.txt`
- `crates/smelt-parser-compat/CLAUDE.md` — how-to-run note

**Docs touched.**
- `docs/specs/architecture.md` — Known Divergences entry updated: accept-direction + fidelity gates now exist; remaining aspirational items narrowed to corpus + oracle strictness.

**Review checklist:**
- [ ] Both directions enforced; fidelity test actually executes printed SQL (not just re-parses)
- [ ] Every seed failure is either fixed or a registry entry — no skips
- [ ] Ratchet baseline mechanics match the hardening-budget pattern
- [ ] CI wiring: tests run in the default `cargo test` invocation with system DuckDB
- [ ] Spec Known Divergences updated, timeless

**Commit.** `test(parser-compat): DuckDB differential oracle — accept + round-trip fidelity gates with ratcheted gap registry`

---

### Phase 4: External corpus ingestion

**Goal.** A vendored corpus of SELECT statements extracted from DuckDB's sqllogictest suite and PostgreSQL's regression suite runs through the Phase 3 harness with a checked-in failure ledger.

**Pre-conditions.** Phase 3 harness.

**TDD tests to write first.**
- `crates/smelt-parser-compat/tests/external_corpus.rs::corpus_statements_parse_or_ledgered` — every statement in `tests/corpus/external/*.sql` either parses cleanly (and round-trips per Phase 3 fidelity) or appears in `tests/corpus/external_ledger.toml` with a category; unledgered failures fail the test.
- `::ledger_has_no_stale_entries` — a ledger entry whose statement now passes fails the test (forces ledger shrinkage as gaps close).
- Unit test for the extraction script's filter: only `SELECT`/`WITH`/`VALUES` statements, no DDL/DML, dedup, size cap (~1–2k statements to keep test time bounded).

**Implementation shape.** One-time extraction script `scripts/extract-sql-corpus.py` (documented, re-runnable): pulls from a pinned DuckDB repo tag (`test/sql/**/*.test` sqllogictest `query`/`statement ok` blocks) and PostgreSQL tag (`src/test/regress/sql/*.sql`), filters to the SELECT-only subset, normalizes, writes `tests/corpus/external/{duckdb,postgres}.sql` plus attribution/license notes (`tests/corpus/external/README.md` — both suites are permissively licensed; keep the notices). The vendored files are committed; the script is not run in CI. Ledger is TOML keyed by statement hash with `category` + free-text note.

**Critical files (allowed to touch in this phase).**
- `scripts/extract-sql-corpus.py`
- `crates/smelt-parser-compat/tests/external_corpus.rs`, `tests/corpus/external/` (vendored corpus, ledger, README)

**Docs touched.**
- `docs/specs/architecture.md` — Known Divergences: corpus grounding now implemented; narrow the entry to oracle strictness.
- `crates/smelt-parser-compat/CLAUDE.md` — corpus refresh instructions.

**Review checklist:**
- [ ] License/attribution notes present for vendored statements
- [ ] Corpus size keeps `cargo test -p smelt-parser-compat` under ~60s
- [ ] Ledger is hash-keyed and stale-entry-checked (shrink-only pressure)
- [ ] No scope creep into fixing ledgered gaps

**Commit.** `test(parser-compat): vendored DuckDB/PostgreSQL SELECT corpus with failure ledger`

---

### Phase 5: Type-oracle generator extension

**Goal.** The type-inference property generators exercise the inference paths they currently never reach: temporal/interval arithmetic, Decimal binary ops, `CAST AS DECIMAL(p,s)/FLOAT`, `EXTRACT(EPOCH)`, mixed naive/tz-aware timestamps, and the supported-but-ungenerated function list.

**Pre-conditions.** None (independent of parser phases; keep after Phase 3 in execution order so proptest failures land on a hardened parser).

**TDD tests to write first.**
- `crates/smelt-db/tests/prop_helpers/` unit tests: reachability assertions that over N=500 generated cases the corpus contains ≥1 each of: interval±timestamp, date−date, Decimal `+`/`*`/`/`, `CAST(… AS DECIMAL(12,3))`, `EXTRACT(EPOCH …)`, a mixed-tz comparison, and ≥10 of the newly added functions (`MEDIAN`, `PERCENTILE_CONT`, `ARRAY_AGG`, `AGE`, `JSON_EXTRACT`, `IFNULL`, `INITCAP`, `TO_CHAR`, `TRANSLATE`, `POSITION`, …). Statistical smoke, not proof — guards against silent generator regression.
- `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` green, with every newly surfaced real divergence either fixed in `type_inference/` or added to `divergences.rs` as an explicit entry (per the repo rule: add a named regression test before fixing any inference bug found).
- Same for `nullability_property_tests` at default cases (the generators are shared where applicable).

**Implementation shape.** `generators.rs`: widen `BinaryOp` operand selection to include Decimal and temporal columns with op-appropriate pairing (the expected-type computation mirrors `binary.rs` §15 decimal formulas and the temporal table); add Cast targets `DECIMAL(p,s)`, `FLOAT`; extend `core_functions` with the missing entries and their expected-type functions; add a tz-mixing weight to the column pool. Expect this phase to *find bugs* — budget review time for divergence triage rather than assuming green.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/prop_helpers/generators.rs`, `divergences.rs`
- `crates/smelt-db/src/type_inference/` — only for bugs the new coverage exposes, each with a named regression test first
- `crates/smelt-db/tests/type_property_tests.rs` — reachability tests

**Docs touched.**
- `docs/specs/types.md` — only if a triaged divergence changes specified semantics (then via the spec, first).

**Review checklist:**
- [ ] Reachability smoke tests exist and pass
- [ ] Every new divergence entry has status (`KnownBug`/`ByDesign`/`BackendSpecific`) and a comment citing the DuckDB behavior
- [ ] Inference fixes each have a prior failing named regression test
- [ ] Generator expected-type logic doesn't duplicate production inference (it may mirror the *spec* formulas; cite §15)

**Commit.** `test(smelt-db): extend type-oracle generators — temporal/decimal arithmetic, decimal casts, missing functions, mixed-tz`

---

### Phase 6: Type-oracle strictness

**Goal.** Close the comparison loopholes: exact integer-width and Decimal(p,s) comparison by default, and a known-unknowns ledger replacing the silent `Unknown`-skip.

**Pre-conditions.** Phase 5 (strictness over the widened generator, otherwise it certifies too little).

**TDD tests to write first.**
- `crates/smelt-db/tests/prop_helpers/` unit tests: `compare_types(Integer, BigInt)` is `Mismatch`; `compare_types(Decimal(10,2), Decimal(38,2))` is `Mismatch`; string-family leniency retained (Text/Varchar stays `Compatible` — registered `ByDesign`).
- `type_property_tests`: a column smelt infers as `Unknown(Dynamic)` fails the property unless the generating expression's shape matches an entry in a new `known_unknowns.rs` registry (same shape as `divergences.rs`); an entry that never fires over the run is reported (staleness pressure, warn-level).
- Full run: `PROPTEST_CASES=2000` green after triage, with every width/precision divergence surfaced by strictness either fixed (e.g. the registered `round_integer` KnownBug, numeric-literal width choices) or a named `divergences.rs` entry.
- `crates/smelt-db/tests/proptests/type_conformance_tests.rs` still asserts zero divergence post-cast-wrapping (its stricter regime must not regress).

**Implementation shape.** `type_comparison.rs`: remove `is_integer_width_compat` and the blanket decimal rule; route tolerated differences through `divergences.rs` entries so each is named. `type_property_tests.rs`: replace the `Unknown` skip with a lookup into `known_unknowns.rs` (expression-kind + function-name keyed). Expect triage: SmallInt-vs-Integer literal width and SUM/decimal precision entries are the likely first additions.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/prop_helpers/type_comparison.rs`, `divergences.rs`, new `known_unknowns.rs`
- `crates/smelt-db/tests/type_property_tests.rs`
- `crates/smelt-db/src/type_inference/` — bug fixes only, named regression test first

**Docs touched.**
- `docs/specs/architecture.md` — Known Divergences: oracle-strictness item resolved; §13 entry now fully implemented except items explicitly listed.
- `CLAUDE.md` (root) — add the new conformance gates to the fail-loud CI-gate list (per the standing rule that architectural invariants land in specs/CLAUDE.md).

**Review checklist:**
- [ ] No blanket compatibility rules remain except named `ByDesign` divergences
- [ ] Unknown-skip is gone; ledger has staleness pressure
- [ ] CLAUDE.md gate list updated
- [ ] Spec Known Divergences updated, timeless

**Commit.** `test(smelt-db): strict type-oracle comparison — exact widths, decimal p/s, known-unknowns ledger`

---

### Phase 7: High-value grammar gaps — TRY_CAST, GROUP BY ALL, ORDER BY ALL, IGNORE/RESPECT NULLS

**Goal.** Implement the four DuckDB idioms common enough to dominate the Phase 3/4 ledgers, with type inference for `TRY_CAST` (target type, always nullable), shrinking the gap registry and ledgers accordingly.

**Pre-conditions.** Phases 3–4 (the ledgers provide the red tests and the shrink evidence); Phase 6 not required.

**TDD tests to write first.**
- `crates/smelt-parser/src/parser/tests.rs::try_cast_parses` — `TRY_CAST(a AS INTEGER)` produces a cast-shaped node; `::group_by_all_parses`, `::order_by_all_parses` (incl. `ORDER BY ALL DESC`), `::ignore_nulls_in_window_parses` (`last_value(a IGNORE NULLS) OVER (…)`, and `RESPECT NULLS`).
- `crates/smelt-db/src/type_inference/tests.rs::try_cast_infers_nullable_target` — `TRY_CAST('x' AS INTEGER)` → Integer, nullable, even over a non-nullable input.
- DuckDB-oracle property/regression: seed statements for these four forms move from `gaps.rs`/ledger to passing; ratchet baseline and ledger counts go **down** (assert the new counts).
- Real fixture: a model in `examples/timeseries` (or `retail_analytics`) adopts `GROUP BY ALL` and `TRY_CAST`; `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` stay green.

**Implementation shape.** Parser: `TRY_CAST` alongside `CAST` in `expr.rs` (shared body, distinct node or flag); `ALL` alternative in `parse_group_by_clause`/`parse_order_by_clause` (`select.rs`); `IGNORE|RESPECT NULLS` as contextual keywords inside function-call args before `OVER` (`expr.rs`). AST wrappers in `ast.rs`. Inference: `TRY_CAST` reuses the CAST path with forced nullability (`literal.rs`); `GROUP BY ALL` expands in schema inference to the non-aggregate select items (mirror DuckDB semantics; spec home is `types.md`/`models.md` if semantics text is needed).
Update printer round-trip for all four forms.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/{lexer.rs, syntax_kind.rs, ast.rs, printer.rs}`, `src/parser/{expr.rs, select.rs, tests.rs}`
- `crates/smelt-db/src/type_inference/` (+ its `tests.rs`), schema-inference site for `GROUP BY ALL`
- `crates/smelt-parser-compat/src/gaps.rs`, `.claude/parser-gaps-baseline.txt`, corpus ledger
- `examples/` — fixture adoption

**Docs touched.**
- `docs-site/docs/reference/language.md` — §GROUP BY extensions (`GROUP BY ALL`), §Type casting (`TRY_CAST`), §Window functions (`IGNORE NULLS`), §SELECT statement (`ORDER BY ALL`) — written as always-existing features.
- `docs/specs/architecture.md` — Known Divergences list shrinks by these four constructs.

**Review checklist:**
- [ ] All four forms parse, print, round-trip, and execute on DuckDB via the Phase 3 fidelity gate
- [ ] `TRY_CAST` nullability verified against the DuckDB value oracle, not just asserted
- [ ] Ratchet/ledger counts decreased and asserted
- [ ] docs-site sections timeless

**Commit.** `feat(parser): TRY_CAST, GROUP BY ALL, ORDER BY ALL, IGNORE/RESPECT NULLS — parse, print, infer`

---

### Phase 8: Function-registry consolidation — one authoritative home per function name

**Goal.** A built-in SQL function's name, signature, and inferred type live in exactly one place: `BuiltinRegistry` (`crates/smelt-types/src/signatures.rs`). The three overlapping name lists that exist today — the registry, the `REGISTRY_MIGRATED` allowlist + hand-written legacy `match` in `crates/smelt-db/src/type_inference/function_call.rs`, and the `SqlFunction` enum in `crates/smelt-types/src/functions.rs` used for the "is this recognized" check — collapse so a function added to the registry is automatically recognized, typed, and classified (aggregate/window/scalar), and a function absent from the registry is *diagnosed*, never half-known. This is the structural fix for the failure mode where a name added to one list but not the others degrades silently.

**Pre-conditions.** Phases 5–6 (the widened generators and strict comparison are the behavior-preservation net for migrating inference paths; do not attempt this migration against the lenient oracle). Phase 7 not required, but if done first its new functions must land registry-only.

**TDD tests to write first.**
- `crates/smelt-types/` (or `smelt-db/tests/`) consistency gate `every_recognized_function_is_registry_backed` — every name `SqlFunction::from_name` recognizes resolves in `BuiltinRegistry`, and every scalar/aggregate/window registry entry is recognized; a name in one list but not the other fails with the missing side named. Written red against today's known drift (enumerate the current mismatches in the failure message before fixing).
- `legacy_match_ratchet` — the count of function names typed by the legacy hand-written `match` in `function_call.rs` (i.e., *not* through `try_registry_inference`) is asserted against a checked-in baseline that only ratchets **down** (`.claude/registry-migration-baseline.txt`, hardening-budget mechanics); this phase drives it to zero or to a named exception list, whichever triage supports.
- Behavior preservation: `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` and default-cases `nullability_property_tests` green after each migration batch — any type change surfaced by migration is either a bug fix (named regression test first, per repo rule) or a `divergences.rs` entry; silent behavior changes are not acceptable.
- `unrecognized_function_still_warns` — an unknown name still produces `DiagnosticCode::UnrecognizedFunction` and `Unknown(Dynamic)` typed through the Phase 6 known-unknowns ledger path (the consolidation must not accidentally make unknown names panic or pass silently).
- Real fixture: `examples/` workspaces stay diagnostic-clean (`example_diagnostics`, `example_workspaces`).

**Implementation shape.** Migrate the legacy `match` arms in `function_call.rs` into `BuiltinRegistry` signatures batch by batch (extend `Signature`/`unify_call` where an arm encodes semantics the current signature language can't express — e.g. argument-dependent return types — rather than keeping the arm; if a genuinely inexpressible case remains, it becomes a named entry in an explicit exception list with a doc comment, not an anonymous match arm). Delete the `REGISTRY_MIGRATED` allowlist once the registry-first path is the only path. Replace `SqlFunction::from_name`-based recognition with a registry lookup (keep the enum only if something else consumes it — then derive it or gate it with the consistency test). `dispatch.rs`'s ExprKind seeding already uses the registry; verify unknown-name default (`Scalar`) still routes to the warning path. Expect the migration to surface latent inference differences — budget triage time.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs`, `crates/smelt-types/src/functions.rs`
- `crates/smelt-db/src/type_inference/function_call.rs`, `dispatch.rs`, `check_types.rs` (recognition path only)
- `crates/smelt-db/tests/` (consistency gate, ratchet test), `.claude/registry-migration-baseline.txt`
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — migration-surfaced entries
- `docs/specs/types.md` — only if a triaged difference changes specified semantics (spec first)

**Docs touched.**
- `docs/specs/architecture.md` — Constraints & Invariants: new invariant "Function-registry single ownership" (one authoritative home per built-in function name; recognition, classification, and typing all derive from `BuiltinRegistry`), written timelessly; Known Divergences entry for any remaining exception-list names.
- `CLAUDE.md` (root) — add the consistency gate + migration ratchet to the fail-loud CI-gate list.

**Review checklist** (material findings only):
- [ ] Consistency gate exists, was red against real drift first, now green
- [ ] Legacy match is gone or every survivor is a named, doc-commented exception; ratchet baseline is 0 or matches the exception list exactly
- [ ] No silent type changes: every migration-surfaced difference has a named regression test or divergence entry
- [ ] Unknown-name path still warns + `Unknown(Dynamic)` through the known-unknowns ledger
- [ ] New invariant landed in architecture.md + CLAUDE.md gate list, timeless wording

**Commit.** `refactor(types): consolidate function name/signature/type into BuiltinRegistry — consistency gate + migration ratchet`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh` (includes example_diagnostics)
- `cargo test -p smelt-parser-compat` — differential + corpus suites green; `.claude/parser-gaps-baseline.txt` and the external ledger reflect only registered gaps
- `PROPTEST_CASES=2000 cargo test -p smelt-db --test type_property_tests --test nullability_property_tests` green under strict comparison
- `cargo test -p smelt-lsp --test example_workspaces` green
- Spot check: `SELECT a FROM t GARBAGE`, `SELECT a GLOB 'x'`, `SELECT 0x1F` all produce diagnostics in the LSP against `examples/test_workspace`
- Registry consolidation gates green: the function-name consistency test passes and `.claude/registry-migration-baseline.txt` is 0 (or exactly matches the named exception list)
- `/smelt:validate architecture` and `/smelt:validate diagnostics` report zero drift for the touched sections
