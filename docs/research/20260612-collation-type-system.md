# Collation Across Engines: A Behavioural-Equivalence Type-System Position for smelt

**Date:** June 2026
**Status:** Research / Design Exploration
**Author:** Andrew Browne, with design input from Claude

This paper investigates how *collation* — the rules that decide string comparison, ordering, and **equality** — should be modelled in smelt's type system, given that the three reference engines (Postgres, Spark, DuckDB) model collation in fundamentally incompatible ways: Spark 4.0 makes collation a field of the string *type*, Postgres derives it as an *expression property* via SQL-standard coercibility rules, and DuckDB attaches it as a `COLLATE` *annotation* with a small built-in set plus an ICU extension. The central claim — held more tentatively than the sibling decimal paper, and offered as a survey-led lean rather than a commitment — is that smelt's *portable language surface* should expose only **binary collation** (byte-wise comparison: `UTF8_BINARY` on Spark, `BINARY`/default on DuckDB, `COLLATE "C"` on Postgres), because that is the one collation whose semantics are bit-identical and version-stable across all three engines. Everything else — locale-aware ordering, case- and accent-insensitive comparison — either requires an explicit engine declaration on the model or is provided as a library function with a portable signature and an engine-dispatched body.

The decision that distinguishes collation from the other three type-system axes is that **collation changes equality, not just sort order**. A case-insensitive collation makes `'A' = 'a'`, which changes the result of `GROUP BY`, `DISTINCT`, joins, and `MIN`/`MAX` — it changes the *rows a model emits*, not merely their presentation order. That makes collation a first-class concern for the output-fingerprint / virtual-environments equivalence relation, and it is collation's analogue of "`Decimal / Decimal` is not portable": the place where *same printed type ≠ same values*.

The centrepiece of this paper is therefore not the portable-surface decision (which follows the decimal template) but the **axis-placement question**: does collation belong in `DataType` (value-domain, like timezone and decimal precision) or beside it in `TypedColumn` (column-population, like nullability)? The three engines themselves disagree, and smelt's answer is genuinely unsettled.

## 0. Authoring Context

> For a continuing reader (the author or any reviewer picking this up in a fresh session). Not part of the design — provenance and confidence so the next person can pick up the thread.

**Origin.** Drafted on 2026-06-12 out of a brainstorming conversation with Claude, immediately after the timezone axis landed. Collation is the fourth and last of the type-system axes the roadmap sequences before Virtual Environments (`nullability → decimal → timezone → collation`); unlike the first three it had no prior research doc, and the roadmap explicitly calls for one "before design begins." This paper is that doc. It is deliberately modelled on `docs/research/20260516-decimal-type-system.md` — same three-layer position (strict portable surface, engine-binding escape hatch, library polyfills) — because the framing transferred cleanly.

**Status.** Research / design exploration. Nothing here is committed surface. By explicit choice this paper is **survey-led**: it leans toward the binary-only portable surface (§5) and toward the `TypedColumn` placement (§6) but commits to neither — the final portable-surface contract and the placement decision are left for the spec cycle. The decimal paper could land a sharp position because its division decision had been pressure-tested in conversation; collation has not, and its defining hazard (equality-affecting collation, §7) deserves to be understood before the surface is frozen.

