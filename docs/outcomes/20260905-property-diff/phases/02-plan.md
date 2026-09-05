# Phase 2 — `PropertyProfile` in `smelt-logical`

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md`, success criterion 1.
**Spec:** `property_diff.md` §"The property profile", §Design "The single-version report becomes
a rendering of the profile", §Constraints 1, 4, 7.

## Objective

Land `smelt_logical::analysis::profile::PropertyProfile` as the single owner of a model's
composition-relevant verdicts, and make `ModelDiagnostics` and `smelt explain --json`
*renderings* of it rather than independent assemblies. Ship the standing gate
`cargo test -p smelt-cli --test property_profile_parity` over both example workspaces. No diff,
no baseline, no CLI flag here.

## Spec delta (required — the profile field list does not match the code)

Five edits, made **first** (timeless-oracle rule: no phase vocabulary):

1. `property_diff.md` items 1–4 → one item. Those verdicts already live in one struct,
   `PropertySet` (`crates/smelt-runtime/src/diagnostics.rs:81`: columns, grain, FDs, determinism,
   comparability, discriminants, literal columns, two barrier flags, `row_identity`,
   `source_bounds`); splitting them back out forks the derivation. New wording: "`properties` —
   the model's derived property set (`PropertySet`)".
2. `PropertyGrain` **is not a type**: it is `smelt_logical::analysis::walk::Grain`
   (`crates/smelt-logical/src/analysis/walk.rs:1795`), aliased at `diagnostics.rs:29`.
3. Item 6's `DiagnosticCode` is unreachable from `smelt-logical` — it lives in
   `crates/smelt-db/src/diagnostics_types.rs:9`, above it. Reword to "the diagnostic-code *name*
   plus the refusal text"; a `smelt-db` test pins names to real variants.
4. `cli.md` §"`smelt explain <model>` maintenance-plan report": `--json` gains an append-stable
   `refusals` array. Refusals are text-only today (`crates/smelt-cli/src/explain.rs:888-895`);
   criterion 1 requires them in JSON.
5. `ui_model_diagnostics.md` §Surface: the response gains `cell_verdicts`, `refusals`, `probes`
   beside the unchanged `properties` key.

## Design decisions

- **Where.** New `crates/smelt-logical/src/analysis/profile.rs`. `PropertySet` + `derive` move
  down verbatim from `diagnostics.rs:81-152` — every input (`model_property_vector`,
  `row_identity`, `derive_model_bounds`, `BoundContext`, `JoinContext`) is already
  `smelt-logical`. `smelt_runtime::diagnostics` re-exports `PropertySet` so `smelt-cli`/
  `smelt-ui` imports are untouched; `DiagnosticsError::PropertyDerivation` wraps a new
  `profile::ProfileError`.
- **Refactor, not new derivation.** `PropertyProfile::assemble(...)` is a pure assembly over
  already-derived inputs (`MaintenancePlanResult.plan.cells`/`.refusals`, `ColumnGroup`s,
  `ProbePlanEntry`s, `ContractConfig`) — the shape `build_model_diagnostics`
  (`diagnostics.rs:1099`) already takes. No walk call beyond `PropertySet::derive`, no SQL scan.
- **Shape.** `PropertyProfile { properties: PropertySet, cell_verdicts: Vec<CellVerdict>,
  refusals: Vec<ProfileRefusal>, probes: Vec<ProfileProbe> }` — deliberately **no model name**
  (the diff keys a `BTreeMap<String, PropertyProfile>`).
  `CellVerdict { group, trigger: String, corner: String, technique: String, row_identity:
  RowIdentityVerdict, contract_point: ContractPoint }`; the three `String`s come from one pure
  `render` helper, so `ExplainCellJson`'s `format!("{:?}", …)` calls (`explain.rs:1591-1595`) are
  *replaced by reads of the verdict* — parity becomes structural, not coincidental.
  `ProfileRefusal { code: String, text: String }`, `text = format!("{:?}", refusal)` (the
  report's own rendering, `explain.rs:893`), `code` from a new exhaustive
  `maintenance::refusal_code(&Refusal) -> &'static str` (no wildcard arm).
  `ProfileProbe { fact, probe, cell, cadence }` — the `ProbePlanEntry` *struct*
  (`crates/smelt-runtime/src/probe_plan.rs:26`) moves down and is re-exported; its builder
  `probe_plan_for_model` stays (it needs `smelt-backend`/`smelt-state`). `cost` is a rendering.
  `ExplainContractPointJson` (`explain.rs:1399`) moves to
  `smelt_logical::contract::ContractPoint`, serde shape byte-identical, old name kept as a
  `smelt-cli` alias; sourced from `contract::effective_contract`, never re-resolved.
- **Re-pointing without output change.** `ModelDiagnostics` (`diagnostics.rs:1057`) drops
  `properties` and gains `#[serde(flatten)] pub profile: PropertyProfile` — flatten keeps the
  `properties` key at the same path (`smelt-ui` serializes `ModelDiagnostics` directly,
  `crates/smelt-ui/src/types.rs:10`) and adds three keys. `ExplainMaintenanceJson`
  (`explain.rs:1442`) keeps `properties` from `profile.properties` and gains `refusals`.
- **Encoding.** Every profile type derives `Serialize, Clone, Debug, PartialEq, Eq`; the gate
  compares `serde_json::to_string` of the subtrees as strings ("byte-identical").
