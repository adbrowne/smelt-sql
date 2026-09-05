# Phase 5 plan — `smelt explain --diff`: text, JSON, `--fail-on`, `--select`, exclusivity

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md` — success criteria 4 and 5.
**Spec:** `docs/specs/property_diff.md` §Surface (flag table, "Output forms"), §Semantics,
§Constraints 6 and 9; `docs/specs/cli.md` §"Exit codes", §Constraints item 5 (append-stable JSON).
**Carry-in:** `phase5-carry.md` items C1–C5 are in scope for this phase, placed in §Tasks.

## Objective

Land the user-facing command. `smelt explain --diff [<ref>]` resolves a baseline, materialises it,
derives profiles for both sides, diffs them, and renders text (default) or JSON (`--json`);
`--fail-on {downgrade,any}` gates exit `1`; `--select` narrows the *reported* set only; combining
`--diff` with `<model>`, `--show-sql`, `--period` or `--technique` exits `2`. Plus the carried
items: the single-model text report renders the profile (C1), a one-sided derivation failure is
reported (C2), a state-downgrade fixture exists (C3), the hardcoded dialect is fixed (C4), and two
stale comments are deleted (C5).

## Spec delta (required — write it first, in this phase)

Three edits to `docs/specs/property_diff.md`, all append-stable:

- **Δ1 — `cause.reason` in the JSON schema.** §Surface "JSON" shows `"cause": { "kind", "of" }`,
  but §Attribution already requires the reason `project configuration changed` on the `of: []`
  case, and Constraint 6 requires a one-sided derivation failure to be reported with "the
  derivation failure as its reason". Add the optional `"reason": "<one line>"` key to the `cause`
  object in the schema block, and state in §Semantics "The diff" that an `added`/`removed` entry
  arising from a *derivation failure* (rather than a genuinely new/deleted model) carries that
  failure text as `cause.reason`. This is what C2 renders; without the delta the CLI would emit a
  field the schema does not name, breaking criterion 5's "emitted exactly".
- **Δ2 — `--select` and the summary.** §Surface says `--select` restricts the reported set but is
  silent on whether the summary counts and `--fail-on` follow the reported or the compared set.
  State it: the summary counts and `--fail-on` are computed over the **reported** set, so the
  printed counts always match the printed blocks; the *compared* set is still every model, which
  is what keeps attribution correct.
- **Δ3 — exit-code cross-reference.** `docs/specs/cli.md` §"Exit codes" says every other mention
  refers back to it; add a "**`smelt explain --diff` specifics:**" paragraph there (exit `1` only
  under `--fail-on`; exit `2` for an unresolvable baseline or an exclusive-flag combination),
  pointing at `property_diff.md`.

C4 is resolved by code (see Task 9), so it needs no §Known Divergences entry.

## Design decisions

**D1 — Where the command lives.** New module `crates/smelt-cli/src/commands/explain_diff.rs`,
entered from `commands::explain::explain` as the *first* branch (before the `model_name` branch),
so the diff path shares none of the single-model path's state. `main.rs` keeps one `ExplainArgs`.

**D2 — Sequencing the two loads.** Working tree first (`find_project_root` → `load_workspace` →
`profiles_for_workspace`), then `resolve_baseline(project_dir, explicit)` →
`materialize(&resolved)` → `load_workspace(checkout.project_root())` → `profiles_for_workspace`.
Working-tree-first means a broken working tree fails before any scratch directory exists. The
`BaselineCheckout` is held in scope until after `edited_set` and both profile maps are built, then
dropped (its `Drop` deletes the scratch dir — Phase 4's guarantee, not re-implemented here).
`DiffGraph::from_dependency_graph(work_graph, edited.names, edited.project_config_changed)` is
built from the **working-tree** graph (§Attribution).

**D3 — Error → exit code.** No new classifier. `BaselineError` already maps to `2` via
`smelt_cli::exit_code_for` (Phase 4). `ProfileWorkspaceError` on either side is wrapped with
context and stays exit `1`: it is a defect in the project's own models, not in the invocation, so
it is a detected failure rather than a usage error. `--fail-on` returns
`CliError::DetectedFailure` → exit `1` through the same generic classifier.
Flag exclusivity is **clap's own** `conflicts_with_all`, not a hand-rolled check: clap exits `2`
on a conflict, matching the spec, and it is the established mechanism in `main.rs` (which already
uses clap's `requires`). No `ArgGroup` needed.

**D4 — `--diff`'s optional value.** `#[arg(long, num_args = 0..=1, default_missing_value = None)]`
over `diff: Option<Option<String>>`: absent → no diff mode; `Some(None)` → default merge-base;
`Some(Some(r))` → explicit ref. Because clap greedily consumes the following token as the ref,
`smelt explain --diff my_model` means "ref `my_model`", not "model `my_model`" — which is exactly
what §Surface specifies. The exclusivity test therefore must write the positional *before* the
flag (`smelt explain my_model --diff`).

