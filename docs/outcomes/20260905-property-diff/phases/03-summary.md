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

## Fix round 1 (review G1–G7)

Commits: `28f0a63f` (clippy from the initial review), plus this round's commit
(`phase(property-diff/3): fix review findings — maintenance_lost, probe/column-presence emission,
grain key loss`).

- **G1 (Critical)**: `refresh: incremental` → `refresh: full` with byte-identical SQL made
  `derive_model_maintenance_plan` return `None` before any refusal is built, so old cells were
  non-empty and new cells/refusals were both empty — N `cell_removed` changes, all graded
  `Neutral` per the existing "still-maintained" rule, zero downgrades. Fixed by a new
  `maintenance_lost`/`maintenance_gained` dimension, emitted once per model in `diff_profile`
  (never derived from individual cell changes), graded `Downgrade`/`Upgrade`. Spec row added to
  §Direction and §"The diff" before the code (spec-first).
- **G2 (Important)**: `diff_probes` matched on `(fact, cell)` but never compared the third field
  (`probe`, the named diagnostic) — a matched probe whose diagnostic changed emitted nothing.
  Fixed: `ProfileProbe` is now destructured with no `..` at the matched-key site; a differing
  `probe` field emits `ProbeRemoved` + `ProbeAdded` (the same removal-plus-addition convention the
  spec already uses for renames elsewhere).
- **G3 (Important)**: the determinism/comparability/discriminant per-column loops iterated
  `old_*` only and required the column on both sides — a column present in one map and absent
  from the other (with `columns` unchanged) produced zero changes. Fixed by iterating the union of
  keys (matching `literal_columns`'s existing pattern) and widening `ChangeKind::Determinism`/
  `Comparability`/`Discriminant`'s `old`/`new` fields to `Option<T>`; a `None` on either side
  grades `Neutral` (no lattice position to rank a missing fact against).
- **G4 (Important, done first)**: extended `every_profile_difference_produces_at_least_one_change`
  with three entry-removal mutations (determinism/comparability/discriminants set to empty while
  `columns` stays put), and added a new `every_profile_field_difference_produces_at_least_one_change`
  covering `ProfileProbe`/`ProfileRefusal` field mutations and the maintenance-lost/gained cases.
  Run against the pre-fix code, both went red exactly as predicted: the probe-diagnostic-change
  case (G2) and the determinism-entry-removal case (G3, from the first extension) both failed
  before any fix landed. Confirms the extension was strong enough.
- **G5 (Important)**: `Grain`'s direction rule compared `KeySet` membership, so
  `Key(["id","region"]) -> Key(["id"])` (a composite key column dropped — "lost a key column" per
  spec) saw the old key as both "lost" (unequal as a whole vector) and "gained" (the new, shorter
  key wasn't in the old set) and graded `Neutral`. Fixed by comparing the UNION of columns each
  side's keys cover instead of key-set membership; a dropped column is a `Downgrade` even when
  something else in the composite also changed (fail-loud tie-break).
- **G6 (Minor)**: `whole_model_changes` (added/removed models) now grades every change `Neutral`
  regardless of its ordinary per-dimension rule — the `cause` already says `added`/`removed`, and
  a per-dimension direction there was noise inflating `--fail-on` for a fact the `cause` field
  already carries. Spec updated: "The diff" now says so explicitly.
- **G7 (Minor)**: the FD diff used `Vec::contains` (membership, not multiplicity) — two copies of
  one FD on the old side and one on the new side registered as "still present". Fixed with a small
  `multiset_excess` helper (linear scan; `DerivedFd` has no `Ord`/`Hash` so a `BTreeMap` count
  wasn't free) reused for both directions.

Gates after the fixes: `cargo test -p smelt-logical --lib analysis::diff` — 49 passed, 0 failed.
`cargo test -p smelt-logical --test diff_purity --test walk_coverage` — 1 + 4 passed.
`cargo test -p smelt-cli --test property_profile_parity` — 3 passed (still green, untouched).
`cargo clippy -p smelt-logical --all-targets -- -D warnings` — clean. `cargo fmt --all -- --check`
— clean.
