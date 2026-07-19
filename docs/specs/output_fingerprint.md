---
feature: output_fingerprint
status: experimental
last_reviewed: 2026-07-19
owners: [andrew]
---

# Output Fingerprint

> **What this is.** A normative spec for smelt's **semantic output-fingerprint** — a content hash over a canonical normal form of a model's `SELECT`, such that two versions with the same fingerprint are *proven* to compute the same relation (same multiset of rows, columns matched by name) for the same inputs. It also defines the **determinism signal** that travels with the fingerprint. This is the equivalence oracle the virtual-environments reuse layer is built on. Out of scope: table reuse, the state store, environment addressing, and plan categorization (see `virtual_environments.md`); the persisted `.smelt/` layout (see `run_state.md`); cross-model column lineage (not yet built). **Disambiguation:** this is *not* the parser-compat "fingerprint-equivalence" in `architecture.md` §"Identity properties", which is a *syntactic* pg_query roundtrip check. This fingerprint is about *semantic output* equality.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences or §References → Plans (history).

## Surface

The fingerprint is a **cross-crate Rust API** in `smelt-fingerprint`. It is not yet user-facing — there is no CLI flag, YAML field, or diagnostic that exposes it directly; its consumers are other smelt crates (the reuse layer, once built).

```rust
pub fn output_fingerprint(
    expanded_select: &SelectStmt,
    output_schema: &[(String, String)],
) -> FingerprintResult;

/// Convenience: parse raw (already function-expanded) SQL to a SELECT, then
/// fingerprint. Returns None if the text does not parse to a SELECT.
pub fn output_fingerprint_from_sql(
    sql: &str,
    output_schema: &[(String, String)],
) -> Option<FingerprintResult>;
```

```rust
pub struct FingerprintResult {
    pub fingerprint: Fingerprint,        // SHA-256 digest, 32 bytes
    pub canonicalisable: bool,           // false ⇒ verbatim fallback was used
    pub deterministic: bool,             // false ⇒ output not a pure function of inputs
    pub non_determinism: Vec<NonDeterminism>, // reasons; empty iff deterministic
    pub missed_reuse: Vec<MissedReuse>,  // where a possible equivalence was conservatively declined
}

pub struct Fingerprint(pub [u8; 32]);   // `to_hex()` for diagnostics/golden tests
pub struct NonDeterminism { pub reason: String }
pub struct MissedReuse { pub reason: String }
```

- `expanded_select` is an **already function-expanded** model `SELECT`. This crate does not resolve `smelt.<path>` refs or expand functions — callers pass the expanded CST.
- `output_schema` is an optional list of `(column_name, type_rendering)` pairs. When non-empty it is folded into the fingerprint so a type-only change is detected; `&[]` fingerprints structure alone (already sound).

## Semantics

### The soundness invariant (load-bearing)

For any two expanded `SELECT`s `A` and `B` and the same `output_schema`:

```
fingerprint(A) == fingerprint(B)   ⇒   A and B compute the same relation
```

where "the same relation" means the same multiset of rows with columns matched **by name** (not position), for the same inputs. Only this direction is guaranteed. The converse is **not** required: the fingerprint may fail to recognise a genuine equivalence (incompleteness), which is always safe — it costs a missed reuse, never a wrong one. A fingerprint match that did *not* denote relation-equality would be a soundness violation (the data-corruption class the whole design exists to prevent).

### Canonical form

The fingerprint is computed over a canonical normal form, not the raw text. The form is either **structured** (`canonicalisable == true`) or a conservative **verbatim** fallback (`canonicalisable == false`).

Normalisation that always applies (and makes the corresponding edits fingerprint-equal):

- **Trivia** (whitespace, line and block comments) is stripped.
- **SQL keywords** are case-folded (keywords are case-insensitive). Identifiers and literals are left verbatim — quoted identifiers are case-sensitive and literal spelling can carry semantics (decimal scale), so folding them would risk unsoundness.

