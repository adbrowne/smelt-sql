# Phase 9 summary — retire the `smelt.yml` `models.<name>.batched:` sub-block

## Shipped

- `ModelConfig::batched` (`crates/smelt-core/src/config.rs`) retired to a renamed,
  always-erroring `deserialize_with` sentinel (`batched_retired: ()`) — any `batched:`
  key on a `smelt.yml` model entry, regardless of contents, is a hard parse error
  naming each declared sub-key's top-level replacement (`unique_key` → `merge_key:`,
  `safety_overrides` → `safety_overrides:`, `nondeterministic_columns` →
  `columns.<c>.contract: plausible`), carrying the caller's own values.
- New top-level `merge_key:` key, declarable both as a `smelt.yml` model override
  (`ModelConfig::merge_key`) and in `.sql` frontmatter (`ModelMetadata::merge_key`,
  added to the frontmatter key catalogue in `crates/smelt-core/src/frontmatter.rs` —
  without this the key silently vanished before deserialization). Frontmatter wins
  wholesale over the smelt.yml spelling. Folds into the internal
  `PartitionGrainConfig.unique_key` representation every existing `batched:`-shaped
  consumer already reads (`fold_smelt_yml_incremental_keys` on the smelt.yml side,
  `fold_top_level_merge_key` on the frontmatter side) — zero downstream consumer
  changes needed.
- `validate_model_configs`'s smelt.yml-side `safety_overrides` double-declaration
  check deleted (unreachable now that the sub-block can't coexist with anything);
  the ephemeral/view "has incremental configuration" checks switched from reading
  `ModelConfig::batched` presence to `merge_key.is_some() || safety_overrides.is_some()`.
- Spec deltas: `docs/specs/models.md` (Surface table, Constraint violations, "Batched
  sub-block retirement" section, Known Divergences — the "separate parsing path"
  clause removed since both surfaces now agree), `docs/specs/incremental_models.md`
  (deleted the now-closed "MERGE-dedup key has no `.sql` frontmatter home" gap
  bullet), `docs/specs/diagnostics.md`. docs-site: `reference/smelt-yml.md` and
  `guide/sql-models.md` updated to `merge_key:`.
- `examples/timeseries/smelt.yml`'s two `batched: {unique_key: [event_id]}` blocks →
  `merge_key: [event_id]`; explanatory comments updated (there and in the two model
  `.sql` files).
- Six new smelt-core tests (`smelt_yml_batched_block_is_retired`,
  `smelt_yml_batched_block_names_every_declared_key`,
  `merge_key_folds_into_incremental_config`, `merge_key_frontmatter_wins_over_smelt_yml`,
  `merge_key_does_not_confer_identity`, `ephemeral_model_with_merge_key_is_refused`,
  `merge_key_parses_in_frontmatter`) plus one rewritten test replacing
  `top_level_safety_overrides_conflicts_with_smelt_yml_batched_sub_block` (its premise —
  validate-then-check-errors — is now unreachable; replaced with a direct parse-time
  refusal assertion).

## Decisions

- **`merge_key:` chosen over `write_key:`/`dedup_key:`/reusing `unique_key:`** — see
  the plan's rationale (grain-changing risk of reusing `unique_key:`, name clarity of
  `merge_key:`); no change from the plan.
- **Frontmatter catalogue gap found and fixed**: `crates/smelt-core/src/frontmatter.rs`'s
  `parse_frontmatter` unified key catalogue filters any key not explicitly listed
  *before* serde ever sees it — `merge_key` silently vanished (deserialized to `None`)
  until added to `CATALOGUE`. This is a real trap for any future frontmatter key
  addition; not previously documented anywhere obvious (found via a debug test, not
  the plan).
- **`09-check.sh`'s paragraph-pairing check scoped to `batched.unique_key` specifically**,
  not the generic `` `batched:` `` mention — several existing paragraphs
  (`materialized_view.md`, `diagnostics.md`, `docs-site/guide/materializations.md`)
  correctly reference the whole-block retirement without needing to re-name
  `merge_key` every time (that framing is about `safety_overrides`/general refusal,
  not the identity-adjacent `unique_key` sub-key this phase's replacement decision
  concerns).

## For the next planner

- Row 10 (docs-site terminology sync, whole-file `§"…"` citation sweep,
  validate + timeless greps) is unaffected by anything discovered here — no new
  ownerless work surfaced.
- `06-check.sh`'s two pre-existing `gap_claims` failures (IP-02, MP-33), already
  assigned to row 10 by the phase-9 planning decision log entry, remain unfixed —
  confirmed pre-existing, not touched by this phase.
- Doc comments in `crates/smelt-maintenance-testkit/src/{render.rs,dag.rs,recipe.rs}`
  still use `batched.unique_key` as informal shorthand for "the declared write key"
  in several places beyond the one (`recipe.rs:880-889`) retargeted here — internal,
  non-normative, not cited by any spec or the paragraph-pairing gate (which only
  scans `docs/`), and not required by this outcome's criteria. Left alone; low-risk
  polish for whoever next touches that file.
- No new criterion-serving gaps found. `PartitionGrainConfig::nondeterministic_columns_retired`
  was NOT deleted (plan task 7's conditional) — an `rg` sweep shows it is still the
  live deserialization path for both `.sql` frontmatter and `smelt.yml` `batched:`
  sub-blocks (via `reject_batched_subblock`'s generic-fixit branch naming it), so the
  condition for deletion ("no remaining user-YAML deserialization path") was not met.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/09-check.sh` — ALL PASS.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,7,8}-check.sh` — ALL PASS.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test example_diagnostics --test incremental --features smelt-cli/duckdb` — 119+48 passed (1 ignored, pre-existing).
- `cargo test -p smelt-lsp --test example_workspaces` — 34 passed.
- `cargo test -p smelt-cli --test bakeoff --test bakeoff_seam --test maintenance_pins --features smelt-cli/duckdb` — 12 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features smelt-cli/duckdb` — 70 passed (covers the `merge_key:`-generating recipe fixture).
