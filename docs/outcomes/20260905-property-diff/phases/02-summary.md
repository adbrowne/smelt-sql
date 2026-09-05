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
- `bash .claude/scripts/verify-phase.sh` (full run): fmt-check FAILED on two mechanical
  (whitespace-only) diffs in `diagnostics.rs`/`smelt-ui/tests/api.rs`; `cargo fmt --all` fixed
  them, `cargo fmt --all -- --check` confirmed clean, and clippy (zero warnings, both feature
  sets), the full workspace `cargo test`, and `example_diagnostics` (119/119) all PASSed in that
  same run and were independently re-confirmed after the fmt fix.

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

   **Superseded by fix round 1, F1 below** — deviation 3 above described the original design; it
   is no longer accurate. `refusal_code` now returns `None` for these three rather than naming a
   code the pipeline can't produce.

## Fix round 1 (review findings)

Seven findings from the phase-2 review, addressed in commit order F1→F7. None redesign the
phase's structure (the `smelt-logical` move, `CellVerdict`-driven JSON, and the flatten all
stand, as instructed).

- **F1 (Critical, blocker) — refusal_code named a diagnostic the pipeline never raises.**
  `refusal_code` now returns `Option<&'static str>` (exhaustive match, no wildcard arm), `None`
  for exactly the three variants (`ReachNotDerivable`, `RepairKeysNotDiscoverable`,
  `RepairSliceUnbounded`) `smelt-db` itself maps to `None`. `ProfileRefusal.code` is now
  `Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]`. Spec-first:
  `docs/specs/property_diff.md` §"The property profile" item 3, `docs/specs/cli.md`, and
  `docs-site/docs/reference/smelt-explain.md` all now say "absent for a refusal that raises no
  diagnostic today". Added an Open Question to `property_diff.md` §Known Divergences (whether
  those three deserve their own `DiagnosticCode` entries) instead of inventing codes, per the
  ruling. The agreement test now asserts both directions (see F2).

- **F2 (Important) — the agreement gate read a `DiagnosticCode` typed into the test, not
  smelt-db's real mapping.** Extracted the inline `match` from `check_file_diagnostics`
  (`crates/smelt-db/src/lib.rs`) into `queries::maintenance::diagnostic_for_refusal(&MaintenanceRefusal)
  -> Option<(DiagnosticSeverity, DiagnosticCode, String)>`; `check_file_diagnostics` now calls it,
  and the existing diagnostics test suites (`check_diagnostics`, `maintenance_diagnostics`, the
  full `cargo test -p smelt-db`) stay green, proving behaviour didn't change. The rewritten
  `refusal_codes.rs` calls `diagnostic_for_refusal` directly and asserts agreement in both
  directions (`refusal_code_names_are_real_variants_and_agree_with_smelt_db`,
  `refusal_code_none_agrees_with_smelt_db_none`). **Visibility deviation**: the work order said
  `pub(crate)`, but `tests/integration/*.rs` compiles as a separate crate that cannot see
  `pub(crate)` items — `pub(crate)` would make the test unable to call the function at all,
  defeating F2's point. Made it `pub` (not re-exported from the crate root) instead; documented
  in the function's own doc comment. Red-green: running the *old* `refusal_code` (naming
  `MaintenanceNoAdmissibleTechnique` for all three unmapped variants) against the *new* test's
  `refusal_code_none_agrees_with_smelt_db_none` would fail, since smelt-db has no
  `MaintenanceRefusal` counterpart to construct for those three — confirming the new test
  actually exercises the F1 bug the old test missed.