**Author confidence (subjective — for a reviewer's calibration):**

- **High** — §2's structural claims (Spark puts collation in the type; Postgres uses coercibility; DuckDB uses a `COLLATE` annotation with a "cannot combine" rule), §4 (current smelt state — grounded in code and spec), §7 (the equality-vs-ordering distinction and its fingerprint consequence — this is the load-bearing observation).
- **Medium** — §2's per-collation *naming* and exact semantics (Spark's `UTF8_LCASE` vs `UNICODE_CI` boundary; DuckDB's `NOCASE`/`NOACCENT`/`NFC` combinability rules; Postgres deterministic/nondeterministic mechanics), §3's intersection cells, §5 (the contract framing transferred from decimal but is less battle-tested here).
- **Low** — §6's recommendation (the placement lean is argued both ways and the recommendation is weak by design), §8 (function-as-polyfill — inherited sketch; collation adds its own open questions about which collation a result carries), the claim that binary collation is *bit-identical* across engines for all Unicode inputs (true for byte-wise comparison of identical encodings; worth an empirical probe across non-ASCII / mixed-normalisation inputs).

**Highest-leverage open decisions** (mirrored in §9):

- **Axis placement (§6).** Value-domain (`DataType`, Spark's model) vs column-population (`TypedColumn`, Postgres's model). This is *the* decision the paper exists to surface; it determines how invasive collation tracking is across inference.
- **The binary-only portable surface (§5).** Whether portable code is restricted to byte-wise collation, with everything else requiring an engine declaration.
- **Default-collation pinning on Postgres (§5.1, §9).** Postgres's default collation is the database locale, *not* binary. Unless smelt pins `COLLATE "C"` on emission, "portable" code silently inherits a locale-dependent comparison on Postgres. This is a real correctness hazard with no analogue in the decimal paper.
- **Equality-affecting collation & the fingerprint fold (§7).** How the output-fingerprint oracle tracks collation so that `fingerprint-equal ⇒ DuckDB relations identical` survives collation differences.

**Background a continuing reader should pre-read** before extending this paper:

- `docs/research/20260516-decimal-type-system.md` — the sibling paper. The three-layer position (§5), the polyfill mechanism (§7→§8 here), and the "refuse rather than approximate" argument all transfer. Read it first.
- `docs/specs/types.md` — §4 (String unification: `Text`/`Varchar`/`Char` are interchangeable, length discarded, string functions return `Text`), §Design ("Axis placement" paragraph, which already names collation as a tentative `TypedColumn` axis), §Constraints ("collation tracking on `Text`" is listed out of scope for v1).
- `docs/specs/output_fingerprint.md` — lists collation among the type-system axes "not folded" into the fingerprint; each is breaking-by-default (conservative rebuild) until tracked and oracle-covered. This is the consumer that makes collation precision pay for itself.
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — the divergence registry. Today it has `string_concat` and `string_functions` entries (smelt infers `Text`, backends return `Varchar`/`String`) but **nothing collation-related**, because smelt emits no `COLLATE` and silently assumes binary everywhere.
- `crates/smelt-types/src/lib.rs` — the `DataType` enum (no collation field today) and `TypedColumn` (carries `nullable: bool`, no collation).

## 1. Motivation and Scope

### 1.1 Why collation is interesting

For a multi-backend pipeline tool aiming at *behavioural equivalence across engines*, every type and operator raises the question: does the engine give the same answer? For string **comparison**, the answer depends entirely on collation, and the three engines diverge on almost every axis of it:

- They disagree on **where collation lives** — in the type (Spark), as a derived expression property (Postgres), or as a `COLLATE` annotation (DuckDB).
- They disagree on the **default** collation — binary on Spark and DuckDB, but the *database locale* on Postgres.
- They disagree on the **vocabulary** of named collations and on the Unicode/ICU version that backs them, so "the same" locale collation can sort differently on two engines.
- They disagree on what happens when **two differently-collated strings combine**.

Crucially, collation is not a presentation detail. It governs `=`, `<>`, `<`, `>`, `ORDER BY`, `GROUP BY`, `DISTINCT`, `JOIN` keys, `LIKE`/pattern matching, `MIN`/`MAX`, and index/dedup behaviour. Because it governs **equality**, a collation change can change *which rows a model produces* — not just their order. That is what makes collation both a real correctness gap and a precision blocker for the output fingerprint (§7).

### 1.2 Scope

This paper covers:

- A per-engine survey of how collation is modelled and what it affects (§2).
- A per-axis divergence inventory (§3).
- The current state of string handling in smelt, grounded in the code and spec (§4).
- A proposed (survey-led, leaning) type-system position for portable collation (§5).
- **The axis-placement question — value-domain vs column-population — as the centrepiece (§6).**
- The equality-vs-ordering distinction and its consequence for the output fingerprint (§7).
- A sketch of the function-as-polyfill mechanism for future portable non-binary collation (§8).
- Deferred questions (§9) and what changes from current smelt (§10).

Out of scope:

- **Downstream propagation of engine-binding.** As in the decimal paper, whatever smelt decides here follows the normal smelt rules about engine-constraint propagation up to a materialisation boundary; this is not a collation-specific decision.
- **The full collation vocabulary smelt should expose in engine-bound models.** §5.2 says engine-bound models get native collation; it does not enumerate a portable named-collation set.
- **Empirical re-verification of current engine behaviour.** Several claims (DuckDB's combinability matrix, Spark's `UTF8_LCASE`/`UNICODE_CI` semantics, the exact set of operations Postgres forbids under nondeterministic collations) deserve a small probe script per engine. Recommended as follow-up, not done here.
- **Text encoding** (UTF-8 vs UTF-16 vs Latin-1). All three reference engines are UTF-8 internally; encoding is assumed fixed and is a separate concern from collation.

## 2. The Three Engines — Survey

### 2.1 Postgres — collation as a derived expression property

- Postgres follows the **SQL-standard coercibility model**: collation is not part of the type, it is a *property derived for each expression*. Every column has a collation (defaulting to the database's locale), and the collation of an expression is computed from its inputs by coercibility rules (an explicit `COLLATE` clause wins; otherwise an implicit column collation propagates; combining two different *implicit* collations is a "collation mismatch" error the user must resolve with an explicit `COLLATE`).
- Two providers: **libc** (OS locales) and **ICU**. ICU is the richer, more portable provider.
- **Deterministic vs nondeterministic.** A *deterministic* collation considers strings equal only if their bytes are equal; a *nondeterministic* collation (ICU only, created with `deterministic = false`) can consider differently-encoded strings equal — case-insensitive, accent-insensitive, or different Unicode normal forms. Nondeterministic collations carry real costs: historically pattern matching (`LIKE`/regex) was disallowed (relaxed in recent versions), B-tree deduplication is disabled, and there is a measurable performance penalty.
- **Default collation is the database locale**, set at `initdb` time — typically *not* binary. `COLLATE "C"` (equivalently `"POSIX"`) is the byte-wise collation.
- Overflow/availability: a collation that does not exist in the database is an error.

### 2.2 Spark 4.0 — collation as a field of the type

- Spark 4.0 introduced collation as a **field on `StringType`** (`SPARK-46830`). Every `StringType` carries a collation; the default is `UTF8_BINARY`, which is byte-wise comparison identical to pre-4.0 string behaviour.
- Named collations include `UTF8_BINARY` (byte-wise), `UTF8_LCASE` (ASCII/UTF-8 lowercasing then byte compare), and ICU-backed locale collations such as `UNICODE` and `UNICODE_CI` (case-insensitive), plus locale variants (`en_US`, `en_US_CI_AI`, …).
- Because collation is in the type, it **participates in the type system**: collation is respected in comparisons, hashing, grouping, and collation-aware string functions (`contains`, `startsWith`, `endsWith`, `upper`/`lower`, trim, translate, …), and Spark defines **collation precedence** rules for combining differently-collated strings (analogous to coercibility, but resolved in the type).
- This is the model that most directly argues for smelt placing collation in `DataType` (§6).

### 2.3 DuckDB — collation as a `COLLATE` annotation

- DuckDB attaches collation via a `COLLATE` annotation on a column or expression; the default is **binary**. A session-wide default can be set with `SET default_collation = …`.
- The stand-alone build ships three collations: `NOCASE` (case-insensitive), `NOACCENT` (accent-insensitive), and `NFC` (Unicode normalisation). Region/language collations require loading the **ICU extension**.
- **Combining collations is restricted.** Comparing two values with different, incompatible collations raises `Cannot combine types with different collation`. `NOCASE` is special-cased as broadly combinable; most others are not. ICU collations can combine with `NOCASE`.
- DuckDB's collation support is the least mature of the three, and this matters: open issues show `NOCASE` producing surprising or incorrect results in `GROUP BY`, `UNION`, `INTERSECT`/`EXCEPT`, and `MIN`/`MAX` (which historically ignored collation entirely). These are concrete evidence that equality-affecting collation is fragile *even within a single engine* (§7).

## 3. Per-Axis Divergence Inventory

| Axis | Postgres | Spark 4.0 | DuckDB | Intersection |
|---|---|---|---|---|
| Where collation lives | derived expression property (coercibility); attachable per column | field on `StringType` (in the type) | `COLLATE` annotation on column/expr | none — three different models |
| Default collation | database locale (usually **not** binary) | `UTF8_BINARY` (binary) | binary | binary **only if** caller pins it |
| Byte-wise collation name | `COLLATE "C"` / `"POSIX"` | `UTF8_BINARY` | `BINARY` / default | **binary — the common ground** |
| Case-insensitive mechanism | nondeterministic ICU collation | `UTF8_LCASE` / `*_CI` | `NOCASE` | concept shared; Unicode semantics differ |
| Named locale collations | libc + ICU (`en-US-x-icu`, …) | ICU (`UNICODE`, `en_US`, …) | ICU extension | overlapping but not name-compatible |
| ICU/Unicode version backing | depends on libc/ICU build | bundled ICU | bundled ICU | **not version-stable across engines** |
| Combining different collations | mismatch error unless coercibility resolves | collation-precedence rules | "cannot combine" error (`NOCASE` special) | error-ish, but rules differ |
| Affects **equality** (`GROUP BY`/`DISTINCT`/join) | yes (nondeterministic) | yes (CI/AI) | yes (`NOCASE`) — and buggy in set ops | **yes — the dangerous axis** |
| Pattern matching under non-binary collation | restricted (improving) | engine-defined | engine-defined | not portable |

The intersection column is the candidate behavioural contract for smelt's portable surface. Only one row has a clean intersection: **binary / byte-wise collation**. Two rows are the language-constraining ones:

1. **Default collation has no intersection.** Spark and DuckDB default to binary; Postgres defaults to the database locale. So even *writing no `COLLATE` clause at all* is not portable — on Postgres it silently means "the locale collation." smelt cannot claim portability for string comparison without *pinning* binary on emission. This is the collation analogue of decimal's "Spark sessions must run ANSI mode" deployment contract, except it bites in the language surface, not just the runtime.
2. **Equality-affecting collation has no portable intersection beyond binary.** Case- and accent-insensitive collations exist on all three engines but under different names, different Unicode versions, and different combinability rules, so two engines asked for "case-insensitive" will not reliably agree on `'ß' = 'SS'` or accent folding. And because these collations change equality, the disagreement shows up as *different output rows*, not approximate values (§7).

## 4. Current State in smelt

Grounded in the code and spec on the `type_system` worktree as of 2026-06-12.

### 4.1 String type representation

`types.md` §4 (String unification): `Text`, `Varchar(max?)`, and `Char(len)` are interchangeable for type-equality — `normalize()` collapses `Text ↔ Varchar(None)`, string operations discard length annotations, and string functions (`UPPER`, `SUBSTRING`, `||`, …) all return `Text`. The `DataType` enum carries **no collation field**, and `TypedColumn` carries `nullable: bool` and no collation. There is, today, exactly one string-comparison semantics in smelt, and it is implicit.

### 4.2 What smelt emits

smelt emits no `COLLATE` clause anywhere, and `to_backend_sql()` emits `VARCHAR`/`STRING`/`TEXT` with no collation modifier. In practice this means:

- On Spark and DuckDB, string comparison is binary by default — smelt accidentally gets the portable answer.
- On Postgres, string comparison inherits the **database locale** — smelt accidentally gets a *non-portable, locale-dependent* answer. A pipeline whose DuckDB run treats `'a' < 'B'` as true (byte order: uppercase before lowercase) may see the opposite on a Postgres database created with an `en_US` locale.

So smelt's *de facto* current position is "binary by omission," but it is unsound on Postgres because it never pins binary. This is the concrete bug the binary-only position (§5) closes.

### 4.3 What the spec already says

The type system spec has already reserved collation's shape, tentatively:

- `types.md` §Design ("Axis placement"): *"Collation's placement is tentative until its own design cycle; its SQL coercibility rules suggest the column channel, like nullability."* — i.e. a lean toward `TypedColumn`, justified by Postgres's coercibility model.
- `types.md` §Constraints: *"collation tracking on `Text`"* is listed **out of scope for v1**.
- `output_fingerprint.md`: collation is among the "type-system axes not folded" into the fingerprint; "same printed type" does not yet imply "same values" for collation, so it is breaking-by-default (conservative rebuild) until tracked and oracle-covered. The axis ordering there is `nullability → decimal → collation`.

This paper is the design cycle those notes defer to. Note that the spec's lean toward `TypedColumn` predates the observation that Spark 4.0 models collation *in the type* — §6 re-opens the question with that evidence on the table.

### 4.4 Divergence registry

`divergences.rs` has `string_concat` and `string_functions` (smelt infers `Text`; backends return `Varchar`/`String`) but **no collation entries**, because smelt tracks no collation. Adopting any position here would add collation-aware entries (e.g. an engine-bound case-insensitive `GROUP BY` whose row count differs from binary).

## 5. Proposed Position: Binary as the Portable Collation (Survey-Led Lean)

The framing has three layers, mirroring the decimal paper, in order of decreasing strictness. This is offered as the recommended direction, not a frozen contract (per §0).

### 5.1 The portable language surface — binary collation only

The portable surface guarantees one string-comparison semantics on every engine: **byte-wise (binary) comparison**, case- and accent-sensitive, Unicode-normalisation-sensitive. Concretely:

- All portable `Text` is binary-collated. `=`, `<`, `ORDER BY`, `GROUP BY`, `DISTINCT`, joins, and `MIN`/`MAX` on `Text` are byte-wise and produce identical results on every engine.
- smelt **pins binary on emission**: it emits `COLLATE "C"` (or the per-column equivalent) on Postgres so the database locale cannot leak in; on Spark and DuckDB binary is already the default, but pinning `UTF8_BINARY` / `BINARY` makes the contract explicit and survives a session that changed `default_collation`. (The exact pinning mechanism — per-column DDL, per-comparison annotation, or session setting — is a deferred question, §9.)
- Declaring or using a **non-binary collation** in portable code (a `COLLATE "en_US"` expression, a case-insensitive comparison) is rejected with a diagnostic directing the user to the two remedies below.

The contract: well-typed portable smelt code produces the **same rows** on every engine, because string equality and ordering are byte-wise everywhere.

### 5.2 Engine-bound models

A model annotated with an engine declaration opts into that engine's native collation machinery for its duration:

- Non-binary collations are available with the engine's native semantics (Postgres ICU/libc collations, Spark `UNICODE_CI`, DuckDB `NOCASE`, …).
- The model's output is a materialised physical table; its string columns carry whatever collation the engine produced, and downstream portable consumers compare them binary unless they too are engine-bound. (Propagation up to the materialisation boundary follows normal smelt rules — out of scope, §1.2.)

### 5.3 Library functions with engine-dispatched bodies

The escape hatch (sketched in §8): a function with a *portable signature* and *engine-specific bodies* could expose, say, a portable case-insensitive equality whose cross-engine equivalence is the author's property-tested responsibility. This is the long-term ergonomic answer to "I need portable case-insensitive grouping." The paper leaves the design room open without committing to ship one.

### 5.4 Default user path

Most users who need locale-aware or case-insensitive comparison will declare an engine on the affected model. The binary-only portable surface is for library authors, genuinely cross-engine pipelines, and users who want the type system to enforce "this comparison is portable." As with decimal, portability is paid for explicitly — here by accepting byte-wise comparison or by reaching for an engine declaration.

## 6. The Axis-Placement Decision (Centrepiece)

Where does collation live: in `DataType` (a *value-domain* axis, like decimal precision and timezone-awareness, participating in equality/promotion/`CAST`), or beside it in `TypedColumn` (a *column-population* axis, like nullability, kept out of every type rule)? The engines themselves split on this, and so does the evidence.

### 6.1 The case for value-domain (`DataType`)

- **Spark 4.0 models it exactly this way** — collation is a field of `StringType`, and it participates in comparison, hashing, and function resolution. If smelt ever wants to track non-binary collation *portably* (via §8 polyfills), it needs collation to flow through inference the way Spark's does.
- Collation changes what comparisons *mean*, which feels like a property of the value domain, not merely of a column's population. `'A' = 'a'` being true or false is a semantic fact about the type's equality, analogous to how `Timestamp` vs `Timestamp WITH TIME ZONE` changes what equality and ordering mean.
- `COLLATE` is, in effect, a kind of cast — a value-domain operation — which argues for treating collation like the other `CAST`-participating axes.

### 6.2 The case for column-population (`TypedColumn`)

- **Postgres / the SQL standard model it this way** — collation is a derived *property of an expression/column*, computed by coercibility rules, not a component of the type. `NOT NULL` is the precedent: a column constraint, not a type.
- The same argument that put nullability on `TypedColumn` applies: putting collation in `DataType` forces every `match` on `DataType` and every unification/promotion rule to answer "what about the collation?", an invasive change. `TypedColumn` already flows everywhere inference goes.
- **If the portable surface is binary-only (§5), collation is near-vestigial in portable code** — it is a constant (binary) everywhere, so a coarse `TypedColumn` marker (`Binary | EngineCollated(name)`) suffices, and full value-domain richness only matters inside engine-bound models, where the engine's own type system already tracks it. Paying the `DataType`-invasion cost to track an axis that portable code holds constant is a poor trade.

### 6.3 Interaction with the binary-only position, and a weak recommendation

The two decisions are coupled. *If* the portable surface is binary-only, the placement question loses most of its force in portable code, because collation is a constant there; it becomes load-bearing only in engine-bound models, whose outputs are materialised tables that smelt reads back as ordinary columns. That argues for the **minimal `TypedColumn` coarse marker now** — consistent with the spec's existing lean — deferring full value-domain richness until and unless portable *non-binary* collation is wanted (which needs the §8 polyfill mechanism anyway).

**Tentative recommendation (low confidence, §0):** start with a coarse `TypedColumn` collation marker, exactly parallel to nullability, recording `Binary` for all portable columns and an engine-specific collation tag for engine-bound outputs. Mirror the nested-nullability extension-point note in `types.md` §Design: *if* Spark-grade type-level collation precision is ever wanted, the extension point is a collation field on the string `DataType` variants (Spark's shape), not a relocation of the column-level marker. This keeps the cheap path cheap and names the migration path without taking it.

This recommendation is held weakly on purpose. The strongest counter-argument is that Spark 4.0 has already proven the value-domain model works and is where the industry is heading; if smelt expects portable non-binary collation to become important, paying the `DataType` cost up front avoids a later migration. The spec cycle should weigh how likely portable non-binary collation is before freezing this.

### 6.4 What §6 does not decide

- The **concrete representation** of the marker (an enum on `TypedColumn`; an interned collation id; a `Binary` vs `Named(String)` split).
- Whether `varchar` length (already a value-domain refinement on the string type) and collation should be unified into a single "string type refinement" story or kept in separate channels.
- The placement of collation on **composite types** (`Array(Text)`, `Struct{f: Text}`). The nullability precedent erases the column channel inside composites (§11 of `types.md`); collation would likely inherit the same conservative treatment (composite string elements are binary unless the engine says otherwise).

## 7. Equality-Affecting Collation and the Fingerprint / VE Tie-In

This is the observation that makes collation matter more than its three siblings for virtual environments.

### 7.1 Ordering-only vs equality-affecting

Collation effects split in two:

- **Ordering-only.** A collation that changes `ORDER BY` results but agrees with binary on equality changes only the *order* of rows, not which rows exist. For a model whose output is a set (order-insensitive), this is invisible to the fingerprint.
- **Equality-affecting.** A case-, accent-, or normalisation-insensitive collation changes `=`, and therefore changes `GROUP BY` keys, `DISTINCT`, join matches, and `MIN`/`MAX`. This changes *which rows a model emits and what they aggregate to*.

The second class is collation's analogue of `Decimal / Decimal`: the place where *same printed type ≠ same values*. Two models with byte-identical SQL but different collation can produce different row sets. DuckDB's own open bugs (`NOCASE` in `GROUP BY` / `UNION` / `INTERSECT` / `EXCEPT` / `MIN`) are concrete proof that this is fragile even within one engine, let alone across three.

### 7.2 Consequence for the output fingerprint

`output_fingerprint.md` requires `fingerprint-equal ⇒ DuckDB relations identical`. Today collation is unfolded, so any model touching strings is breaking-by-default (conservative rebuild). The fold must therefore hash the **collation** of each string column alongside its type and nullability — but the binary-only portable surface makes this cheap:

- **Portable models are trivially collation-stable** — every string column is binary, so collation contributes a constant to the hash and never forces a rebuild. The common case costs nothing.
- **Only engine-bound models** carry a non-binary collation the fingerprint must distinguish, and there the collation tag (§6.3) is exactly what gets hashed.

This is a clean story: the binary-only surface that buys portability also buys cheap fingerprint precision for collation. It also means the axis ordering in `output_fingerprint.md` (`nullability → decimal → collation`) can land collation last with the least marginal cost — provided the marker exists to hash.

### 7.3 Interaction with determinism

Postgres's term "nondeterministic collation" is about *equality* (it may equate differently-encoded strings), not about run-to-run nondeterminism — given a fixed collation, the result is deterministic. The genuine nondeterminism hazard is **cross-engine and cross-version ICU skew**: the "same" named locale collation backed by different Unicode versions can sort or fold differently. This is why the portable surface stops at binary (version-stable) and why any future portable named collation (§8) would have to pin an ICU version as a deployment contract (§9), analogous to decimal's ANSI-mode requirement.

## 8. The Function-as-Polyfill Mechanism (Future Direction)

As with decimal §7, the mechanism by which library code could later expose portable *non-binary* collation is worth keeping the design space open for, though not designed here.

### 8.1 Shape

A function with a portable signature and engine-dispatched bodies — e.g. a portable case-insensitive equality:

```
fn ci_eq(a: Text, b: Text) -> Boolean {
  engine duckdb   => a = b COLLATE NOCASE
  engine spark    => a = b COLLATE UTF8_LCASE
  engine postgres => a = b COLLATE case_insensitive   -- a nondeterministic ICU collation
}
```

From the call site, `ci_eq(...)` is portable. From the author's perspective, the cross-engine equivalence (do all three agree on `'ß'`/`'SS'`, on Turkish dotted-I, on accent folding?) is a property-tested obligation — and collation is *exactly* the domain where those edge cases bite, so the test discipline matters more here than for decimal division.

### 8.2 Open type-system questions (collation-specific)

- **What collation does a polyfilled comparison's *result* carry?** A `Boolean` result carries none, but a polyfilled *transformation* (e.g. a portable case-fold returning `Text`) must declare the collation of its output — and whether that output is itself portable.
- **Grouping/joining through a polyfill.** `GROUP BY ci_key(x)` requires the polyfill to produce a *grouping key* that is consistent across engines, which is strictly harder than a pairwise comparison — it needs a canonical fold (e.g. normalise + lowercase to a binary key), not an engine-native CI collation. The clean portable answer may be "normalise to a binary key" rather than "use the engine's CI collation," which sidesteps collation entirely. This is worth its own analysis.
- **Planner capability inference.** A model using `ci_eq` is compatible only with engines for which a body exists; this composes across the call graph exactly as in the decimal paper.

### 8.3 Why this lives in libraries, not the language

The language commits to *which comparisons are guaranteed identical without proof* — here, binary only. Libraries commit to *which comparisons are guaranteed identical with author-supplied evidence*. Collation is the strongest case for this split: the space of "approximately case-insensitive" behaviours is large, locale-specific, and Unicode-version-sensitive, so freezing any one of them into the language would be a mistake, while a library can ship a documented, property-tested fold and own its caveats.

## 9. Deferred Questions

Recorded so the position in §5–§6 is not relitigated under their pressure.

- **The binary-pinning mechanism on Postgres.** Per-column `COLLATE "C"` in DDL, per-comparison `COLLATE "C"` in emitted SQL, or a session/database-level setting? Per-comparison is most local and robust to upstream schema; per-column is cheaper at runtime but requires owning the DDL. Undecided.
- **Ordering-only collation in portable code.** Could portable code allow `ORDER BY x COLLATE "en_US"` for *presentation* (final output ordering) since it does not change row content? Tempting, but it changes row *order*, which matters if a downstream consumer or the fingerprint is order-significant. Likely "no in portable code; declare an engine," but worth a carve-out analysis analogous to decimal's literal-denominator question.
- **Pattern matching (`LIKE`/regex) under collation.** Postgres restricts pattern matching under nondeterministic collations; the portable contract should probably say `LIKE` is binary-only too. Not analysed here.
- **Unifying `varchar` length and collation.** Both are refinements on the string type; whether they share a representation (a `StringType { max_len, collation }` value-domain shape, Spark-like) or stay in separate channels (length in `DataType`, collation in `TypedColumn`) is the concrete form of the §6 decision.
- **ICU version pinning as a deployment contract.** Any future portable named collation (§8) requires all engines to agree on an ICU/Unicode version. This is a deployment requirement (like decimal's ANSI mode), not a language-surface decision, but it must be named before a portable non-binary collation ships.
- **Collation × nullability interaction.** Both are `TypedColumn` axes under the recommended placement; no interaction is anticipated, but a `COALESCE` across differently-collated branches would need a coercibility answer.
- **Composite-type collation.** Whether `Array(Text)` / `Struct{f: Text}` elements track collation or are conservatively binary (mirroring the nested-nullability erasure in `types.md` §11).

## 10. What Changes from Current smelt

If the position is adopted, the following concrete changes follow (sequenced by a future `/smelt:plan`).

### 10.1 Type representation

- A collation marker is added — under the recommended lean, a coarse field on `TypedColumn` (`Binary | EngineCollated(name)`), parallel to `nullable: bool`. Portable inference sets it to `Binary` for every string column; engine-bound model outputs carry the engine's collation tag.

### 10.2 Emission

- `to_backend_sql()` pins binary on Postgres for portable string comparisons/columns (emit `COLLATE "C"`), closing the silent-locale bug in §4.2. Spark/DuckDB already default to binary; pinning is for robustness against session defaults.

### 10.3 Diagnostics

- A new diagnostic when non-binary collation is used in portable code: *"collation `<name>` is not portable; either compare byte-wise (the default) or declare an engine on this model."* — mirroring the decimal-division message's two-remedy shape.

### 10.4 Divergence registry and property oracle

- `divergences.rs` gains collation entries for engine-bound behaviour (e.g. a `NOCASE` `GROUP BY` whose row count differs from binary), registered `ByDesign`.
- The type/value property oracle is extended to exercise collated string columns, asserting that portable (binary) comparison agrees across engines and that engine-bound collation produces the engine-native result.

### 10.5 Fingerprint fold

- Once the marker exists, `output_fingerprint.md`'s fold hashes the collation tag alongside type and nullability. Portable models hash a constant `Binary` and never rebuild on collation; only engine-bound collation differences force a rebuild. The standing fingerprint soundness oracle must stay green for schemas differing only in collation.

### 10.6 Deployment contract

- Documented: portable string semantics are byte-wise; locale-aware or case-insensitive comparison requires an engine declaration (today) or a property-tested polyfill (future). Any future portable named collation additionally requires a pinned ICU version across engines.

## 11. Summary

- The three reference engines model collation incompatibly: Spark 4.0 puts it **in the type**, Postgres derives it as an **expression property** via coercibility, DuckDB attaches it as a **`COLLATE` annotation**. They also diverge on the default collation (binary on Spark/DuckDB, **database locale on Postgres**), on named-collation vocabulary, and on the ICU version backing locale collations.
- smelt today tracks no collation and emits no `COLLATE`, so it is binary-by-omission — which is accidentally correct on Spark/DuckDB but **silently locale-dependent on Postgres**.
- The proposed (survey-led, leaning) position: the **portable surface exposes only binary collation**, pinned explicitly on emission; non-binary collation requires an engine declaration; library polyfills are the long-term ergonomic answer. This is the collation analogue of the decimal-division exclusion.
- The defining property of collation is that it **changes equality**, so equality-affecting collations change a model's *output rows*, not just their order — making collation a first-class concern for the output-fingerprint equivalence relation. The binary-only surface makes that fold cheap: portable models are trivially collation-stable.
- The **centrepiece open decision is axis placement** — value-domain (`DataType`, Spark's model) vs column-population (`TypedColumn`, Postgres's model). The weak recommendation is a coarse `TypedColumn` marker now, with a `DataType` collation field named as the migration path if portable non-binary collation ever becomes important. The spec cycle should settle it.
- Binary-pinning mechanics, ordering-only carve-outs, pattern matching, `varchar`-length unification, ICU version pinning, and composite-type collation are deferred.

## 12. References

- `docs/research/20260516-decimal-type-system.md` — the sibling paper; the three-layer position, polyfill mechanism, and "refuse rather than approximate" argument transfer directly.
- `docs/specs/types.md` — §4 (String unification), §Design ("Axis placement"; collation named as a tentative `TypedColumn` axis), §Constraints (collation out of scope for v1).
- `docs/specs/output_fingerprint.md` — collation listed among unfolded type-system axes; breaking-by-default until tracked; axis ordering `nullability → decimal → collation`.
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — current divergence registry (`string_concat`, `string_functions`; no collation entries yet).
- `crates/smelt-types/src/lib.rs` — `DataType` (no collation field) and `TypedColumn` (`nullable: bool`, no collation).
- Postgres docs: [Collation Support](https://www.postgresql.org/docs/current/collation.html); Daniel Vérité, [Nondeterministic collations](https://postgresql.verite.pro/blog/2019/10/14/nondeterministic-collations.html).
- Spark: [SPARK-46830 — Introducing collation into Spark](https://issues.apache.org/jira/browse/SPARK-46830); [Spark SQL data types](https://spark.apache.org/docs/latest/sql-ref-datatypes.html).
- DuckDB docs: [Collations](https://duckdb.org/docs/current/sql/expressions/collations); collation set-operation issues [#17251](https://github.com/duckdb/duckdb/issues/17251), [#20308](https://github.com/duckdb/duckdb/issues/20308), [#3821](https://github.com/duckdb/duckdb/issues/3821).