The structured form additionally recognises these refactors as equivalent:

- **Projection reordering.** The top-level projection is keyed **by name** (a model boundary is consumed by name, so output column order is not observable). `SELECT a, b` and `SELECT b, a` are equal. When position *is* observable or names are ambiguous, the projection falls back to an ordered list (see below).
- **Internal CTE / FROM-alias renaming.** Binding names (CTE names, derived-table aliases) are alpha-renamed to positional canonical names (`r0`, `r1`, …) in binding order. A name is renamed only where it denotes the binding (qualifier before `.`, CTE binding before `AS`, alias after `AS`, bare table reference in `FROM`), never where it denotes a column.
- **Single-use CTE ≡ derived table.** When the `FROM` is a single subquery — a written derived table `(Q) AS x`, or a reference to the query's single single-use CTE — it is represented by the **recursive fingerprint** of its inner `SELECT`. This collapses `WITH c AS (Q) … FROM c` and `… FROM (Q) AS c`, and recurses, so a refactor *inside* the inlined body is still recognised.

The structured form carries: the (by-name or ordered) projection, the folded `output_schema` types, the normalised `FROM`, `WHERE`, `GROUP BY` (order-insensitive set), `HAVING`, and a `DISTINCT` flag.

### Conservative verbatim fallback

When the builder cannot prove a structural representation safe, it hashes the normalised token stream of the whole statement (`Canon::Verbatim`). This is still sound (any change re-fingerprints) but recognises *only* trivia/keyword-case equivalence. The fallback is recorded in `missed_reuse` and fires for:

- **Set operations** (`UNION`/`INTERSECT`/`EXCEPT`) — top-level column position becomes observable across branches.
- **Row-slicing tail clauses** (`LIMIT`/`OFFSET`/`FETCH`) and **`QUALIFY`** — they select *which* rows survive; dropping them would make a top-N/paginated/window-filtered change a false equivalence. A bare `ORDER BY` with **no** slice is soundly ignored (the relation is an unordered multiset).
- **Recursive `WITH`** — a recursive CTE references its own binding name, so alias renaming would change meaning.
- **Wildcard projection** (`SELECT *`) — columns cannot be enumerated without schema resolution.
- **Joins / multi-table `FROM` and multi-CTE queries** are kept in flat normalised-token form (not verbatim, but not inlined): the `FROM` token string includes the joins, so they remain sound, just not deeply canonicalised.

A projection with a column that has no determinable output name, or duplicate output names, downgrades to an **ordered** projection (still sound; position-sensitive).

### Determinism signal

`deterministic` reports whether the model's output is, as far as a structural detector can establish, a **pure function of its inputs**. It is computed independently of the fingerprint and **does not affect the fingerprint value**: identical SQL fingerprints identically whether deterministic or not. A model is flagged non-deterministic (with one `NonDeterminism` reason per cause) when its CST — including nested derived tables and CTE bodies — contains any of:

- a **non-deterministic built-in call**: randomness (`random`, `uuid`, `gen_random_uuid`, …), wall-clock/transaction time (`now`, `current_timestamp`, `current_date`, …), or session/transaction identity (`txid_current`, `nextval`, `version`, …);
- a **parenless temporal special** (`current_timestamp`, `current_date`, … written without parentheses, surfacing as a bare identifier) not used as a column qualifier;
- an **order-sensitive aggregate** (`array_agg`, `list`, `string_agg`, `group_concat`, `listagg`, `any_value`, `arbitrary`) — its result depends on a fold order a relation does not fix, and smelt has no aggregate-`ORDER BY`/`WITHIN GROUP` syntax to pin it;
- a **row-slicing tail clause** (`LIMIT`/`OFFSET`/`FETCH`) — the surviving rows are fixed only by a total `ORDER BY`, which the detector cannot prove.

The detector is **conservative**: it errs toward flagging. `deterministic == true` must mean reproducible; `deterministic == false` may over-report (worst case: the model is rebuilt rather than reused). Order-*insensitive* aggregates (`sum`/`count`/`min`/`max`/`avg`) stay deterministic.

