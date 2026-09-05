# Outcome: Dialect emission vocabulary — templates and operand-conditional verdicts

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/specs/multi_backend.md` §"Template emission", §"Operand-conditional verdicts" (spec commit `03828a14`); GitHub issues #173, #174, #177, #178, #179, #181
**Spec anchors:** `docs/specs/multi_backend.md` §"Operator lowering", §"Emission is scoped to call position", §"Template emission", §"Operand-conditional verdicts", §"Cross-engine emission audit", §Constraints ("Template interpretation is generic", "Operand-conditional verdicts are settled on the compile path"); `docs/specs/architecture.md` §Constraints item 14

## The outcome

A built-in whose target spelling is a fixed shape over its own arguments is a `Template` row in
`BuiltinRegistry`, interpreted by one generic printer routine that knows no function names; the
hand-written `ModuloCall`/`PowerCall` rewrites are gone and every surviving `RewriteId` says why a
placeholder could not express it. A verdict that depends on arity or operand type is an ordered list
of arms ending in `otherwise`, settled on the compile path from the projection's own type inference
and handed to a printer that holds no type context. With that vocabulary the DuckDB and Spark gap
ratchets are paid down to the rows that are type-inference bugs rather than missing lowerings, `//`
is lowered per operand class instead of refused wholesale, and the PostgreSQL emission column no
longer publishes unverified claims. Every arm and template is verified against a live engine by the
audit before it is claimed — never from documentation.

## Success criteria (checkable)

1. `Emission::Template` exists; `RewriteId::ModuloCall` and `RewriteId::PowerCall` are removed;
   a pinned test asserts the printed SQL for `%`, `^` and `**` on every dialect is byte-identical
   to the pre-migration output.
2. Registry construction validates templates: unit tests show a placeholder index beyond the
   arity, an unreferenced argument, and a non-call template at a window position each fail to
   build, and a test constructs the full registry successfully.
3. A template call carrying `DISTINCT`, `FILTER (WHERE …)`, `WITHIN GROUP`, an argument-list
   `ORDER BY`, `IGNORE NULLS`, a named (`=>`) argument, or `*` is refused at compile time with
   `UnsupportedOnBackend` naming the modifier (one test per modifier).
4. An `OperandClass` classifier is a total function over `DataType`, single-owned beside the
   registry; conditional entries are resolved on the compile path and the printer receives settled
   verdicts only — `emission_ownership` structurally asserts `printer.rs` references no type
   context and no arm resolution; `dialect_seam` asserts `a // b` with an unresolvable operand is
   refused on Spark and that no compile entry point reaches the printer with a conditional unsettled.
