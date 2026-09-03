# Phase 29 plan — key-grain frontmatter / run-window validation gaps

## Objective

Close the two key-grain validation gaps named in `incremental_shapes.md` §Known Divergences
"The key grain": a window-forward keyed run started with an incomplete event-time window must
refuse instead of silently drop-and-recreating the target, and `safety_overrides:` on a
key-addressed model must be a hard frontmatter error naming the key-grain rule. Advances the
success criterion that every still-live Known-Divergence bullet in the incremental specs is
either closed or honestly re-scoped.

## Planning-time findings (the bullets' premises are partly wrong — plan against these)

1. **`safety_overrides:` on a keyed model already errors — with the wrong code.** The top-level
   key folds into `metadata.batched` (`fold_top_level_safety_overrides`,
   `crates/smelt-core/src/metadata.rs`), and `validate_timeseries`'s
   `metadata.batched.is_some() && !metadata.is_partition_grain()` check then returns
   `PartitionGrainRequiresRefreshIncremental` — which tells a keyed author to add
   `grain: partition`, the opposite of the rule. Pinned today by
   `crates/smelt-core/tests/refresh_axis.rs::refresh_keyed_forbids_incremental`. The gap is a
   dedicated, correctly-named refusal, not the absence of one.
2. **The same predicate over-refuses a derived partition shape.** `is_partition_grain()` reads
   the *declared* `grain:` field, but `grain:` is check-only (`models.md` §"Refresh axis"): a
   model with `timeseries:` + `refresh: incremental` + `safety_overrides:` and no written
   `grain:` is refused today, though `models.md` admits `safety_overrides` on any
   partition-shaped output. Route the check through `ModelMetadata::resolved_grain()` instead.
3. **The windowless keyed arm is the `--full-refresh` path in disguise.** `execute.rs`'s
   `_ =>` arm (~L2425) drop+creates from the whole-source SELECT. Its snapshot-reconcile
   sibling refusal (~L2170) is a plain `anyhow::bail!`, not a coded diagnostic — mirror that
   shape. The drop+create must survive as the `request.full_refresh` escape; only the
   *unflagged* windowless window-forward run refuses.

## Spec delta

- `docs/specs/incremental_shapes.md` §Known Divergences "The key grain" — delete the
  "window-forward keyed run with no event-time window silently full-refreshes" bullet and the
  "`safety_overrides:` on a key-addressed model is not a hard error" bullet.
- `docs/specs/incremental_shapes.md` §Surface diagnostics table (keyed rows) — add
  `KeyedForbidsSafetyOverrides` (Error): a key-addressed model declares `safety_overrides:`;
  names the key-grain rule (every keyed rejection guards the equivalence invariant; the escape
  is to remodel or move to `refresh: materialized_view`), never `grain: partition`.
- `docs/specs/diagnostics.md` §"Keyed refresh mode" — add the same row; rewrite the retirement
  paragraph's closing sentence, which currently asserts `PartitionGrainRequiresRefreshIncremental`
  is the catcher for the folded-`batched` keyed case.
- `docs/specs/incremental_models.md` §CLI "Run flags" — state the window-forward keyed refusal
  explicitly (both flags required; one flag alone refuses; `--full-refresh` is the rebuild
  escape), matching the snapshot-reconcile bullet's existing shape.
- `docs-site/docs/reference/cumulative-aggregate.md:143` — rewrite the "currently falls back to
  a single-shot full refresh" sentence as the required-flags rule plus the `--full-refresh`
  escape.

## Tests

Red-green, in this order:

1. `smelt-core/tests/refresh_axis.rs::keyed_safety_overrides_is_keyed_error` — `grain: key` +
   `unique_key:` + folded `safety_overrides` → `MetadataError::KeyedForbidsSafetyOverrides`
   (replaces/retargets `refresh_keyed_forbids_incremental`'s assertion; keep a bare-`batched`
   case still asserting `PartitionGrainRequiresRefreshIncremental` where reachable).
2. `…::keyed_safety_overrides_without_declared_grain_is_keyed_error` — identity declared, no
   written `grain:` → same error via `resolved_grain()`.
3. `…::safety_overrides_on_derived_partition_shape_is_admitted` — `timeseries:` +
   `refresh: incremental`, no written `grain:` → `Ok(())` (finding 2's over-refusal).
4. `smelt-db` diagnostics test — the new `MetadataError` maps to
   `DiagnosticCode::KeyedForbidsSafetyOverrides` and reaches `file_diagnostics()` (CLI + LSP
   parity), with an LSP code string.
5. `smelt-runtime/tests/keyed_run_window_required.rs::window_forward_keyed_run_without_window_refuses`
   — real `execute_project` over a staged keyed fixture (copy `keyed_reprocessed_window_refusal.rs`'s
   harness); error names both flags; target table is not dropped.
6. `…::window_forward_keyed_run_with_only_start_refuses` — one flag alone refuses.
7. `…::window_forward_keyed_run_with_full_refresh_flag_rebuilds` — `full_refresh: true` still
   drop+creates.
8. `…::snapshot_reconcile_keyed_run_without_window_still_runs` — regression guard for the arm
   above the new refusal.

## Tasks

1. Add `MetadataError::KeyedForbidsSafetyOverrides` (message names the key-grain rule and the
   two escapes) in `crates/smelt-core/src/metadata.rs`.
2. In `validate_timeseries`, split the `batched.is_some()` check: key-addressed effective shape
   (`resolved_grain() == Some(Grain::Key)`) → the new error; otherwise gate the existing
   `PartitionGrainRequiresRefreshIncremental` on `resolved_grain() != Some(Grain::Partition)`
   rather than `!is_partition_grain()`. Update the function's rule-list doc comment.
3. Add the `DiagnosticCode` variant + the `map_metadata_error_to_diagnostic` arm in
   `smelt-db/src/lib.rs` (exhaustiveness gate) and the LSP code string in `smelt-lsp/src/backend.rs`.
4. In `execute.rs`'s keyed dispatch, insert a refusal ahead of the windowless `_ =>` arm: when
   the classification is not snapshot-reconcile and `!request.full_refresh`, `anyhow::bail!`
   naming the model, both flags, and the `--full-refresh` escape — mirroring the
   snapshot-reconcile bail's wording and spec citation.
5. Audit fixtures/examples that run a window-forward keyed model with no window (`examples/timeseries`,
   `examples/web_analytics`, `examples/cumulative_classifier_gate`, the conformance recipes) and
   supply a window or `--full-refresh` where the run is meant to rebuild.
6. Land the spec + docs-site edits from §Spec delta.

## Verification

- `cargo test -p smelt-core --test refresh_axis --test config_refresh_axis`
- `cargo test -p smelt-db --test integration --test maintenance_diagnostics`
- `cargo test -p smelt-runtime --test keyed_run_window_required --test keyed_reprocessed_window_refusal --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics`
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "silently full-refreshes|safety_overrides:\` on a key-addressed model" docs/specs` — no hits

## Commit message

`feat(keyed): refuse a windowless window-forward run and safety_overrides on a keyed model`
