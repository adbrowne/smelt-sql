# Phase 3 plan — `diff_profiles` in `smelt-logical`

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md` (success criterion 2)
**Spec:** `docs/specs/property_diff.md` §"The diff", §"Direction", §"Attribution", §Constraints 2, 3, 6
**Builds on:** phase 2 (`crates/smelt-logical/src/analysis/profile.rs`)

## Objective

A pure `diff_profiles(old, new, graph) -> PropertyDiff` in `smelt-logical`: one exhaustive
direction table, the spec's matching rules, added/removed/unshifted, and attribution to nearest
edited ancestors. No I/O, no ledger, no backend, no git — the edited set is an **input** (phase 4
computes it). Renderers (phases 5–7) consume `PropertyDiff`; none is written here.

## Ground truth found (file:line)

- Technique ladder: `crates/smelt-logical/src/maintenance/mod.rs:211` — `Technique` has exactly the
  five variants the spec ladder names (`DeleteInsert`, `KeyedFold`, `ColumnScopedMerge`,
  `InPlaceUpdate`, `PerGroupRecompute`), is `Copy` and **already derives `Serialize`** (:210). All
  unit variants, so its serde name equals its `Debug` string — see §"Ladder" below.
- `Corner` `:192` — four unit variants, no `Serialize` (Debug-rendered by phase 2). Direction is
  always `neutral`, so no enum is needed.
- Graph: `smelt_core::graph::DependencyGraph` (`crates/smelt-core/src/graph.rs:19`), reachable
  (`smelt-logical` depends on `smelt-core`, `crates/smelt-logical/Cargo.toml:12`). `get_upstream`
  (`:532`) returns **model** deps only — `build` (`:43`) deliberately drops `smelt.sources.*` refs
  from the edge map (`:75`). Source edges live on `ModelFile.refs`
  (`crates/smelt-core/src/discovery.rs:43`); `PropertySet.source_bounds` also keys by source name.
- `BoundResult` `crates/smelt-logical/src/analysis/source_bounds.rs:242` — `Bounded{col, before,
  after} | Unbounded | NotDerivable`, `PartialEq + Eq`; `Seconds(:24)` is `Ord`.
- `ContractPointView` `crates/smelt-logical/src/contract/mod.rs:169` — `frozen_horizon:
  Option<String>`, `deferral: Option<String>`, `deferral_origin`. **Display strings only.**
  Two in-flight phase-2 review fixes land before this phase and this plan assumes them:
  `ProfileRefusal.code` becomes `Option<String>` (`maintenance::refusal_code -> Option<&'static
  str>`; `ReachNotDerivable`, `RepairKeysNotDiscoverable`, `RepairSliceUnbounded` have no
  `DiagnosticCode`), and `ContractPointView` gains `retain_departed`
  (`EffectiveContract.retain_departed: Option<RetainDeparted>`, `contract/mod.rs:73`;
  `RetainDeparted::Bool(bool) | Tombstone { .. }` — a *presence* relaxation, not an interval).
- `Determinism` lattice `Clean < Run < Row` (`analysis/walk.rs:1830`, `Ord`); `Comparability`
  `Comparable < Incomparable` (`:1855`, `Ord`).
- `serde_json` is a workspace dep (`Cargo.toml:64`) but **not** yet a `smelt-logical` dep.

## Spec delta (do this first, in this phase, before any code)

Seven edits to `docs/specs/property_diff.md`. All are additive (§Constraints 9 holds).

1. **Dimension coverage hole (the important one).** §"The diff" makes a model *shifted* when
   `P_old[m] != P_new[m]`, but the 16-dimension list cannot express six profile fields:
   `PropertySet.functional_dependencies`, `.comparability`, `.literal_columns`,
   `.has_set_op_barrier`, `.has_fan_out_join`, and `CellVerdict.row_identity` (§"The diff" lists
   only technique/corner/contract point as matched-cell fields). Today a model can differ on those
   and be reported **shifted with an empty `changes` array** — a fail-loud violation
   (§Constraints 6). Add six dimensions with their direction rows:
   `cell_row_identity` (downgrade `Key → WholeRow`, upgrade the reverse),
   `comparability` (downgrade `Comparable → Incomparable`),
   `fd_added` / `fd_removed` (neutral both; matched on `(key, determines)`),
   `literal_column` (neutral; matched on column name, `old`/`new` the literal text),
   `set_op_barrier` and `fan_out_join` (downgrade `false → true`, upgrade the reverse — both are
   FD/keying barriers).
2. **`source_bound` totality.** The table names only `Bounded ↔ Unbounded` and interval width;
   `NotDerivable` is unclassified. Specify the total order `Bounded ≻ {Unbounded, NotDerivable}`,
   with `Unbounded ↔ NotDerivable` **neutral** (both force a full read; neither is worse).
   Interval width is `before + after`.
3. **`contract_point` widening is not machine-comparable today.** `ContractPointView` keeps only
   the display string, so "its interval widened" cannot be decided without re-parsing rendered
   text (the re-parse bug class, `CLAUDE.md`). Add `frozen_horizon_seconds: Option<u64>` and
   `deferral_seconds: Option<u64>` to `ContractPointView`, sourced from `DataLatency.seconds`
   (`crates/smelt-core/src/config.rs:583`) in the existing `From<EffectiveContract>` impl. Record
   this in §"The property profile" item 2. Purely additive JSON; the `property_profile_parity`
   gate re-run proves the shared shape. `retain_departed` (landing from the phase-2 review fix) is
   a **presence** relaxation with no interval: specify in the `contract_point` direction row that
   absent → present is a downgrade, present → absent an upgrade, and a change of *shape* only
   (`Bool` → `Tombstone`, or a different tombstone column) is neutral — there is no width to
   widen. Whether `Some(Bool(false))` counts as present follows `EffectiveContract::is_default`
   (`contract/mod.rs:79`) rather than being decided here.
4. **Refusal matching key.** §"The diff" says refusals match on `(code, text)`. Three `Refusal`
   variants have no `DiagnosticCode` at all, so the key is `(Option<code>, text)`: a `None`-coded
   refusal matches another `None`-coded refusal with the same `text`, and never matches a
   `Some(_)`-coded one. Say so in §"The diff", and in §Surface's JSON schema note that a
   `refusal_added`/`refusal_removed` change's `old`/`new` carries `code: null` for those three.
   Without this the three would have collapsed onto one key under the old placeholder code.
5. **`probes` field list.** §"The property profile" item 4 says `(fact, probe, cell, cadence)`;
   `ProfileProbe` carries `fact, probe, cell` (phase 2 dropped the `cost`/cadence rendering as a
   presentation concern). Strike `cadence`.
6. **`determinism` direction row** — restate as "a column moved up the `Clean < Run < Row`
   lattice" (downgrade) / down it (upgrade), matching the real three-point lattice rather than the
   binary "run-deterministic → nondeterministic".
7. **`cell_technique` encoding.** State that `old`/`new` are the technique's serde name
   (`"KeyedFold"`), unchanged from the single-version report.

## The type design

```rust
// crates/smelt-logical/src/analysis/diff.rs

