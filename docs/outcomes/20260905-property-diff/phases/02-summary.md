# Phase 2 summary — `PropertyProfile` in `smelt-logical`

## Shipped

- `smelt_logical::analysis::profile`: `PropertySet` + `derive` moved verbatim from
  `smelt-runtime::diagnostics` (every input already lived in `smelt-logical`); `ProfileError`;
  `ProbePlanEntry` struct (builder stays in `smelt-runtime`, needs `smelt-backend`/`smelt-state`);
  `CellVerdict` + `render_cell_verdict`; `ProfileRefusal::from_refusal`; `ProfileProbe`;
  `PropertyProfile { properties, cell_verdicts, refusals, probes }` + `PropertyProfile::assemble`
  (pure composition, no new walk/SQL scan).
- `smelt_logical::maintenance::refusal_code(&Refusal) -> &'static str` — exhaustive, no wildcard —
  plus `smelt-db`'s `refusal_code_names_are_real_variants` agreement gate (ruling R2).
- `smelt_logical::contract::ContractPointView` (the JSON-rendering shape moved from
  `smelt-cli`'s `ExplainContractPointJson`, now a type alias) + `From<EffectiveContract>`.
  **Named `View`, not `Point`** — `smelt_logical::contract` already has an unrelated
  `ContractPoint` enum (the conformance-oracle lattice point), a collision the plan didn't
  anticipate. Deviation from the plan's literal name; called it myself rather than stopping.
- `smelt_logical::maintenance::cell_trigger_address` — single-owned trigger→address resolution,
  replacing two independent copies (`smelt-cli::explain`'s private fn and `smelt-runtime`'s
  inline match).
- `ModelDiagnostics` flattens `PropertyProfile` (`#[serde(flatten)] pub profile: PropertyProfile`)
  — `properties` key unchanged, `cell_verdicts`/`refusals`/`probes` appended. `cells` (the
  technique-preview array) is untouched and distinct from `cell_verdicts` (deliberately, to avoid
  a flatten key collision).
- `build_model_diagnostics` gained `refusals`, `probe_entries`, `contract_cfg` params and now
  assembles the profile; call sites fixed in `smelt-cli` (`commands/explain.rs`), `smelt-ui`
  (`build.rs`), and both crates' tests.
- `build_maintenance_plan_json`'s cell loop now **reads** `CellVerdict` fields instead of
  recomputing `format!("{:?}", …)` — structural parity, not coincidental. `ExplainMaintenanceJson`
  gained `refusals: Vec<ProfileRefusal>`.
- Gate: `cargo test -p smelt-cli --test property_profile_parity` (3 tests) — byte-identical JSON
  comparison over `examples/timeseries` (non-vacuous: 3 real probes, cell_verdicts present;
  refusals happen to be 0 today, asserted conditionally) and a `PropertySet::derive`-only smoke
  pass over every `examples/retail_analytics` model (documented in the test's own module doc
  comment — that workspace has zero maintained models, so its report-comparison loop is correctly
  empty).
- New `explain_json_carries_refusals` test (a staged `ScanUnbounded` fixture — no example model
  currently refuses) and an extension to `smelt-ui`'s `diagnostics_endpoint_returns_full_payload`.
- Spec edits (R1, committed first): `property_diff.md` collapses items 1–4 into one `PropertySet`
  field, `PropertyGrain`→`Grain`, refusal code is a name (not `DiagnosticCode`), and — found while
  implementing — item 2 renamed `cells`→`cell_verdicts` (the model-diagnostics response already
  has an unrelated `cells` key). `cli.md`/`ui_model_diagnostics.md` document the new surface.

## Discoveries for Phase 3 (`diff_profiles`)

- **Shape**: `PropertyProfile { properties: PropertySet, cell_verdicts: Vec<CellVerdict>,
  refusals: Vec<ProfileRefusal>, probes: Vec<ProfileProbe> }`, no model name (key a
  `BTreeMap<String, PropertyProfile>` at the caller).
- **Matching keys**: `CellVerdict` matches on `(group, trigger)` (both `String`, `{:?}`-rendered);
  `ProfileRefusal` matches on `(code, text)` (both `String`); `ProfileProbe` matches on
  `(fact, cell)`. `PropertySet.source_bounds` is already a `BTreeMap<String, BoundResult>` keyed
  by source name — match there directly, no derivation needed.
- **Encoding gotcha**: `trigger`/`corner`/`technique` are pre-rendered `String`s (via
  `render_cell_verdict`'s `format!("{:?}", …)`), not the underlying enums — diffing them is a
  string comparison, and a diff's `old`/`new` for `cell_technique` will just be these strings
  (matches the spec's "reuse the single-version report's encodings" requirement for free).
- **`PlanCell` has no `Serialize`** — deliberate choice made here: `CellVerdict` is the
  serializable *projection* `PropertyProfile` carries; `PlanCell` itself stays internal
  (`smelt-logical`'s maintenance-plan purity boundary). Phase 3's `diff_profiles` should operate
  on `CellVerdict`, never reach back into a raw `PlanCell`.
- `ContractPointView` (not `ContractPoint` — that name is taken) is the type for
  `contract_point` diffs.

## Gate status

- `cargo test -p smelt-logical --lib` (profile:: + refusal_code_tests) — green.
- `cargo test -p smelt-db --test integration refusal_codes` — green.
- `cargo test -p smelt-runtime --test diagnostics --test execute_parity` — green.
- `cargo test -p smelt-cli --test property_profile_parity --test explain_maintenance --test explain_show_sql --test explain_probes` — green (37 tests total).
- `cargo test -p smelt-ui --test api` — green.
- `cargo test -p smelt-core --test hardening_budget` — green, baseline untouched (no new
  unwrap/expect/println in production code; moved code carried none).
- `bash .claude/scripts/verify-phase.sh` — see final report line in the implementer's closing
  message.

## Deviations from the plan

1. `ContractPoint` → `ContractPointView` (name collision with an existing lattice-oracle enum of
   the same name in `smelt_logical::contract`, not anticipated by the plan).
2. `property_diff.md` item 2's field name is `cell_verdicts`, not `cells` as originally drafted in
   the R1 spec edit — found while implementing (the model-diagnostics response already has an
   unrelated `cells` key at the same JSON level via `serde(flatten)`; using `cells` for both would
   silently collide). Documented in the spec's own item 2 now.
3. The three `Refusal` variants with no `DiagnosticCode` of their own
   (`ReachNotDerivable`/`RepairKeysNotDiscoverable`/`RepairSliceUnbounded`) are named
   `MaintenanceNoAdmissibleTechnique` by `refusal_code` — the closest real code covering the same
   "no technique admits this trigger" failure, not a 1:1 semantic match. Documented in
   `refusal_code`'s own doc comment; the agreement test does not assert `smelt-db` actually emits
   that code for these three (it doesn't, today).
