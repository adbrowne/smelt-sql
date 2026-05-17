# Engine-Branched Functions: Cross-Engine Polyfills with Engine-Dispatched Bodies

**Date:** May 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper develops a mechanism for smelt functions that present a single portable signature to callers while dispatching internally to engine-specific bodies. The motivating use case is the `Decimal / Decimal` divergence catalogued in `20260516-decimal-type-system.md`, but the mechanism is general: anywhere two or more reference engines diverge on a primitive that users repeatedly reach for (date arithmetic, regex, string null handling, percentile interpolation, JSON path), the library layer should be able to bridge the gap without forcing the call site to declare an engine. The paper's central proposal is that smelt functions gain an *engine-block* form in which each block contributes a body for one named engine; the type system checks each block against the function's declared return type independently; and the set of engines a function supports flows through the call graph as a *capability set* that constrains and informs the planner. The paper is a sibling to the decimal position — that paper decides what is excluded from the portable language surface; this paper sketches the library-level mechanism that, over time, lets carefully-authored functions reintroduce excluded operations under a different contract.

## 0. Authoring Context

> For a continuing reader (the author or any reviewer picking this up in a fresh session). Not part of the design — provenance and confidence so the next person can pick up the thread.

**Origin.** Drafted on 2026-05-17, the day after `20260516-decimal-type-system.md`. The decimal paper closes with §7 ("The Function-as-Polyfill Mechanism") flagged as Future Direction and sketches a one-screen function shape; it explicitly defers the full design. Andrew asked for that mechanism to be developed in its own paper and landed before tackling the decimal spec, on the grounds that the polyfill story changes the framing of which exclusions are tolerable on the portable surface.

**Status.** Research / design exploration. The function syntax shown in §3 is illustrative, not committed surface. The capability-set framing in §5 is the load-bearing idea; the rest is consequences and examples. The connection to the existing `smelt.functions` design (`20260413-smelt-functions.md` §5 Black Box Functions) is intentional — engine-branched functions are the natural extension of black-box signatures from "no body" to "N bodies, one per engine."