## Design

**Soundness over completeness, with a verbatim floor.** A false "equivalent" silently corrupts data; a missed equivalence merely forgoes a reuse. So every structural rule must be output-preserving, and anything unproven drops to a verbatim hash. Completeness grows rule by rule, each new rule gated by the DuckDB oracle property test — the same discipline `smelt-db`'s type property tests use against DuckDB. Rationale and the SQLMesh comparison: `docs/research/20260601-virtual-environments.md` §5.

**By-name multiset, not ordered tuples.** A model boundary is consumed by name downstream, so output column *order* is not part of the relation's identity and a projection reorder is a true equivalence. Keying the projection by name is what lets smelt recognise reorders that SQLMesh's syntactic edit-script rebuilds on. Position is preserved only where it becomes observable (set operations, duplicate/un-named columns), via the ordered-projection fallback.

**Determinism is orthogonal to the fingerprint, and only ever *narrows* reuse.** The fingerprint answers "same query?"; determinism answers "is this query a pure function of its inputs?". A model can be fully `canonicalisable` yet non-deterministic (`SELECT random() AS r FROM t`). Keeping the flag out of the hash means the reuse layer — not the hash — decides what to do with a non-deterministic match. This is a **narrow structural auto-derivation** specific to the fingerprint's reuse judgement; it does **not** conflict with `planner_integration.md` §"Properties are author-declared in v1" (which keeps the *planner's* `deterministic` property author-declared) because it never *widens* eligibility — it only flags *more* models as non-reusable. A declared `deterministic: true` on a function is an author assertion the planner trusts; this detector is a conservative floor that catches *inline* non-determinism with no call node to tag.

**`first`/`last` are unreachable as aggregates.** They are order-sensitive, but they are smelt keywords (`NULLS FIRST`/`LAST`), so `first(a)` does not lex as a call and cannot be written as an aggregate today — there is nothing to match. They are intentionally absent from the deny-list.

**Recursive sub-fingerprint.** Representing a single-subquery `FROM` by the fingerprint of its inner `SELECT` (rather than its token text) is what makes CTE-inline ≡ derived-table hold *and* lets an inner-body refactor stay equivalent. Soundness is inherited: equal nested fingerprints denote equal nested relations.