- **F3 (Important) — `covers_every_example_model` was a tautology.** Chose the "derive
  `discovered_with_plan` independently" option (not deletion): added
  `count_models_with_maintenance_plan`, which asks `smelt_db::maintenance_plan_report` directly
  for every discovered model without going through `build_diagnostics_for`'s much larger
  pipeline. Observed: before the fix, `compare_workspace`'s loop incremented both `checked` and
  `discovered_with_plan` on the same unconditional lines inside the same `let Some(..) = .. else
  { continue }` branch, so the assertion `checked == discovered_with_plan` could never fail short
  of a panic — any bug that dropped a model *after* that point (a stray filter, a swallowed
  `Err` later in the pipeline) would have gone undetected. The independent count now gives the
  assertion real content.

- **F4 (Important) — the UI parity test's independent assembly passed `&[]` for probes.**
  `crates/smelt-ui/tests/api.rs`'s `assemble_diagnostics_independently` now calls
  `smelt_runtime::probe_plan::probe_plan_for_model` with the same arguments, in the same order,
  as the real endpoint (`crates/smelt-ui/src/build.rs`), including `key_locality` (added to the
  test's `maintenance_plan_report` destructure — it wasn't captured before). The test still
  passes, but now for the right reason: the endpoint's probe wiring is actually exercised by the
  comparison, not incidentally skipped because the fixture declares no probe-backed fact.

- **F5 (elevated to load-bearing) — `ContractPointView` dropped `retain_departed`.** Added
  `retain_departed: Option<String>` to `ContractPointView` (`"true"` for the boolean form,
  `"tombstone: <col>"` for the tombstone form, mirroring `EffectiveContract::render_label`'s own
  rendering), wired it through `From<EffectiveContract>`, and included it in `is_default()`.
  Updated `docs/specs/cli.md`'s JSON description of `contract_point` to mention it. This is the
  field Phase 3's `contract_point` direction rule will diff — without it, a `retain_departed`
  change could never surface as a downgrade.

- **F6 (Minor, doc notes)** — `ContractPointView`'s doc comment now explicitly distinguishes it
  from the neighbouring `ContractPoint` lattice-oracle enum (same module, deliberately distinct
  types). `property_profile_parity.rs`'s module doc now states plainly that since
  `build_maintenance_plan_json` reads `CellVerdict`, this gate compares JSON derived from the
  profile against the profile itself — a real tripwire against a parallel derivation
  reappearing, but not an independent correctness oracle for the profile's own values (that's
  `maintenance_conformance`/`maintenance_diagnostics`'s job).

- **F7 (open question, answered)** — the reviewer couldn't find where `examples/timeseries`'s 3
  probes come from. Traced it directly: `user_daily_spend` yields 1 probe
  (`mutation_profile.kind: append_only`, from its own declared source consumption) and
  `daily_events_enriched` yields 2 (`referential_integrity`, both against `raw.users` —
  `examples/timeseries/models/sources/raw/users.yml` declares `referential_integrity: [user_id]`,
  which the reviewer's search missed because it's a declared fact in a source `.yml` file, not
  model SQL/frontmatter). `1 + 2 = 3` matches the phase-2 summary's claim exactly. These are real
  declared-fact probes, not an incidental non-vacuity accident — the `total_probes > 0` assertion
  rests on genuine fixture content, safe for Phase 5 to lean on.

Deferred item (per the work order, not touched): `crates/smelt-cli/src/explain.rs` still renders
refusals/cell corner/technique via `{:?}` on the raw plan in the TEXT report — ledgered for
Phase 5.

### Gate status (fix round 1)

- `cargo check --workspace --all-targets` — clean.
- `cargo test -p smelt-logical --lib` — 746 passed.
- `cargo test -p smelt-db --test integration refusal_codes` — 2 passed
  (`refusal_code_names_are_real_variants_and_agree_with_smelt_db`,
  `refusal_code_none_agrees_with_smelt_db_none`).
- `cargo test -p smelt-cli --test property_profile_parity --test explain_maintenance --test
  explain_show_sql --test explain_probes` — all passed (37 tests).
- `cargo test -p smelt-ui --test api` — 2 passed.
- `cargo fmt --all -- --check` — clean.
- `bash .claude/scripts/verify-phase.sh` — see report for this fix round's final run.
