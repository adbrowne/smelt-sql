# The SQL Analyzer as a Standalone Release: Multi-Dialect Parsing, Verified Typing, and an Extensible Walk

**Date:** 2026-09-07
**Status:** Research / position. Nothing here is committed work; the proposed phase
order is a starting point for a spec, not a plan.
**Author:** Andrew Browne, with design input from Claude
**Inputs:** the crate inventory as of 2026-09-07 (`crates/*`), `docs/specs/datagen.md`,
`crates/smelt-logical/src/analysis/walk.rs`, `crates/smelt-types/src/signatures.rs`,
`crates/smelt-oracle-testkit/`, `crates/smelt-parser-compat/`,
`docs/research/20260905-engine-defined-semantics.md`,
`docs/research/20260809-creative-ideas.md` (the one-line "embeddable analyzer" idea),
`docs/research/20260719-crates-publishing.md`.

## 0. Authoring context

> For a continuing reader. Provenance and confidence, not design.

**Origin.** A 2026-09-07 conversation that started from "how do I evolve my way to a
production-ready smelt release, and are there components I could release first that are
more broadly useful?" Two candidates were named: `smelt-datagen`, and the SQL parser and
generator. The conversation then widened in three steps, each recorded as its own section
below:

1. Is a *multi-dialect* parser with per-dialect type checking a sane ambition? (§5)
2. Why does every dbt-adjacent product "cheat" on this job, and what makes it different
   here? (§3)
3. Could the composition walk be made extensible, and what would the Rust and Python API
   look like? (§7)

**Confidence.** The inventory in §4 was verified by reading the tree on the day. The
market observations in §3 are from memory of public announcements and should be
re-checked before anything is written down as fact in user-facing material. The effort
estimates in §8 are gut feel calibrated against the size of comparable work in this
repo, not a plan.

**Relationship to prior notes.** This note builds directly on
`20260905-engine-defined-semantics.md`, which already retired the "one portable dialect"
promise in favour of "a typed frontend over the target engine's own semantics, verified
against a live oracle". That note answers *what smelt promises*. This one asks what
falls out if the machinery behind that promise is released on its own, and what changes
if the parser accepts each engine's native dialect rather than one PostgreSQL-tracked
surface.

## 1. The question

Smelt's route to a production-ready release runs through the maintenance engine, which is
deep, novel, and needs the market's trust before anyone runs it against a warehouse. That
trust is slow to earn. The question is whether some component can ship first that is
broadly useful on its own, builds a release muscle, and ideally brings users toward the
core thesis rather than away from it.

Three candidates were considered.

## 2. Candidates

### 2.1 `smelt-datagen`: cheap, ship it, expect little pull

`smelt-datagen` has no production dependency on any other smelt crate (only `arrow`,
`parquet`, `rand`, `clap`, `serde_yaml`), a normative spec, and a CLI. Its distinctive
features are real: seed determinism, entity pools, linked pools for joint distributions,
Hive-partitioned Parquet output, a scale factor. But the space is crowded (Faker, Mimesis,
SDV, Mockaroo, the TPC generators), none of those users would discover smelt through it,
and it says nothing about smelt's thesis.

**Position.** Release it as a *reps* exercise for the release machinery: crates.io,
binary builds, a docs-site page, versioning, a changelog. Bound it at about a week. Do
not treat it as a growth bet.

### 2.2 The analyzer: parser + types + lineage + dialect printer

This is the strong candidate, for a specific reason: sqlglot's limits are exactly the
things this codebase has already paid for.

| sqlglot limit | What the tree already has |
|---|---|
| Lossy AST, fails on partial input | Rowan lossless CST with error recovery (§4.1) |
| `annotate_types` is best-effort, unverified | Type and nullability inference compared exactly to DuckDB, width and decimal scale included, with a two-sided divergence ledger (§4.3) |
| Heuristic transpilation | Registry-driven, per-`(dialect, position)` emission with a live cross-engine value audit and a generated coverage table (§4.2) |
| Lineage as a helper on top | Column lineage produced by the same walk that proves grain and bounds (§4.4) |

Be honest about what it is *not*. Today it parses one dialect (PostgreSQL-tracked,
DuckDB-verified) and emits three. sqlglot's draw is parsing twenty. The honest
positioning today is "one SQL, many targets, verified", not "a sqlglot replacement". §5
is about whether that positioning should change.