**Two fingerprint artifact classes.** This document's fingerprint is a **model-SQL structural**
comparison function — ephemeral, recomputed by both sides at decision time, never persisted, with
zero migration obligation (§Design "The fingerprint is an ephemeral comparison function, not a
stored artifact", below). A second, unrelated artifact — the **row-content fingerprint sidecar**
(`sources.md` §"The fingerprint sidecar") — digests external source *row content* (not SQL
structure) to synthesize a change feed for a `mutable_snapshot` source with no native one. The two
share no digest algorithm requirement, API, or canonical form; they both happen to use content
hashing because that is the natural tool for "did this content change," not because they are the
same mechanism, and a soundness result about one says nothing about the other. The sidecar's
persistence is not an exception to this spec's "never persisted" principle below — it persists a
*different quantity* (last-seen external row content) for a *different purpose* (cross-run change
detection over data smelt does not control), where this spec's fingerprint persists nothing
because both sides of its comparison — two versions of smelt-owned, always-recompilable SQL — are
always available to recompute. The sidecar's own schema-versioning strategy — so the persisted
digest format itself can evolve without silently comparing against a stale-format row — is that
spec's answer to give, not this one's: it stamps every stored row with a digest-construction
version alongside a projection identity and a model-definition hash, and treats any stored row
whose stamp does not match the freshly computed one as absent rather than comparing against it
(`sources.md` §"The fingerprint sidecar" — "Invalidation").

**The fingerprint is an ephemeral comparison function, not a stored artifact.** Equivalence is computed by fingerprinting *both* sides with the *current* compiler at decision time; the digest is never persisted. This frees the canonicalisation algorithm to change between releases with zero migration obligation and no version-stable-form contract — the comparison is always apples-to-apples. The persisted artifact is the expanded logical SQL (see `run_state.md`), not the hash. Rationale: research §5.6 and Open Question 15.

## Constraints & Invariants

- **Soundness property.** `fingerprint(A) == fingerprint(B) ⇒ DuckDB confirms A and B are the same relation`, asserted over generated query pairs (formatting/reorder/rename/CTE-inline transforms plus semantics-changing transforms, single-table and join shapes, and the §5.5 value axes). This is the gate that must hold before any reuse is wired to execution.
- **Determinism reproducibility property.** Any query flagged `deterministic` yields the same relation when built twice in independent DuckDB instances; every generated non-deterministic construct flips the flag.
- **Determinism never widens reuse.** The flag may only restrict reuse eligibility, never enable a reuse the fingerprint alone would not.
- **Pure and stable.** The fingerprint is a pure function of `(expanded_select, output_schema)` and is byte-stable across runs of the same compiler (canonical structures are ordered — `BTreeMap`/`BTreeSet`, no hash-iteration nondeterminism).
- **Single-model only.** No state store, no environments, no cross-model lineage are computed here.
- **Explicitly out of scope (sound because conservative):** cross-model column lineage (dead-column removal, downstream-spared changes); type-system axes not yet tracked (decimal precision/scale, `Text` collation, nullability); order-sensitive **window** functions.

## Known Divergences / Open Questions

- **Not yet wired into the runtime.** `smelt-fingerprint` is a prototype consumed only by its own tests; nothing in the compile/execute pipeline calls it yet. The reuse layer that will is specified in `virtual_environments.md` and unbuilt. Tracking: `docs/research/20260601-virtual-environments.md` §8.
- **Type-system axes not folded.** "Same printed type" does not yet imply "same values" for decimal precision/scale, collation, or nullability. Each is breaking-by-default (a conservative verbatim/structural distinction) until the type system tracks it and a DuckDB-oracle property test covers it. Ordering of these axes is open (likely nullability → decimal → collation). See research §5.5, Open Question 11.
- **Window-function non-determinism not detected.** Order-sensitive window functions (`row_number`/`rank`/`lag`/`first_value` over a non-total `ORDER BY`) are not yet flagged; this needs `OVER`-clause analysis rather than a name match, gated by the same determinism property test.
- **Cross-model lineage absent.** The "eclipse" over SQLMesh — reuse when a change cannot alter any column a downstream model consumes — needs a cross-model column-lineage analyser smelt does not have yet. Until then the fingerprint proves single-model equivalence only. See research §4(b), §5.2.

## References

- **Code**: `crates/smelt-fingerprint/src/lib.rs` (public API), `crates/smelt-fingerprint/src/canonical.rs` (canonical form), `crates/smelt-fingerprint/src/determinism.rs` (determinism detector), `crates/smelt-fingerprint/src/hash.rs` (encoder)
- **Tests**: `crates/smelt-fingerprint/tests/soundness_prop.rs` (the soundness gate), `tests/determinism_prop.rs` (determinism gate), `tests/corpus_equivalent.rs` / `tests/corpus_negative.rs` (golden equivalences and non-equivalences), `tests/determinism.rs` (determinism unit cases), `tests/oracle_tests.rs`
- **User docs**: none yet (not user-facing)
- **Plans (history)**: none yet — predecessor research is `docs/research/20260601-virtual-environments.md`
- **Related specs**: `virtual_environments.md` (the reuse layer built on this), `run_state.md` (what is persisted), `planner_integration.md` (the author-declared `deterministic` property), `functions.md` (function-property declarations), `architecture.md` (the distinct parser-compat fingerprint; crate responsibilities), `types.md` (the type vocabulary folded as `output_schema`), `sources.md` (the row-content fingerprint sidecar — a distinct artifact class, boundary drawn in §Design "Two fingerprint artifact classes")
