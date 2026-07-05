# Design forks: recommended resolutions

- **Date**: 2026-07-06
- **Status**: research — **proposals awaiting ratification**; nothing here is implemented, and none of it should be implemented until Andrew signs off per fork
- **Part**: 3 of the refresh-as-maintenance-plan series ([index](README.md))
- **Related**: [01-framework.md](01-framework.md) (the maintenance-plan framework), [02-loop-findings.md](02-loop-findings.md) (the empirical findings these forks came out of), [08-code-placement.md](08-code-placement.md) (the plan-derivation layer several resolutions land in)
- **Sources of record**: `docs/research/property-discovery/ledger.md` (cells G-11, G-10, FIX-2, and the G-01/G-06 asides), `docs/research/property-discovery/unsupported.md`

The property-discovery loop runs under a policy that mechanical, test-backed fixes may land
autonomously (FIX-1 did) but behaviour- or contract-affecting decisions must BLOCK for human
review. Three cells blocked, and two further correctness findings were flagged as asides inside
other cells' write-ups. This document takes an explicit position on each — recommendation,
rationale, and rejected alternatives — so ratification is a yes/no per fork rather than a design
session. Every code claim below was re-verified against the tree at the time of writing, not
copied from the ledger.

---

## F1. G-11 — the outer output clamp is unqualified and binder-ambiguous

### Finding (ledger cell G-11; reproduced red)

`inject_time_filter` (`crates/smelt-runtime/src/transformer.rs:299-340`) builds the outer output
clamp as a **bare textual predicate** — `{event_time_column} >= '{start}' AND
{event_time_column} < '{end}'` — and splices it into the model's outermost `WHERE`. Its one
production call site (`crates/smelt-runtime/src/execute.rs:995-999`) passes the frontmatter's
bare `event_time_column` and fires for every model that is not the transparent single-source,
zero-margin slice (`is_transparent_single_source`, `transformer.rs:268-276`). A self-referential
batched model always has ≥2 bounded sources, so it always takes this path — and the direct-join
form documented in `docs/specs/batched_models.md` §"Window independence and self-referential
models" exposes the partition column under the same bare name from both the driving source and
the `smelt.<self>` reference. DuckDB rejects the compiled SQL with `Binder Error: Ambiguous
reference to column name`. The spec's own documented pattern does not execute.

### Options

- **(a) Qualify the clamp to the resolved driving-fact alias.** Thread the alias resolution
  `smelt-logical` already performs (`resolve_join_driving_fact` in
  `source_bounds.rs`, `resolve_single_anchor` in `rules/cumulative.rs`) through to
  `smelt-runtime` at compile time, and emit `t.d >= … AND t.d < …`.
- **(b) Wrap the whole query in an outer subquery before clamping.** Emit
  `SELECT * FROM (\n{sql}\n) AS _smelt_output_clamp WHERE {col} >= … AND {col} < …`, so the
  clamp's FROM scope exposes exactly the model's own output columns and nothing from any inner
  alias.
- **(c) Hybrid**: wrap only in the non-transparent case.

### Recommendation: **(b), the subquery wrap** — which is (c) for free

The decisive argument is not mechanical convenience but **what the clamp means**. The spec
(`docs/specs/model_transforms.md` §"Source-filter pushdown + the two clamps") defines the outer
clamp as filtering **the outermost projection** — the model's *output* — so that the write window
equals the output window. A predicate over an inner FROM alias is a filter over an *input*, which
is a different operation: it is only coincidentally equivalent when the output column is a bare
pass-through of that input column. The moment `event_date` is *derived* (`DATE(event_ts_utc)`,
a timezone rebase, a `COALESCE` across UNION arms), option (a) clamps the wrong expression or
fails to find the column at all. The subquery wrap is the structurally faithful implementation of
the spec's own sentence: the clamp ranges over the output schema, by output column name, always.

Secondary arguments, each independently sufficient:

- **It closes the whole ambiguity class, not one instance.** Any multi-source model whose FROM
  items expose same-named columns hits the same binder error — including the G-06 aside (two
  timeseries sources sharing a partition-column name; see F5). Option (a) fixes only the shapes
  whose driving fact resolves uniquely, and needs a new answer for the 3-way join where
  resolution is ambiguous (fail? fall back to… what?). The wrap has no such residue.
- **It respects the layering.** `smelt-runtime` is deliberately alias-unaware; `transformer.rs`
  is a pure text/AST transform. Option (a) either duplicates `smelt-logical`'s alias resolution
  in `smelt-runtime` (drift risk between two resolvers) or adds a new compile-time data flow for
  a value the wrap makes unnecessary.
