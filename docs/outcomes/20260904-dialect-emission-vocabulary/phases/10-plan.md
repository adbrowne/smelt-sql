# Phase 10 plan — land the invariant, the user-facing refusal reasons, and the issue trail

## Objective

Close criterion 7's remaining leg (the invariant text in `architecture.md` §Constraints item 14 and
CLAUDE.md must name templates and the compile-path settlement rule) and the outcome's last unclaimed
deliverables: user-facing documentation of the two *new* families of `UnsupportedOnBackend` reason
this outcome introduced (template-modifier refusals, operand-class refusals), the ROADMAP record, and
the tracking-issue trail naming exactly what the human-run BigQuery sweep still owes. Every doc claim
about diagnostic text is pinned against live compiler output, never hand-copied — the same
measure-don't-assert rule the rest of the outcome ran under.

## Spec delta

`docs/specs/architecture.md` §"Constraints & Invariants" item 14 (**edit first**):

- The ownership sentence gains templates and settlement: a built-in's per-dialect, per-position
  emission may be `Native`, `Rename`, `Template`, `Restructure`, `Rewrite`, `Unsupported`, or
  `Conditional`; a `Template` is registry *data* (a `{n}`-placeholder spelling over the call's own
  positional arguments) interpreted by one generic printer routine that knows no function names, and
  a `Conditional` is an ordered arm list ending in `otherwise` **settled on the compile path** from
  the projection's own type inference, so the printer receives `SettledEmission` only and holds no
  type context.
- A new sub-bullet for the build-time validation gate: placeholder range, argument coverage,
  variadic rejection, call-shape at window positions, and a `Template`-verdict arm inside a
  `Conditional` are all validated at registry construction
  (`cargo test -p smelt-types --test registry_coverage`).
- The Consistency-gate bullet's parenthetical is already correct after phase 9; leave it.
- Cross-reference `multi_backend.md` §"Template emission" / §"Operand-conditional verdicts" (already
  normative since `03828a14`; **no edit to `multi_backend.md` is in scope** beyond fixing anything
  found stale while reading it).

`CLAUDE.md`, the **Function-registry single ownership** bullet: mirror the same two facts in one
sentence each (template = registry data interpreted generically; conditional arms settled on the
compile path, printer holds no type context), and add `cargo test -p smelt-dialect --test
template_emission --test operand_conditional` plus `cargo test -p smelt-types --test
registry_coverage` to the named gates.

## Tests

Red-green, all in `crates/smelt-runtime/tests/dialect_seam.rs` (extend the existing doc-sync gate;
factor its marker-extraction into a helper taking `(marker, model_sql, backend)`):

1. `docs_quoted_template_modifier_refusal_matches_the_live_diagnostic` — a new
   `<!-- unsupported-on-backend-template-modifier-refusal-text -->` block in
   `docs-site/docs/reference/diagnostics.md` must equal, byte for byte, the live error from
   compiling a template-row call carrying `DISTINCT` (`DATE_SUB` is the function-call template row
   on DuckDB, per phase 4). Red first: add the test, watch it fail against an absent/wrong block.
2. `docs_quoted_operand_class_refusal_matches_the_live_diagnostic` — a new
   `<!-- unsupported-on-backend-operand-class-refusal-text -->` block must equal the live error from
   compiling `a // b` on **Spark** with an operand whose class is neither `Integral`, `Floating` nor
   `Decimal` (a `Text` column, or an unresolvable one), i.e. the `otherwise -> Unsupported` arm.
3. `docs_quoted_refusal_text_matches_the_live_diagnostic` — existing; must stay green through the
   helper refactor.

If either new model turns out not to reach the refusal (e.g. the parser rejects the modifier before
the emission check), the correct response is to pin whatever construct *does* reach it and say so in
the summary — never to invent doc text for a message the compiler does not emit.

## Tasks

1. Read `docs/specs/multi_backend.md` §"Template emission" and §"Operand-conditional verdicts";
   apply the `architecture.md` item 14 edit above (spec first).
2. Apply the CLAUDE.md **Function-registry single ownership** edit.
3. Refactor `dialect_seam.rs`'s doc-quote extraction into a helper; add tests 1 and 2 (red).
4. Run the two new models through the real compile path, capture the **actual** error text, and write
   `docs-site/docs/reference/diagnostics.md`: under `UnsupportedOnBackend`, add two short subsections
   — "A template's spelling cannot carry a modifier" (list the seven refused modifiers; one pinned
   example block) and "A verdict that depends on operand type" (`//` on Spark; one pinned example
   block) — each with a one-line **Fix**. Green.
5. `docs-site/docs/guide/targets.md` §"Cross-engine SQL compilation": add a short
   "Per-operand-type lowering" note — `a // b` is integer division on Spark for integral operands and
   `/` for floating/decimal, and is refused when the operand type is not known at compile time —
   linking to the diagnostics page. Keep it under ~12 lines; no new claims beyond what the registry
   rows state.
6. `docs/ROADMAP.md`: flip the 2026-09-04 "dialect emission vocabulary" parallel track to complete
   (✅, September 6, 2026) with the measured outcome — `dialect_gaps_duckdb` 12 → 4,
   `dialect_gaps_spark` 27 → 4, PostgreSQL emission dialect retired — and name what remains:
   BigQuery's 42 rows (#179, plus the BigQuery arms of #173/#174) awaiting the human-run
   `scripts/bigquery-dialect-audit.sh` sweep, and the four `#175`/`#176` type rows per dialect.
7. Tracking issues via `gh` on `adbrowne/smelt-sql` — comment on each, then close only where the
   issue's own subject is fully paid down:
   - **#177** (DuckDB, 12 rows): comment with the closures and the four surviving `#175`/`#176`
     rows; **close** — no DuckDB emission verdict is missing.
   - **#178** (Spark, 27 rows): same shape; **close**.
   - **#173**, **#174**, **#179**: comment only, **leave open** — state that the vocabulary
     (`Template`, `Conditional`, compile-path settlement, arm-keyed audit probes) has landed and is
     live-verified on DuckDB and Spark, that #174's Spark arms are closed, and that what remains is
     exactly a human-run BigQuery value-leg sweep because the spec forbids authoring a verdict from
     documentation. Name the script and the fact that its value leg bills.
   - Leave **#175**, **#176**, **#180**, **#182** untouched.
8. Write `phases/10-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (the pre-existing `smelt-core --test baseline` and
  `smelt-runtime` python temp-file flakes are known-unrelated — confirm by isolated
  `--test-threads=1` rerun, do not "fix" them here).
- `cargo test -p smelt-runtime --test dialect_seam` — the three doc-sync legs.
- `cargo test -p smelt-db --test dialect_audit` (DuckDB legs in-process) — the coverage doc-sync gate
  must stay green after any regeneration.
- `cargo test -p smelt-types --test registry_coverage` and
  `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test operand_conditional`.
- `git diff --stat` must show **no** change to `.claude/dialect-gaps-baseline.txt` or to any
  `crates/*/src` emission row — this is a documentation phase.
- `gh issue view 177 --json state` / `178` to confirm the closes landed.

## Commit message

`docs(dialect): land the template + settlement invariant, refusal docs, and the issue trail`
