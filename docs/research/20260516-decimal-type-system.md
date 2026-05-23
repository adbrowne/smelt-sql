# Decimals Across Engines: A Behavioural-Equivalence Type-System Position for smelt

**Date:** May 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper investigates how `DECIMAL` should be modelled in smelt's type system, given that the three reference engines — Postgres, Spark, and DuckDB — diverge meaningfully in defaults, growth rules, division semantics, and overflow handling. The central claim is that smelt's *portable language surface* should expose only operations on `Decimal` that are *provably equivalent* across engines; everything else either requires an explicit engine declaration on the model or is provided as a library function with a portable signature and engine-dispatched body. The most consequential consequence of this position is that **`Decimal / Decimal` is not in the portable surface**: division of decimals demands an engine declaration. The paper situates this against today's smelt behaviour (which already records the divergence in its property-test divergence registry) and sketches what a "polyfill function" mechanism would have to look like to eventually relax the restriction.

## 0. Authoring Context

> For a continuing reader (the author or any reviewer picking this up in a fresh session). Not part of the design — provenance and confidence so the next person can pick up the thread.

**Origin.** Drafted on 2026-05-16 out of a brainstorming conversation with Claude. The conversation started with Andrew asking to understand decimal semantics in Postgres, Spark, and DuckDB, with the goal of writing a research paper on how to model decimals correctly in smelt's type system. The conversation converged toward a layered position: strict portable surface, library polyfills, engine-binding as the default escape hatch.

**Status.** Research / design exploration. Nothing here is committed surface. The decision in §6 (no portable `Decimal / Decimal`) is recorded as the conversation's current resting point, *not* as a spec commitment. Literal-denominator carve-outs were explicitly deferred (see §8) and should not be relitigated here.