### 2.3 The SQL LSP: the most leveraged *face* for the analyzer

A library needs bindings and documentation before anyone tries it. An LSP over plain
`.sql` files with a catalog hookup (DuckDB directly; `information_schema` for the rest)
is a product, and there is a genuine gap: the only serious contender is Postgres-only
(Supabase's `postgres-language-server`, built on `libpg_query`). `smelt-lsp` is most of
the way there. Editor distribution is also where smelt's own extensions (`smelt.ref`,
pipe syntax, the meta-language) ride along for free.

**Position.** The analyzer is the bet. The LSP is its launch surface. Python bindings
are second, because that is where the users are.

## 3. Why everyone cheats, and the one team that did not

Every dbt-adjacent product avoids parsing and typing SQL properly, and each avoidance
names the part of the job that scared them:

- **dbt core** never parses. Jinja renders strings and the warehouse is the type checker,
  at run time, after the query has been paid for.
- **SQLMesh and the lineage vendors** parse via sqlglot with best-effort typing. Nobody
  publishes a conformance number because there is none.
- **Malloy** narrows the surface to a language it controls, so it never parses anyone
  else's SQL.
- **Snowflake and Databricks** own the engine and never need an external analyzer.

The counterexample matters. SDF built a Rust, multi-dialect, statically typed analyzer;
dbt Labs acquired them in early 2025 and made it the core of the Fusion engine. So the
job was worth a company. But the result is source-available under dbt's licence and
welded to dbt Cloud, which leaves the *open, engine-verified* analyzer unbuilt.

Cheating is rational. The surface is unbounded and moving, "correct" has no agreed bar,
and the payoff lands years later, mostly on the ecosystem rather than the vendor. Every
team that starts sees an open-ended grind with no way to ship partial progress.

What is different here is not ambition. It is that the grind has been made **finite and
shippable**:

- The oracle rule bounds the claim to what a live engine verifies.
- The ratchets turn partial coverage into a number that only moves one way, so an
  incomplete dialect is a published coverage table rather than a failure.
- The autonomy loop makes the grind itself cheap: promoting ledger entries into typing
  rules and closing parser gaps against a corpus is exactly the mechanical,
  oracle-checked work it does well.

The framing is therefore not "smelt tackles the big job nobody wants". It is **"the big
job is only big if you cannot verify it, and the verification harness already exists"**.
The published coverage table is the product. Everything else is the grind behind it.

## 4. What exists today (verified 2026-09-07)

### 4.1 Parser

`smelt-parser` depends only on `rowan`. Its grammar is already a *superset*: the syntax
kinds carry sections literally labelled "Multi-dialect superset keywords" and
"Multi-dialect superset nodes" (`QUALIFY`, `PIVOT`/`UNPIVOT`, lambdas, `ARRAY` subscript
and slice, `STRUCT` literals, `ROW`, `EXTRACT`, `COLLATE`, `ANY`/`ALL`, `WITHIN GROUP`,
`FILTER`, `DISTINCT ON`, `TABLESAMPLE`). There is no dialect parameter on the parser;
the superset is accepted uniformly.

Conformance: the DuckDB differential (accept and fidelity directions) has its gap
ratchet at zero against a 53-statement seed, and the external corpus gate runs vendored
DuckDB and PostgreSQL SELECT corpora against a shrink-only ledger. Real-world coverage
against SQL nobody in this repo wrote is untested.

### 4.2 Registry and emission

`smelt-types::signatures::BuiltinRegistry` holds 146 entries. Each carries per-dialect
native return types (`with_engine_native(DialectId, DataType)`) and a per-`(dialect,
position)` emission table (`Native` / `Rename` / `Rewrite` / `Restructure` /
`Unsupported`). `DialectId` is `{DuckDb, SparkSql, PostgreSql, BigQuery}`. The
cross-engine audit derives probes from the registry and runs schema and value legs
against live engines; the coverage table `docs/reference/dialect-coverage.md` is
generated from it and doc-sync gated.

### 4.3 Type inference and oracles

Type inference lives in `crates/smelt-db/src/type_inference/` (fifteen modules). The
module header says it is Salsa-free (no `#[salsa::tracked]`), but it lives inside
`smelt-db`, which drags the whole graph. `smelt-oracle-testkit` has `TypeOracle` and
`ValueOracle` implementations for DuckDB, Spark, and BigQuery. The Spark divergence list
in `crates/smelt-db/tests/prop_helpers/divergences.rs` has 29 `spark_type` entries.

