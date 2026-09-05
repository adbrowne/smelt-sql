# Phase 3 summary — `diff_profiles`

Commits: `99015a32` (spec-first: seven deltas to `property_diff.md`, ruling R1/R6),
`e86b43a1` (code: `smelt_logical::analysis::diff` + prerequisite type changes).

## What shipped

`Dimension` (23 variants): `Grain, RowIdentity, SourceBound, CellTechnique, CellCorner,
CellRowIdentity, CellAdded, CellRemoved, RefusalAdded, RefusalRemoved, ContractPoint, ProbeAdded,
ProbeRemoved, ColumnAdded, ColumnRemoved, Determinism, Discriminant, Comparability, FdAdded,
FdRemoved, LiteralColumn, SetOpBarrier, FanOutJoin`. `Direction {Downgrade, Upgrade, Neutral}`.
`ChangeKind` carries the typed old/new per dimension; `Change`/`Cause`/`ModelDiff`/`PropertyDiff`/
`DiffGraph`; pure `diff_profiles(old, new, graph)`.

**Exhaustiveness mechanism**: `ChangeKind::direction`/`dimension`/`subject` are each one `match
self { ... }` with no `_` arm — a new `ChangeKind` variant fails `non-exhaustive patterns` at
compile time. Field coverage: `diff_property_set` opens `let PropertySet { columns, grain,
functional_dependencies, determinism, comparability, discriminants, literal_columns,
has_set_op_barrier, has_fan_out_join, row_identity, source_bounds } = old;` (and again for `new`)
with **no `..`** — a field added to `PropertySet` later is a compile error until diffed. Same
pattern in `diff_cell_verdicts` over `CellVerdict`, and in `diff_profile` over `PropertyProfile`.

Prerequisites: `CellVerdict.technique` is now `Technique` (compared as enum, not re-parsed from
its rendered string); `PropertySet`/`PropertyProfile` derive `PartialEq`; `ContractPointView`
gained `frozen_horizon_seconds`/`deferral_seconds` so widening is machine-comparable.
`property_profile_parity` stayed green untouched (ruling R3 confirmed).

`crates/smelt-logical/tests/diff_purity.rs` **is a real, executing cargo integration target** —
no `autotests = false` or `[[test]]` override in `smelt-logical/Cargo.toml`, so it's
auto-discovered, and it appeared and passed in an actual `cargo test -p smelt-logical --test
diff_purity --test walk_coverage` run: `Running tests/diff_purity.rs ... test
diff_module_performs_no_io_reads_no_ledger_snapshot_or_backend ... ok`, 1 passed.

## Deviation from plan

`cell_removed` when the *whole model* stops being maintained (`still_maintained == false`) is
graded `Neutral`, not `Downgrade` — the spec's row only names removal from a still-maintained
model; grading the whole-model case here would double-count a loss already visible via every
other cell's removal and the refusal set.

## What Phase 4 must hand back

`DiffGraph { upstream: BTreeMap<String, Vec<String>>, edited: BTreeSet<String>,
project_config_changed: bool }`. `upstream` = direct (not transitive) edges, model **and**
source names (build via `DiffGraph::from_dependency_graph(&DependencyGraph, edited,
project_config_changed)`, which adds back `smelt.sources.*` edges from `ModelFile.refs` that
`DependencyGraph::build` drops). `edited` = the working-tree-vs-baseline edited-file set per
spec §"Attribution" (model SQL/override/source-declaration diffs), keyed by the **same names**
as `upstream`'s nodes. `project_config_changed` = whether a project-level `smelt.yml` key
differs between the two versions; only observed when a shifted model has **no** edited ancestor.
A `None`-coded refusal (the three uncoded `Refusal` variants) matches only another
`None`-coded refusal with identical `text` — never a `Some(_)`-coded one, even with the same text.