**Author confidence (subjective — for a reviewer's calibration):**

- **High** — §2 (engine survey for Postgres, Spark, DuckDB max-precision / overflow / NaN axes), §4 (current smelt state — grounded in code), §6 (the division position, after explicit user agreement).
- **Medium** — §2's division-formula details (the Spark precision-growth formula is well-known; the *current* DuckDB division behaviour for `Decimal / Decimal` is worth empirical re-verification on a recent version), §5 (the contract framing).
- **Low** — §7 (function-as-polyfill — sketched only; many design questions about return-type inference across engine branches are open), §3's notes on Spark `allowPrecisionLoss` edge cases.

**Highest-leverage open decisions** (mirrored in §8):

- **The literal-denominator carve-out.** Whether `Decimal / <exactly-invertible literal>` is allowed in portable code. Deferred — current position is "no exceptions, declare an engine." A future relaxation is conceivable but not designed.
- **The portable-decimal stdlib.** Whether smelt ships a `smelt.divide(a, b, scale => N)` style function with engine-dispatched body. This is the long-term ergonomic answer to "but I really need portable division"; the type-system machinery to support it (§7) is undesigned.
- **Whether the position generalises to other type-family changes.** Division is the salient case for decimals; analogous "type family changes" probably exist for date/time arithmetic across engines. The framing in §5 should be portable.

**Background a continuing reader should pre-read** before extending this paper:

- `crates/smelt-db/tests/prop_helpers/divergences.rs` — the existing divergence registry. Six decimal-related entries today: `sum_integer`, `avg_decimal`, `sign_decimal`, `decimal_division`, `abs_decimal`, `abs_decimal_schema_resolved`. The `decimal_division` entry is marked `ByDesign` against DuckDB.
- `crates/smelt-db/src/type_inference/binary.rs` — current binary-op decimal handling. The promotion ladder is `Double > Float > Decimal > BigInt > Integer > SmallInt`; results are currently emitted as `Decimal { precision: 38, scale: 10 }` regardless of input parameters (§4).
- `crates/smelt-db/src/type_inference/literal.rs` — decimal literal inference (`DataType::Decimal { precision, scale }`).
- `docs/specs/types.md` (if present) — `DataType` vocabulary; the strict-by-default doctrine the position here leans on.
- `docs/research/20260507-typed-meta-programming.md` — establishes the layered language model (meta-world, expansion, fragment sorts). Section §7 of this paper rhymes with the meta-world layering: portable language surface vs. library code that bridges engines.

## 1. Motivation and Scope

### 1.1 Why decimals are interesting

For a multi-backend pipeline tool, the goal of *behavioural equivalence across engines* turns into a concrete question for every type and operator: does the engine give the same answer? For most SQL types — INTEGER, BIGINT, TEXT, DATE, BOOLEAN — the answer is "yes, within the obvious caveats." For `DECIMAL`, it is materially "no":

- The three engines disagree on the default `(precision, scale)`.
- They disagree on the maximum allowed precision (38 vs effectively unbounded).
- They disagree on the growth formula for `+`, `-`, `*`.
- They disagree on the *result type family* for `/` — one engine drops out of decimal-land entirely.
- They disagree on overflow handling (error vs NULL, configurable).
- They disagree on `NaN` representability.

That is enough divergence that smelt cannot honestly claim portability without taking a position. This paper takes one.

### 1.2 Scope

This paper covers:

- A per-axis inventory of decimal divergences across Postgres, Spark, and DuckDB (§2, §3).
- The current state of decimal handling in smelt's type system, grounded in the code (§4).
- A proposed type-system position for portable decimal arithmetic (§5).
- The specific decision that the portable surface excludes `Decimal / Decimal` (§6).
- A sketch of how library functions with engine-dispatched bodies could later expose portable division (§7).
- Deferred decisions (§8).
- What changes from current smelt behaviour if the position is adopted (§9).

Out of scope:

- **Downstream propagation of engine-binding.** Within smelt, every model is a materialisation boundary; engine-specific computation produces concrete physical tables whose column types are whatever the engine yielded. Downstream models consume data, not engine-tainted expressions. Propagation rules across model boundaries are therefore not a concern of this paper.
  FEEDBACK: It's not that EVERY model is a materialization boundary - but that the engine limitation will only follow until the materialization boundary. Either way it's not a key decision here - whatever we do for decimal will follow the normal smelt rules about engine constraint propogation.
- **The full stdlib polyfill surface.** §7 sketches what the mechanism needs but does not commit to specific stdlib functions, names, or signatures.
- **Empirical re-verification of current engine behaviour.** Several claims (especially DuckDB's current `Decimal / Decimal` result type) deserve a small probe script. Recommended as a follow-up, not done here.

## 2. The Three Engines — Survey

### 2.1 Postgres (`NUMERIC` / `DECIMAL`)

- `DECIMAL` is a synonym for `NUMERIC`. The user-facing type is `NUMERIC(p, s)` or unconstrained `NUMERIC`.
- **Unconstrained `NUMERIC`** is a real type, not a default elaboration. It allows up to 131,072 digits before the decimal point and up to 16,383 after. There is no equivalent in Spark or DuckDB.
- Arithmetic is **exact and grows as needed**. `NUMERIC(10,2) * NUMERIC(10,2)` yields `NUMERIC(20,4)` with no truncation.
- Division returns a `NUMERIC` result at high (but configurable) precision; specifics depend on Postgres version and `extra_float_digits` / arithmetic settings.
- Overflow → **error**.
- Supports `NaN` (atypical for a fixed-point numeric type).

### 2.2 Spark (`DECIMAL`)

- Default `DECIMAL` (no parameters) is `DECIMAL(10, 0)`.
- **Maximum precision is 38.** The implementation is backed by `java.math.BigDecimal` but the SQL layer enforces 38.
- Arithmetic uses Hive/SQL-Server-derived formulas:

  | Op | Precision | Scale |
  |---|---|---|
  | `a + b`, `a - b` | `max(p1 - s1, p2 - s2) + max(s1, s2) + 1` | `max(s1, s2)` |
  | `a * b` | `p1 + p2 + 1` | `s1 + s2` |
  | `a / b` | `p1 - s1 + s2 + max(6, s1 + p2 + 1)` | `max(6, s1 + p2 + 1)` |
  | `a % b` | `min(p1 - s1, p2 - s2) + max(s1, s2)` | `max(s1, s2)` |

- When a result exceeds precision 38, Spark applies **clipping**. With `spark.sql.decimalOperations.allowPrecisionLoss = true` (default), scale is reduced (rounded) until precision fits. With `false`, the result is `NULL`.
- Overflow under ANSI mode (`spark.sql.ansi.enabled = true`) → **error**. Under legacy mode → **NULL**.
- Does **not** support `NaN` for decimal.

### 2.3 DuckDB (`DECIMAL`)

- Default `DECIMAL` is `DECIMAL(18, 3)`.
- **Maximum precision is 38**, implemented over four physical widths (INT16/INT32/INT64/INT128) selected by precision range.
- Arithmetic for `+`, `-`, `*` follows decimal-preserving rules similar to Spark's.
- **Division** is the qualitatively different case: `Decimal / Decimal` historically promotes to `DOUBLE` to avoid infinite-precision growth. The smelt divergence registry today records this explicitly (`decimal_division` entry, `ByDesign`). Worth empirically re-verifying on the currently linked DuckDB version (v1.5.0 per `CLAUDE.md`).
- Overflow → **error** (or `NULL` via `TRY_CAST`).
- Does not support `NaN` for decimal.

## 3. Per-Axis Divergence Inventory

| Axis | Postgres | Spark | DuckDB | Intersection |
|---|---|---|---|---|
| Default `DECIMAL` | unconstrained | `(10, 0)` | `(18, 3)` | n/a — smelt must elaborate explicitly |
| Max precision | ~131,072 | 38 | 38 | **38** |
| `+ / - / *` formula | exact, unbounded growth | Hive-style, clip to 38 | Hive-style, error on overflow | Spark's formula, with static result-size check |
| `/` result type | `NUMERIC` (high precision) | `DECIMAL` (Hive formula) | `DOUBLE` | **none — different type families** |
| Overflow on `+/-/*` | error | `NULL` (default) or error (ANSI) | error | error, contingent on Spark ANSI mode |
| `NaN` for decimal | yes | no | no | **no** |

The intersection column is the candidate behavioural contract for smelt's portable surface. Every cell where "intersection" is `none` or `n/a` is a place where smelt must either pick a position or refuse to provide the operation portably.

The two cells that materially constrain the language:

1. **`/` has no intersection.** The result type *family* differs. Compensating casts can produce approximately-equal numeric values but cannot produce *the same SQL type*. This is the core argument for §6.
2. **Overflow has a configuration-contingent intersection.** Equivalence is achievable but extends into the runtime environment: smelt must require ANSI mode (or equivalent) on Spark sessions. Not a language-surface decision, but a deployment contract.

## 4. Current State in smelt

Grounded in the code on `main` as of 2026-05-16.

### 4.1 Type representation

`DataType::Decimal { precision: u32, scale: u32 }` is the in-memory representation. A sentinel `Decimal { precision: 0, scale: 0 }` is used in the divergence registry as a wildcard matching any `Decimal` regardless of parameters — a hint that precision and scale are not always fully tracked through inference.

### 4.2 Binary-op inference

`crates/smelt-db/src/type_inference/binary.rs` implements a promotion ladder:

> Priority: `Double > Float > Decimal > BigInt > Integer > SmallInt`

When either operand is `Decimal`, the result is currently emitted as `Decimal { precision: 38, scale: 10 }` — a fixed placeholder, **not** the result of applying Spark-style growth formulas to the input parameters. This means that today smelt does not compute output precision/scale from input precision/scale for binary arithmetic. Adopting the position in §5 would require fixing this.

### 4.3 Literal inference

`crates/smelt-db/src/type_inference/literal.rs` produces `DataType::Decimal { precision, scale }` for decimal literals, with `precision` and `scale` derived from the textual literal.

### 4.4 Known divergences (decimal-related)

From `crates/smelt-db/tests/prop_helpers/divergences.rs`:

| id | Smelt infers | DuckDB | Spark | Status |
|---|---|---|---|---|
| `sum_integer` | `BigInt` | `Decimal(38, 0)` | — | `ByDesign` |
| `avg_decimal` | `Double` | `Double` | `Decimal` (wildcard) | `ByDesign` |
| `sign_decimal` | `SmallInt` | `SmallInt` (TINYINT) | `Decimal` (wildcard) | `ByDesign` |
| **`decimal_division`** | `Decimal(38, 10)` | `Double` | — | **`ByDesign`** |
| `abs_decimal` | `Unknown` | `Decimal` (wildcard) | — | `KnownBug` |
| `abs_decimal_schema_resolved` | `Double` | `Decimal` (wildcard) | — | `ByDesign` |

Two patterns are visible:

- **Smelt currently aligns with DuckDB on aggregates and unary functions** (`AVG`, `SIGN`, `ABS` on schema-resolved decimal), accepting Spark divergences as `ByDesign`.
- **Smelt currently keeps `Decimal` on division** while DuckDB returns `Double`. The divergence registry calls this `ByDesign` against DuckDB. The position in §6 *reverses* this: smelt's portable surface would refuse the operation entirely; engine-bound DuckDB models would natively follow DuckDB's `Double` result.

### 4.5 What the current state implies

The codebase is honest about the divergences existing but the *position* it has taken — preserve smelt's preferred type even when DuckDB diverges, mark as `ByDesign` — is essentially "smelt's type is the truth." That is the opposite of behavioural equivalence: it tells the user one thing while the engine returns another. The position in §5–§6 inverts this: in the *portable* surface, smelt either offers an operation with a result type the engines can all honour, or refuses the operation.

## 5. Proposed Position: Behavioural Equivalence as a Contract

The proposed framing has three layers, in order of decreasing strictness.

### 5.1 The portable language surface

The set of operators and types whose semantics smelt guarantees identically on every supported engine. For decimals:

- `Decimal(p, s)` with `1 <= p <= 38`, `0 <= s <= p`.
- `+`, `-`, `*` defined by Spark-style growth formulas, with a **compile-time check** that the result fits in `p <= 38`. If the result would exceed 38, the operation is rejected at compile time and the user is told to cast inputs down first. This statically rules out Spark's clip-and-round path and DuckDB's runtime overflow path — both become unreachable from well-typed portable code.
- Overflow at runtime is contractually an error. Smelt requires Spark sessions to run with ANSI mode enabled (or equivalent).
- `/` is **not in the portable surface** (§6).
- `NaN` is not representable in `Decimal`.

The contract: well-typed portable smelt code produces the same answer on every engine, modulo runtime overflow errors that surface identically on every engine.

### 5.2 Engine-bound models

A model annotated with an engine declaration opts into the engine's native semantics for the duration of that model. Inside an engine-bound model:

- `Decimal / Decimal` is allowed; the result type follows the engine.
- Engine-specific decimal types (e.g. Postgres unbounded `NUMERIC`) are available.
- The result of the model is a materialised physical table whose column types are concrete and engine-derived.

The model's *output schema* may contain types that are not directly inhabitable in the portable surface (e.g. unbounded `NUMERIC`). Downstream consumers must either also be engine-bound or cast the offending columns into portable types. Because every model is a materialisation boundary, this propagation question is local — it does not require global inference (out of scope per §1.2).

### 5.3 Library functions with engine-dispatched bodies

The escape hatch that prevents the portable surface from being painfully thin. A function declares a *portable signature* and provides *engine-specific bodies*. The type checker verifies that every body produces the declared return type; the author shoulders the equivalence proof (likely via property tests). From a caller's perspective, the function is portable. From the inside, it polyfills the divergence.

This is the long-term answer to "I want portable decimal division": eventually it lives in stdlib as a function whose signature is portable and whose bodies are engine-dispatched. The type-system support for this mechanism is sketched in §7. **This paper does not commit to shipping such a stdlib function**; it commits to leaving the design room open.

### 5.4 Default user path

Most users will declare an engine on their models. The portable surface is for:

- Library authors building cross-engine abstractions.
- Pipelines that genuinely span engines (the planner deciding to run a model on DuckDB locally and the same model on Databricks at scale).
- Users who want the type system to actively enforce "this code is portable."

This is a deliberate inversion of the framing where portability is the default and engine-specific is exotic. In practice, portability is paid for explicitly — both by accepting a smaller operator set and by the discipline of staying inside the contract.

## 6. The Division Decision

**Position: `Decimal / Decimal` is not in the portable language surface. To divide decimals, the model must declare an engine.**

### 6.1 Why not synthesise an equivalent

The natural attempt would be: smelt picks a result type (say Spark's formula), and on DuckDB emits `CAST(a / b AS DECIMAL(p', s'))` to coerce the runtime result back into decimal-land. This produces *approximately* equivalent answers — but not the same answer, because:

- DuckDB computes `a / b` internally in a non-decimal type before the cast back. The intermediate rounding differs from Spark's exact decimal division.
- Postgres computes `a / b` at high precision, then a smelt cast back to a chosen `(p', s')` rounds further.
- Across engines, the rounding mode and the order in which precision is lost may differ.

The result is that smelt would silently emit code that produces results agreeing to some number of digits but not bit-equal. That is a weaker contract than "the answer is the same," and once the language tolerates approximate equivalence under the same surface, the user has no way to tell which operations are exactly portable and which are within-epsilon portable. The portability claim becomes vague.

The alternative — refuse the operation — keeps the contract sharp: portable means exact, engine-bound means native. The user has a clear forking point.

### 6.2 What the user does instead

Three remedies are available, all explicit:

- **Cast inputs to a floating type and divide.** `a::DOUBLE / b::DOUBLE`. Portable, generic SQL, works on every engine. The result type is `Double`, which is in the portable surface. The user has explicitly left decimal-land.
- **Declare an engine on the model.** `Decimal / Decimal` becomes available with the engine's native semantics. The model commits to one engine.
- **(Future, see §7.) Use a stdlib polyfill function** that wraps engine-specific division in a portable signature.

The compiler error for `Decimal / Decimal` in portable code should list the first two of these explicitly. A future error message can add the third if and when a stdlib polyfill ships.

### 6.3 Why this matters for the divergence registry

Today, `decimal_division` is `ByDesign` and smelt's type system claims `Decimal(38, 10)` while DuckDB returns `Double`. Under §6, this divergence would no longer exist in well-typed portable code, because the operation is not allowed there. Engine-bound DuckDB models would natively return `Double`, matching the engine. The divergence registry entry becomes a historical artifact rather than an ongoing tolerance.

### 6.4 What §6 does not decide

- **Literal-denominator carve-outs** (e.g. allowing `x / 100` because `1/100` is exactly representable in decimal). Deferred — see §8.
- **Modulo (`%`)** is not analysed here. Worth checking whether `Decimal % Decimal` has a clean cross-engine intersection. Speculatively, yes, because the result type formulas across engines agree more cleanly than for `/`.
- **Integer division** (`Decimal / Integer` or `Decimal // Decimal`) is a separate question. Some engines provide a distinct floor-division operator with clean semantics; worth a separate analysis.

## 7. The Function-as-Polyfill Mechanism (Future Direction)

The interesting research contribution beyond the division decision itself is the *mechanism* by which library code can later expose portable operations whose primitives diverge. Sketched here as motivation for keeping the design space open; not designed in detail.

### 7.1 Shape

A function with a portable signature and engine-dispatched body:

```
fn divide(a: Decimal(p, s), b: Decimal(p, s)) -> Decimal(p', s') {
  engine duckdb   => CAST(a / b AS DECIMAL(p', s'))
  engine spark    => a / b
  engine postgres => ROUND(a / b, s')
}
```

From the call site, `divide(...)` is a portable function returning `Decimal(p', s')`. From the type checker's perspective, each engine branch must produce a result of the declared return type. From the author's perspective, the equivalence between branches is a property tested empirically (and possibly proved for specific cases).

### 7.2 Open type-system questions

- **Return-type inference across branches.** Are `p'` and `s'` declared by the function author or inferred from inputs? If declared, the function is a fixed shape; if inferred, the inference rule itself must be portable.
- **Incomplete coverage.** What happens when one engine cannot implement the function at all (e.g. it requires a primitive the engine lacks)? Options: function is unavailable on that engine (call site error if the planner picks that engine), or function provides a `default` branch with documented lossiness.
- **Planner interaction.** The function's engine-dispatched bodies inform the planner: a caller of the function is compatible with engines for which a body exists. This is a form of *capability inference* through the call graph.
- **Composition.** If function `f` is portable across {duckdb, spark} and function `g` is portable across {spark, postgres}, what is the portability set of `f` composed with `g`? Intersection — only spark. Inference must compute this set across the program.

### 7.3 Why this lives in libraries, not the language

The language commits to *which operations are guaranteed equivalent without proof*. Libraries commit to *which operations are guaranteed equivalent with author-supplied evidence (typically property tests)*. The boundary lets the language stay small and the library ecosystem stay rich; it also gives different stakeholders different contracts to evaluate. A user reading the language reference knows what is guaranteed unconditionally; a user importing a polyfill library knows what is guaranteed *modulo the library's test discipline*.

This rhymes with the meta-language layering in `20260507-typed-meta-programming.md`: the language provides the typed primitives, library functions extend the practical surface, neither layer hides its boundary.

## 8. Deferred Questions

Recorded here so the position in §5–§6 is not relitigated under their pressure.

- **Literal-denominator carve-outs.** Whether portable code can divide by an exactly-invertible decimal literal (powers of 2 and 5, equivalently: literals whose reciprocal terminates in decimal). The argument for: covers the common analytics cases (`/100`, `/10`, `/1000`, percentages, basis points) without forcing an engine declaration. The argument against: introduces a special-case rule the user must learn, and the same effect can be achieved with explicit multiplication (`x * 0.01`) or an engine declaration. **Current position: no exceptions.**
- **A portable `smelt.divide` stdlib function.** Conceptually clean (§5.3, §7); design-cost real. Defer until the type-system support for engine-dispatched function bodies is itself designed.
- **NaN representability for Postgres-bound models.** Postgres supports `NaN` in `NUMERIC`; the other engines do not. If smelt has a `Decimal` value in a Postgres-bound model, should the type carry a `Nullable | NaN-able` flag? Probably yes; not designed here.
- **Maximum-precision policy beyond 38 for Postgres-bound models.** Postgres supports unbounded `NUMERIC`. A Postgres-bound model could in principle declare `NUMERIC` (no parameters) and have access to the unbounded type. Whether this is exposed as a distinct smelt type (`NumericUnbounded`?) or as a flag on `Decimal` is undesigned.
- **Modulo and integer division.** Out of scope here; deserve a parallel analysis if the position in §6 is adopted.
- **Date/time arithmetic.** The framing of §5 (portable surface, library polyfills, engine-binding) is likely portable to other type-family-changing operations. Date/time arithmetic across engines is a plausible second case study.

## 9. What Changes from Current smelt

If the position is adopted, the following concrete changes follow.

### 9.1 In `type_inference/binary.rs`

- Result of `Decimal +/-/*` becomes `Decimal(p', s')` computed by Spark's growth formulas from input precision/scale, not the current fixed `Decimal(38, 10)` placeholder.
- The check `p' <= 38` is enforced at compile time; violation produces a diagnostic asking the user to cast inputs down.
- `Decimal / Decimal` produces a diagnostic — "decimal division is not portable; either cast inputs to `Double` or declare an engine on this model."
- Inside an engine-bound model, the existing per-engine behaviour applies (e.g. `Double` result on DuckDB).

### 9.2 In `divergences.rs`

- `decimal_division` becomes irrelevant for portable code (the operation is no longer allowed). For engine-bound DuckDB code, smelt would natively produce `Double`, matching the engine — no divergence to record.
- `sum_integer`, `avg_decimal`, `sign_decimal`, `abs_decimal*` are unaffected by this paper; they belong in a separate analysis of aggregate and unary-function inference.

### 9.3 In the spec surface

- A new `docs/specs/decimal.md` (or a section of `docs/specs/types.md`) describing the portable contract, engine-declaration syntax, and the rejection of `Decimal / Decimal` in portable code.
- Worked examples in user docs showing the three remedies (cast-to-double, engine declaration, future polyfill function).

### 9.4 In the property-test suite

- Property tests would need to *not generate* `Decimal / Decimal` expressions in the portable-code generator. Engine-bound generators (per engine) would generate it freely and assert engine-native result types.
- New tests verifying that the `p' <= 38` compile-time check fires on overflowing arithmetic.

### 9.5 In the deployment contract

- Spark sessions running smelt models must enable ANSI mode (`spark.sql.ansi.enabled = true`). Documented as a deployment requirement. Smelt's Spark backend should refuse to run if the session is in legacy mode.

## 10. Summary

- The three reference engines diverge on decimal defaults, max precision, growth formulas, division result-type family, overflow handling, and NaN.
- Smelt today has a `Decimal { precision, scale }` type but does not compute output precision/scale through arithmetic and tolerates a `Decimal` vs `Double` divergence with DuckDB on division.
- The proposed position: portable language surface offers `Decimal(p, s)` with Spark-style growth formulas, statically capped at precision 38, with `/` excluded entirely. Engine-bound models get native semantics. Library functions with engine-dispatched bodies are the long-term ergonomic answer; the language preserves the design room without committing to ship one now.
- The headline decision: **no `Decimal / Decimal` in portable code; declare an engine to divide.**
- Literal-denominator carve-outs, NaN, unbounded `NUMERIC`, modulo/integer division, and the polyfill stdlib are deferred.

## 11. References

- `crates/smelt-db/tests/prop_helpers/divergences.rs` — current divergence registry (decimal entries enumerated in §4.4).
- `crates/smelt-db/src/type_inference/binary.rs` — current binary-op inference; the placeholder `Decimal(38, 10)` result is here.
- `crates/smelt-db/src/type_inference/literal.rs` — decimal literal inference.
- `docs/research/20260507-typed-meta-programming.md` — parent stylistic reference; the language/library layering in §5 rhymes with the meta-world/data-world split there.
- Postgres docs: [Numeric Types](https://www.postgresql.org/docs/current/datatype-numeric.html).
- Spark docs: [Decimal type](https://spark.apache.org/docs/latest/sql-ref-datatypes.html), [Arithmetic Operations](https://spark.apache.org/docs/latest/sql-ref-functions-builtin.html).
- DuckDB docs: [Numeric Types](https://duckdb.org/docs/sql/data_types/numeric).