### 4.4 The walk

`crates/smelt-logical/src/analysis/walk.rs` (about 3,600 lines) normalizes a parsed
model into a `QueryTree` (CTEs in dependency order, set-operation arms, derived tables,
expression-position subqueries) and folds it bottom-up with a `Transfer` trait:

```rust
pub trait Transfer {
    type Verdict: Clone;
    fn leaf(&self, leaf: &LeafInput<'_>, cx: &NodeCx) -> Self::Verdict;
    fn operator(&self, op: &OpNode<'_>, children: &[Self::Verdict], cx: &NodeCx) -> Self::Verdict;
}
pub fn walk<T: Transfer>(tree: &QueryTree, transfer: &T) -> T::Verdict;
```

`NodeCx` carries the nesting path, the alias-to-source map, and projected-column lineage.
An unrecognisable construct becomes an explicit `Unsupported` node so a fail-closed
transfer rejects the subtree above it. There are about a dozen in-tree implementors:
grain provenance, lineage, trajectory/footprint, fingerprint, monotonicity trace, output
delta, reach/bounds, partition-grain admission, scope presence, skew, and the composite
`PropertyTransfer`.

Two things the walk lacks for external use: `OpNode` borrows rowan nodes and exposes
parser AST types, so the CST would leak into any public API; and transfers cannot read
one another's verdicts at a node, so they recompute what a sibling already proved.

## 5. Multi-dialect parsing with per-dialect type checking

**Claim.** With the structure in §4 and a lot of testing, this is not a crazy ambition.
The codebase is closer to it than "one dialect" suggests. What is missing is narrower
than a rewrite.

### 5.1 What is actually missing

1. **A dialect parameter on the parser.** The real grammar differences are mostly not
   keywords. They are identifier quoting (backticks, double quotes, brackets), string
   escapes and literal forms (`r''`, `b''`, typed literals), operator precedence and
   meaning, `GROUP BY ALL`, `SELECT * EXCEPT` versus `EXCLUDE` versus `REPLACE`, lateral
   column aliases, `FROM`-first, `LATERAL VIEW`, `UNNEST ... WITH OFFSET`. A superset
   grammar plus a dialect flag handles nearly all of it at the syntax level. That is
   sqlglot's approach and it works there.
2. **Dialect-keyed type inference.** Today there is one smelt semantics and per-engine
   differences are filed as *divergences*. Those 29 Spark entries are not divergences;
   they are per-dialect typing rules waiting to be promoted. The registry's
   `engine_native` field already points the way.
3. **Per-dialect differential runs on native SQL.** The proptest oracle currently checks
   smelt-*printed* SQL against the engine. The multi-dialect version parses that engine's
   *own* SQL and checks the inference against the same engine.

### 5.2 Where it is genuinely hard (not the grammar)

The trap is identical syntax with different semantics: integer division, decimal
precision and scale propagation (every engine has its own formula), implicit coercion
lattices (PostgreSQL strict, Spark and BigQuery lenient), `^`, substring indexing, `LIKE`
case sensitivity, NULL ordering defaults, per-function nullability. Type checking is
where a multi-dialect claim is either true or marketing.

It is tractable here because the rules do not have to be read out of documentation. **The
differential oracle is the spec.** Generative testing against the live engine tells you
each engine's rule, and the two-sided ledger keeps the claim honest.

### 5.3 The scoping rule that keeps it sane

**Support a dialect only if its engine can run in CI.** That is what the existing
conformance invariants already imply, and it draws the line cleanly:

| In | Out (until someone funds an account) |
|---|---|
| DuckDB, PostgreSQL, SQLite, local Spark, BigQuery via dry-run | Snowflake, T-SQL, Oracle, Redshift |

Promising twenty dialects on syntax alone is how sqlglot earned "limited".

### 5.4 What it gives smelt

It dissolves the "which dialect" decision instead of answering it, completing the move
`20260905-engine-defined-semantics.md` started. Users write models in their engine's
dialect; smelt analyzes them; cross-engine moves become a **typed transpile** that is
admissible only when the analyzed output schema agrees on both sides. That is the
existing `projection_dialect_invariance` gate generalized into a product feature, and a
much stronger flexibility story than one canonical dialect with emission tables.

