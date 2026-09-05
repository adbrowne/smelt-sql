# Engine-Defined Semantics: Retiring the Portable-Surface Promise

**Date:** 2026-09-05
**Status:** Research / position
**Author:** Andrew Browne, with design input from Claude

This note proposes that smelt stop promising a SQL dialect that behaves identically
across engines, and instead promise a **typed frontend over the target engine's own
semantics**. The engine defines what an expression computes. Smelt defines what a
*model* means on top of that — materialisation, incrementality, grain, the equivalence
invariant — and to do so it needs a small, enumerable set of facts about each engine,
verified against a live oracle. Portability stops being a language guarantee and becomes
a *diagnostic*: smelt can tell a user exactly where a model's meaning differs between
two engines, because it understands both.

## 0. Authoring Context

> For a continuing reader. Provenance and confidence, not design.

**Origin.** A conversation on 2026-09-05 that started from the question "do you believe
the promise that smelt SQL behaves identically across DuckDB, Spark, Databricks, BigQuery
and Snowflake?" The honest answer was no for the strong form, yes for a weak form (one
specified semantics, a verified subset per backend, compile-time refusal outside it). The
follow-up reframing — most smelt users run one engine, maybe two; portability matters
less than smelt *understanding* the engine it targets — is the position developed here.

**Relationship to prior positions.** `20260516-decimal-type-system.md` §5 proposed three
layers: a portable surface with guaranteed-identical semantics, engine-bound models with
native semantics, and engine-branched library functions bridging the two. It already
observed (§5.4) that most users would declare an engine. This note goes one step further:
the portable surface is retired as a *guarantee*. There is one semantic regime, the
engine's. What the decimal paper called "portable" becomes a checkable property of a
model relative to a named set of engines, computed from per-engine profiles rather than
enforced by a restricted language. `20260517-engine-branched-functions.md` remains
relevant as library mechanism; its capability-set framing carries over unchanged.

**Author confidence:**

- **High** — §1 (the strong promise does not hold and no shipped system has delivered it),
  §3 (the engine-profile interface is small and enumerable), §5 (the error-model hole in
  the equivalence invariant is real and currently unstated).
- **Medium** — §6 (the cross-engine diagnostic as the product goal; the shape is clear,
  the UX is not), §7 (impact table on existing gates; some entries need checking against
  code).
- **Low** — §10 (open questions, especially float tolerance and how "unverified on target"
  should surface to the user).

**Pre-reading:** `docs/specs/multi_backend.md` (§Surface "Capability matrix" — the line
"they do not differ in which smelt models a user may write" is the sentence this note
changes), `docs/specs/incremental_models.md` §"The equivalence invariant",
`20260516-decimal-type-system.md` §5–6, `20260823-registry-dialect-emission-audit.md`.

## 1. Why the strong promise does not hold

"Behaves identically" fails on semantics, not syntax. The spelling problems are solved by
the emission registry. The semantic problems are not solvable by spelling, and each of
these is a place where two of the named engines give different answers to the same
well-formed expression:

- **Numeric.** Decimal precision and scale propagation through `*` and `/` differs on
  every engine. Integer overflow errors on DuckDB and BigQuery, wraps silently on legacy
  Spark, errors on ANSI Spark. Division by zero and failed casts are errors on some engines
  and `NULL` on others. Composition matters: each leaf can conform while a three-operator
  expression diverges, because scale is propagated by the engine's rules, not smelt's.
- **Time.** Spark timestamps are session-local; BigQuery separates `DATETIME` from
  `TIMESTAMP`; Snowflake has `NTZ`/`LTZ`/`TZ`. Week start, day-of-week numbering,
  month-end arithmetic, and format-string vocabularies all differ.
- **Identifiers.** Snowflake upper-folds unquoted names; DuckDB and Spark fold
  case-insensitively; BigQuery is case-sensitive for columns. Output projection names are
  part of the equivalence promise, so this reaches the source-derived projection rule.
- **Regex and collation.** RE2, Java regex, PCRE-ish, POSIX-ish. Not reconcilable except by
  restricting to a verified common subset. Collation and string ordering differ likewise.
- **Aggregates without a single answer.** Float sums are order-dependent within one
  engine, so "identical" already has to mean "equal under a per-type tolerance".
  Approximate-distinct sketches, percentile interpolation, `STRING_AGG` ordering,
  tie-breaking in `MODE` / `ANY_VALUE`.
- **Nulls and sets.** Default null placement in `ORDER BY`; union type coercion; `EXCEPT`
  semantics; array indexing (1-based DuckDB, 0-based BigQuery and Snowflake).
