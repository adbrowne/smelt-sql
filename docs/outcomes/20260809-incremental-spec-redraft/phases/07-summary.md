# Phase 7 summary — retire `nondeterministic_columns`

## Shipped

- `PartitionGrainConfig::nondeterministic_columns` (`crates/smelt-core/src/config.rs`) renamed to
  a retirement sentinel `nondeterministic_columns_retired: ()` with an always-erroring
  `deserialize_with`: any YAML presence of `nondeterministic_columns` (SQL frontmatter *or*
  `smelt.yml`'s `models.<name>.batched:` block) is a hard parse error naming
  `columns.<c>.contract: plausible` per declared column, with no `smelt.yml` spelling for the
  replacement.
- New `MetadataError::PlausibleContractOnSkeletonColumn` / `DiagnosticCode::PlausibleContractOnSkeletonColumn`
  (`smelt-core/src/metadata.rs`, `smelt-db/src/diagnostics_types.rs`, `smelt-db/src/lib.rs`,
  `smelt-lsp/src/backend.rs`): the ported skeleton-position bar — a `columns.<c>.contract:
  plausible` declaration naming the `event_time_column`, `partition_column`, or a `unique_key`
  member is refused, replacing the old list-form check that scanned `batched.nondeterministic_columns`.
- `check_nondeterminism` (`smelt-logical/src/rules/incremental.rs`) now reads a new
  `ModelInfo.plausible_columns: BTreeSet<String>` field instead of
  `inc_config.nondeterministic_columns`; threaded through every production `ModelInfo`/`RuleContext`
  construction site (`smelt-runtime::safety::build_model_graph`, `smelt-db`'s LSP diagnostic path,
  `smelt-cli`'s `explain` command) from the model's declared `columns.<c>.contract == Plausible`
  set. Presentation-only sites (`analyze_batch_safety` callers in `smelt-ui`/`smelt-cli::explain`)
  default it empty since that analysis never reads it.
- `batched_subblock_fixit_message` restructured to extract `nondeterministic_columns`'s raw
  column list directly from the YAML mapping before struct-deserializing the rest, since the
  retirement sentinel makes whole-struct deserialize fail whenever the key is present — the
  per-key fix-it (`columns.foo.contract: plausible`) still carries the caller's own values.
- Spec edits: `docs/specs/models.md` (§"Constraint violations", "Batched sub-block retirement",
  Known Divergences), `docs/specs/model_properties.md` (probe registry row swapped for
  `columns.<c>.contract: plausible`; Known Divergences gap bullet deleted — migration is complete),
  `docs/specs/incremental_models.md` (dangling "two spellings coexist" gap bullet deleted),
  `docs/specs/diagnostics.md` (new code registered; `smelt.yml` `batched.nondeterministic_columns`
  refusal documented).
- `examples/incremental_nondeterministic_columns` fixture comment reworded; e2e/unit test doc
  comments across `smelt-logical`, `smelt-cli`, `smelt-core` reworded to name the surviving
  surface instead of the retired one.

## Decisions

- Kept the renamed sentinel field's Rust *type* the same shape pattern as before (unit `()`
  rather than deleting the field outright) so the retirement fires with a precise per-column
  fix-it rather than a generic "unknown field" error — matches the plan's explicit ask to reuse
  `batched_subblock_fixit_message`'s phrasing style.
- `ModelInfo` gained `#[derive(Default)]` to make the ~40 test-literal-construction sites across
  the workspace cheap to update (`..Default::default()` where a full field list wasn't already
  present) without touching unrelated test semantics.
- Left `crates/smelt-planner/src/python_bridge.rs`'s `python`-feature-gated test alone — it was
  already broken independent of this phase (constructs `PartitionGrainConfig { enabled: true, ... }`,
  a field that has never existed on the real type). The `python` feature is off by default and not
  part of any standing gate, so fixing it is out of scope here; flagged for the next planner.

## For the next planner

- The `python_bridge.rs` pre-existing breakage above (unrelated `enabled` field) should get its
  own follow-up if the `python` feature is ever exercised in CI.
- Row 8 (`grain: key_per_partition` + dead `IncrementalStrategy` variants) and row 9 (`batched:`
  block's remaining `unique_key`/`safety_overrides` smelt.yml keys) are unaffected by this phase's
  edits — `docs/specs/models.md`'s retirement paragraph now explicitly states the `smelt.yml`
  `batched:` block still carries those two keys, ready for row 9 to retire them without further
  disentangling `nondeterministic_columns`.
- Row 10's whole-file `§"…"` citation sweep should double-check the `model_properties.md`
  §"Probe obligation" registry row I retitled (`columns.<c>.contract: plausible` replacing
  `nondeterministic_columns (superseded)`) doesn't dangle any citation elsewhere in the corpus —
  I only verified the standing `probe_obligation.rs` gate and this phase's own `07-check.sh`, not
  a full-corpus `rg` sweep (that's row 10's job).

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/07-check.sh` → all green.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/02-check.sh` … `06-check.sh` → all
  still green.
- `bash .claude/scripts/verify-phase.sh` → ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-lsp --test example_workspaces` → 34 passed.
- `cargo test -p smelt-cli --test example_diagnostics` → 119 passed, 1 ignored (pre-existing).