**Author confidence (subjective — for a reviewer's calibration):**

- **High** — §1 (motivation and the inventory of cross-engine divergences worth polyfilling), §5 (capability inference as set intersection), §7 (composition), and the framing that equivalence is author-supplied evidence, not a language guarantee.
- **Medium** — §3 (concrete syntax — there are several plausible shapes; the engine-block form is favoured but not the only candidate), §6 (planner interaction — the integration with model-level engine declarations is sketched but not fully reconciled with `engine: auto` semantics).
- **Low** — §8 (equivalence testing framework — only sketched), §9 (default branches and lossy branches — these are real ergonomic pressures but the design space is genuinely open).

**Highest-leverage open decisions** (mirrored in §11):

- **Whether the type system has a notion of "lossy" branches.** A `divide` polyfill whose Postgres branch rounds and whose Spark branch doesn't has *different* answers, not slightly different ones; calling it from portable code papers over a real semantic gap. The choices are: refuse to ship lossy branches (caller must declare engine), allow them but mark the function with a different effective type (e.g. `divide` is exact, `divide_approx` is lossy), or allow them silently and trust the author.
- **Whether engine sets are open or closed.** If smelt later adds Snowflake, do existing functions automatically become unavailable on Snowflake until a branch is added, or does smelt's engine list freeze for compatibility? This affects who owns the upgrade obligation.
- **Whether the planner gets cost hints per branch.** Two engines may both *correctly* implement a function but at wildly different cost. Without a hint, the planner picks blindly; with one, the library author is on the hook for performance estimates that may not generalise.

**Background a continuing reader should pre-read** before extending this paper:

- `docs/research/20260413-smelt-functions.md`, especially §5 (Black Box Functions) and §6 (Context Bindings). This paper assumes the function model from that paper and extends its body system.
- `docs/research/20260516-decimal-type-system.md`, especially §5 (Behavioural Equivalence) and §7 (Function-as-Polyfill sketch).
- `docs/specs/architecture.md` for the planner / engine-selection model as currently spec'd.

## 1. Motivation

The decimal paper makes a clean argument that smelt's portable language surface should expose only operations that are *provably equivalent* across the reference engines, and that everything else either requires an explicit engine declaration on the model or comes from a library function whose engine-specific bodies are checked to produce equivalent results. That argument is only as good as the library mechanism. If smelt has no way to *write* a portable function whose internals are engine-specific, the portable surface stays painfully thin in practice and users default to engine-bound models for everything — which collapses the multi-backend pitch.

Decimal division is the cleanest example but is far from the only one. The same shape recurs throughout cross-engine SQL:

- **Date arithmetic.** `DATEDIFF`, `DATE_ADD`, month/quarter boundary handling all differ. Spark's `MONTHS_BETWEEN` returns a fractional double; DuckDB's `date_diff('month', ..., ...)` returns an integer with rounding-toward-floor. Postgres has neither — month differences are computed manually from `EXTRACT`.
- **Regex.** Engines split between POSIX (Postgres, DuckDB) and PCRE-leaning (Spark) variants; even within "POSIX" the metacharacter set varies. A portable `matches(s, pattern)` function is not absurd if it commits to a restricted regex subset and dispatches to each engine's operator.
- **String null handling.** `CONCAT` is null-propagating in some engines and null-eliding in others; `||` behaves differently again. A portable `concat_safe` could pin the semantics.
- **Percentile and quantile estimation.** `approx_percentile` exists in several engines with different algorithms (t-digest vs GK-sketch). Exact equivalence is not achievable; *bounded approximate* equivalence is.
- **JSON path.** `JSON_EXTRACT`, `->`, `->>`, and JSONPath dialect differ widely. Modest portable functions for "extract scalar" or "extract array" cover most analytics use.

In every case the divergence is well-understood, the user-facing operation is conceptually portable, and the bridge code is mechanical. What's missing is the language-level mechanism for an author to *write* the bridge once and have callers consume it as a normal portable function. That mechanism is the subject of this paper.

This mechanism also has a second purpose beyond library code: it gives smelt a principled relaxation path for portable-surface exclusions. The decimal paper's "no portable `Decimal / Decimal`" position is sustainable in part because the design room for `smelt.divide(a, b)` is preserved. If engine-branched functions land, the decimal exclusion does not have to be revisited — it gets a library answer instead.

## 2. Where this fits in the existing function design

`docs/research/20260413-smelt-functions.md` introduces smelt functions as parameterised fragments with portable signatures. §5 of that paper introduces *black box functions* — signatures without bodies, used to bind opaque engine primitives into the type system. Engine-branched functions are the natural midpoint between the two: a *portable signature* with *multiple bodies*, one per engine, each one effectively a black-box body for that engine.

The taxonomy from §4 of the functions paper extends as:

| Function form | Body shape | Portability | Audited by |
|---|---|---|---|
| Single-body function | One smelt-language body | Inherits from the body's primitive set | Type system (mechanical) |
| Black-box function | No body, just a signature | Wherever its primitive is available | Author (declaration) |
| **Engine-branched function** | **N bodies, one per engine** | **Intersection of declared engines** | **Author (per-branch type check + equivalence claim)** |

The third row is what this paper proposes. The mechanism does not replace either of the existing forms; it extends the language with a third authoring option that sits naturally alongside them. Existing single-body functions written in the portable subset remain portable for free; existing black-box functions remain available on their declared engines. Engine-branched functions are the right tool only when (a) the operation is genuinely available on more than one engine, (b) the engines disagree on the primitive's syntax or semantics, and (c) the author can defend an equivalence claim across the branches.

## 3. Surface syntax (illustrative)

The syntax shown here is a concrete-enough form to discuss; the design space includes alternatives that are not analysed in this paper.

```smelt
fn divide(a: Decimal(p, s), b: Decimal(p, s)) -> Decimal(p, s) {
  engine duckdb   { CAST(a / b AS DECIMAL(p, s)) }
  engine spark    { a / b }
  engine postgres { ROUND(a / b, s) }
}
```

The function header is identical to a single-body function. The body is an *engine block list* in which each block contributes a body for one named engine. Each block's expression is parsed in the target engine's SQL dialect (or in a portable superset that includes that engine's operators); smelt's type checker treats each block as an independent obligation to produce a value of the declared return type.

Important properties of the form:

- **Shared generic parameters.** `p` and `s` are bound once at the function header and are visible in every branch. The function is not three separate functions joined by union — it is one function with one signature.
- **No fall-through.** The blocks are exhaustive on their listed engines, not a chain of conditionals. There is no `else` branch; an engine not listed is not supported (see §9 for the open question of `default`).
- **No engine-conditional expressions inside a single body.** Mixing `CASE WHEN engine() = 'duckdb' …` inside one body is explicitly *not* the chosen form: it scales badly, it confuses the type checker, and it conflates two kinds of branching (data-dependent vs engine-dependent). The engine-block list separates them at the syntactic level.

An alternative considered and not adopted: a single body parameterised over an *engine module* import (`use engine`; then `engine.divide(a, b)`). This pushes dispatch into the type system through implicit traits; it loses the property that each branch is a plain SQL expression in the target dialect, which is the main thing library authors actually want to read.

## 4. Type system

Each engine block must produce a value of the function's declared return type, checked independently in that engine's type environment. The function as a whole has the declared signature; the branches share inputs and output type but are otherwise unrelated for type-checking purposes.

Two subtleties:

**Width tightening.** Engines often have looser native return types than the declared one. Spark's `Decimal / Decimal` grows precision according to its formula; if the function declares `Decimal(p, s)` and the Spark branch writes `a / b` directly, the actual Spark result is wider. The type checker rejects that branch and requires the author to insert a cast (`CAST(a / b AS DECIMAL(p, s))`) or to declare the wider return type. This is *exactly* the discipline the polyfill is meant to enforce: width discipline at the function boundary is what makes the function portable for callers.

**Branch-local diagnostics.** Type errors in the Spark branch should not block callers from running the function on DuckDB. Smelt has two reasonable behaviours: (a) require all declared branches to type-check before any caller can use the function (strict — the function is either complete or absent); (b) admit partially-broken functions and surface a planner-level error only when the planner tries to pick a broken branch (lenient — useful while editing). The strict behaviour is simpler and is the current author's preference; the lenient behaviour is friendlier in an LSP context. Either is defensible; this paper does not commit.

The function's *signature* — its declared parameter and return types — is what callers see and reason about. No part of the engine list bleeds into the signature visible at the call site. The capability set (which engines support the function) is a separate piece of metadata, discussed next.

## 5. Capability inference

Every function has a *capability set*: the set of engines on which it can run. For an engine-branched function, that set is the engines listed in its body. For a single-body function (no engine blocks), the set is the engines on which its primitives are all available — typically every engine, if it stays inside the portable subset, but smaller if it uses any black-box function with a narrower capability.

Capability sets propagate through the call graph by intersection:

```
capability(f) = ⋂ { capability(g) | g is called transitively by f's body }
            ∩ engines_listed_by(f)         -- if f is engine-branched
```

For a model, the capability set is the intersection of the capability sets of every function it calls, combined with any explicit `engine:` declaration on the model itself. The result is the set of engines on which the model can run. If that set is empty, the model is incoherent — it uses functions whose engine support is disjoint — and the compiler reports a precise error that names the conflicting functions and their respective engine sets.

This is the same set-intersection inference that powers, for example, Haskell's typeclass constraints — but here the "type" being inferred is the *engine compatibility* of the program, and the set is concrete (typically of size 1–3) rather than abstract.

## 6. Planner interaction

The planner's job is to pick an engine for each materialisation boundary. Today (see `docs/specs/architecture.md`), the planner reads an engine declaration off the model — or falls back to a default — and emits SQL in that dialect. Engine-branched functions change two things.

**The model's compatible engine set is now a constraint, not a declaration.** A model without an explicit `engine:` declaration *infers* a capability set from its body (per §5). The planner is free to pick any engine in that set; if the user wants to pin the choice, they add `engine: duckdb` and the planner verifies the model's capability set includes `duckdb`.

**Engine choice may differ per model in a pipeline.** Two models that share data can land on different engines, with the planner emitting a cross-engine transfer between them (parquet exchange — see project memory: Spark writes parquet, DuckDB reads it, no copy step). The capability set of each model is local; the planner's global plan is the assignment of engines to models that minimises some cost while respecting all capability constraints.

This is a meaningful generalisation of the current planner, but it is *additive*: a model that declares an engine and uses only single-body functions written in the portable subset behaves exactly as it does today. The planner only consults capability sets when it has degrees of freedom.

## 7. Composition

Capability sets compose by intersection through the call graph:

- A function `f` that calls only portable primitives has `capability(f) = {all engines}`.
- A function `g` with engine blocks for `{duckdb, spark}` has `capability(g) = {duckdb, spark}`.
- A function `h(x) = g(f(x))` has `capability(h) = capability(g) ∩ capability(f) = {duckdb, spark}`.
- A function `k(x) = g(x) + p(x)` where `capability(p) = {spark, postgres}` has `capability(k) = {spark}`.

A compilation error fires when the intersection narrows to the empty set anywhere in the call graph. The error message should name the call site and the two functions whose intersection failed, not just report "no compatible engine" at the model level — the localisation matters for actionable feedback.

This propagation is straightforward set inference; the engineering work is in the LSP and diagnostic surfaces. A user editing a function should see, in the LSP hover, the current capability set of that function. A user editing a model should see which engines the model can run on. Both should update incrementally as edits land.

## 8. Equivalence evidence

The mechanism guarantees that each branch produces a value of the declared return type. It does *not* guarantee that the branches produce the same value on the same input. That equivalence is the function author's responsibility.

Provisional discipline:

- **Property-test colocation.** Every engine-branched function ships with a property test in its module that drives all branches with shared inputs and asserts equality. The test failure is the public signal that the function is broken.
- **Smelt-provided harness.** A helper like `smelt.test.equivalence(fn_ref, generator)` that runs the function on every engine in its capability set against shared inputs and compares results. The generator is type-directed (similar to the existing property-test infrastructure in `crates/smelt-db/tests/type_property_tests.rs`).
- **Engine availability for tests.** Equivalence tests need every supported engine present in CI. This is already the implicit assumption for the existing property tests against DuckDB; extending the harness to multi-engine widens the CI matrix non-trivially. Pragmatically, library functions may have to declare their *intended* capability set and have CI enforce that all declared engines are testable.

This is not a hard guarantee. It is a discipline that, if followed, makes equivalence claims defensible. The language deliberately stops short of trying to prove equivalence statically — that would require an engine-by-engine formal semantics that does not exist for any of the candidate backends.

## 9. Default branches and lossy branches

Two design pressures push against the strict "exact equivalence, no fallback" framing:

**A `default` branch.** Some functions are "mostly portable" — they have a clean DuckDB and Spark implementation, but Postgres lacks the primitive and the function is impossible there without a wholesale rewrite. A `default` branch would let the function ship anyway, with a documented warning that on Postgres the function does X instead of Y. The argument against: this hides the divergence behind a function call. The caller has no way to know that on Postgres they're getting a different operation. Provisional position: **no `default` branch in v1**. Functions are unavailable on engines without an explicit body. Users wanting fallbacks construct them at the call site or in their own wrappers.

**Lossy branches.** Some operations are approximately equivalent across engines (`approx_percentile` is the canonical case). Insisting on exact equality discards the entire category. Options:

- *Refuse to ship lossy branches.* All polyfills are exact; "approximate" library functions are out of scope.
- *Allow lossy branches with a marker.* The function carries a `lossy` flag in its signature, visible to callers; calls from portable code generate a warning unless the caller acknowledges lossiness with a syntactic marker (`divide.lossy(a, b)` or similar).
- *Allow lossy branches silently.* Trust authors; document the tolerance in the function's user-facing description.

Provisional position: **investigate the `lossy` marker option but do not commit to it in v1**. Initial polyfill library ships only exact functions; the lossy/exact distinction is reopened when concrete pressure (e.g. percentile demand) makes it acute. This keeps the initial design surface small while not closing the door.

## 10. Worked examples

### 10.1 `divide` — the decimal motivator

Discussed in §3 and in `20260516-decimal-type-system.md` §7. The exact form depends on the literal-denominator carve-out question deferred in the decimal paper; the polyfill mechanism is agnostic.

### 10.2 `months_between`

```smelt
fn months_between(a: Date, b: Date) -> Integer {
  engine duckdb   { date_diff('month', b, a) }
  engine spark    { CAST(MONTHS_BETWEEN(a, b) AS INT) }
  engine postgres {
    (EXTRACT(YEAR FROM a) - EXTRACT(YEAR FROM b)) * 12
    + EXTRACT(MONTH FROM a) - EXTRACT(MONTH FROM b)
  }
}
```

Equivalence claim: integer-valued months between two dates, floored toward zero, defined for any date inputs. Property test runs all three branches on a generator of `(Date, Date)` pairs and asserts equal results. The cast inside the Spark branch is the width-tightening discussed in §4.

### 10.3 `matches`

```smelt
fn matches(s: Text, pattern: Text) -> Boolean {
  engine duckdb   { regexp_matches(s, pattern) }
  engine spark    { s RLIKE pattern }
  engine postgres { s ~ pattern }
}
```

Equivalence claim: the function is portable *only* over a documented regex subset (e.g. ASCII character classes, basic quantifiers, no lookaround). The author's property test enumerates patterns from that subset; the documentation states the subset. Patterns outside the subset are an author-responsibility error, not a smelt error.

### 10.4 `json_extract_text`

```smelt
fn json_extract_text(j: Json, path: Text) -> Text {
  engine duckdb   { json_extract_string(j, path) }
  engine spark    { get_json_object(j, path) }
  engine postgres { j #>> string_to_array(trim(both '$.' from path), '.') }
}
```

The Postgres branch demonstrates that the polyfill mechanism can include real translation work — the function's body in each engine is whatever it takes; the obligation is only that the result types match. The portable surface offered to callers is uniform.

## 11. Deferred questions

- **Lossy vs exact branches.** (§9.) The cleanest v1 ships only exact polyfills; the lossy story is real but reopened later under concrete pressure.
- **Default branches.** (§9.) Same disposition.
- **Engine-version targeting.** Some engine primitives are version-gated (DuckDB 1.5+, Spark 3.4+). Whether engine names in branches are sufficient or whether `engine duckdb >= 1.5` is needed depends on how stable the supported-engine matrix is in practice. Defer.
- **Open vs closed engine sets.** If smelt later adds Snowflake, do existing functions auto-fail on Snowflake, or does the library author have to add a branch before users can target Snowflake? Affects who owns the upgrade obligation and whether old library code is forward-compatible. Defer.
- **Cost hints per branch.** Two branches can both be correct but at wildly different cost (e.g., Postgres implementing a sketch operation with a window function vs Spark using a native sketch). The planner may want hints. Defer — this is a planner-side concern that doesn't change the function design.
- **LSP capability surfacing.** The LSP should show capability sets in hover and explain compatibility errors at the call site. Design is mechanical once the inference rule is fixed; not detailed here.
- **Interaction with `smelt.functions.as_struct` and parameter contexts.** Engine-branched functions interact with the parameter-context system from `20260413-smelt-functions.md` §6 in ways this paper has not analysed. Likely no conflict; verify before implementation.

## 12. What this paper does not do

- It does not commit to specific syntax for engine blocks (§3 is illustrative).
- It does not design the equivalence-testing harness in detail (§8 is provisional).
- It does not commit to shipping any specific stdlib polyfill, including the `divide` function that motivates the entire investigation.
- It does not decide between strict (whole-function type-check) and lenient (per-branch) checking (§4).
- It does not specify the LSP affordances around capability sets (§5, §7).
- It does not interact with the planner's cost model.

It commits to one thing only: that the *shape* of the mechanism — portable signature, N engine-dispatched bodies, capability inference by set intersection through the call graph — is the right shape, and that the smelt function design has room for it as a natural extension of §5 (Black Box Functions) from `20260413-smelt-functions.md`.

## 13. Summary

Engine-branched functions are the library-level mechanism that makes the decimal paper's "narrow portable surface" position liveable. The mechanism is: a function declares a portable signature and one body per supported engine; each body is type-checked independently to produce the declared return type; the set of engines the function supports propagates through the call graph by intersection and constrains the planner's engine choice. The author claims equivalence between branches and defends it with property tests; the language does not attempt to prove equivalence statically. The result is a clean separation: the language guarantees only what it can mechanically verify, and the library ecosystem extends the practical surface under an explicit equivalence discipline. The mechanism is open-ended enough to absorb future divergences (date arithmetic, regex, JSON) using the same shape, without further language changes.

## 14. References

- `docs/research/20260413-smelt-functions.md` — function design; §5 Black Box Functions is the direct predecessor of this mechanism.
- `docs/research/20260516-decimal-type-system.md` — motivating divergence; §5.3 and §7 sketch the polyfill role this paper develops.
- `docs/research/20260507-typed-meta-programming.md` — the layering principle (language guarantees vs library extensions) carried into the function system.
- `docs/specs/architecture.md` — planner and engine-selection model; this paper extends it with capability-set inference.