- **Physical.** `MERGE`, transactions, atomic swaps — orchestration rather than dialect,
  but the equivalence invariant is discharged through them.

Prior art confirms the shape. ZetaSQL is the only system that delivers one semantics
across engines, and it needed a full reference implementation plus a compliance suite to
define it. SQLGlot, Ibis, Malloy and SQLMesh all landed on best-effort translation with a
documented divergence list. Nobody has shipped the strong promise without owning an
execution engine.

Smelt's implicit reference semantics today is DuckDB, because DuckDB is the differential
oracle for parsing, typing and the maintenance conformance gate. That choice was never
made explicit. Under the strong promise, the multi-backend question becomes "how
expensively can DuckDB semantics be reproduced on Spark and BigQuery", and the answer for
numerics and regex is "only by cast-wrapping or restricting every expression".

## 2. The proposal

**Smelt SQL has no expression semantics of its own.** A model compiled for target *T* means
what *T* says it means. Smelt's promises are about models, not expressions:

1. The output schema (names, types, nullability) smelt reports for a model is what *T*
   will produce. This is the existing type-oracle strictness gate, made per-engine.
2. The maintenance plan derived for a model is correct on *T*: `incremental_state ==
   full_refresh` under *T*'s semantics, including *T*'s error model (§5).
3. Every construct in the model is either **verified** on *T*, or the user is told it is
   **unverified** (§9). Nothing is silently assumed.
4. When a user names two engines (a dev engine and a prod engine, or a planner-split
   pipeline), smelt reports every expression whose result type, nullability, determinism,
   or error behaviour differs between them (§6).

Promise 4 is the multi-engine product. It is strictly more useful to a one-engine user
than portability, because the common workflow — develop locally on DuckDB, deploy to a
warehouse — is exactly the case the strong promise was meant to cover and could not.

What this removes: the portable surface as a language-level guarantee; the pressure to
lower numeric semantics to a reference; the need for a reference semantics at all. What it
keeps: every existing emission and lowering mechanism (spelling, position restructuring,
`Unsupported` refusal), because those are about whether *T* can express the construct, not
about what it means.

## 3. The engine semantic profile

Smelt's model-level promises depend on a bounded set of facts about expression behaviour.
That set *is* the interface between smelt and an engine, and it should be written down as
one. Per construct (registry entry, operator, cast, literal form), per engine:

| Fact | Consumer in smelt | Today |
|---|---|---|
| Result type, incl. decimal `(p, s)` rules, integer width, division result | type inference, source-derived projection, cast-wrap | DuckDB-exact oracle; Spark/BigQuery differences are `divergences.rs` entries |
| Nullability | nullability inference, key-null admission | DuckDB oracle only |
| Determinism | walk leaf verdict, maintenance-plan admission | registry classification, engine-blind |
| Monotonicity and other walk leaf verdicts | property composition walk | engine-blind |
| Error model: error / `NULL` / wrap on overflow, div-by-zero, failed cast | equivalence invariant (§5), planner batch safety | absent |
| Ordering semantics that feed a property: null placement, collation | grain/FD folding, window-frame verdicts | absent |
| Identifier folding | projection names, ref resolution | per-dialect in printer, not in the type system |

Everything outside this table is **engine-defined and passed through**: smelt does not
know or care what a regex means, only that it is deterministic and returns `BOOLEAN`.
The table is small enough to verify per engine and large enough to carry every model-level
promise. It should be a spec section, and the registry (`BuiltinRegistry` in
`smelt-types/src/signatures.rs`) is the natural single owner: it already keys emission on
`(DialectId, Position)`; result-type rules and property classification become further
per-dialect columns of the same row.

`divergences.rs` changes meaning under this proposal. Today each entry is a *tolerated
difference from the DuckDB reference*. It becomes *the engine's declared result type*,
which is the same data with the opposite sign — and it stops being a test fixture and
becomes registry data the compiler consumes.

## 4. What smelt still owns

The switch does not touch smelt's own correctness obligations, and it is worth listing
them so the retreat from portability is not misread as a retreat from rigour:

- **SQL smelt authors itself** — maintenance statements, ledger DDL/DML, restructure CTEs,
  cast-wrapping, the merge-less conditional-write group. These are smelt's semantics, on
  every engine, and are already gated per engine (`statement_parity`,
  `maintenance_conformance` on DuckDB, Spark and BigQuery). Unchanged.
- **Expressibility lowering** — `Emission::Rewrite` / `Restructure` / `Unsupported`.
  Unchanged; these decide whether *T* can express a construct, not what it means.