- **Note the clamp is already only injected in the non-transparent branch**
  (`execute.rs:992-1001`), so "wrap only when non-transparent" (option c) is what landing (b) at
  the existing call site *is*. The transparent slice keeps its current no-clamp fast path
  untouched.

**What (b) costs, stated honestly:**

- **A spec edit.** `model_transforms.md`'s two-clamps entry gains one sentence: the output clamp
  is applied to a wrapping projection over the model's output schema, not spliced into the
  model's own outermost `WHERE`. This is a description change, not a semantics change — the
  clamped window is identical.
- **A deliberate contract drop.** `transformer.rs::tests::test_with_join` (lines 835-850) passes
  an already-qualified column (`orders.created_at`) into `inject_time_filter` and expects it
  spliced into the model's own WHERE. Under the wrap, a qualified inner-alias name is
  *definitionally wrong* (the outer scope has no `orders`). A repo-wide check shows this calling
  convention is **test-only** — the sole production caller passes the bare frontmatter column —
  so the recommendation is to **drop the qualified-name convention explicitly**: the function's
  contract becomes "an unqualified column of the model's output schema", enforced by a debug
  assertion or an error on a dotted name, and `test_with_join` is rewritten to the new contract.
  This is the one place option (b) changes an existing test's meaning rather than just its text;
  it should be called out in the commit.
- **One level of SQL nesting** on every non-transparent model. No measurable engine cost; CTE
  (`WITH … SELECT`) bodies wrap legally in DuckDB and Spark.

**Rejected: (a)**, because it answers "which input alias owns the output column" — a question the
output clamp should never need to ask — and imports an alias-resolution dependency plus an
unresolved 3-way-join ambiguity to do so. If a future *scan-side* optimization needs
driving-fact resolution in the runtime, that is a separate discussion; the clamp is the wrong
tenant for it.

### Implementation shape

`smelt-runtime/src/transformer.rs`: change `inject_time_filter` (or introduce
`inject_output_clamp` and retire the old name) to emit the wrap; reject dotted column names.
Red tests, in order: (1) un-skip/rewrite G-11's reproduction — the **direct-join**
self-referential shape (`model_shapes::running_balance_self_ref_direct_join`) must execute
green with no subquery workaround, and `G-08`'s existing subquery-wrapped fixture must still
pass; (2) a two-timeseries-sources-same-partition-column-name model (the G-06 aside shape)
executes green; (3) `test_scan_widens_but_output_clamp_stays_exact_to_run_window`
(`transformer.rs:707-759`) still passes verbatim — the wrap must not disturb the two-window
invariant; (4) `test_with_join` rewritten to the unqualified contract. Then update
`docs/specs/model_transforms.md` and the `batched_models.md` self-referential section (which can
then also delete `G-08`'s undocumented subquery-wrap workaround from the loop's fixture notes).

---

## F2. G-10 — `JoinContext` cannot express a composite unique key

### Finding (ledger cell G-10; Link-B only — the consumer is dormant)

`join_shape::JoinContext` (`crates/smelt-logical/src/analysis/join_shape.rs:36-56`) declares
uniqueness as `HashMap<String, HashSet<String>>` — a set of columns **each of which alone** is
unique. `fan_out` (lines 63-94) accordingly asks whether *any single* equality column matches a
declared key. A genuine composite natural key (`ON f.user_id = d.user_id AND f.dt = d.dt`, proven
one-to-one by ground-truth proptest in the ledger cell) is unclassifiable and falls to
`OneToMany` — a false negative that would refuse a dimension-horizon MERGE the join could safely
take. Fail-closed, so over-conservative, never unsound. Both `fan_out` and its intended consumer
(`dimension_horizon_merge`) have zero production call sites today.

### Recommendation: candidate-key sets, subset-matched

Change the declaration to **a list of candidate keys per source, each key a column set**:

```rust
pub struct JoinContext {
    /// source/alias -> candidate keys; each inner set jointly unique.
    pub unique_keys: HashMap<String, Vec<BTreeSet<String>>>,
}
```

with the generalized check in `fan_out`: the join is `OneToOne` iff **some declared candidate key
is a subset of the AND-ed equality columns** collected for that side
(`equality_columns_for_table` already collects the full AND-walk, lines 100-159, so only the
match predicate changes). The existing single-column behaviour is the singleton-set special case;
keep `with_unique_key(source, col)` as sugar and add `with_composite_unique_key(source, &[cols])`.

Why subset (not equality) of the equality columns: extra equality conjuncts beyond a key can only
*narrow* the match set, never widen it — if `{user_id, dt}` is unique, `ON … user_id AND dt AND
region` is still at-most-one. This also answers the ledger's caveat about a *mis*-declared single
column of a composite key: that remains a mis-declaration (the declared-and-checked doctrine
applies — see below), not something the matcher should compensate for.

