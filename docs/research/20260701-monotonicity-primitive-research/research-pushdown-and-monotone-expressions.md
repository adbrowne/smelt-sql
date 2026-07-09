# Research: predicate-pushdown soundness & static detection of monotone scalar expressions

**Purpose.** Ground the design doc `docs/research/20260701-expanding-incremental-eligibility.md` — specifically **Part 4** ("eligibility = maximal predicate-pushdown depth") and the closing **monotonicity-primitive** section — in authoritative prior art. The doc reframes incremental eligibility as: the model is incrementalisable exactly when selection on the projected `event_time` column *commutes* with every operator between the outer `SELECT` and the source, and the deepest point σ can be pushed is where the filter should be written. That reframing needs two bodies of theory: (1) the classical **predicate-pushdown soundness** laws (when does σ commute with π / ⋈ / ∪ / γ?), and (2) a **static test for monotone scalar expressions** (when is a projected timestamp an order-preserving function of a source partition column, so a range predicate on it can be pushed to the source?).

Verification note: every source below was checked for existence via search and, where possible, fetched. Sources that could not be text-verified (binary-served PDFs, JS-rendered pages, paywalls) are flagged inline. Prefer the primary sources; the flagged secondaries are terminology/corroboration only.

---

## Synthesis — the key insight

**smelt's "eligibility = commutation = pushdown validity" reframing is exactly the classical predicate-pushdown soundness question, restated for a temporal predicate.** The doc's Part 4 walk — "start at the outer `event_time` column, push σ toward the sources one operator at a time, stop at the first operator it does not commute with; where it stops is where to write the filter, and how far it got is the eligibility verdict" — is a per-query instance of the logical-optimization law set in Garcia-Molina/Ullman/Widom §16.2 (splitting selections, pushing selections through projection, into join inputs, over union, below grouping). Each row of the doc's §4.3 table maps to one classical law and its precondition:

| Part 4 / doc row | Classical law | Precondition |
|---|---|---|
| transparent body (project/filter/rename) → push to source scan | σ commutes with π and with σ; σ_θ(π(R)) ≡ π(σ_θ(R)) | θ references only projected/renamed columns |
| aggregation, `GROUP BY` key ⊇ `partition_column` → push below the aggregate | σ pushes below γ (GMUW §16.2.7) | predicate references only grouping columns; **no empty grouping set** |
| `UNION ALL` branches → push into each branch | σ distributes over bag/set union (GMUW §16.2.2) | branch schemas share the predicate's columns |
| fact ⋈ lookup dim → push to fact input only | σ_θ(R ⋈ S) ≡ σ_θ(R) ⋈ S | θ references only R's attributes (the driving fact) |
| `DISTINCT` / `LIMIT` / cross-window frame → nowhere | σ does **not** commute past these | — (pushdown wall = eligibility wall) |