**D5 — Renderer factoring (the question this phase must answer).** The text renderer is **not** in
`smelt-cli`. A new pure module `smelt_logical::analysis::diff_render` owns the rendering, over a
new pure envelope in `smelt_logical::analysis::diff`:

```rust
pub struct BaselineInfo { pub r#ref: String, pub commit: String, pub resolved_as: String }
pub struct DiffReport { pub baseline: BaselineInfo, pub edited_files: Vec<String>,
                        pub summary: DiffSummary, pub models: Vec<ModelDiff> }
```

`DiffReport`'s `Serialize` **is** the §"Output forms" JSON schema, top-level key order included —
so JSON is `serde_json::to_string_pretty(&report)`, never a hand-assembled `json!` that could
drift from the type the other renderers read. `diff_render` exposes exactly four functions:

- `change_line(&Change) -> String` — glyph + dimension + subject + `old → new`
- `reason_line(&Change) -> Option<String>`
- `model_block(&ModelDiff) -> String` — the header line with its cause, then the change lines
- `text_report(&DiffReport) -> String` — header, blocks in `report.models` order, summary line;
  the whole output is the single `property diff vs <ref>: no models shifted` line when
  `models` is empty.

Phase 6's Markdown renderer is a second function in the same module reusing `change_line` and
`model_block`'s rows; Phase 7's LSP builds its `PropertyDowngrade` message from `change_line` +
`reason_line` on the same `Change`, and its lens counts from `DiffSummary`. **Ordering is not
re-derived anywhere**: `diff_profiles` already sorts `models` topologically then by name
(`diff.rs`, `topological_order`), so every renderer iterates `report.models` as given. That is
what makes Phase 7's Constraint 5 parity gate cheap — the LSP compares against the same vector the
CLI serialises, with no second sort and no second message assembly. `smelt-lsp` reaches
`smelt-logical` through its existing `smelt-db` dependency (the layering rule permits this; the
LSP's need for `smelt-runtime`'s `profiles_for_workspace` is Phase 7's problem, not this phase's).

**D6 — `--select`.** Reuse the existing selector path verbatim from `commands::explain`
(`resolve_selector_args` → `parse_selector` → `graph.select_models` →
`filtered_execution_order`) against the **working-tree** graph, producing a name set; then
`report.models.retain(...)` and recompute `report.summary` from the retained models. Compared set
untouched (D2 already derived every model on both sides).

**D7 — C1's shape.** `build_maintenance_plan_report` gains a `profile: &PropertyProfile`
parameter and renders `corner`, `technique`, and the refusal list from it instead of from
`result.plan.cells` / `result.plan.refusals`. To get a profile on the plain-text path *without*
paying for the full `build_model_diagnostics` (which runs one emitter per technique per cell), the
profile half of that builder is extracted into
`smelt_runtime::diagnostics::build_model_profile(model, bound_ctx, plan_cells, column_groups,
refusals, probe_entries, contract_cfg) -> Result<PropertyProfile, DiagnosticsError>` —
`PropertySet::derive` + the per-cell `effective_contract` fold + `PropertyProfile::assemble`, with
no registry, resolver, schema or target. `build_model_diagnostics` calls it, so there is still
exactly one assembly path (Constraint 1). The single-model command calls it before building the
report; `profiles_for_workspace` is unchanged.

## TDD test list

Every test below is written and observed **red** before its implementation.

**`crates/smelt-logical/tests/diff_render.rs`** (new; pure, no git, no fixtures)

1. `text_report_of_an_empty_diff_is_one_line` — a `DiffReport` with no models renders exactly
   `property diff vs <ref>: no models shifted\n`. Red: module does not exist.