**Where the declared key comes from:** the source-level `unique_key` declaration proposed in
[05-source-properties.md](05-source-properties.md) — already composite-valued there
(`unique_key: [user_id, dt]`, or a list of lists for multiple candidate keys). The plan-derivation
layer ([08-code-placement.md](08-code-placement.md)) builds `JoinContext` from those declarations
the same way `BoundContext` is built from `timeseries:` blocks today (`smelt-logical` stays
catalog-free; callers inject facts). Consistent with §10 of the framework: the key is
**declared and checked** — a declared unique key is a checkable assertion (a cheap
`GROUP BY key HAVING COUNT(*) > 1` probe at load/build time is the natural validator, opt-in per
backend cost), never silently inferred.

**Rejected alternatives:**

- *Inferring composite uniqueness from data.* Violates derive-else-declare's declaration arm for
  identity facts (framework §10): an inferred key can flip with data, silently changing plan
  admission. Uniqueness of a source is a contract the modeller states.
- *Keeping single-column keys and special-casing pairs.* No simpler than the general subset check
  and leaves 3-column keys (real: `(tenant_id, user_id, day)`) unclassifiable.
- *Doing nothing until a consumer exists.* Cheap now, but the declaration surface (source
  `unique_key`) is being specced in this series regardless — designing it single-column-only and
  migrating later is strictly more work.

### Implementation shape

`smelt-logical/src/analysis/join_shape.rs` only (plus the eventual context-construction site in
the plan-derivation layer). Red test: flip G-10's existing
`fan_out_cannot_express_composite_unique_key_and_conservatively_classifies_one_to_many`
expectation to `OneToOne`; keep the ground-truth proptest as-is; add a negative test (a declared
composite key only *partially* covered by the equality columns stays `OneToMany`). Timing: land
together with, or immediately before, the first real consumer of `fan_out` — see sequencing.

---

## F3. FIX-2 — wiring (or not) the dormant `input_delta_discovery`

### Finding (ledger cells FIX-2 and SC-2)

`input_delta_discovery` (`crates/smelt-logical/src/analysis/input_delta.rs:88-94`) classifies a
clocked source `WindowForward` **regardless of `mutation_profile`** — the `_ if shape.has_clock`
arm precedes any consideration of `Mutable` — and has zero production call sites; a tripwire test
(`crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs`) now guards that
emptiness. SC-2 confirmed the hazard the classifier would license if wired naively: a
forward-only consumer never revisits an in-place update to an already-processed partition.

### Recommendation: wire it **only** as a Link-B input to the plan-derivation layer — never piecemeal into today's batched path

In the framework's terms ([01-framework.md](01-framework.md) §5), `input_delta_discovery` is a
per-input **Link-B fact**: it names the delta channels an input offers. Its consumer is the
maintenance-**plan constructor** ([08-code-placement.md](08-code-placement.md)), which combines
it with column mutation-sensitivity to fill `(column-group × trigger)` cells — not the batched
execution driver, which is technique-level machinery and (per G-01…G-09) is already correct
without it.

Concretely, when the plan layer lands, the function's contract changes from "one verdict per
source" to **per-trigger channels**, because a clocked `Mutable` source genuinely has two:

- **Creation channel**: `WindowForward` — new rows arrive by the clock; sound as today.
- **Mutation channel**: in-place updates, which `WindowForward` cannot see. The plan must either
  assign this channel `SnapshotDiff`/`ChangeFeed` (when the profile supports it) or record the
  **named trade** SC-2 established empirically — *"in-place mutations are recovered only by an
  explicit backfill of the affected window; forward advance never revisits"* — as a CONDITIONAL
  entry in the model's guarantee ledger (framework §6), surfaced to the operator rather than
  silent.

The current match-arm ordering (clock trumps mutability) is exactly the collapse of these two
channels into one that the plan layer must not inherit; fixing the signature *is* the wiring
work. Until then the function stays dormant and **the tripwire test stays** — its removal happens
in the same commit that lands the sanctioned consumer, which is precisely the review point the
tripwire exists to force (its failure message already routes the author to SC-2).

**Rejected alternatives:**

- *Wire it into today's batched driver* (e.g. auto-re-scan mutated partitions). Invents new
  maintenance semantics ad hoc, in the execution layer, ahead of the framework that is supposed
  to own admission — the exact "silent contract content" the series argues against.
- *Delete it as dead code.* The classification is correct and needed by the plan layer; deleting
  and re-writing it later loses its unit tests and the SC-2 linkage for no saving.

---

## F4. The BigInt truncation bug (G-01 aside) — silent aggregate corruption for single-segment sources

### Finding (ledger cell G-01, "related finding"; verified against the tree)