The second half of the reframing — pushing a predicate on a **derived** `event_time` (e.g. `DATE_TRUNC(created_at)`, `CAST(ts AS DATE)`) down to a **source partition column** — is the *monotone-function range rewrite*: `f(x) BETWEEN a AND b ⟺ x BETWEEN f⁻¹(a) AND f⁻¹(b)` when `f` is order-preserving. This is undecidable for arbitrary `f` (Richardson's theorem), so every production system that does it uses a **conservative whitelist of known-monotone built-ins**.

**The three closest production implementations of the monotone-expression primitive smelt needs are:**

1. **ClickHouse `IFunctionBase::getMonotonicityForRange`** — the one production engine that reasons about function monotonicity *at plan time* to push a predicate on a derived expression back onto a sorted source key. It returns a four-boolean `Monotonicity` verdict (`is_monotonic`, `is_positive`, `is_always_monotonic`, `is_strict`) per function *per range*, consumed by primary-key/partition analysis (`KeyCondition`). This is the closest structural analog to the verdict smelt's classifier must return.
2. **Apache Iceberg partition transforms** — formalizes transform monotonicity as a per-transform `preserves_order` boolean plus a `project(predicate)` method that rewrites a source-column predicate into a partition-value predicate. Order-preserving transforms (`year/month/day/hour`, `truncate`, `identity`) can project a **range**; `bucket` (a hash) can only project **equality**. This is the cleanest *formalization* of exactly smelt's licensing rule.
3. **Delta Lake generated columns** — the inverse layout (partition column is the derived one) but the identical enabling condition: a **fixed whitelist** of order-preserving generation expressions (`CAST(ts AS DATE)`, `YEAR/MONTH/DAY/HOUR`, prefix `DATE_FORMAT`, `SUBSTRING`) whose inverse image of a range is again a range, letting a query on the source column produce a partition filter.

The relational incumbents (Oracle, PostgreSQL, SQL Server) are the **negative baseline**: they deliberately do *not* reason about monotonicity — any function wrapping the partition key defeats pruning, and the sanctioned workaround is to materialize the transform as a virtual/generated column. DuckDB (a smelt target) provides the *downstream* mechanism — zonemaps plus statistics propagation — that a smelt-side monotone rewrite would feed, but it will not derive the source-range from a derived-column predicate itself. **This is precisely the gap smelt's compiler-side monotonicity primitive fills, and Part 4's argument for "push at compile time rather than trust the engine" is directly supported: only ClickHouse would do this rewrite for you, and smelt is multi-backend.**

---

## Area 1 — Predicate-pushdown formal soundness (relational algebra)

### Garcia-Molina, Ullman & Widom, *Database Systems: The Complete Book* (2nd ed.) — the canonical law set
- **Citation:** Hector Garcia-Molina, Jeffrey D. Ullman, Jennifer Widom. *Database Systems: The Complete Book*, 2nd edition. Pearson/Prentice Hall, 2009. ISBN 978-0131873254. Chapter 16 "The Query Compiler," §16.2 "Algebraic Laws for Improving Query Plans."
- **URL:** https://dl.acm.org/doi/book/10.5555/560797 (ACM catalog); https://books.google.com/books/about/Database_Systems.html?id=gaEuAAAAQBAJ
- **Contribution & mapping.** The canonical citable statement of the logical-optimization toolkit smelt's Part 4 walk *is*. §16.2 subsections give exactly the laws the doc invokes: commutative/associative laws (16.2.1); laws involving selection incl. the splitting law σ_{p∧q}(R) = σ_p(σ_q(R)) and distribution of σ over bag **and** set union/intersection/difference (16.2.2); "Pushing Selections" (16.2.3); laws for projection (16.2.4); joins and products, incl. pushing a selection into the join input whose attributes the predicate references (16.2.5); duplicate elimination (16.2.6); grouping/aggregation (16.2.7). smelt's §2.2 (σ distributes over UNION), §3.2 (σ pushes through project/filter), and §5.3 (σ into the fact-side join input) are direct instances. *Flag: §16.2 sub-numbering is from the standard 2nd-ed. ToC; the Stanford ToC host is HTTP-only and could not be re-fetched — treat sub-section numbers as standard-edition.*

### The single-input join-pushdown condition
- **Statement:** σ_θ(R ⋈ S) ≡ σ_θ(R) ⋈ S **iff** θ references only attributes of R (symmetrically for S); a conjunctive predicate is split and each conjunct routed to the side(s) whose attributes it mentions.
- **Citable in:** GMUW §16.2.2–16.2.5, and the lecture notes below.
- **Mapping.** This *is* smelt's §5.4 "driving-fact" condition: a join is incrementalisable when `event_time`'s predicate is expressible over exactly one input (the fact), so σ descends into that input alone and every other input stays full-scanned. The soundness-critical precondition — "predicate expressible over that input's schema alone" — is the formal statement of "identify the single clock-bearing input."

### University lecture notes (secondary / teaching restatements)
- **Citations & URLs:** UW-Madison CS564 Lecture 19 "Query Optimization" https://pages.cs.wisc.edu/~paris/cs564-s18/lectures/lecture-19.pdf ; CMU 15-445/645 Lecture 13 "Query Planning & Optimization I" https://15445.courses.cs.cmu.edu/fall2021/notes/13-optimization1.pdf ; Northeastern CS3200 "Relational Query Optimization" https://www.khoury.northeastern.edu/home/kathleen/classes/cs3200/19-QOptimize.pdf
- **Contribution & mapping.** Self-contained σ/π/⋈-notation restatements of the same equivalences (split conjunctive selections; commute σ with π and ⋈; distribute σ over union; push σ to the referenced join input) as the *logical-rewrite* layer that precedes cost-based search — useful for citing the laws without the textbook. *Flag: all three URLs exist; the CS564 and CMU PDFs are served as compressed binary the fetch tool could not convert — existence verified, verbatim content not.*

### Predicate pushdown below GROUP BY — the empty-grouping-set caveat
- **Statement:** σ_p pushes below γ (GROUP BY/aggregation) when p references only grouping columns (GMUW §16.2.7) — the filter partitions groups without altering per-group aggregates. **Exception:** invalid when the aggregation has an **empty grouping set** (global aggregate), because a group can survive with zero rows; the predicate must then be retained *above* the aggregate.
- **Corroborating production source:** PrestoDB PR #11297 "Fix incorrect predicate pushdown through empty grouping set." https://github.com/prestodb/presto/pull/11297/files
- **Mapping.** Directly grounds smelt's §3.2 row 3 / §4.3 row 2 ("aggregation whose `GROUP BY` key ⊇ `partition_column` → push below the aggregate, safe because each group lives in one window"). The Presto PR is a real-optimizer statement of the precondition *and* its exception — smelt's classifier must likewise require the `event_time`/`partition_column` predicate to reference only grouping keys, and must exclude the global-aggregate (no `GROUP BY`) case. *Flag: the frequently-cited UW CSE444 "Predicate Pushdown (through grouping)" slide is now behind a NetID login wall — excluded as non-citable.*

---

## Area 2 — Monotone / order-preserving expression detection in real optimizers

### ClickHouse `IFunctionBase::getMonotonicityForRange` + the `Monotonicity` struct — THE production implementation
- **Citation:** ClickHouse/ClickHouse, `src/Functions/IFunction.h`, `master`. (Consumed by `src/Storages/MergeTree/KeyCondition.cpp`.)
- **URL:** https://github.com/ClickHouse/ClickHouse/blob/master/src/Functions/IFunction.h
- **The struct (verified verbatim from source):**
  ```cpp
  struct Monotonicity {
      bool is_monotonic = false;        // non-decreasing OR non-increasing on the range
      bool is_positive = true;          // true => non-decreasing; false => non-increasing
      bool is_always_monotonic = false; // monotonic on the whole input domain
      bool is_strict = false;           // strictly (not weakly) monotone
  };
  virtual Monotonicity getMonotonicityForRange(
      const IDataType &, const Field & left, const Field & right) const;
  ```
  Called only when `hasInformationAboutMonotonicity()` is true; `left`/`right` may be `NULL` to signal an unbounded end.
- **Mapping — the closest structural analog to smelt's primitive.** These four booleans are exactly the vocabulary smelt's classifier needs to *push a range predicate on a derived `event_time` back onto the source*: `is_monotonic` (may I push at all?), `is_positive` (must I **flip** `<`↔`>` when rewriting?), `is_always_monotonic` (can I skip the per-range endpoint check?), `is_strict` (does `<` stay `<` or weaken to `<=`?). `KeyCondition` uses this to rewrite a predicate on `toStartOfDay(ts)`/`toDate`/`negate`/`CAST` into a predicate on the sorted primary key. smelt's "projected `event_time` is a monotone function of the source partition column" is `toStartOfDay` reporting `is_monotonic = true`. Corroborating real PRs/issues (verified to exist): #71440 (negate monotonicity), #14513 (binary-operator monotonicity), #28087 (`toDate(datetime64)` index analysis). *Flag: struct verified in `IFunction.h`; `KeyCondition.cpp` not deep-read line-by-line but its consumer role is well corroborated.*

### Altinity — "Learning to appreciate monotonic functions in ClickHouse" (mechanism + whitelist)
- **Citation:** Altinity technical blog. **URL:** https://altinity.com/blog/learning-to-appreciate-monotonic-functions-in-clickhouse
- **Contribution & mapping.** Explains the **factor-transformation** trick that makes *piecewise*-monotone date functions usable: each transform `T` is paired with a coarser factor `F` (e.g. `toDayOfWeek` factored by `toStartOfWeek`); `T` is monotone on any range over which `F` is constant, so `getMonotonicityForRange` evaluates `F` at both endpoints and, if equal, declares `T` monotone *on that range*. This is a technique smelt could reuse for `event_time` expressions that are only piecewise-monotone. It enumerates ClickHouse's actual whitelist: **fully monotone** (`toYear`, `toStartOfWeek/Day/…`, `sqrt`, `exp`, `floor/ceil/round`, `plus`, `multiply`), **piecewise** (`toMonth`, `toDayOfMonth`, `toDayOfWeek`, `toWeek`), **casts/type conversions**. Benchmark payoff: `toYear(FlightDate) = …` reads 1,308 marks in 78 ms (monotone → key retained) vs. 513 ms for non-monotone `toString(FlightDate)`.

### Oracle — partition pruning defeated by any function on the key (negative baseline)
- **Citation:** *Oracle Database VLDB and Partitioning Guide (21c)*, Ch. 3 §3.1 "Partition Pruning." **URL:** https://docs.oracle.com/en/database/oracle/oracle-database/21/vldbg/partition-pruning.html
- **Contribution & mapping.** Emphatic that **any** function on the partition key defeats pruning: "When you manipulate a partition column with any function or transformation, such as `CAST` or `TRUNC`, partition pruning is not taking place… irrespective of the nature of the function" — *no monotone exception*. Flags implicit conversions as a silent trap (`DATE` key compared to a `TIMESTAMP` value → `PARTITION RANGE ALL`; the `BETWEEN TO_DATE(...) AND TO_DATE(...)` form that leaves the key untouched → `PARTITION RANGE ITERATOR`). Oracle's sanctioned workaround is to materialize the transform as a **virtual column** and partition on that — the "push the transform into the physical layout" answer, opposite to ClickHouse's plan-time reasoning. Grounds smelt's Part 4 §4.4 argument: relying on the engine's optimizer is fragile because major engines simply won't do it.

### PostgreSQL / SQL Server — partition elimination requires the bare key (negative baseline)
- **Citations & URLs:** PostgreSQL 18 §5.12 "Table Partitioning" https://www.postgresql.org/docs/current/ddl-partitioning.html ; Microsoft Learn "Partitioned Tables and Indexes" https://learn.microsoft.com/en-us/sql/relational-databases/partitions/partitioned-tables-and-indexes ; Conor Cunningham (orig. SQL Server QO architect) "An Introduction to Partition Elimination" https://www.sqlskills.com/blogs/conor/an-introduction-to-partition-elimination/
- **Contribution & mapping.** Both require the predicate to compare the partition column *directly* to constants. PostgreSQL states the rule of thumb: constraints "should contain only comparisons of the partitioning column(s) to constants using B-tree-indexable operators," and gives the exact smelt anti-pattern: `WHERE EXTRACT(YEAR FROM logdate) = 2008` is **not** pruned, while `WHERE logdate >= '2008-01-01' AND logdate < '2009-01-01'` **is**. SQL Server eliminates only when filtering on the raw partitioning column; the escape hatch is manual `$PARTITION.fn(col)` ordinal filtering. This is the negative baseline: absent monotonicity reasoning, the framework (smelt) must synthesize the "range on `f(ts)` ⇒ range on `ts`" rewrite itself — these engines will not.

### DuckDB — zonemaps + statistics propagation (the downstream mechanism smelt feeds)
- **Citations & URLs:** DuckDB "Indexing" performance guide https://duckdb.org/docs/stable/guides/performance/indexing ; DuckDB blog "Optimizers: The Low-Key MVP" (2024-11-14) https://duckdb.org/2024/11/14/optimizers ; source `src/optimizer/statistics_propagator.cpp` https://github.com/duckdb/duckdb/blob/main/src/optimizer/statistics_propagator.cpp
- **Contribution & mapping.** DuckDB auto-creates **zonemaps** (min-max indexes) per row group and skips groups whose range excludes a filter value (also read from Parquet row-group metadata). The `STATISTICS_PROPAGATION` pass walks the plan carrying min/max/null stats and synthesizes new filters — canonically from **join equalities/comparisons** (e.g. `t1.a ∈ [25,50]` on `t1.a = t2.a` → range filter on `t2.a` resolved against `t2` zonemaps). *Caveat to flag:* the verified sources do **not** show DuckDB inferring "range on `date_trunc(ts)` ⇒ range on `ts`" via function monotonicity. So DuckDB will exploit the source-column range predicate (via zonemaps) *once smelt has done the monotone rewrite*, but will not perform that rewrite for you — supporting the "push at compile time" thesis for a DuckDB target. (Claim is strongly-supported-by-absence, not an explicit disclaimer in the docs.)

---

## Area 3 — Derived / generated-column partition pruning (the closest industry analogs)

### Apache Iceberg — partition transforms & `preserves_order` (the cleanest formalization)
- **Citation (spec):** *Apache Iceberg Table Spec — "Partition Transforms".* Apache Software Foundation (`format/spec.md`, `main`). **URL:** https://iceberg.apache.org/spec/
- **Citation (order property):** PyIceberg `pyiceberg/transforms.py`, `main`. **URL:** https://py.iceberg.apache.org/reference/pyiceberg/transforms/
- **Citation (hidden partitioning):** *Iceberg Docs — "Partitioning".* **URL:** https://iceberg.apache.org/docs/latest/partitioning/ (secondary walkthrough: Dremio, "Hidden Partitioning…" https://www.dremio.com/blog/hidden-partitioning-how-iceberg-eliminates-accidental-full-table-scans/)
- **Contribution & mapping — the reference design for smelt's primitive.** Iceberg's closed transform set (`identity`, `bucket[N]`, `truncate[W]`, `year`, `month`, `day`, `hour`, `void`) is a function from source value → partition value, and the spec states partition specs "transform predicates to partition predicates" during scan planning. The strongest formalization is in source: each transform carries a boolean **`preserves_order`** — `TimeTransform` (and its `Year/Month/Day/Hour` subclasses) → `True`; `Identity`, `Truncate` → `True`; `Bucket`, `Void` → `False` — and a **`project(predicate)`** method. Order-preserving transforms can project a *range* (`<`, `<=`, `>`, `>=`) predicate onto the partition field; `bucket` can project only **equality** (`=`, `IN`). **This is precisely smelt's licensing rule:** a range-predicate pushdown is sound only when the derivation is order-preserving; monotone time transforms qualify, a hash does not. smelt's static primitive is essentially a compile-time `preserves_order` classifier + a `project`-style rewriter over the model's `event_time = f(source_col)` expression. *Flag: the `spec/` and `docs/partitioning/` HTML are JS-rendered (fetch returned only chrome); `preserves_order` values were cross-checked against `transforms.py` source because a rendered API page misreported inherited defaults — the source confirms time transforms ARE order-preserving.*

### Delta Lake — generated columns & derived partition filters (the whitelist analog)
- **Citation:** *"Delta Lake generated columns."* Databricks Documentation. **URL:** https://docs.databricks.com/aws/en/delta/generated-columns (mirror: https://learn.microsoft.com/en-us/azure/databricks/delta/generated-columns). Trino's implementation of the same: https://github.com/trinodb/trino/issues/19455
- **Contribution & mapping — a near-exact inverse analog.** A partition column is defined by a generation expression, e.g. `date GENERATED ALWAYS AS (CAST(ts AS DATE))`. When a query filters on the **base** column, "Delta Lake looks at the relationship between the base column and the generated column, and populates partition filters based on the generated partition column if possible" — i.e. the generation expression is inverted to derive a partition filter. Automatic pushdown is limited to a **specific whitelist** of order-preserving expressions: `CAST(col AS DATE)` (TIMESTAMP), `YEAR(col)` and the `YEAR/MONTH/DAY/HOUR` families, `SUBSTRING(col, pos, len)` (STRING), and prefix-form `DATE_FORMAT(col, fmt)` (`yyyy-MM`, `yyyy-MM-dd-HH`). Generation expressions must be **deterministic** (no UDFs/aggregates/window/table functions); confirm via `EXPLAIN`. This is Delta's version of smelt's inverse case (partition col is derived, source is queried) with the *identical* enabling condition — a fixed catalogue of monotone/prefix-preserving functions whose inverse image of a range is a range. Delta's whitelist is a battle-tested enumeration smelt can mirror.

### Apache Hive — explicit partition columns (prior-art baseline / the anti-pattern)
- **Citation:** *Apache Hive — "DynamicPartitions".* Apache Software Foundation. **URL:** https://cwiki.apache.org/confluence/display/Hive/DynamicPartitions
- **Contribution & mapping.** Hive partition columns are explicit, physically materialized as directory paths, with **no** declared source→partition transform. The metastore prunes only when a query references the partition column *directly, un-wrapped*; filtering on the underlying source timestamp forces a full scan ("accidental full table scan") because Hive cannot relate source and derived columns. This is the failure mode Iceberg/Delta were built to fix, and it demonstrates *why smelt needs a static primitive at all*: absent a machine-checkable monotone relationship between derived `event_time` and source partition key, a system falls back to Hive behavior. smelt's contribution is to recover the Iceberg/Delta capability at compile time from the model's SQL.

---

## Area 4 — Range-predicate rewriting through monotone functions

### Explain Extended — "Sargability of monotonic functions" (the target rewrite, worked)
- **Citation:** Alex Bolenok ("Quassnoi"), "Things SQL needs: sargability of monotonic functions," Explain Extended, 19 Feb 2010. **URL:** https://explainextended.com/2010/02/19/things-sql-needs-sargability-of-monotonic-functions/
- **Contribution & mapping.** A precise worked statement of smelt's exact rewrite: for strictly monotone `f` with known inverse, `f(x) BETWEEN a AND b ⟺ x BETWEEN f⁻¹(a) AND f⁻¹(b)`, converting a non-sargable predicate into an index range seek on `x`. Walks common SQL functions and their monotonicity (`YEAR`/`MONTH`/date truncation, integer division, `EXP`, `FLOOR`, `LEFT`-substring, arithmetic) and notes that wrapping the indexed column in a function normally defeats the index *unless* the optimizer knows the function is order-preserving. Primary framing for "monotone ⇒ range-preserving pushdown" — the operation Part 4 §4.5 wants the classifier to perform.

### SARGability background (terminology)
- **Citations & URLs:** Microsoft "Predicate Pushdown and why should I care?" https://learn.microsoft.com/en-us/archive/blogs/blogdoezequiel/predicate-pushdown-and-why-should-i-care ; "SARGability and Predicate Pushdown," The Skilled Coder https://theskilledcoder.com/posts/dbms-sql/sargability-and-predicate-pushdown
- **Contribution & mapping.** Establish the mechanism: a predicate is SARGable when it can drive an index seek, which needs the column *un-wrapped*; `BETWEEN` decomposes to `col >= a AND col <= b`. The payoff of smelt's monotone rewrite is precisely converting a residual/scan predicate into a seekable/prunable source-column range. Secondary/vendor sources, terminology only.

### Commercial optimizers track monotonicity for range pushdown (existence evidence)
- **Citation:** US Patent 8,176,035, "Detecting and tracking monotonicity for accelerating range and inequality queries." **URL:** https://patents.google.com/patent/US8176035
- **Contribution & mapping.** Evidence that commercial optimizers implement monotonicity-tracked range pushdown (function-based index range scans). Cite only as existence evidence, not as a peer-reviewed proof.

---

## Area 5 — Decidability limits (why a whitelist, not a general prover)

### Richardson's theorem (primary — undecidability of expression properties)
- **Citation:** Daniel Richardson, "Some Undecidable Problems Involving Elementary Functions of a Real Variable," *The Journal of Symbolic Logic* 33(4), 1968, pp. 514–520. **URL / DOI:** https://doi.org/10.2307/2271358 (JSTOR https://www.jstor.org/stable/2271358)
- **Contribution & mapping.** The foundational undecidability result: for expressions built from rationals, π, ln 2, `x`, +, −, ×, composition, and `sin`/`exp`/`abs`, it is undecidable whether an expression is identically zero, and whether it is nonnegative everywhere (i.e. whether `A(x) < 0`). Since deciding monotonicity of `f` reduces to deciding the sign of its difference/derivative everywhere, **general expression monotonicity is not statically decidable** — the formal justification for smelt's §4.6 conservatism ("when it cannot prove the outer `event_time` traces back monotonically to the source, it stays at the outer clamp or rejects") and for using a whitelist rather than a general prover.

### Encyclopedic confirmations
- **Citations & URLs:** Wikipedia "Richardson's theorem" https://en.wikipedia.org/wiki/Richardson%27s_theorem ; Wolfram MathWorld https://mathworld.wolfram.com/RichardsonsTheorem.html
- **Contribution & mapping.** Both state the precise expression class and the undecidable predicates (equality, identity-to-zero, nonnegativity). Wikipedia notes that removing `sin` recovers decidability (Tarski–Seidenberg for real closed fields) — a clean way to explain why *polynomial / order-preserving casts* are decidable but general elementary functions are not. Non-paywalled anchors for the claim.

### Wang — undecidability of existence of zeros (companion)
- **Citation:** Paul S. Wang, "The Undecidability of the Existence of Zeros of Real Elementary Functions," *JACM* 21(4), 1974, pp. 586–589. **URL / DOI:** https://doi.org/10.1145/321850.321856
- **Contribution & mapping.** Strengthens Richardson: existence of a real zero of a real elementary function is undecidable. Detecting where `f(x)` crosses a threshold — a prerequisite for reasoning about order-preservation over an interval — is itself undecidable. Reinforces that no optimizer can prove a user-defined expression monotone in general.

### The practical consequence — a fixed whitelist
- Because monotonicity is undecidable in general, production optimizers recognize a **hard-coded whitelist** of built-ins known to be order-preserving and apply the range rewrite only for those (ClickHouse's `getMonotonicityForRange` overrides, Iceberg's `preserves_order`, Delta's generated-column whitelist, PostgreSQL/Oracle refusing everything else). Supporting: Explain Extended frames current engines as exploiting monotonicity only for a small hardcoded set; the PrestoDB PR shows the same conservative case-by-case guarding.

---

## Proposed monotone-builtin whitelist for smelt (synthesized)

Synthesized from ClickHouse (`getMonotonicityForRange` overrides + Altinity enumeration), Iceberg transforms (`preserves_order`), Delta generated-column pushdown, and the Explain-Extended list. The primitive answers: *does this projected `event_time` expression trace back monotonically to a real source partition column, and how must the range endpoints be rewritten?* Each entry notes the direction and endpoint-inclusivity handling a rewriter must get right (from ClickHouse's `is_positive` / `is_strict`).

**Recognize as monotone (license range pushdown to source):**

| Expression form | Monotone? | Notes for the rewriter |
|---|---|---|
| identity / pure rename / alias (`created_at AS event_time`) | strictly increasing | trivial; the transparent slice (doc §3.2, §4.3 row 1) |
| `DATE_TRUNC(unit, ts)` / `toStartOf{Day,Hour,Month,Year}` | **weakly** increasing (non-strict) | order-preserving but many-to-one; `<` on truncated value may need care at bucket boundaries — push as a **closed** source range covering the truncation bucket |
| `CAST(ts AS DATE)` (timestamp → date) | weakly increasing | Delta's canonical whitelist entry; widening/precision-lowering but order-preserving |
| order-preserving / widening casts: `INT→BIGINT`, `DATE→TIMESTAMP`, numeric widening | strictly increasing | ClickHouse `FunctionsConversion` monotonicity; excludes lossy/reordering casts (e.g. → string, → unsigned wrap) |
| `EXTRACT`/`YEAR`/`MONTH`/`DAY`/`HOUR` **used with matching range bounds** | piecewise increasing | monotone only *within* a factor-constant range (ClickHouse factor-transform trick); safest to require the outer predicate be a range on the extracted unit and map to the source interval, à la Iceberg `year/month/day/hour` |
| `ts + INTERVAL k` / `ts - INTERVAL k` (constant shift) | strictly increasing | invert by subtracting/adding the constant |
| `x + c`, `x - c` (constant addend) | strictly increasing | |
| `x * c`, `x / c` for **constant c > 0** | strictly increasing | **flip** comparison direction if `c < 0` (`is_positive = false`) — reject/flip, never assume increasing |
| `FLOOR(x)`, `CEIL(x)`, `ROUND(x)` | weakly increasing | many-to-one; push as closed range |
| `truncate[W]` / `SUBSTRING(col,1,k)` prefix | weakly increasing (order-preserving prefix) | Iceberg `truncate`, Delta `SUBSTRING`/prefix `DATE_FORMAT`; **prefix only** (fixed leading length) |

**Reject for range pushdown (not order-preserving — equality-only at best, or opaque):**

- Hashing / `bucket[N]` / `MOD`, `%` — non-monotone (Iceberg `bucket.preserves_order = False`; equality/`IN` projection only, never a range).
- `x²`, `ABS(x)`, and other fold-at-zero functions — non-monotone over a domain spanning the fold point.
- `CAST(ts AS STRING)` / `toString(...)` — lexical order ≠ temporal order (Altinity's non-monotone benchmark case).
- Non-deterministic (`RANDOM`, `NOW`/`CURRENT_TIMESTAMP` when row-varying, `UUID`) — no fixed inverse; overlaps smelt's B5 gate.
- Any UDF / unrecognized function — conservative default: not monotone (Richardson's theorem → cannot prove; Delta forbids UDFs in generation expressions).
- Composition is monotone only if **every** component is monotone *and directions compose consistently*; a single non-monotone component poisons the chain.

**Design guidance for the primitive (from the prior art):**
1. Return a **verdict object, not a boolean** — mirror ClickHouse's `Monotonicity` (`is_monotonic`, direction/`is_positive`, `is_strict`) so the rewriter knows whether to flip the comparison and whether to keep endpoints open or closed. This also satisfies Part 4 §4.5's "classifier returns the deepest injection point, not safe/unsafe."
2. **Conservative default is non-monotone** (Richardson). Unknown/unrecognized ⇒ stay at the outer clamp or reject; never push an unlicensed filter (doc §4.6).
3. For **weakly** monotone (many-to-one) date-truncation/floor cases, push a **closed** source range covering the full pre-image bucket — the equivalence is over buckets, not points.
4. Prefer Iceberg's `project`-style framing: emit the *derived* source predicate as the artifact, rather than trusting the backend optimizer to rediscover it (Part 4 §4.4) — essential because Oracle/PostgreSQL/SQL Server won't, and only ClickHouse among common backends would.

---

## Consolidated source list

- GMUW, *Database Systems: The Complete Book* 2e, Ch.16 §16.2 — https://dl.acm.org/doi/book/10.5555/560797
- PrestoDB PR #11297 (group-by pushdown caveat) — https://github.com/prestodb/presto/pull/11297/files
- UW CS564 L19 / CMU 15-445 L13 / NEU CS3200 (RA optimization laws) — see Area 1
- ClickHouse `IFunction.h` (`Monotonicity` / `getMonotonicityForRange`) — https://github.com/ClickHouse/ClickHouse/blob/master/src/Functions/IFunction.h
- Altinity, monotonic functions in ClickHouse — https://altinity.com/blog/learning-to-appreciate-monotonic-functions-in-clickhouse
- Oracle VLDB & Partitioning Guide, §3.1 — https://docs.oracle.com/en/database/oracle/oracle-database/21/vldbg/partition-pruning.html
- PostgreSQL §5.12 Table Partitioning — https://www.postgresql.org/docs/current/ddl-partitioning.html
- SQL Server partitioned tables — https://learn.microsoft.com/en-us/sql/relational-databases/partitions/partitioned-tables-and-indexes ; Cunningham — https://www.sqlskills.com/blogs/conor/an-introduction-to-partition-elimination/
- DuckDB indexing/zonemaps — https://duckdb.org/docs/stable/guides/performance/indexing ; optimizers blog — https://duckdb.org/2024/11/14/optimizers ; `statistics_propagator.cpp` — https://github.com/duckdb/duckdb/blob/main/src/optimizer/statistics_propagator.cpp
- Iceberg spec (transforms) — https://iceberg.apache.org/spec/ ; PyIceberg `transforms.py` (`preserves_order`) — https://py.iceberg.apache.org/reference/pyiceberg/transforms/ ; hidden partitioning — https://iceberg.apache.org/docs/latest/partitioning/
- Delta generated columns — https://docs.databricks.com/aws/en/delta/generated-columns ; Trino #19455 — https://github.com/trinodb/trino/issues/19455
- Hive DynamicPartitions — https://cwiki.apache.org/confluence/display/Hive/DynamicPartitions
- Explain Extended, sargability of monotonic functions — https://explainextended.com/2010/02/19/things-sql-needs-sargability-of-monotonic-functions/
- US Patent 8,176,035 (monotonicity tracking) — https://patents.google.com/patent/US8176035
- Richardson (1968), JSL — https://doi.org/10.2307/2271358 ; Wikipedia — https://en.wikipedia.org/wiki/Richardson%27s_theorem ; MathWorld — https://mathworld.wolfram.com/RichardsonsTheorem.html
- Wang (1974), JACM — https://doi.org/10.1145/321850.321856

**Verification flags:** GMUW §16.2 sub-numbers from standard 2e ToC (host HTTP-only). CS564/CMU lecture PDFs exist but served as binary (existence verified, not verbatim). UW CSE444 grouping-pushdown slide behind login — excluded. Iceberg spec/docs pages JS-rendered — `preserves_order` cross-checked against `transforms.py` source (time transforms confirmed order-preserving). DuckDB "no monotone push-through" is supported-by-absence, not an explicit disclaimer. ClickHouse `KeyCondition.cpp` not deep-read; `Monotonicity` struct verified in `IFunction.h`.