5. `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_duckdb` ≤ 5 and `dialect_gaps_spark` ≤ 4,
   with every remaining row a `type_gap` (tracked by #175/#176), and `dialect_gaps_bigquery` not
   raised. Each closure is a `Rename`, `Template`, conditional arm, or `Unsupported { reason }`
   verified by the audit's schema and value legs on the live engine, never by reading docs.
6. Every `RewriteId` variant's doc comment carries a line stating which call structure a
   placeholder cannot name; `emission_ownership` fails a variant without one.
7. `docs/reference/dialect-coverage.md` regenerated with `template` as a verdict kind and
   conditional cells rendered as the set of their arms; the doc-sync gate is green.
   `docs/specs/architecture.md` §Constraints item 14 and CLAUDE.md's function-registry invariant
   name templates and the compile-path settlement rule.
8. #181 is closed per the Decision log: `PostgreSQL` is removed from `SqlDialect`/`DialectId`,
   `BackendCapabilities::postgresql()`, the coverage table's column, and `dialect_gaps_postgres`
   (baseline edit carrying a sign-off line); the pg_query grammar anchor in `smelt-parser-compat`
   and ROADMAP §"PostgreSQL Backend" are untouched.
9. Standing gates green: `verify-phase.sh`, `emission_ownership`, `dialect_seam`,
   `projection_dialect_invariance`, `restructure_multiplicity`, `dialect_audit` (DuckDB legs
   in-process; Spark legs against `scripts/spark-up.sh`), `registry_consistency`,
   `type_property_tests`.

## Out of scope

- **BigQuery live verification** (#179 in full; the BigQuery arms of #173 and #174). The value leg
  bills and needs the human-run `scripts/bigquery-dialect-audit.sh` behind a passphrase prompt, so a
  headless step cannot run it, and the spec forbids authoring a verdict from documentation. The
  mechanism ships here, proven on DuckDB and Spark; a human-run sweep lands the BigQuery rows as a
  follow-up. Migrating an *existing* BigQuery rewrite to a template is in scope because its
  byte-identical output is pinned offline (criterion 1).
- **#175 (`FIRST`/`LAST` lexed as keywords) and #176 (inference returning `Unknown`)** — parser and
  inference bugs, not lowering gaps; they are the rows criterion 5 leaves standing.
- **#180 (user-facing divergence page)** and **#182 (LSP consumes registry position)** — separate
  outcomes.
- **A PostgreSQL oracle and audit leg** — rejected by the Decision log entry for #181.
- `//` on DuckDB (native) and any change to what smelt SQL accepts.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Retire the PostgreSQL emission dialect (#181): remove the variant, capabilities, coverage column, and baseline entry with sign-off; keep the pg_query grammar anchor | done |
| 2 | `Emission::Template` in the registry with build-time validation; the generic interpreter in the printer with structural parenthesisation; migrate `ModuloCall`/`PowerCall` with byte-identical pins | done |
| 3 | Compile-time refusal of modifiers a template cannot carry; `emission_ownership` extended (no names in the interpreter, every `RewriteId` justified); coverage table gains `template` | done |
| 4 | Close the DuckDB gaps (#177) with templates/`Unsupported`, verified by the in-process audit; tighten `dialect_gaps_duckdb`; add the end-to-end compile-path modifier-refusal test over the first function-call template row | done |
| 5 | Operand-conditional verdicts: `OperandClass`, arms with mandatory `otherwise`, compile-path settlement threaded into the printer, `dialect_seam` refusal for unresolved wrong-number entries | done |
| 6 | Audit probes every arm: fixture columns per class, coverage totality counts arms, ledger rows keyed by arm, coverage table renders arm sets | planned |
| 7 | Close the Spark gaps (#178) and the Spark arms of #174 (`LOG` arity, `DAYOFWEEK`), `//` per operand class, `TRUNC`/`TO_JSON` by class — verified on a live Spark via `scripts/spark-up.sh`; tighten `dialect_gaps_spark` (block, never fake, if the server cannot start) | pending |
| 8 | Land the invariant in `architecture.md` item 14 and CLAUDE.md; docs-site diagnostics page for the new `UnsupportedOnBackend` reasons; ROADMAP; update the tracking issues with what remains for the BigQuery sweep | pending |

## Decision log

- 2026-09-04 — **#181, PostgreSQL emission verdicts: retire the dialect from the emission set
  rather than add an oracle.** Evidence: no backend crate; `smelt-backend` already maps
  `PostgreSQL` to Spark's maintenance dialect as a placeholder; only 3 explicit registry rows name
  it, the other ~165 "Native" claims are the default; ROADMAP records the backend as deprioritised.
  Under this spec every verdict must be measured live, so keeping the variant would force each new
  template or arm to carry a PostgreSQL row nobody can verify — the exact unverified claim #181
  objects to — and would grow the gap ratchet with rows no backend will ever serve. The parser's
  pg_query grammar anchor is a different thing and stays. When a PostgreSQL backend is built, the
  audit derives its column from probes; nothing authored today would survive that anyway.
  Recorded as the recommendation the user asked for; reverse by deleting phase 1 and criterion 8.

- 2026-09-06 — **Phase 1 scope note (no reshape).** Planning found two string-keyed
  PostgreSQL paths beyond the emission enums: `backend_dialect_for` /
  `backend_write_capabilities_for` in `smelt-db/src/queries/maintenance.rs`, and the `"postgres"`
  branch of `smelt-logical`'s `as_struct` lowering. Both are unreachable surface —
  `Target::backend_type` already rejects `type: postgres` — so removing them is not a user-visible
  behaviour change, and they are folded into phase 1 rather than becoming a phase of their own.

- 2026-09-06 (implement 01) — phase 1 done. `DialectId::PostgreSql`/`SqlDialect::PostgreSQL`,
  `BackendCapabilities::postgresql()`, the three registry rows, the two string-keyed
  `smelt-db`/`smelt-logical` paths, and the baseline entry are all removed; two new durable gates
  added (`no_registry_row_names_a_retired_dialect`, `baseline_names_exactly_the_audited_dialects`);
  coverage doc regenerated; #181 was already closed. `verify-phase.sh` ALL GREEN. No new gaps
  surfaced; the pg_query anchor and ROADMAP are confirmed untouched.

- 2026-09-06 (plan 02) — **No reshape.** Phase 1's summary surfaced nothing outside its task
  list, so phases 2–8 stand as written. Phase 2's plan fixes three design points the outcome left
  open, all inside its own scope: `Emission::Template` carries a `&'static str` with zero-based
  `{n}` placeholders; validation is a pure `validate_template` called from the registry seed
  (asserting, not `.expect(`, so the hardening ratchet is untouched); and a template on a
  **variadic** signature is rejected at build time, since a placeholder cannot name a variadic
  tail. A `PAREN_EXPR` argument counts as an atom on substitution — double-wrapping it would break
  the byte-identical pins criterion 1 requires.

- 2026-09-06 (implement 02) — phase 2 done. `Emission::Template` +
  `validate_template` + `is_call_shaped_template` shipped in
  `smelt-types::signatures`; `RewriteId::ModuloCall`/`PowerCall` deleted; `%`,
  `^`, `**` now templates; generic `print_template` interpreter in
  `smelt-dialect::printer` dispatched from both the function-call and
  operator emission sites. Discovered no `PAREN_EXPR` `SyntaxKind` exists in
  this grammar — the plan's design-decision text named it informally; the
  real mechanism is the transparent `EXPRESSION` wrapper the parser puts
  around every function argument and parenthesised group, verified
  empirically. Argument-level wrapping is gated on the whole template being
  call-shaped (never wraps for `MOD`/`POWER`, matching the pinned
  byte-identity tests); a non-call template additionally wraps its own whole
  output. `docs/reference/dialect-coverage.md` regenerated;
  `docs/specs/multi_backend.md`'s stale divergence bullet removed. Full
  `verify-phase.sh` green plus every gate the plan's Verification section
  named. See `phases/02-summary.md` for detail.

- 2026-09-06 (plan 03) — **Small reshape: the end-to-end leg of criterion 3 moves to phase 4.**
  Every `Emission::Template` row today is an infix operator (`%`, `^`, `**`), and a `BINARY_EXPR`
  can carry none of the seven modifiers, so phase 3 has no production call site to refuse
  end-to-end. Phase 3 therefore ships the refusal as a pure detector wired into
  `unsupported_emissions`, tested one-modifier-per-test against real parsed SQL; phase 4's row
  gains the compile-path test once it registers the first function-call template. Nothing leaves
  the outcome. Phase 3 also needs no spec delta — `multi_backend.md` §"Template emission" already
  states the refusal rule and the per-`RewriteId` justification line normatively.

- 2026-09-06 (implement 03) — phase 3 done. `template_unsupported_modifier` refuses a template
  call carrying `DISTINCT`, `FILTER`, `WITHIN GROUP`, an argument-list `ORDER BY`, `IGNORE`/
  `RESPECT NULLS`, a named argument, or `*`, inspecting only the call's own children and its own
  `ARG_LIST`'s direct children (never `descendants()`, avoiding the nested-call misattribution
  trap). `emission_ownership` gained two gates (every `RewriteId` justified with a `Not a
  template:` doc line; the interpreter holds no target text). Coverage legend gained
  `template:X`; doc regenerated. Discovered empirically that `COUNT(*)`'s argument is
  `EXPRESSION(EXPRESSION(STAR))`, not one wrapper layer — `is_star_expression` peels to
  arbitrary depth. `verify-phase.sh` ALL GREEN after updating 5 line-number-keyed
  `.claude/unknown-census.toml` entries shifted by the `signatures.rs` doc-comment edit. See
  `phases/03-summary.md`.

- 2026-09-06 (plan 04) — **No reshape.** Phase 3's summary surfaced nothing outside its task
  list. Planning measured the seven DuckDB emission rows against a live DuckDB: four names
  (`INITCAP`, `TO_CHAR`, `QUOTE_IDENT`, `QUOTE_LITERAL`) do not exist on the engine at all and
  get `Unsupported`; `DATE_SUB` is a template (`{0} - {1}`) and becomes the first *function-call*
  template row, so it carries criterion 3's end-to-end refusal test; the two `PERCENTILE_*`
  `Position::Window` ledger rows are already `Emission::Unsupported` in the registry and are
  simply redundant. Open risk recorded in the plan rather than deferred: templating `DATE_SUB`
  makes its type leg run for the first time, and `DATE_ADD`'s existing row suggests the
  unquoted `INTERVAL 1 DAY` argument infers `Unknown`. The plan carries a bounded contingency
  (infer the unquoted interval literal; correct `DATE_ADD`/`DATE_SUB`'s return type to
  `Timestamp`, matching `binary.rs` and the engine) so criterion 5's `≤ 5` stays reachable
  without touching the `#175`/`#176` rows it deliberately leaves standing.

- 2026-09-06 (implement 04) — phase 4 done. DuckDB gap count 12 → 6:
  `PERCENTILE_CONT`/`PERCENTILE_DISC` Window rows deleted (redundant with the
  registry's existing `Unsupported`); `INITCAP`/`TO_CHAR`/`QUOTE_IDENT`/
  `QUOTE_LITERAL` given `Emission::Unsupported`; `DATE_SUB` given
  `Emission::Template("{0} - {1}")` — the first function-call template row,
  closing criterion 3's deferred end-to-end leg. Measured against the pinned
  DuckDB 1.5.4 library, not the ambient CLI (v1.4.4) — the CLI would have
  wrongly claimed `INITCAP` unsupported; the audit's own two-sided check
  caught it. Contingency triggered: the plan's unquoted-`INTERVAL`-literal
  guess was wrong (that literal already infers correctly); the real cause is
  `DATE_ADD`/`DATE_SUB` having no `SqlFunction` enum variant at all, so the
  registry's return type (corrected to `Timestamp`, matching `binary.rs`) is
  never reached — landed as fresh `type_gap` rows per the bail-out clause
  (count 6, not 4). Also rewrote `a_ledger_row_the_engine_now_accepts_is_reported_stale`
  to unit-test a new pure `classify_accepted` helper directly, since phase 4
  closed the only DuckDB Schema-leg row that test used to borrow. `verify-phase.sh`
  ALL GREEN. See `phases/04-summary.md`.

- 2026-09-06 (plan 05) — **No reshape** (phase 4's summary explicitly says phase 5 stands).
  Planning fixed four design points inside phase 5's own scope. (a) `OperandClass` lives in
  `smelt-types::signatures` as a total `of(&DataType)` with no `_` arm. (b) The arm's verdict is
  a distinct `SettledEmission` type, so a nested conditional is unrepresentable rather than
  merely rejected, and "the printer receives settled verdicts only" is a type-level fact rather
  than a convention. (c) Settlement runs inside `print_checked_for` — the single funnel
  `dialect_seam` already guards — fed by the same `TypeContext` the projection derives from; a
  caller without one settles every class as `Unresolved`, landing on `otherwise`. (d) Since
  `smelt-dialect` cannot depend on `smelt-db`, the walk takes a type-lookup callback owned by
  `smelt-runtime`. Two small spec amendments follow from the code: there is no `DataType::Json`,
  so the class list swaps `Json` for `Binary` (`Blob`), and `Null`/`Unknown` both classify
  `Unresolved`. No production registry row becomes conditional here — every candidate row
  (#173 BigQuery, #174/#178 Spark, `//` per class) needs a live engine the phase cannot reach,
  so the mechanism is tested on synthetic signatures and the rows land in phase 7, as the
  outcome already schedules. Nothing leaves the outcome.

- 2026-09-06 (implement 05) — phase 5 done. `OperandClass`/`SettledEmission`/`ConditionalArm`/
  `CallFacts`/`Signature::settle_at` in `smelt-types`; `crates/smelt-dialect/src/emission_settle.rs`
  (new) does the compile-path walk and the printer's lookup-or-arity-fallback; `printer.rs`,
  `restructure.rs`, `emission_check.rs` moved off `emission_at` onto `settle_at`; `print_checked_for`
  now threads a `TypeContext` into settlement, same construction `derive_projection_for` uses. Spec
  edits landed (`Json` → `Binary`, `Null`/`Unknown` → `Unresolved`, Known Divergences narrowed). No
  production row is `Conditional` yet (phase 7's job, per plan 05's note); mechanism tests use
  synthetic signatures (`registry_coverage.rs`) plus the real `//` entry for walk-mechanics coverage
  (`operand_conditional.rs`, new). `verify-phase.sh` ALL GREEN. See `phases/05-summary.md`.

- 2026-09-06 (plan 06) — **No reshape**; phase 5's summary named phase 6 as exactly this work and
  found nothing out of scope. Planning fixed four design points inside phase 6's own scope. (a) The
  fixture gains `iv_interval` and `bin_blob` columns so every `OperandClass` has a typed column,
  except `Unresolved`, which no typed column can classify — it is probed with a bare `NULL`
  literal, and that is the phase's one spec sentence. (b) Arm probes are derived per *distinct arm
  guard across dialects*, keeping the probe axis dialect-independent as it is today, and the
  argument classes for arm *k* are found by searching assignments until `settle_at` actually
  resolves to arm *k* — so an arm shadowed by an earlier one is reported by index rather than
  silently under-probed. (c) `TypeConstraint`→column (existing) and `OperandClass`→column (new) stay
  two separate mappings: they answer different questions, and merging them would perturb the
  existing probe set and surface gaps that belong to phase 7. (d) The registry-wide arm gates land
  green-but-vacuous because no production row is `Conditional` yet; the mechanism is proven
  red-green on synthetic signatures, and the gate must exist before phase 7's rows or those arms
  land unprobed. `.claude/dialect-gaps-baseline.txt` must be unchanged by this phase.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