## 6. The normalization seam: one IR, three payoffs

The maintenance walk and its leaf classifiers assume a single semantics. If the CST
becomes dialect-sensitive there are two options:

- make the walk and every classifier dialect-aware, or
- insert a **normalization step after parse** that desugars dialect-specific forms into
  one canonical core, so `smelt-logical` stays single-semantics.

The second is the right seam. It is the analogue of sqlglot's normalizer, and the same
canonical IR serves three purposes at once:

1. **Dialect normalization** (§5): the walk never sees dialect sugar.
2. **The stable public IR** the extensible walk needs (§7): owned, versioned, no rowan
   borrow, no parser AST leaking into the API.
3. **The object a typed transpile is checked on** (§5.4): analyze both sides to the same
   IR and compare output schemas.

Where the seam sits relative to the existing layers: `smelt-parser` (CST, per-dialect
grammar flags) → **normalize** (canonical IR, owned) → `smelt-logical` (walk, unchanged
in kind) and → `smelt-dialect` (print from the IR, not the CST). The seam is the design
decision that needs a spec before any of the rest, because it decides whether
`smelt-logical` is touched.

## 7. Making the walk extensible

The walk is already an extension point in shape (§4.4). What it lacks is a stable data
model at the boundary and composition between transfers. Both have well-trodden prior
art.

### 7.1 Prior art, closest first

- **Apache Calcite `RelMetadataQuery`.** Per-operator handlers compute metadata: column
  origins (lineage), unique keys (grain), column uniqueness and functional dependencies,
  predicates, row counts. Handlers receive an `mq` object so one analysis can ask for
  another's result at the same node; users register providers to add metadata kinds.
  smelt's walk is Calcite metadata with a lattice discipline and a fail-loud rule. The
  composition mechanism is the thing to borrow.