- **Type and nullability reports** — now promised per engine (§3), which is a widening of
  the oracle, not a loosening.
- **The maintenance plan** — derived once, pure, but its admission verdicts (determinism,
  monotonicity, error model) now take the engine profile as input. The plan for the same
  model may legitimately differ between engines; that is a feature, and §6 reports it.

## 5. The error-model hole in the equivalence invariant

`incremental_models.md` promises `incremental_state(S) == full_refresh(inputs ∈ S)` for
every maintained model under any valid run sequence. The conformance gate discharges it on
one engine, with expression evaluation assumed inert. That assumption is false as soon as
an expression can error or wrap:

- A full refresh sums a column over all rows and overflows. The incremental path sums
  each batch, none of which overflows, and merges. On DuckDB and BigQuery: one path errors,
  the other succeeds. On legacy Spark: both succeed with different numbers. Neither is the
  equality the invariant states.
- A failed cast on one row fails the full refresh but only the batch containing that row
  on the incremental path, leaving the table in a state no full refresh could produce.
- The reverse: an incremental batch that overflows a running aggregate where the full
  refresh, computed in a different order, does not.

This is not caused by the proposal; the proposal exposes it. Today the gate's recipe pool
does not generate overflow or cast failure, so the hole is invisible. The invariant needs a
stated clause. Two candidate shapes:

- **Error-symmetry:** the invariant holds when neither path raises; if either raises, the
  run is a failure with no partial state — which requires atomicity that Spark-on-Parquet
  and BigQuery do not offer for statement groups.
