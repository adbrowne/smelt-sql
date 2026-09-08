# Phase 11 summary — Close-out audit

## Evidence table

| # | Criterion | Evidence | Result |
|---|-----------|----------|--------|
| 1 | Quantifier decided | `docs/specs/incremental_models.md` §"The equivalence invariant" line 694-702 ("Trimmed history narrows replayability, never `S`" — a partition aged past `retention:` stays in `S`; a recompute reaching past the bound is refused, `SourceRetentionExceeded`). `crates/smelt-cli/tests/maintenance_conformance/.../s_tracker.rs`'s `s_restricted_oracle_sql` still materialises from tracker-recorded rows (`materialize_rows`), never the physical source relation — the oracle is the unrelaxed `full_refresh(inputs ∈ S)` over all history ever processed, matching the decided quantifier. | Read, matches — no drift found |
| 2 | Declaration | `examples/broken/models/sources/retention_bad_interval.yml`, `retention_exceeded_events.yml`, `retention_unclocked.yml`, `retention_zero.yml`; `crates/smelt-cli/tests/example_diagnostics/retention_diagnostics.rs` | `example_diagnostics` 128/129 pass (1 unrelated ignore), `diagnostics_catalogue` in `verify-phase.sh` PASS |
| 3 | Reach vs. retention in the walk | `crates/smelt-logical/src/analysis/retention_reach.rs` — doc comment states "no text scan of its own", input is `derive_model_bounds`'s `BoundResult` (the walk's product); `rg -il retention crates/smelt-logical/src` shows all sites are the walk/derive/plan/refusal/retention layers, no ad hoc scan | `walk_coverage` 5/5 pass |
| 4 | Refuse or degrade, never silent | `crates/smelt-logical/tests/retention_admission.rs` — `every_retention_verdict_maps_to_a_refusal_a_downgrade_or_an_admitted_fit` plus siblings | `smelt-logical --test retention_admission` 14/14 pass |
| 5 | Bound moving is an event | `crates/smelt-runtime/tests/retention_admission.rs` — doc comment: "admissible at authoring time, but a backfill run whose window is old enough ages that reach past the retained bound"; run-time re-evaluation via `execute_project` | `smelt-runtime --test retention_admission` 4/4 pass |
| 6 | Conformance | `crates/smelt-cli/tests/maintenance_conformance/gate/retention_pool.rs` — `retention_pool_actually_trims_rows`, `retention_pool_upholds_equivalence_under_an_advancing_bound`, `an_aged_backfill_past_the_advancing_bound_refuses_and_leaves_state_unchanged` | `maintenance_conformance` 104/104 pass (full suite, unfiltered) |
| 7 | Explain + docs | `crates/smelt-cli/tests/explain_model/retention.rs`, `crates/smelt-cli/tests/explain_maintenance/docs_and_technique.rs::docs_site_diagnostics_reference_lists_every_source_retention_code` | `explain_model` 55/55, `explain_maintenance` 33/33, `cli_docs_coverage` 104/104 pass |
| 8 | Gates + ratchets | see Gates below | all green, ratchets unmoved |

No criterion's evidence was found missing or thinner than claimed. No spec prose had drifted.
No new test was written — the audit found nothing to close.

## Decisions

- No reshape needed; row 11 was the last row.
- No spec correction needed — `docs/specs/incremental_models.md`'s quantifier paragraph
  matches the conformance oracle's actual materialisation source.

## For the next planner

Nothing outstanding for this outcome. The two items named as genuinely out of scope
(succession/retention lifetime interaction, and the `smelt-ui --allow-full-refresh`
affordance) remain correctly recorded in outcome.md's "Out of scope" section and require
no further action here.

## Gates

- `cargo test -p smelt-logical --test walk_coverage --test retention_admission` — 5/5, 14/14 pass
- `cargo test -p smelt-runtime --test retention_admission --test retention_full_refresh --test statement_parity --test execute_parity` — 4/4, 5/5, 4/4, 41/41 pass
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics --test cli_docs_coverage` — 104/104, 128/129 (1 unrelated ignore), 104/104 pass
- `cargo test -p smelt-cli --test explain_model --test explain_maintenance` — 55/55, 33/33 pass
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, workspace test, example_diagnostics)
- `bash .claude/scripts/large-file-check.sh` — OK, no baseline moved
- `git status --porcelain .claude/` — clean