/// The dimension a change is reported under — the JSON `dimension` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension { Grain, RowIdentity, SourceBound, CellTechnique, CellCorner,
    CellRowIdentity, CellAdded, CellRemoved, RefusalAdded, RefusalRemoved, ContractPoint,
    ProbeAdded, ProbeRemoved, ColumnAdded, ColumnRemoved, Determinism, Discriminant,
    Comparability, FdAdded, FdRemoved, LiteralColumn, SetOpBarrier, FanOutJoin }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction { Downgrade, Upgrade, Neutral }

/// The typed payload of one difference. **This is the one table**: `direction`
/// and `dimension` are exhaustive `match`es over it with no wildcard arm.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    Grain { subject: String, old: Grain, new: Grain },
    RowIdentity { old: RowIdentityVerdict, new: RowIdentityVerdict },
    SourceBound { source: String, old: BoundResult, new: BoundResult },
    CellTechnique { cell: String, old: Technique, new: Technique },
    CellCorner { cell: String, old: String, new: String },
    CellRowIdentity { cell: String, old: RowIdentityVerdict, new: RowIdentityVerdict },
    CellAdded { cell: String, new: Box<CellVerdict>, still_maintained: bool },
    CellRemoved { cell: String, old: Box<CellVerdict>, still_maintained: bool },
    RefusalAdded(ProfileRefusal), RefusalRemoved(ProfileRefusal),
    ContractPoint { cell: String, old: ContractPointView, new: ContractPointView },
    ProbeAdded(ProfileProbe), ProbeRemoved(ProfileProbe),
    ColumnAdded(String), ColumnRemoved(String),
    Determinism { column: String, old: Det, new: Det },
    Comparability { column: String, old: Comp, new: Comp },
    Discriminant { column: String, old: Discriminants, new: Discriminants },
    FdAdded(DerivedFd), FdRemoved(DerivedFd),
    LiteralColumn { column: String, old: Option<String>, new: Option<String> },
    SetOpBarrier { old: bool, new: bool }, FanOutJoin { old: bool, new: bool },
}