- **Error-model as a walk input:** a construct whose error model on *T* is `wrap` is
  inadmissible for maintenance on *T* unless the author accepts a declared relaxation
  (this is a contract-lattice point in the sense of `incremental_models.md` §"The contract
  lattice": declaration, oracle transform, probe emitter). `error` constructs are admitted
  under error-symmetry; `NULL`-returning constructs are simply typed nullable.

The second is the one that fits the existing architecture. It also gives the Spark backend
a reason to pin ANSI mode as a declared capability rather than an unstated assumption —
the decimal paper already said "smelt requires Spark sessions to run with ANSI mode" and
nothing enforces it.

## 6. The cross-engine divergence diagnostic

If smelt holds an engine profile for each of *A* and *B*, then for any model it can
compute, per expression node, whether the §3 facts agree. Where they do not, it can say
precisely where and how:

```
models/orders_daily.sql:14:22
  E0xxx  `revenue / quantity` differs between targets
         duckdb   : DECIMAL(38,18), errors on quantity = 0
         databricks: DECIMAL(38,6), errors on quantity = 0 (ANSI)
         → result scale differs; downstream comparisons may not agree
```

This is the product goal for multi-engine work. Its value does not depend on anyone
running a model on two engines in production. It depends on the profile being correct for
each engine, which is the same verification cost §9 already requires.

It also subsumes the decimal paper's portable surface. "Portable across {A, B}" is now a
model-level verdict — the diagnostic is empty — rather than a restricted language the user
has to write in. A user who wants portability enforced turns the diagnostic into an error
in `smelt.yml`; everyone else gets it as information.

## 7. Impact on existing invariants and gates

| Invariant / gate | Change |
|---|---|
| Type-oracle strictness (`type_property_tests`) | Becomes per-engine. `divergences.rs` entries become declared engine result types (§3). The DuckDB leg stays per-PR; Spark and BigQuery legs run where the engine is available. |
| Function-registry single ownership | Widens: per-dialect result-type rule and per-dialect property classification join spelling as registry columns. The consistency gate's "every name resolves" check gains "every dialect column is populated or marked unverified". |
| Property composition walk | Leaf classifiers take a `DialectId`. Walk-coverage gate unchanged in shape. |
| Maintenance-plan purity | Unchanged in shape; the pure derivation gains the engine profile as an input. |
| Equivalence invariant + conformance gate | Needs the error-model clause (§5) and recipes that generate overflow and cast failure. |
| Contract lattice | Likely gains one point (error-model relaxation, §5). |
| Source-derived projection | Unchanged. Identifier folding becomes a profile fact but projection is still derived from the source CST. |
| Emission-ownership gate, dialect-seam refusal | Unchanged. |
| Cross-engine emission audit | The value leg's purpose changes: today a mismatch is a bug or a ledger entry; under this proposal a *type* mismatch is a profile fact to record, and only a mismatch *against the recorded profile* is a bug. The ledger should split accordingly. |
| `multi_backend.md` "they do not differ in which smelt models a user may write" | Retire. Backends differ in expressibility (capabilities, refusal) **and** in expression semantics (profile). |

## 8. Relation to prior research

- **`20260516-decimal-type-system.md`.** Its survey (§2–3) and divergence inventory are
  exactly the numeric rows of the engine profile. Its §5.1 portable surface is retired as a
  guarantee; §5.2 engine-bound models become the only kind; §5.4's observation that most
  users declare an engine becomes the premise. The §6 division decision ("`/` is not in the
  portable surface") dissolves: `/` means what the engine says, and the diagnostic reports
  when two engines disagree.
- **`20260517-engine-branched-functions.md`.** Unchanged as library mechanism. The
  capability-set framing (§5 there) is how a function's engine support flows through the
  call graph; under this proposal the profile supplies the per-branch result type the type
  checker verifies each body against. The "lossy branch" open question becomes a
  diagnostic: two branches with different profile facts are reported, not forbidden.
- **`20260823-registry-dialect-emission-audit.md`.** The audit's probe generation is the
  verification engine for the profile (§9). The two-sided ledger needs the split noted in
  §7.

## 9. The verification bound

"Smelt understands how the engine behaves" is only as true as the oracle coverage behind
the profile. Warehouse documentation is wrong often enough that every profile fact needs
live-engine verification, and that cost scales with engines × constructs × facts. Today
Spark runs nightly, BigQuery is a manual sweep, Snowflake does not exist.

The mitigation is the pattern the repo already uses everywhere else: an explicit
**unverified** state per `(construct, engine, fact)`, distinct from verified, surfaced to
the user as a diagnostic tier (as the `Unknown` census does for types), and ratcheted
down. An unverified fact must never be silently assumed equal to DuckDB's. That is the
fail-loud discipline applied to the profile.

Compositional probes are the biggest coverage gap regardless of direction. The audit
probes one construct at a time; the profile's decimal and error-model rows are exercised
only by multi-operator expressions. A generated compositional probe pool (numeric chains,
timestamp arithmetic chains, cast chains) is the first new gate this proposal needs.

## 10. Engines

The list in the originating question — DuckDB, Spark, Databricks, BigQuery, Snowflake —
has one conflation and several omissions.

- **Databricks ≠ open-source Spark.** Databricks SQL defaults to ANSI mode and supports
  `QUALIFY`; OSS Spark Connect as run in CI does not. The same statement can return `NULL`
  on one and error on the other. They need separate profiles, or a Spark profile
  parameterised by the ANSI flag, with the flag pinned per target.
- **Missing with real user pull:** Redshift, Trino / Athena / Starburst. Then ClickHouse,
  Postgres (retired as an emission dialect in #181 but still a source of users), Fabric.
- **Snowflake** is the largest named target with no backend. Its identifier folding and
  three timestamp kinds are the two profile rows most likely to surprise.

Under this proposal adding an engine is: a profile (initially all-unverified), an oracle,
and the audit sweep. It is not a lowering exercise.

## 11. Open questions

1. **Float equality.** "Same result" for `DOUBLE` aggregates must be a per-type tolerance.
   Who declares it, and does the conformance oracle apply it per column type?
2. **Error-model clause shape** (§5) — error-symmetry vs. walk-input-with-lattice-point.
   Recommendation: the latter.
3. **Surfacing "unverified".** A warning per expression is noisy for a new engine where
   everything is unverified. Per-model summary? Per-target summary at `smelt build`?
4. **Does the profile need per-version rows?** DuckDB's division semantics changed at 0.8;
   Spark's at ANSI. A target already pins an engine; whether it pins a version is a
   `smelt_yml.md` question.
5. **Planner engine-split.** When the planner moves a model between engines, the
   diagnostic in §6 must be empty for that model, or the planner refuses the split. This
   is the one place portability remains load-bearing, and it is now a checkable
   precondition rather than an assumption.

## 12. Suggested next steps

Spec-first, in this order:

1. `docs/specs/multi_backend.md` — replace the "do not differ in which models a user may
   write" sentence with the profile interface (§3) and the verified/unverified state (§9).
   Retire the implicit DuckDB-reference framing explicitly.
2. `docs/specs/incremental_models.md` — the error-model clause on the equivalence
   invariant (§5), and the corresponding contract-lattice point.
3. `docs/specs/model_properties.md` — leaf verdicts are per-dialect.
4. Plan: registry columns for result-type rule and property classification; `divergences.rs`
   promoted to registry data; compositional probe pool; conformance recipes that overflow.
5. Plan: the cross-engine divergence diagnostic (§6), DuckDB × Spark first since both
   oracles exist.