2. `change_line_uses_the_specified_glyphs` — one `Change` per `Direction` renders `▼`/`▲`/`●`.
3. `model_block_headers_render_each_cause_kind` — `(edited)`, `(added)`, `(removed)`,
   `(downstream of a, b)`, and the `of: []` case rendering its `cause.reason`.
4. `text_report_preserves_diff_profiles_ordering` — a `DiffReport` whose `models` are in a
   deliberately non-alphabetical order renders in that order (proves the renderer does not sort).
5. `report_json_matches_the_spec_schema_keys` — `serde_json` of a populated `DiffReport` has
   exactly the top-level keys `baseline`, `edited_files`, `summary`, `models`; `baseline` has
   `ref`/`commit`/`resolved_as`; a `cause` with no reason omits the key (Δ1).

**`crates/smelt-cli/tests/property_diff_cli.rs`** (new; temp git repo, spawns `CARGO_BIN_EXE_smelt`)

Git helpers (`git`, `git_commit`, `copy_dir`) are re-created here from
`crates/smelt-core/tests/baseline.rs` (a test-binary-local module, not importable across crates)
and `crates/smelt-cli/tests/exit_codes.rs`'s `copy_dir` — the same shapes, not new inventions.

6. `diff_with_a_model_argument_is_a_usage_error` — `smelt explain user_daily_spend --diff` exits
   `2`. Likewise `--diff --show-sql`, `--diff --period 2024-01-01..2024-01-02`,
   `--diff --technique keyed_fold` (criterion 4's exclusivity clause; four assertions).
7. `diff_outside_a_git_work_tree_exits_2` — a copied project in a plain temp dir, no `git init`.
8. `diff_with_an_unknown_ref_exits_2` — `--diff nonexistent-ref`.
9. **`sum_to_max_downgrades_the_edited_model_and_its_downstream`** (criterion 4's fixture test) —
   copy `examples/timeseries` into a temp git repo, `git init -b main`, commit, then edit
   `models/user_daily_spend.sql` `SUM(amount)` → `MAX(amount)`, leaving it uncommitted. Assert on
   `smelt explain --diff --json`: `user_daily_spend` has `cause.kind == "edited"` and at least one
   `cell_technique` change with `direction == "downgrade"`; `user_spend_rollup` (its downstream)
   has `cause.kind == "downstream"` with `of == ["user_daily_spend"]`. Exit `0`.
10. `a_formatting_only_edit_yields_no_models_shifted` — same repo, append a trailing SQL comment
    line to a model instead; text output is exactly the `no models shifted` line, exit `0`.
11. `fail_on_downgrade_exits_1_and_fail_on_any_exits_1` — the Test-9 repo with `--fail-on
    downgrade` and with `--fail-on any`, both exit `1`; the Test-10 repo with `--fail-on any`
    exits `0`.
12. `select_narrows_the_reported_set_but_not_attribution` — Test-9's repo with
    `--select user_spend_rollup --json`: `models` has exactly that one entry, its `cause` still
    names `user_daily_spend` (attribution unaffected), and `summary.shifted_models == 1` (Δ2).
13. `baseline_side_derivation_failure_is_reported_with_its_reason` (**C2**) — a repo whose
    committed version of a model derives no profile and whose working-tree version does; the
    model is reported `added` with `cause.reason` carrying the failure text, never omitted.
14. `diff_json_top_level_matches_the_schema` — the Test-9 repo's `--json` parses and has the five
    schema-mandated shapes (baseline object keys, `edited_files` containing the edited path,
    summary counts consistent with the models array).

**`crates/smelt-cli/tests/property_profile_parity.rs`** (extend)

15. `state_downgrade_fixture_technique_matches_the_report` (**C3**) — a new small standalone
    fixture workspace under `crates/smelt-cli/tests/fixtures/state_downgrade/` (deliberately NOT
    `examples/timeseries`): `smelt.yml` with `state:\n  warehouse_tables: none` and one
    `refresh: incremental` model whose derived cell would otherwise take a ledger-requiring
    technique. Assert the profile's `cell_verdicts[..].technique` equals the `technique` field
    `smelt explain <model> --json` prints for that cell, and that at least one cell carries a
    non-`None` `state_downgrade` — so the `resolve_availability` wiring Phase 4 added, and the
    `state_downgrade` dimension, both have a live producer. Red today: no such fixture exists.

**`crates/smelt-cli/tests/explain_maintenance.rs`** (extend)

16. `text_report_technique_matches_the_profile_technique` (**C1**) — for a model in
    `examples/timeseries`, the `technique:` line of the text report and the `technique` value of
    `--json` (which reads the profile) agree; kept as a live gate that the two sides share a
    source. Red before D7's rewiring only if the two are made to differ — so write it as an
    assertion over a *constructed* `PropertyProfile` whose technique differs from the raw plan
    cell's, at the `build_maintenance_plan_report` unit level, which is genuinely red today.

**`crates/smelt-runtime/tests/profile_workspace.rs`** (extend)

17. `profiles_use_the_models_own_target_dialect` (**C4**) — a two-target fixture (`duckdb` default
    plus a `spark` target bound to one model via `smelt.yml`) asserts the Spark-targeted model's
    availability resolution used `SqlDialect::SparkSQL`. Red before Task 9.

**`crates/smelt-cli/tests/cli_docs_coverage.rs`** — no new test; the existing
`every_long_flag_is_documented` turns red the moment `--diff`/`--fail-on` are declared, and Task 11
turns it green.

## Tasks

Each task is independently reviewable and leaves the tree compiling.

1. **Spec delta.** Δ1, Δ2 (`docs/specs/property_diff.md`), Δ3 (`docs/specs/cli.md`). Timeless
   voice; no phase vocabulary. Commit alone, first.
2. **C5 (cosmetic).** Delete the stale comment at `crates/smelt-cli/tests/property_profile_parity.rs:355-357`
   and the stale "does not happen in a real derivation" comment at
   `crates/smelt-logical/src/analysis/diff.rs:1333`. Fold into Task 1's commit.
3. **`DiffReport`/`BaselineInfo` envelope** in `smelt_logical::analysis::diff`, with
   `From<&smelt_core::baseline::ResolvedBaseline> for BaselineInfo` so the mirror cannot drift.
   Tests 5.
4. **`diff_render` module** (D5): `change_line`, `reason_line`, `model_block`, `text_report`.
   Tests 1–4.
5. **C2 rendering:** `diff_profiles`' caller side — a model present in one side's
   `WorkspaceProfiles::failures` and absent from that side's `profiles` is emitted as
   `added`/`removed` with `cause.reason` = the failure text. This is assembled in the CLI's
   report-building step (the failure maps are not part of `diff_profiles`' pure inputs, and adding
   them there would widen a pure signature for a presentational fact). Test 13.
6. **`build_model_profile` extraction** in `smelt-runtime::diagnostics` (D7 first half);
   `build_model_diagnostics` rewired to call it. No behaviour change; no new test of its own.
7. **C1:** `build_maintenance_plan_report` takes `&PropertyProfile` and renders corner, technique
   and refusals from it; `commands::explain` builds the profile via Task 6 before the report.
   Test 16.
8. **C3 fixture** + Test 15.
9. **C4:** make `smelt_runtime::execute::sql_dialect_for_target` `pub(crate)` and use it inside
   `profiles_for_workspace`'s per-model loop (the loop already resolves `target`), moving the
   `availability_for_run` call and the `MaintenanceDialect` derivation inside the loop, keyed on
   the model's own target. Delete the "Scope note" paragraph in `profile.rs`'s module doc that
   documents the hardcoding. Test 17.
10. **The command** (D1–D4, D6): `ExplainArgs` gains `diff`, `fail_on`, with
    `conflicts_with_all = ["model_name", "show_sql", "period", "technique"]` on `diff`;
    `commands/explain_diff.rs` implements the pipeline and both renderings. Tests 6–12, 14.
11. **Docs:** `docs-site/docs/reference/cli.md` §"smelt explain" — flag rows for `--diff` and
    `--fail-on` (verbatim `--diff`/`--fail-on` literals, which is what `cli_docs_coverage` greps
    for), plus a short diff-mode subsection; `docs-site/docs/reference/smelt-explain.md` — a
    "Property diff" section with the text form, the JSON schema, the exit codes, and a worked
    example. Timeless voice.
12. **Summary:** `docs/outcomes/20260905-property-diff/phases/05-summary.md` (≤40 lines).

## Risks

- **R1 (biggest).** Test 9 assumes editing `user_daily_spend` `SUM` → `MAX` actually downgrades
  its cell *and* propagates a shift to `user_spend_rollup`. Neither is guaranteed by inspection —
  `user_spend_rollup` merely passes `total_amount` through. Mitigation: run the diff against the
  fixture repo by hand **before** fixing the assertion text; if `user_spend_rollup` does not
  shift, pick the downstream that does (`user_spend_running_total`, `daily_revenue`'s
  descendants) and record the substitution in the summary. Do **not** weaken the assertion to
  "some model shows `downstream`" — criterion 4 names a specific pairing.
- **R2.** `--diff` alone consuming the next token as a ref (D4) can surprise; the exclusivity
  tests must be written in the argument order noted, or they will pass for the wrong reason.
- **R3.** C1's Task 7 changes a widely-asserted text surface. `explain_docs_freshness` asserts
  hand-pasted `smelt explain` excerpts in `docs-site/` are byte-identical to what the binary
  prints — if the profile's rendering of `corner`/`technique` differs by even a space from the
  `{:?}` form, that gate goes red and the excerpts must be regenerated. Check it explicitly.
- **R4.** `profiles_for_workspace` runs the full per-model derivation twice per invocation over
  `examples/timeseries`; Test 9's wall time may be minutes. Keep the fixture repo build to one
  `TempDir` shared across the tests that can share it.
- **R5.** Moving the availability/dialect derivation inside the loop (Task 9) changes
  `property_profile_parity`'s inputs; re-run it after Task 9, not only at the end.

## Verification gate (staged; explicit `timeout` on each Bash call, none over 10 min)

```
cargo fmt --all -- --check
CARGO_BUILD_JOBS=6 cargo check --workspace --all-targets 2>&1 | tail -30
bash .claude/scripts/clippy-gate.sh 2>&1 | tail -40
CARGO_BUILD_JOBS=6 cargo test -p smelt-logical --test diff_render --quiet 2>&1 | tail -20
CARGO_BUILD_JOBS=6 cargo test -p smelt-runtime --test profile_workspace --quiet 2>&1 | tail -20
CARGO_BUILD_JOBS=6 cargo test -p smelt-cli --test property_diff_cli --quiet 2>&1 | tail -30
CARGO_BUILD_JOBS=6 cargo test -p smelt-cli --test property_profile_parity --test explain_maintenance \
  --test explain_docs_freshness --test cli_docs_coverage --test exit_codes --quiet 2>&1 | tail -30
CARGO_BUILD_JOBS=6 cargo test -p smelt-core --test hardening_budget --quiet 2>&1 | tail -20
CARGO_BUILD_JOBS=6 cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20
```

Then the full `CARGO_BUILD_JOBS=6 cargo test --workspace --quiet 2>&1 | tail -40`, narrowed
per-crate if it exceeds ten minutes. Any new production `unwrap`/`expect` is classified or
converted before `hardening_budget` is re-baselined; the baseline is not lowered.

## Commit message

```
feat(explain): smelt explain --diff — text and JSON property diff

`smelt explain --diff [<ref>]` resolves a git baseline, derives every
model's property profile at both versions, diffs them, and reports the
shifted models as text or JSON (`docs/specs/property_diff.md` §Surface).
`--fail-on {downgrade,any}` gates exit 1; `--select` narrows the reported
set without narrowing the compared set, so attribution stays correct;
`--diff` conflicts with `<model>`, `--show-sql`, `--period` and
`--technique` via clap, exiting 2.

Rendering is single-owned in `smelt_logical::analysis::diff_render` over a
`DiffReport` envelope whose `Serialize` is the spec's JSON schema, so the
Markdown form and the editor's lens/diagnostic surfaces render the same
value in the same order (§Constraints item 5) rather than re-deriving it.

Also: the single-model text report now renders the property profile rather
than the raw plan (§Constraints item 1); a one-sided derivation failure is
reported as added/removed with its reason (§Constraints item 6); profiles
resolve availability against each model's own target dialect instead of a
hardcoded DuckDB; a state-downgrade fixture gives that dimension a live
producer.
```
