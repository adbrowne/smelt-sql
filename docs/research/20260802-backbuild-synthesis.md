# Backbuild synthesis — optimized migration scripts from model-definition diffs

**Date**: 2026-08-02
**Status**: research (pre-spec — deliberate; implementation plan at
`docs/plans/20260802-backbuild-synthesis.md`; spec extraction happens at wiring time)

## 0. The question

When a model's definition changes, today's outcomes are polar. Either the change is a purely
additive column add on a *maintained* model — the definition-change trigger
(`docs/specs/incremental_models.md` §"The definition-change trigger") backfills the new column
group — or the deployed table is rebuilt from scratch (`schema_evolution.md`'s `FullRefresh`;
`virtual_environments.md`'s "breaking ⇒ rebuild and cascade"). Between those poles sits a large
class of edits whose effect on the deployed table is reachable by a targeted script — an
`ALTER`, a column-scoped `UPDATE`/`MERGE`, a predicate-scoped `DELETE`/`INSERT` — orders of
magnitude cheaper than recomputing a large table.

**Backbuild synthesis**: given only the before and after model definitions (plus declared
physical facts), factor the diff into atomic semantic changes, prove per atom that a targeted
script reaches the same end state a full rebuild would, and emit the ordered script. Anything
unprovable is a named refusal (fail-loud discipline), and full refresh remains the fallback.

Scope decisions taken up front (2026-08-02 discussion):

- **Any table-materialized model**, not only maintained/timeseries models. A plain `full`
  table is arguably the biggest win — today it rebuilds entirely on any edit.
- **Standalone pure module** in `smelt-logical`, not wired into any pipeline yet. Test cases
  are the deliverable; CLI/runtime/virtual-environment wiring and the maintained-model ledger
  integration come later.
- **Verification is DuckDB oracle equivalence**, matching the maintenance-conformance
  convention: build the old table from the before-definition, run the emitted script, assert
  table-equality with a fresh full build of the after-definition over the same inputs.

## 1. Relationship to existing machinery

| System | What it already does | What backbuild adds |
|---|---|---|
| Definition-change trigger (`incremental_models.md`, `maintenance/derive.rs::derive_column_added`) | Classifies an **added** column on a **maintained** model: `SkeletonAdd` refuse / `PureBackfill` in-place `UPDATE` / `UpstreamRederive` column-scoped `MERGE` | Same verdict vocabulary, generalized: any table model, plus renames, changed expressions, row-set changes |
| Additive-only model-diff (`analysis/model_diff.rs`, `model_properties.md`) | Proves an edit *purely adds* columns derivable from `{existing} ∪ {monotone dims}`; anything else (including any change to an existing column) is `NotAdditive` — the "L3 residue" left to declared migration intent | Backbuild **deliberately crosses the L3 boundary**: with both definitions in hand, a changed existing column *is* syntactically detectable, and its new expression can be re-proven derivable — no declared intent needed |
| Schema evolution (`schema_evolution.md`, `smelt-state/schema_tracking.rs`) | Physical change classification (safe widenings, blocked drops) and backend `ALTER` capability; `default:`/`backfill:` are **user-declared** backfill expressions | Backbuild **derives** the backfill expression from the definition diff instead of asking the user to declare one; schema evolution keeps owning DDL capability/execution mechanics |
| Output fingerprint (`smelt-fingerprint`, `output_fingerprint.md`) | Model-level canonical-form equivalence: sound "nothing changed" judgement | The no-op short-circuit (case A0), and — at wiring time — a per-expression refinement that downgrades detected changes to no-ops |
| Virtual environments (`virtual_environments.md`) | Fingerprint-equal ⇒ reuse; changed ⇒ rebuild (breaking) or reuse-downstream (non-breaking) | A third disposition for the *changed* class: clone-and-patch instead of rebuild. A consumer of backbuild, not a dependency |
| Manipulation layer (`docs/research/20260726-beyond-ivm-differentiation.md` §11.2) | Column-scoped backfill as a verb over plan cells | Backbuild is that verb's *derivation*: the definition diff decides which cells (columns × regions × rows) need touching |

## 2. The contract

Let `I` be the current state of the model's inputs, `before` and `after` the two definitions,
and `T_old` the deployed table.

**Precondition.** `T_old == eval(before, I)` — the table is up to date with its inputs under
the old definition. Every equivalence claim below is relative to this.

**Guarantee.** After executing the emitted script against `T_old` and `I`:
`T == eval(after, I)` — equal as a multiset of rows with columns matched by name and type.
This is the same equality the maintenance conformance gate asserts
(`incremental_models.md` §"The equivalence invariant"), applied to a definition change rather
than a data change.

**Why the precondition is load-bearing.** Scripts factor into two families:

- **Self-read scripts** (rename, derive-from-stored, predicate-tighten `DELETE`): they read
  only `T_old`. If the precondition fails, the result is still *internally coherent* — it is
  `eval(after, I')` for whatever stale `I'` the table reflects — but not equal to
  `eval(after, I)`.
- **Upstream-read scripts** (column pull-through, join enrichment, difference `INSERT`): they
  bake in *current* upstream state. If the precondition fails, the touched columns/rows
  reflect fresh upstream while untouched siblings reflect stale upstream — exactly the
  **enrichment-decoupling** posture of
  `docs/research/20260726-beyond-ivm-differentiation.md` §5.8. For a maintained model, the
  reconciliation ledger can grade this honestly (the new column group's processed-input
  vector differs from its siblings'); for a plain table it is simply a documented
  precondition. The oracle harness includes a stale-input case that *demonstrates* the
  divergence, so the contract's edge is tested, not just stated.

**Options, not choices.** For each atomic change the classifier returns *every* admissible
technique, each independently proven — an option set, not a verdict (2026-08-02 decision).
Cases are not mutually exclusive: a changed expression derivable both from stored columns and
from upstream yields both the in-place `UPDATE` and the column-scoped merge; a bare B4
pull-through admits both emitter shapes. Choosing among options is a cost model's job,
deliberately deferred — until it exists, callers select, and tests verify every option
independently. The model-level baseline option **FullRefresh**
(`CREATE OR REPLACE TABLE t AS <after>`) is always present, so the eventual cost model
compares targeted scripts against the rebuild uniformly.

**Refusal posture.** Admission stays fail-closed per technique: everything the classifier
cannot prove for an atom is a named inadmissibility record (`fail-loud discipline`,
`architecture.md`). A *composed targeted script* exists only when every atom has at least one
admissible option — partial application is never offered; an atom with an empty option set
leaves `FullRefresh` as the model's only option, with the refusals naming why.

**Idempotence.** `UPDATE`-family steps are idempotent for deterministic expressions. Plain
`INSERT`-family steps (E2/E4/F1) are not; each carries an anti-join guard on row identity
where identity is available, making re-runs safe, and is otherwise documented one-shot. This
mirrors the run-recovery posture of `incremental_models.md` (idempotent recovery is a
property worth paying a guard for).

**Determinism caveat.** A non-deterministic expression in the before-definition means
`T_old` is not uniquely `eval(before, I)`; self-read scripts remain coherent relative to the
stored draw, upstream-read recomputation may diverge from stored siblings. Same posture as
`output_fingerprint.md`'s determinism signal: don't silently pretend; surface it.

## 3. Inputs and outputs

**Input is the real CST.** Both definitions are parsed by `smelt-parser` (one-shot, no
Salsa); all diffing and classification operates on typed CST nodes (`smelt_parser::Expr`
etc.), exactly as `analysis/model_diff.rs` already does (its fail-closed checks are
`SyntaxKind` checks). Physical facts arrive as plain data alongside the SQL, mirroring the
`ModelInputs` pattern:

```text
BackbuildInputs {
  table: physical name of the deployed table
  row_identity: Option<Vec<String>>          — declared, or derived from GROUP BY
  added_column_types: name → SQL type string — supplied by caller (type inference lives in
                                               smelt-db, above this crate; tests hand-write)
  sources: name → { physical_name, unique_key: Option<Vec<String>> }
}
```

**Equivalence judgement is stratified.** `smelt-fingerprint` depends on `smelt-db`, which
sits *above* `smelt-logical`, so the canonical-form equivalence cannot be called from the
classifier. The pure module therefore ships a **syntactic comparator**: token-stream equality
modulo trivia (whitespace/comments). It is sound and conservative — a reformatted expression
compares equal; a semantically-equal rewrite compares different and is treated as changed
(costing an unnecessary column rebuild, never a wrong result). At wiring time the
fingerprint's canonical form runs *first*, as a refinement that downgrades detected changes
to no-ops. Rejected alternative: moving the canonicaliser into `smelt-logical` — it is
type-aware by design and belongs where typed CSTs exist.

**Output is strings, per the emitter contract.** Backbuild emitters live beside the
maintenance emitters in `smelt-logical` and return ordered statement strings
("pure string construction over a caller-supplied body", `maintenance/emit.rs`). The parser's
grammar covers model SELECTs, not `MERGE`/`ALTER`/`UPDATE` DML; building output CSTs would
grow the grammar for no consumer. String-level emission is also what keeps statement text
byte-for-byte observable (the statement-parity posture). Two consequences:

- **Alias requalification is a CST rewrite, not string surgery.** An expression fragment
  lifted from the after-definition (`orders.total`) must be requalified for its statement
  context (`u.total` inside `UPDATE t ... FROM orders u`; bare stored-column references for
  self-read `UPDATE`s). This is a small CST-fragment rewriter (resolve each column reference
  against the diff's alias map, re-print), tested on its own.
- **DDL strings emitted here are test-grade DuckDB dialect.** Backend DDL generation
  (`smelt-backend-duckdb/ddl_duckdb.rs`, Spark capability matrix) already exists under
  `schema_evolution.md`; unifying backbuild's `ALTER` emission with it is wiring-time work.

## 4. The catalogue

Case IDs (A0, B1, …) are the canonical names used by tests and the plan. Per case: what in
the `DefinitionDiff` detects it, what must be proven, and the script shape. A case describes
one admissible *technique*; when an atom satisfies several cases' proofs, all the resulting
techniques are returned as options (§2 "Options, not choices").

### A. No-op class

- **A0 — refactor / formatting / CTE reshuffle.** *Detect*: whole-definition syntactic
  equality modulo trivia (pure module) or fingerprint equality (wiring refinement). *Script*:
  empty. Because fingerprints are computed over expanded SQL, a ref repointed to a
  fingerprint-identical upstream is also A0 — at the wiring layer only.

### B. Additive column class

Shared precondition, the **row-set-unchanged proof**: everything *except* the SELECT list —
FROM/JOIN tree, WHERE, GROUP BY, DISTINCT/dedup, set-operations — is unchanged (modulo
trivia), except where an atom explicitly licenses a change (B4's added join). This is the
general-model analogue of the skeleton: same doctrine as `SkeletonAdd` (a change to which
rows exist is a grain change), derived from the diff rather than the maintenance skeleton.

- **B1 — new column = pure function of existing stored columns** (constants included).
  *Detect*: added SELECT item whose dependency set ⊆ existing output columns
  (`collect_dependencies` walk, fail-closed on subqueries/windows/opaque calls — the
  machinery `model_diff.rs` already has). *Script*:
  `ALTER TABLE t ADD COLUMN c <ty>; UPDATE t SET c = <requalified expr>;`
  (`emit_in_place_update` shape). No upstream read.
- **B2 — rename** (user's degenerate case). *Detect*: a dropped column `d` and an added
  column `a` whose expression is identical (modulo trivia) to `d`'s old expression — matched
  as a pair *before* B1/C1 classification so drop+add is not misread. *Script*:
  `ALTER TABLE t RENAME COLUMN d TO a;` — zero rows touched. Ambiguity (two dropped columns
  with identical expressions) refuses rather than guessing. On backends without column
  rename the same atom admits an `ADD` + copy-`UPDATE` + `DROP` option — enumerated when
  dialect variants arrive with wiring.
- **B3 — 1:1 pull-through from an upstream at the model's own grain** (user's scenario 1).
  *Detect*: added SELECT item whose dependencies resolve to columns of an upstream already
  in the FROM tree. *Prove*: the output contains a 1:1 pull-through of that upstream's
  unique key (lineage through the SELECT list), and the upstream's `unique_key` is declared
  in `BackbuildInputs` — together, each target row addresses at most one upstream row.
  *Script*: `ALTER ADD`, then column-scoped
  `UPDATE t SET c = <requalified expr over u> FROM <upstream> u WHERE t.<k> = u.<k>` (the
  `emit_column_scoped_merge` family). Rows filtered out of `t` by the model's WHERE are
  simply never matched — the join touches only existing rows.
- **B4 — new column via a newly-added LEFT JOIN** (user's scenario 2; fan-out — one
  dimension row enriches many target rows). *Detect*: FROM-tree diff = one added
  LEFT JOIN (two or more: B7); added SELECT items reference the new alias; nothing else
  references it (WHERE/GROUP BY/other SELECT items unchanged). *Prove* the join cannot change the row
  set: LEFT JOIN (never removes rows) + at-most-one match (join key unique on the dimension
  side — declared `unique_key` or the FD machinery `analysis/functional_dependency.rs`).
  *Script*: two shapes, chosen by expression:
  - bare pull-through (`d.x`): `UPDATE t SET c = d.x FROM dim d WHERE t.jk = d.jk` —
    unmatched rows stay at the post-`ALTER` NULL, which is exactly LEFT-JOIN semantics;
  - general expression (`COALESCE(d.x, 'none')` — NULL-extension must be *evaluated*, not
    skipped): `UPDATE t SET c = (SELECT <expr> FROM dim d WHERE d.jk = t.jk)` — the scalar
    subquery NULL-extends on no match and *errors on multiple matches*, a free runtime
    uniqueness probe. The oracle decides both shapes' correctness; cost is wiring-time
    tuning.
- **B5 — new aggregate column at unchanged GROUP BY grain.** *Prove*: skeleton unchanged;
  aggregate inputs available upstream. *Script*: column-scoped `MERGE` from a
  re-aggregation `SELECT <keys>, <agg> FROM <upstream> GROUP BY <keys>` keyed on the
  GROUP BY keys (which are the row identity by construction). Full upstream scan, but only
  one column written. Tier 3.
- **B6 — new window-function column over stored columns** (`ROW_NUMBER() OVER
  (PARTITION BY stored ORDER BY stored)`). *Prove*: window reads only stored columns.
  *Script*: self-read
  `UPDATE t SET c = s.c FROM (SELECT <id>, <window> AS c FROM t) s WHERE t.<id> = s.<id>` —
  needs row identity, no upstream. Tier 3.
- **B7 — sequential multi-join enrichment** (two or more added LEFT JOINs, backfilled one
  step at a time — e.g. fact → dim1, then dim2 keyed on a column dim1 provides). *Detect*:
  FROM-tree diff = k added LEFT JOINs, ordered by reference dependency (a later join's ON
  condition references columns an earlier join provides). *Prove*: per join, the full B4
  row-set-preservation proof (LEFT + unique key on the joined side + no stray references),
  with one extension — a later join's key may reference a column an *earlier step
  backfills*, provided that column is part of the added output and therefore stored by the
  time the step runs. *Script*: the B4 backfill per join, in dependency order — backfill
  join 1's columns first, then join 2's backfill keys on the now-stored column. A later
  join keying on an earlier join's column that is **not** stored in the output refuses
  (the step would need a multi-hop traversal; §7).

### C. Removals and type changes

- **C1 — dropped column** → `ALTER TABLE t DROP COLUMN d;`. Classification and the
  opt-in flag doctrine (`--allow-column-removal`) stay owned by `schema_evolution.md`;
  backbuild's job is only to *sequence* the drop (last, after rename extraction).
- **C2 — type change**: safe widening → `ALTER COLUMN TYPE` (schema evolution owns the
  widening table); a representation change (`c` now cast differently) is not a C-case at
  all — it is a changed expression, D1/D2.

### D. Changed-expression class

*Detect*: same column name, before/after expressions differ modulo trivia. This is the
deliberate L3-boundary crossing: no declared migration intent, the diff itself is the intent.
The formatting-only false positive is absorbed by the trivia-insensitive comparator now and
the fingerprint refinement at wiring.

- **D1 — new expression derivable from stored columns.** The most valuable case in
  practice: "fix a bug in one column of a 10 TB table". *Prove*: the new expression's
  dependencies are available in the stored row — either the referenced input is stored 1:1
  under some output column (lineage through the *unchanged* SELECT items), or the
  expression references only output columns. *Script*: column-scoped
  `UPDATE t SET c = <requalified expr>;` — siblings untouched. Note the subtlety: the new
  expression is defined over *inputs*, so derivability means finding stored 1:1
  representatives of those inputs, not blindly substituting the old `c`.
- **D2 — new expression needs an upstream read** (changed enrichment logic). Same proof
  and script as B3 with the trigger being an expression change rather than a column add.

### E. Row-set (predicate) class

SELECT list unchanged; the WHERE diff is a **conjunct-set diff** (split at top-level `AND`s
only — a syntactic conjunct algebra, deliberately not a general implication prover; anything
not expressible as added/removed conjuncts refuses).

Three-valued logic is load-bearing everywhere here: `WHERE p` keeps rows where `p` is TRUE,
so set complements must be written `IS NOT TRUE`, never bare `NOT`:

- **E1 — filter tightened** (conjunct `q` added, `q` evaluable over stored columns).
  *Script*: `DELETE FROM t WHERE <q'> IS NOT TRUE;` where `q'` is `q` requalified to stored
  columns. Bare `NOT q'` would wrongly *keep* `q'=NULL` rows a rebuild would drop. No
  upstream read at all — the cheapest script in the catalogue after A0/B2.
- **E2 — filter loosened** (conjunct `q` removed). *Script*: insert exactly the difference
  slice — the after-definition SELECT with `AND (<q> IS NOT TRUE)` appended (rows the old
  predicate excluded), with an identity anti-join guard where identity exists.
- **E3 — arbitrary predicate change** = added ∧ removed conjuncts: compose E1 + E2. A
  predicate rewritten in a way the conjunct algebra cannot factor refuses.
- **E4 — time-horizon extension** (`ts >= X` → `ts >= Y`, `Y < X`) — the classical
  "backfill more history", called out separately because it is so common and because the
  difference slice `ts >= Y AND ts < X` is a *region-scoped* INSERT in the maintenance
  sense (a clean partition interval, the shape `emit_delete_insert`'s region machinery
  already speaks). Mechanically an E2 whose removed/added conjuncts are range predicates
  on one column.

### F. Structural class

- **F1 — new UNION ALL branch.** *Detect*: branch multiset diff by per-branch syntactic
  equality; exactly one added branch, others unchanged. *Script*: `INSERT INTO t SELECT …
  <branch>;` — UNION ALL is additive, the branch is exactly the delta. (Plain `UNION`
  dedups across branches: refuse.)
- **F2 — removed UNION ALL branch.** Needs a provenance predicate distinguishing the
  branch's rows in the stored table (a discriminator constant/column —
  `analysis/discriminants.rs`); with one, `DELETE WHERE <discriminator>`; without, refuse.
  Tier 3.
- **F3 — ref repointed to a different upstream.** Not decidable from the two definitions
  alone; at the wiring layer, expansion + fingerprint makes the equivalent-repoint case A0.
  Otherwise refuse.

### G. Honest refusals

- **G1 — grain change**: GROUP BY keys added/removed, DISTINCT toggled, dedup ordering
  changed → refuse ("effectively a new model" — the `SkeletonAdd` doctrine verbatim).
- **G2 — join-multiplicity change** (INNER→LEFT, LEFT→INNER, join condition edited) →
  refuse. Future rung: **probe-gated** admission — e.g. INNER→LEFT is a no-op iff no row
  actually lacked a match, checkable at runtime by a count probe
  (`emit_count_preservation_probe` exists); data-dependent verdicts are a different
  contract, deferred.
- Opaque expressions, subqueries or windows in *added* columns (except B6), CTE-section
  changes (below), LIMIT/ORDER BY changes: refuse with named reasons.

**CTE posture (initial).** A definition whose `WITH` prefix is unchanged modulo trivia
diffs its final SELECT normally; a changed CTE refuses (conservative — the walk machinery
can chase lineage through CTEs, and relaxing this is future work, not day-one scope).

### H. Composites

A multi-edit diff factors into atoms; the script is the atoms' statements in a fixed
dependency order:

```text
renames (B2) → ALTER ADD / ALTER TYPE (B*, C2) → DELETEs (E1) →
column UPDATEs/MERGEs (B1/B3/B4/B7/D*) → INSERTs (E2/E4/F1) → ALTER DROPs (C1)
```

Rationale: renames first so requalified expressions reference final names; deletes before
updates so updates touch fewer rows; inserts after updates because inserted rows come from
the after-definition SELECT and are already correct (a deterministic re-update would be
harmless but wasted); drops last so nothing mid-script loses a column it reads. Within the
update slot, B7 steps run in their derived dependency order — the one place ordering is
data-dependent rather than fixed by variant. An atom with no admissible option blocks any
composed targeted script — `FullRefresh` stays the model's only option (§2). The oracle
harness includes composite cases precisely because ordering bugs are silent in single-atom
tests.

## 5. Priority ranking

Ranked by practical value ÷ new machinery, weighing how often the edit occurs in real
pipelines and how galling the full refresh it replaces is:

| # | Case | Rationale |
|---|------|-----------|
| 1 | Diff foundation + A0 | Substrate everything keys off: SELECT-list diff with trivia-insensitive expression comparison, conjunct-set diff, skeleton comparison, branch diff |
| 2 | B1 + B2 | Cheapest scripts (no upstream read; rename touches zero rows), emitter exists, user scenario 3 |
| 3 | D1 | Same script as B1; "fix one column of a huge table" is the highest-value real-world case |
| 4 | B3 | User scenario 1; emitter exists; needs the grain-link proof |
| 5 | B4 | User scenario 2; the one substantial new proof (row-set preservation for an added join) |
| 6 | E1 | Trivially cheap script; self-contained; three-valued-logic care |
| 7 | E4 | "Extend the history window" is among the most common real model edits |
| 8 | E2 + D2 | Rounds out predicates and expression changes; reuses earlier machinery |
| 9 | F1 | Cheap detection (branch diff), cheap script |
| 10 | B7 | Sequential multi-join enrichment; builds directly on B4's proof, adds only the dependency ordering |
| 11+ | B5, B6, F2, C-sequencing polish, probe-gated G2 | Tier 3 — real but rarer, or needing runtime probes |

## 6. Architecture

```text
crates/smelt-logical/src/backbuild/
  mod.rs        — BackbuildOptions { atoms: Vec<AtomAnalysis> } where AtomAnalysis =
                  { change: AtomicChange, options: Vec<BackbuildOption>,
                    inadmissible: Vec<BackbuildRefusal> } — every admissible technique per
                  atom (§2 "Options, not choices"), plus the always-present model-level
                  FullRefresh baseline; assemble(options, selection) applies the H ordering
                  to one chosen option per atom (pure data throughout, mirrors
                  maintenance::MaintenancePlan's role)
  diff.rs       — DefinitionDiff: CST-level factoring of (before, after)
                    select_list: added / dropped / changed / unchanged (per column, Expr pairs)
                    where_clause: conjunct-set diff (top-level ANDs)
                    skeleton: FROM/JOIN tree, GROUP BY, dedup — unchanged | added_left_joins | changed
                    set_ops: UNION ALL branch multiset diff
  classify.rs   — per-atom proofs → option sets; consumes analysis::{model_diff dependency
                  walk, functional_dependency, discriminants}; fail-closed
  requalify.rs  — CST-fragment column-reference rewriter (expression → statement context)
  emit.rs       — backbuild statement emitters (ALTER ADD/RENAME/DROP, UPDATE FROM,
                  scalar-subquery UPDATE, DELETE … IS NOT TRUE, guarded INSERT);
                  reuses maintenance::emit shapes where they match
```

Invariant compliance, noted now so wiring inherits it:

- **Maintenance-plan purity / statement single-ownership** (`architecture.md` item 12):
  the plan is pure data derived by pure functions; every statement is emitter-authored in
  `smelt-logical`. Backends (and the test harness) execute, never author. Building in the
  right crate from day one means wiring is consumption, not rework.
- **Property-composition-walk rule**: backbuild's judgements are *diff-level*, a new
  category — but wherever a proof needs a model-property verdict (FD/uniqueness, lineage,
  discriminants) it consumes the existing analysis outputs; per-expression dependency
  collection stays a leaf classifier over one bounded node. No ad hoc scans over raw SQL
  text.
- **Fail-loud**: every refusal carries the atom and a named reason; no silent fallback.

**Conformance harness** (`crates/smelt-logical/tests/backbuild_conformance.rs` — the crate
already has a `duckdb` dev-dependency): stage inputs → `CREATE TABLE t AS <before>` →
apply script → `CREATE TABLE expected AS <after>` → assert multiset equality
(`EXCEPT ALL` in both directions — plain `EXCEPT` is set-based and would miss duplicate-row
count drift — plus column name/type comparison). Refusal goldens assert the
named reason. One stale-input case documents the §2 precondition. Every enumerated option is verified
independently — each option's script applies to a fresh copy of the staged before-table, so
a case admitting two techniques proves both. Explicit BB-case tests now; generative recipe
sampling (testkit-style, à la `maintenance_conformance`) deferred.

## 7. Open questions

1. **INSERT idempotence.** The anti-join guard needs row identity; for identity-less models
   (no GROUP BY, no declared key) E2/E4/F1 scripts are one-shot. Acceptable, or should
   identity-less models refuse INSERT-family atoms outright?
2. **Rename-match ambiguity.** Two dropped columns with syntactically identical expressions:
   refuse (current position) or match by position? Refusal is sound; revisit if it bites.
3. **Cost model.** Deliberately deferred (2026-08-02): the module enumerates every
   admissible option per atom and never chooses. The open question narrows to the eventual
   chooser's inputs — table/upstream sizes, backend capabilities (a rename is free on
   DuckDB but a copy on Spark+Parquet), staleness tolerance — and where it runs (wiring
   layer, comparing targeted scripts against the always-present FullRefresh baseline).
4. **Where "before" comes from when wired.** `.smelt/schemas/` stores schema + hash, not
   SQL; the expanded-logical-SQL snapshot (`run_state.md` §"Snapshot and environment
   store") is the principled source and also what fingerprint comparison wants. Wiring
   likely rides on the virtual-environments state store.
5. **Maintained-model ledger integration.** The definition-change trigger instantiates new
   groups at `S = ∅` and catches up; a backbuild script instead advances the group to
   *current* in one shot. Reconciling the two (backbuild as "instantiate + immediate
   full-input catch-up", which is what §"The definition-change trigger" already describes)
   is wiring-time design.
6. **Spark dialect.** Emitters are DuckDB-dialect first. `UPDATE … FROM` and scalar-subquery
   UPDATE have Spark/Delta variants (`MERGE`-based); the `MaintenanceDialect` enum is the
   template. Deferred with the rest of wiring.
7. **Multi-hop enrichment beyond B7.** B7 covers added-join chains whose intermediates are
   stored output columns. A later join keying on an earlier join's *unstored* column would
   need a multi-hop backfill statement (one UPDATE traversing both joins, or a temp
   intermediate) — refused for now with a named reason. Likewise an added join plus a
   changed expression referencing it. Stretch driven by real examples, not speculation.

## References

- `docs/specs/incremental_models.md` §"The definition-change trigger", §"The equivalence
  invariant", §"Statement emission (single owner)"
- `docs/specs/model_properties.md` §"Derived proofs" → additive-only model-diff
- `docs/specs/schema_evolution.md` (change classification, `default:`/`backfill:`, backend
  capability matrix)
- `docs/specs/output_fingerprint.md`, `docs/specs/virtual_environments.md`,
  `docs/specs/run_state.md` §"Snapshot and environment store"
- `docs/research/20260726-beyond-ivm-differentiation.md` §5.8 (enrichment decoupling),
  §11.2 (manipulation layer)
- Code: `crates/smelt-logical/src/analysis/model_diff.rs`,
  `crates/smelt-logical/src/maintenance/{derive,emit}.rs`,
  `crates/smelt-fingerprint/src/lib.rs`
- Plan: `docs/plans/20260802-backbuild-synthesis.md`