- **MLIR's dataflow framework.** `SparseForwardDataFlowAnalysis::visitOperation(op,
  operandLattices, resultLattices)` over a `Lattice<T>` with `join`. The cleanest
  published statement of "transfer function over a lattice", matching `Verdict` exactly,
  and a reference for how unknown ops are handled (smelt's `Unsupported` node).
- **DataFusion `TreeNode`.** `apply`, `transform_up`, `rewrite` with `TreeNodeVisitor`
  and `TreeNodeRewriter`, plus `AnalyzerRule` and `OptimizerRule`. The Rust idiom people
  expect. Its weakness, walking the raw plan with no scope context, is what `NodeCx`
  already fixes.
- **sqlglot.** `Expression.walk`, `transform`, `traverse_scope`, and
  `lineage(column, sql, schema, dialect)`. The API shape Python users already know; mirror
  its ergonomics, not its lossiness.
- **Attribute grammars** (JastAdd, Silver, Kiama). Synthesized attributes referencing
  other attributes on the same node: the theory behind cross-transfer composition.
- **Python `ast.NodeVisitor`** for the callback-protocol shape.

### 7.2 Rust API additions

1. **A stable, owned IR at the boundary.** The canonical IR from §6. `OpNode` and
   `LeafInput` become views over it, not over rowan.
2. **A product combinator** so several transfers run in one walk.
3. **A query handle** passed into `operator`, Calcite-style, so a transfer can read
   another transfer's verdict for the same node instead of recomputing it. This is what
   turns the built-ins (grain, lineage, bounds, determinism) into building blocks.
4. **One entry point.** Parse against a catalog and dialect; get an analysis handle;
   `run(transfer)`.

Sketch (shape only):

```rust
let analysis = Analyzer::new(dialect, &catalog).analyze(sql)?;   // parse + normalize + type
let lineage  = analysis.lineage();                                // built-in verdicts
let grain    = analysis.grain();
let mine     = analysis.run(&MyTransfer { .. });                  // custom transfer
// inside MyTransfer::operator: cx.query::<Grain>() reads the sibling verdict
```

### 7.3 Python API

Two levels, because most users never write a transfer:

- **Data out.** The normalized tree plus every built-in verdict as dataclasses or JSON:
  lineage, grain, functional dependencies, bounds, fingerprint. Several node types
  already derive `Serialize`. This alone covers the sqlglot-lineage use case with
  verified answers.
- **Callback protocol.** A class with `leaf(self, leaf, cx)` and
  `operator(self, op, children, cx)`, registered via `analysis.walk(MyTransfer())`; Rust
  wraps it in a `Transfer` impl that calls back through PyO3 (already in the tree). Per-
  node boundary crossings are fine: a query has tens of nodes, not millions.

### 7.4 The one real risk

Exposing the tree freezes its shape. Every field of the IR and `NodeCx` becomes API
other products depend on. Version the IR explicitly and keep `Unsupported` as the escape
hatch so grammar growth never breaks a downstream transfer.

## 8. Proposed crate split and phase order

Starting point for a spec, not a plan. Ordered so that each step pays off inside smelt
even if the release stalls after it.

| # | Step | Why this order | Feel |
|---|---|---|---|
| 0 | Release `smelt-datagen` (crates.io + binaries + docs page) | Release reps; no dependency on anything below | ~1 week |
| 1 | **Spec the normalization seam** (§6): canonical IR, its versioning rule, what desugars where, how `smelt-logical` consumes it | The decision everything else hangs on; decides whether the walk is touched | spec, not code |
| 2 | Extract type inference + `smelt-types` into a Salsa-free `smelt-analyzer` crate behind a catalog trait | Same move as `smelt-logical`; improves layering regardless. Structural gate: `cargo tree -p smelt-analyzer -i salsa` shows nothing | weeks |
| 3 | Land the canonical IR; move `smelt-dialect` printing onto it; walk consumes it | Removes the rowan borrow from `OpNode`; enables §7 | weeks |
| 4 | Dialect flag on the parser; per-dialect proptest runs on *native* SQL for DuckDB, PostgreSQL, Spark; promote `spark_type` divergences into dialect-keyed rules | The long pole, but the loop does it: oracle-checked, ratcheted, mechanical | loop time; weeks per engine |
| 5 | Public walk API: product combinator, query handle, stable `NodeCx` | Small once 3 is in | ~1 week |
| 6 | SQL LSP on plain `.sql` files with DuckDB / `information_schema` catalogs | Launch surface; `smelt-lsp` mostly exists | weeks |
| 7 | Python bindings (data-out first, callback protocol second); wasm | Where the users are | weeks |

Expect four to eight weeks of loop time for 2–5 combined. Step 4 is bounded by the
CI-oracle rule in §5.3, not by ambition.

## 9. Risks and open questions

- **External SQL you did not write.** Releasing invites it. The gap ratchet is at zero
  against a 53-statement seed, so real-world coverage is unknown. Treat this as the
  feature: external users become a free corpus for the hardest, most visible part of
  smelt, and the ratchets already exist to absorb it. But expect a flood, and decide up
  front how gaps are triaged (label, ledger entry, loop).
- **Smelt extensions in a general-purpose analyzer.** `smelt.ref`, pipe syntax, and the
  meta-language must be feature-gated or presented as a documented superset. Undecided.
- **Does the walk stay single-semantics?** §6 says yes via normalization. The
  counter-case is a property whose truth depends on the engine (e.g. NULL ordering
  affecting a monotonicity trace). Those facts belong in the registry as engine facts the
  walk reads, per `20260905-engine-defined-semantics.md`, not in the walk itself. Needs
  confirming against each existing transfer.
- **IR versioning policy.** Once external transfers exist, what is a breaking change?
  Adding a node kind is not (they see `Unsupported`); renaming a field is. Write the rule
  down before step 5.
- **Licence and name.** The crates-publishing note records stale 0.1.x listings that can
  only be yanked, not deleted. Decide the public crate names (`smelt-analyzer`?
  something un-smelt-branded?) before step 2 publishes anything.
- **Does this move smelt toward production?** Not directly. It ships the part of smelt
  that is already closest to production quality and lets the maintenance engine earn
  trust behind it. That is the trade being made, and it should be made knowingly.

## 10. Suggested next artifacts

1. `docs/specs/canonical_ir.md` (or a section of `architecture.md`): the normalization
   seam, its invariants, and how it relates to the property-composition-walk rule.
2. A spec diff to `architecture.md` adding "Support a dialect only if its engine runs in
   CI" as a constraint alongside the conformance-gate invariants.
3. A short release checklist for `smelt-datagen` to run step 0 immediately.