## TDD test list (each red before its code)

1. `smelt-logical/src/analysis/profile.rs` unit `property_set_moves_intact` — the moved `derive`
   over a `GROUP BY` model yields the grain/columns the pre-move `smelt-runtime` units asserted.
2. `smelt-logical/src/maintenance/mod.rs` unit `every_refusal_has_a_code` — one value per
   `Refusal` variant returns a non-empty code; a future variant is a compile error.
3. `smelt-db/tests/integration/refusal_codes.rs` `refusal_code_names_are_real_variants` — every
   name parses to a `DiagnosticCode` variant and, for the variants
   `queries/maintenance.rs:1385-1470` maps, equals the code `lib.rs` emits.
4. `smelt-logical/tests/profile.rs` `profile_assembles_cells_refusals_probes` — two cells, one
   `ScanUnbounded`, one probe → expected verdicts; `contract_point` default with no `contract:`.
5. `smelt-runtime/tests/diagnostics.rs` `flatten_keeps_properties_key` —
   `to_value(&d)["properties"]["grain"]` resolves; `refusals`/`probes`/`cell_verdicts` present.
6. `smelt-cli/tests/explain_maintenance.rs` `explain_json_carries_refusals` — a refusing model
   shows it in `--json`'s `refusals` with the text the report prints.
7. `smelt-cli/tests/property_profile_parity.rs` (the gate)
   `report_json_matches_profile_encoding` — for every model in `examples/timeseries` and
   `examples/retail_analytics`, build the profile in-process and the report JSON via the
   resolution `explain_maintenance.rs::build_report_for` uses; assert byte-identical strings for
   `properties`, `cell_verdicts` vs the report cells' scalar fields, per-cell `contract_point`,
   `refusals`, `probes`. Plus `covers_every_example_model` (checked == discovered, both
   workspaces) and `covers_at_least_one_maintained_model` (≥1 non-empty `cell_verdicts`).
   **`examples/retail_analytics` declares no `grain:` and no incremental models** — all 25 have
   an empty plan, so its leg covers only `properties` and the empty-cells path. Say that in the
   test doc comment rather than letting a reader assume otherwise.

## Tasks

1. Spec edits 1–5.
2. `maintenance::refusal_code` + test 2.
3. `smelt-db` agreement test 3 (register in `tests/integration/mod.rs`).
4. Move `PropertySet`/`derive` into `analysis/profile.rs`, re-export, `ProfileError`. Test 1.
5. Move the `ProbePlanEntry` struct and `ContractPoint` down; keep re-export + alias.
6. `PropertyProfile` + `CellVerdict`/`ProfileRefusal`/`ProfileProbe` + `assemble`. Test 4.
7. Re-point `ModelDiagnostics` (flatten) + `build_model_diagnostics`; fix `smelt-ui`. Test 5.
8. Re-point `build_maintenance_plan_json` to read `CellVerdict`; add `refusals`. Test 6.
9. The gate `property_profile_parity.rs`. Test 7.
10. `docs-site/docs/reference/smelt-explain.md` — the new `refusals` key.

## Risks / trip hazards

- `build_maintenance_plan_json` zips `plan_cells` with `CellStatements` (`explain.rs:1555`); the
  profile must stay statement-free or the CLI gains a compile path it does not need.
- `serde(flatten)` collides on any key `ModelDiagnostics` already has — hence no `model` field.
- Two technique encodings already exist: `ExplainCellJson.technique` is `format!("{:?}")`,
  `PlanCellDiagnostics.admitted_technique` is a `Technique`. Match the *report's* String.
- Several `Refusal` variants map to `None` in `smelt-db` (`queries/maintenance.rs:1425`,
  `1451-1452`); `refusal_code` must still name a real code, and test 3 asserts agreement only
  where `smelt-db` emits one.
- `hardening_budget` is two-sided: moving code shifts per-crate counts — re-run `--update` with
  the reason in the commit body.
- Do not touch `docs/specs/diagnostics.md` or `docs-site/docs/reference/diagnostics.md` (Phase 1).

## Verification gate

```
cargo test -p smelt-logical --test profile --test walk_coverage 2>&1 | tail -40
cargo test -p smelt-db --test integration refusal_codes 2>&1 | tail -20
cargo test -p smelt-runtime --test diagnostics --test execute_parity 2>&1 | tail -30
cargo test -p smelt-cli --test property_profile_parity --test explain_maintenance \
  --test explain_show_sql --test explain_probes 2>&1 | tail -40
cargo test -p smelt-ui --test api 2>&1 | tail -20
bash .claude/scripts/verify-phase.sh
```

## Commit message

```
feat(property-diff): PropertyProfile single-owned in smelt-logical

Move PropertySet, ProbePlanEntry and the contract-point JSON shape into
smelt-logical and assemble them, with the plan's cell verdicts and refusals,
into analysis::profile::PropertyProfile. ModelDiagnostics flattens the profile
(`properties` unchanged; cell_verdicts/refusals/probes append-stable) and
`smelt explain --json` renders its cells and new `refusals` array from it.
Adds exhaustive maintenance::refusal_code and the standing gate
`cargo test -p smelt-cli --test property_profile_parity`.

Spec: property_diff.md §"The property profile" (Grain not PropertyGrain; the
profile carries the whole PropertySet; refusal codes are names since
DiagnosticCode lives above smelt-logical), cli.md, ui_model_diagnostics.md.
```