impl ChangeKind {
    pub fn dimension(&self) -> Dimension { /* exhaustive match, no `_` arm */ }
    /// §"Direction" — the single direction table.
    pub fn direction(&self) -> Direction { /* exhaustive match, no `_` arm */ }
    pub fn subject(&self) -> String { /* exhaustive */ }
    fn old_json(&self) -> Option<serde_json::Value>;   // report encodings, via serde
    fn new_json(&self) -> Option<serde_json::Value>;
}

#[derive(Debug, Clone, Serialize)]
pub struct Change { pub dimension: Dimension, pub subject: String,
    pub direction: Direction, pub old: Option<Value>, pub new: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")] pub reason: Option<String>,
    #[serde(skip)] pub kind: ChangeKind }

#[derive(Debug, Clone, Serialize)] #[serde(rename_all = "snake_case")]
pub enum CauseKind { Edited, Added, Removed, Downstream }
#[derive(Debug, Clone, Serialize)]
pub struct Cause { pub kind: CauseKind, pub of: Vec<String> }

#[derive(Debug, Clone, Serialize)]
pub struct ModelDiff { pub model: String, pub cause: Cause, pub changes: Vec<Change> }
#[derive(Debug, Clone, Serialize)]
pub struct PropertyDiff { pub models: Vec<ModelDiff>,
    pub summary: DiffSummary /* downgrades, upgrades, neutral, shifted_models */ }

/// The working-tree graph plus the edit provenance the diff attributes with.
/// Built by the caller (phase 4/5) — `diff_profiles` never touches git.
/// `upstream` carries **model and source** edges, because
/// `DependencyGraph::get_upstream` filters sources out and §Attribution walks to
/// "every edited model **or source**".
#[derive(Debug, Clone, Default)]
pub struct DiffGraph { pub upstream: BTreeMap<String, Vec<String>>,
    pub edited: BTreeSet<String>, pub project_config_changed: bool }
impl DiffGraph { pub fn from_dependency_graph(g: &DependencyGraph, edited: BTreeSet<String>,
    project_config_changed: bool) -> Self { /* adds source edges from ModelFile.refs */ } }

pub fn diff_profiles(old: &BTreeMap<String, PropertyProfile>,
                     new: &BTreeMap<String, PropertyProfile>,
                     graph: &DiffGraph) -> PropertyDiff;
```

**How "a missing direction rule is a compile error" is enforced.** Two structural mechanisms,
because the requirement has two halves:

- *Direction totality* — `ChangeKind::direction` is one `match self { … }` with no `_` arm and no
  catch-all binding. Adding a `ChangeKind` variant is a `non-exhaustive patterns` compile error.
  This holds for the value-dependent rows too (technique ladder, bound widening, grain, contract
  point): the variant carries the **typed** old/new, so its arm computes the direction from those
  values (`ladder_rank(new).cmp(&ladder_rank(old))`) rather than returning a constant. A
  per-`Dimension` constant table could not express those rows — this is why the table is keyed on
  `ChangeKind`, not on `Dimension`, and `Dimension` is derived from it.
- *Field coverage* — `diff_property_set` opens with an irrefutable destructure
  `let PropertySet { columns, grain, functional_dependencies, determinism, comparability,
  discriminants, literal_columns, has_set_op_barrier, has_fan_out_join, row_identity,
  source_bounds } = new_set;` **with no `..`**, so a field added to `PropertySet` later is a
  compile error rather than a silently undiffed field. Same for `CellVerdict` and
  `PropertyProfile`. This is what closes spec-delta 1's hole permanently.

### Ladder: enum, not string

`CellVerdict.technique` is a `String` today. Compare on the **enum**: change the field to
`Technique` (already `Serialize`, all unit variants ⇒ its serde name is byte-identical to the
`format!("{:?}")` phase 2 stored, so `property_profile_parity` stays green). Cost: one field-type
change in `profile.rs` + `render_cell_verdict`, and any consumer reading `.technique` as a
`String`. The alternative — `Technique::from_str` over the rendered string inside the differ — is
the re-parse-your-own-output bug class and is rejected. `trigger`/`corner` stay `String`: their
`Debug` and serde renderings differ (payload variants), and their direction is `neutral` anyway.

### Other decisions

- **`shifted` is defined by the changes, and the two must agree.** `diff_profiles` computes
  changes; a `PartialEq` on `PropertyProfile` (new derive; `PropertySet` needs `PartialEq` added)
  is asserted **equal to** `changes.is_empty()` by a unit test. If they ever disagree the test
  fails loudly rather than a change vanishing.
- **Cell subject** is `"<group>@<trigger>"` (the §"The diff" match key `(group, trigger)` rendered
  the way the text form's `cell revenue@orders` line reads).
- **Reasons are quoted, never re-derived** (§Design): a `refusal_added` change's reason is the
  `ProfileRefusal.text`; a `source_bound` change's reason is the `BoundResult`'s own text where it
  carries one; every other dimension has `reason: None`. The differ gets no new derivation.
- **Ordering**: `models` is ordered by the graph's `execution_order()` position (upstream first),
  then name — computed by the caller and passed as `DiffGraph.upstream`'s topological order;
  ties broken by name so the output is deterministic.
- **Attribution** (`attribute`): BFS upward over `graph.upstream` from the model, stopping at the
  first edited node on each path (never passing through it), collecting into a sorted `Vec`. Own
  file edited ⇒ `Edited`. No edited ancestor and `project_config_changed` ⇒ `Downstream` with
  `of: []` and the model-level reason `project configuration changed`.

## TDD test list (red before green)

All in `crates/smelt-logical/src/analysis/diff.rs` `#[cfg(test)] mod tests` unless noted. Each
test builds profiles with a small `fn profile(...)` fixture builder, no I/O.

Direction table, one per §Direction row (assert `Direction` **and** `Dimension`):
1. `technique_downgrade_walks_the_ladder_down` — `KeyedFold → DeleteInsert` ⇒ `Downgrade`;
   `DeleteInsert → KeyedFold` ⇒ `Upgrade`. Plus `ladder_is_total` asserting the five ranks are
   distinct and ordered `KeyedFold > ColumnScopedMerge > InPlaceUpdate > PerGroupRecompute >
   DeleteInsert`.
2. `cell_removed_from_maintained_model_is_a_downgrade` / `cell_added_is_an_upgrade`.
3. `source_bound_unbounding_is_a_downgrade`, `source_bound_widening_is_a_downgrade`
   (`before+after` grew), `source_bound_narrowing_is_an_upgrade`,
   `source_bound_unbounded_to_not_derivable_is_neutral` (spec delta 2).
4. `grain_lost_is_a_downgrade` (non-empty → empty, and lost a key column), `grain_gained_is_an_upgrade`.
5. `row_identity_key_to_whole_row_is_a_downgrade` (+ reverse upgrade).
6. `refusal_added_is_a_downgrade` / `refusal_removed_is_an_upgrade`; plus
   `uncoded_refusals_match_on_text_not_on_a_shared_placeholder` — two `None`-coded refusals with
   different `text` on the two sides yield one `refusal_removed` + one `refusal_added`, and an
   unchanged `None`-coded refusal yields no change (the `(Option<code>, text)` key).
7. `contract_relaxation_added_is_a_downgrade`, `contract_horizon_widened_is_a_downgrade`
   (90 days → 180 days via the new `*_seconds` fields), `contract_relaxation_removed_is_an_upgrade`,
   `retain_departed_appearing_is_a_downgrade` / `..._removed_is_an_upgrade`, and
   `retain_departed_shape_change_is_neutral` (`Bool` → `Tombstone`).
8. `probe_removed_is_a_downgrade` / `probe_added_is_an_upgrade`.
9. `determinism_clean_to_run_is_a_downgrade`, `determinism_run_to_row_is_a_downgrade`,
   `determinism_row_to_clean_is_an_upgrade`.
10. `column_added_removed_discriminant_and_corner_are_neutral`.
11. `comparability_loss_is_a_downgrade`, `set_op_barrier_appearing_is_a_downgrade`,
    `fan_out_join_appearing_is_a_downgrade`, `fd_and_literal_changes_are_neutral` (spec delta 1).

Structural cases:
12. `model_only_in_new_is_added_with_null_olds` — cause `added`, every change `old = null`.
13. `model_only_in_old_is_removed` — symmetric.
14. `identical_profiles_are_unshifted` — model absent from `models`, summary all zero.
15. `every_profile_difference_produces_at_least_one_change` — for each of a list of one-field
    mutations covering **every** `PropertySet`/`CellVerdict` field, assert
    `(old != new) == !changes.is_empty()`. This is the spec-delta-1 regression guard.
16. `renamed_column_is_removal_plus_addition` — `ColumnRemoved(old)` + `ColumnAdded(new)`, no
    rename dimension; likewise `renamed_model_is_removed_plus_added`.
17. `cells_match_on_group_and_trigger` — same group, different trigger ⇒ removed+added, not a
    technique change.

Attribution:
18. `own_file_edited_is_cause_edited`.
19. `downstream_names_nearest_edited_ancestor` — `src → a(edited) → b`: `b` gets
    `downstream of [a]`, not `[src]`.
20. `attribution_stops_at_the_first_edited_node` — `a(edited) → b(edited) → c`: `c` ⇒ `[b]`.
21. `edited_source_is_a_valid_ancestor` — an edited **source** node attributes correctly
    (the edge `DependencyGraph::get_upstream` drops).
22. `two_edited_ancestors_are_both_listed_sorted`.
23. `no_edited_ancestor_yields_project_config_cause` — `of: []`, reason
    `project configuration changed`.

Serialization / summary:
24. `change_json_matches_the_spec_schema` — one change serializes with exactly
    `dimension/subject/direction/old/new` (+`reason` only when present), snake_case dimension.
25. `summary_counts_directions` over a diff with a mix.

Purity (structural):
26. `crates/smelt-logical/tests/diff_purity.rs` — source-scan gate over `analysis/diff.rs`
    asserting no `std::fs`, `std::process`, `Command`, or `smelt_state`/`smelt_backend` token
    appears (§Constraints 2), in the shape of the existing `walk_coverage` gate.

## Tasks (each independently reviewable)

1. **Spec edits 1–6** in `docs/specs/property_diff.md`; commit before code (spec-first rule).
2. `ContractPointView` gains `frozen_horizon_seconds` / `deferral_seconds` from
   `DataLatency.seconds`; re-run `property_profile_parity` and `explain_maintenance`. **Rebase on
   the phase-2 review fix first** — `retain_departed` on the view and `Option<String>` on
   `ProfileRefusal.code` are that fix's, not this phase's; if either has not landed when this
   phase starts, stop and say so rather than implementing them here.
3. `CellVerdict.technique: Technique`; `PropertySet` + `PropertyProfile` + `CellVerdict` derive
   `PartialEq`; fix consumers. Prove JSON byte-identity via the existing parity gate.
4. New module `analysis/diff.rs` with `Dimension`, `Direction`, `ChangeKind`, `Change`, `Cause`,
   `ModelDiff`, `DiffSummary`, `PropertyDiff`; `dimension()`/`direction()`/`subject()` as
   wildcard-free exhaustive matches. Add `serde_json` to `crates/smelt-logical/Cargo.toml`.
   Tests 1–11, 24.
5. `diff_property_set` / `diff_cell_verdicts` / `diff_refusals` / `diff_probes` with the spec's
   matching rules and the no-`..` destructure. Tests 12–17, 25.
6. `DiffGraph` + `DiffGraph::from_dependency_graph` (source edges from `ModelFile.refs`) +
   `attribute`. Tests 18–23.
7. `diff_profiles` top level: ordering, summary, wiring. Purity gate test 26. Export from
   `crates/smelt-logical/src/analysis/mod.rs` and the crate root.

## Risks

- **Task 3 ripples.** `CellVerdict.technique` is read by `smelt-runtime`/`smelt-cli`/`smelt-ui`
  renderers. Mitigation: the type change is mechanical and the parity gate is the oracle; if any
  consumer genuinely needs the string, `technique.to_string()` at that call site, not a re-parse.
- **Spec delta 1 is a scope increase** (six extra dimensions, ~10 extra tests). It is not
  optional: without it §Constraints 6 is violated by construction. Keep the six rules trivial.
- **`DiffGraph` vs the literal `graph` parameter.** The success criterion says
  `diff_profiles(old, new, graph)`; the edited set rides inside `DiffGraph` rather than as a
  fourth argument, keeping the arity and the purity seam (phase 4 fills `edited`). Flagging so the
  reviewer does not read it as drift.
- **Depends on the in-flight phase-2 review fix** (`ProfileRefusal.code: Option<String>`,
  `ContractPointView.retain_departed`). Both are load-bearing here: the first stops three
  distinct refusals collapsing onto one diff key, the second is the only way the `contract_point`
  rule can observe a `retain_departed` relaxation. Verify both are in the tree before task 4.
- **Ordering depends on a topological order the caller supplies.** If `DiffGraph.upstream` is
  cyclic the sort must not hang — fall back to name order and note it (a cyclic graph is already
  a `GraphError` upstream).

## Verification gate

```bash
cargo test -p smelt-logical --lib analysis::diff 2>&1 | tail -30
cargo test -p smelt-logical --test diff_purity --test walk_coverage 2>&1 | tail -20
cargo test -p smelt-cli --test property_profile_parity --test explain_maintenance 2>&1 | tail -20
cargo test -p smelt-core --test hardening_budget 2>&1 | tail -10
bash .claude/scripts/verify-phase.sh
```

## Commit message

```
feat(property-diff): diff_profiles — exhaustive direction table, matching rules, attribution

Adds smelt_logical::analysis::diff: Dimension/Direction/ChangeKind/Change/Cause/
PropertyDiff and the pure diff_profiles(old, new, graph). The direction table is one
wildcard-free match over ChangeKind, so a new dimension without a rule is a compile
error; PropertySet/CellVerdict are destructured without `..`, so an undiffed profile
field is one too. Attribution walks the working-tree graph (model *and* source edges)
to the nearest edited ancestors, with the of: [] project-config case.

Spec first: property_diff.md gains six dimensions closing a hole where a model could be
shifted with an empty changes array, a total source_bound order including NotDerivable,
ContractPointView interval seconds (widening was not machine-comparable from the display
string) plus the retain_departed presence rule, an (Option<code>, text) refusal key for the
three uncoded refusals, the Clean<Run<Row determinism row, and drops probe `cadence`.
```
