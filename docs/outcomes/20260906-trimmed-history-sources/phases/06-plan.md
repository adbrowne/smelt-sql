# Phase 06 plan — Whole-table recompute against a trimmed source

## Objective

A whole-table recompute (`--full-refresh`, `smelt rebuild`) over a model reading a
declared-`retention:` source reaches past every finite bound, and today silently rebuilds
the table from whatever history survives. Refuse it when stored state exists, and license
it — with a *recorded* downgrade, never silence — for a first build, a smelt-forced full
refresh, and an explicit `--allow-full-refresh`. Advances criteria 4 (refuse or degrade,
never silent) and 8 (gates green).

## Spec delta (first, before code)

- `docs/specs/sources.md` §Semantics 5 "Retention refusal": add a paragraph — a whole-table
  recompute reaches past every finite bound, so it is refused when stored output exists,
  and licensed-with-a-recorded-downgrade in exactly three cases (first build, smelt-forced
  full refresh, explicit `--allow-full-refresh`). Non-incremental models are outside the
  gate, with the one-line reason. Extend the `SourceRetentionExceeded` row (§"Diagnostic
  codes") from "a backfill window" to "a backfill window **or** a whole-table recompute".
- `docs/specs/diagnostics.md` lines ~95-96: same wording extension on both rows.
- `docs/specs/model_properties.md` §"Reach versus retained history": one sentence noting the
  whole-table case is decided from the *declaration*, not the derived reach (reach is
  unbounded by construction).

## Tests (red-green, in this order)

`crates/smelt-logical/src/maintenance/retention.rs` (`full_refresh_tests` module):

1. `no_declared_retention_admits_a_full_refresh` — empty `SourceRetentions` ⇒ `Admit`.
2. `a_full_refresh_over_a_retained_source_with_stored_state_refuses` — ⇒ `Refuse`, naming
   the source and its retained bound.
3. `an_explicit_license_records_the_loss_instead_of_refusing` — ⇒ `Licensed { losses }`.
4. `a_first_build_is_licensed_not_refused` — no stored state ⇒ `Licensed`.
5. `a_smelt_forced_full_refresh_is_licensed_never_refused` — forced ⇒ `Licensed`.
6. `every_input_lands_in_exactly_one_verdict` — totality over the full
   (retention present? × stored state? × license) matrix; the no-silent-under-read property.

`crates/smelt-runtime/tests/retention_full_refresh.rs` (real DuckDB via `execute_project`,
same harness shape as `tests/retention_admission.rs`):

7. `a_full_refresh_over_a_trimmed_source_refuses_and_leaves_the_table_intact` — build
   forward first, then `full_refresh: true`; error names `SourceRetentionExceeded` and the
   source, and the target table still holds its pre-run rows.
8. `allow_full_refresh_licenses_the_rebuild_and_reports_it_once` — succeeds; exactly one
   `maintenance_warning` naming the source and bound.
9. `a_first_build_full_refresh_succeeds_and_reports_the_loss` — no prior table ⇒ succeeds
   with the warning.
10. `a_forward_only_run_is_unaffected_by_the_gate` — regression guard: ordinary
    incremental run over the same fixture still succeeds with no warning.

`crates/smelt-cli`:

11. `allow_full_refresh_flag_reaches_the_execute_request` — `smelt build` and
    `smelt rebuild` pass `--allow-full-refresh` through instead of hardcoding `false`.

## Tasks

1. Make the three spec edits above.
2. `smelt-logical` `maintenance/retention.rs`: add `RetainedSource { source, retained }`,
   `FullRefreshRetention { Admit, Licensed { losses }, Refuse { losses } }` and the pure
   total `full_refresh_retention_verdict(retentions, stored_state: bool, license: FullRefreshLicense)`
   (`FullRefreshLicense { None, Explicit, Forced }`); re-export from `maintenance`. Tests 1-6.
3. `smelt-runtime` `execute/retention_admission.rs`: `model_source_retentions(model_file, source_infos)`
   — reuse `smelt-db`'s `build_source_retentions` if it is reachable from here, otherwise mirror
   its ref→source mapping — plus a thin `check_full_refresh_retention` returning
   `Result<Vec<String>, RetentionAdmissionError>` (warning lines on the licensed path).
4. Wire ONE call site in `execute/project/mod.rs`, immediately after `force_full_refresh`
   finishes settling (after the `in_place_update_cell` block, before the keyed dispatch) so
   the forced case is visible and no data statement has yet run: gate on
   `plan.incremental.is_some()`; `full_refresh = request.full_refresh || request.rebuild ||
   force_full_refresh`; refuse via `Err(anyhow!(err))`, license via
   `reporter.maintenance_warning`. Document the placement choice in the call-site comment.
5. `smelt-cli`: add `--allow-full-refresh` to `BuildArgs`/`RebuildArgs` and pass it into
   `ExecuteRequest` (both currently hardcode `false`, which would make the refusal
   unlicensable from those two commands). Test 11.
6. Audit and fix fallout: `rg -n "full.refresh" crates/*/tests examples` for anything that
   full-refreshes a model over `examples/github_activity`'s 45-day sources; update the test
   or pass the license, whichever matches its intent.
7. Tests 7-10, then the gates below; re-run `.claude/scripts/large-file-check.sh --update`
   only if the wiring pushes `execute/project/mod.rs` past baseline, with a sign-off line.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-runtime --test retention_full_refresh --test retention_admission --test execute_parity --test statement_parity --test availability_seam`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(maintenance): refuse or license a whole-table recompute over a trimmed source`