`add_source_info_to_type_context` (`crates/smelt-db/src/queries/schema.rs:1356-1376`) requires
`address_segments.len() >= 2` and **silently `continue`s** otherwise, dropping *every declared
column* of that source from the `TypeContext`. A source YAML at scan root gets a single-segment
address by construction (`crates/smelt-core/src/sources.rs:325-334` derives segments from the
file stem). With the column types gone, `SUM(val)` over a `DOUBLE` falls through to the
historical `BigInt` default, and `wrap_with_type_casts` faithfully emits `CAST(total AS BIGINT)`
— silently truncating fractional aggregates. This is the **uncovered variant** of
`docs/research/20260417-0.3-regression-triage.md` bug #3: that bug was an *empty* `TypeContext`
(CLI wrapper path, since fixed and pinned by
`crates/smelt-db/tests/proptests/aggregate_widening.rs`); this one is a *populated* context with
one source's rows dropped by an arity check. Same blast radius: silent corruption of financial
aggregates.

### Recommendation: fix now, out of band of this series; single-segment sources must **work**, and the residual degenerate case must **fail loud**

Two changes, both mechanical:

1. **Handle `len == 1` correctly instead of skipping.** The two-segment requirement is an
   implementation artifact of the `{schema}.{table}.{col}` key format, not a real constraint —
   `TypeContext::add_source_column` (`type_context.rs:224-236`) *already* also registers a
   schema-free `{table}.{col}` simple key, which is exactly the identity a single-segment source
   resolves by. Register single-segment sources under `segs[0]` as the table with no schema
   qualifier (an `Option<&str>` schema parameter, or an explicit single-segment branch). The
   loop's standard fixtures — and any real project that keeps source YAMLs at scan root — are
   legitimate layouts; they must type-resolve, not error.
2. **Fail loud on the genuinely degenerate case.** `len == 0` (or whatever residue remains after
   1) becomes a diagnostic, not a `continue` — per the fail-loud discipline this is a textbook
   unclassified silent fallback, the class the CI gates exist to catch.

Priority: **this jumps the queue** ahead of everything else in this document. It is independent
of the maintenance-plan framework, currently corrupting any fractional aggregate over a
scan-root source, and small. Red test: a single-segment `SourceInfo` with a `DOUBLE` column →
`infer_select_column_types` on `SELECT SUM(val) …` returns `Double`-family, not `BigInt`; plus an
end-to-end assertion that G-01's fixture no longer needs its whole-number workaround (the
`arb_disjoint_windows` value constraint can then be lifted, which also widens the loop's own
coverage). Triage note closed against regression-triage bug #3 and `aggregate_widening.rs`'s
header.

**Rejected alternative:** *diagnostic-only (refuse single-segment sources).* Punishes a
legitimate layout to preserve an internal key-format assumption; the simple-key path shows the
identity model already accommodates schema-less sources.

---

## F5. The same-named multi-timeseries clamp ambiguity (G-06 aside)

### Finding (ledger cell G-06, closing note)

Two timeseries sources sharing a partition-column name (`events.d`, `refunds.d`) make the
injected bare outer clamp `WHERE d >= … AND d < …` binder-ambiguous. G-06 sidestepped it by
renaming; it was flagged for whoever next touches filter emission.

### Recommendation: **subsumed by F1** — no separate work

This is the same defect as G-11 with a different trigger: per-source scan filters are already
scoped (each source ref is wrapped in its own filtered subquery by `inject_source_filters`), so
the only unqualified predicate in play is the outer clamp, and F1's wrap removes its access to
inner FROM scopes entirely. The F1 implementation-shape list already includes this shape as red
test (2). If F1 were rejected, this would need its own (worse) fix — qualification with the same
driving-fact ambiguity — which is a further argument for F1 as recommended.

---

## Priority and sequencing

| order | item | why here | blocks the maintenance-plan spec? |
|---|---|---|---|
| 1 | **F4** BigInt truncation | live silent corruption, framework-independent, small | no — but do it first anyway |
| 2 | **F1** output-clamp wrap (+F5) | the spec's own documented self-referential pattern doesn't execute; every multi-source example in [07-example-catalogue.md](07-example-catalogue.md) is exposed to the ambiguity class | **yes** — spec examples must run |
| 3 | **F2** composite keys | declaration surface should be specced composite-valued from day one (05); code change lands with the first `fan_out` consumer | the *declaration* shape yes; the code no |
| 4 | **F3** delta-channel wiring | absorbed into the plan-derivation layer (08); no standalone landing | folded in |

The through-line: F1 and F4 are ratify-and-fix (mechanical once approved, each with a red test
already identified); F2 splits into a spec-now/code-later pair; F3 has no independent existence —
it is one input of the plan-derivation layer and should be reviewed as part of
[08-code-placement.md](08-code-placement.md), not separately.
