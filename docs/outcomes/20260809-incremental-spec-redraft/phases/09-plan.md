# Phase 9 plan — retire the `smelt.yml` `models.<name>.batched:` sub-block

## Objective

Close the last `batched.*` config fossil (criterion 2): `smelt.yml`'s
`models.<name>.batched:` sub-block stops parsing, with a per-key fail-loud fix-it in the same
shape phase 7 used. Its `safety_overrides` key already has a top-level smelt.yml spelling; its
`unique_key` key does not, so this phase lands the named top-level replacement the phase-1
outline left for it: **`merge_key:`**, the MERGE-dedup/write key, declarable in `.sql`
frontmatter and as a `smelt.yml` model override, explicitly non-identity-conferring.

## The replacement decision (fixes the open item in `phases/01-outline.md`'s deletion list)

`batched.unique_key` is **not** the same fact as top-level `unique_key:`. Top-level `unique_key:`
is the identity fact: it feeds `derive_grain` and moves a clocked model into the composed
key+time shape, which a row-shaped body with no `GROUP BY` cannot occupy. `batched.unique_key` is
only the row-dedup key `Technique::ColumnScopedMerge` writes on
(`crates/smelt-runtime/src/diagnostics.rs:922-934`, `examples/timeseries/smelt.yml:46-64`).
Mapping it onto `unique_key:` would change the derived grain of the two `examples/timeseries`
models that use it, so the retirement needs its own spelling. Name chosen: `merge_key:` — it
names the technique it addresses, and does not collide with the planned per-cell
`maintenance.cells[].write` addressing pins. Rejected: `write_key:` (reads as the per-cell write
pin), `dedup_key:` (says nothing about where the fact is consumed), reusing `unique_key:`
(grain-changing, above).

## Spec delta (first — spec-first rule)

- `docs/specs/models.md` §"The Relation Contract" — add `merge_key:` as a declared key
  (frontmatter + `smelt.yml` model override, frontmatter wins wholesale, single-string sugar),
  stated as the write/dedup key that never confers identity and never changes the derived grain.
- `docs/specs/models.md` §"Constraint violations" — replace the `smelt.yml`
  `models.<name>.batched.nondeterministic_columns` row (`:245`) with a row covering the whole
  `smelt.yml` `batched:` sub-block: hard error naming each declared sub-key's replacement
  (`unique_key` → top-level `merge_key:`, `safety_overrides` → top-level `safety_overrides:`,
  `nondeterministic_columns` → `columns.<c>.contract: plausible`). Rewrite `:250`'s paragraph
  accordingly, and retarget the `.sql`-frontmatter `batched.unique_key` fix-it (`:244`, `:250`)
  from top-level `unique_key:` to `merge_key:` — the current text prescribes the grain-changing
  mapping this phase's analysis rejects.
- `docs/specs/models.md` §Known Divergences (`:343`) — delete the "separate parsing path" clause;
  the two surfaces now agree.
- `docs/specs/incremental_models.md` — the `batched.*` mentions (notably the §Known Divergences
  entry near `:2687` and any §Surface row) restated against `merge_key:`.
- `docs/specs/diagnostics.md` — register the smelt.yml `batched:` retirement refusal next to the
  existing `nondeterministic_columns` one; note it is a `serde` parse error, not a
  `DiagnosticCode`, if that is what the implementation lands.
- `docs-site/docs/reference/smelt-yml.md` — `batched:` rows replaced by `merge_key:`;
  `docs-site/docs/guide/sql-models.md` swept for the same.

## Tests (red-green)

- `smelt-core::config::smelt_yml_batched_block_is_retired` — `models.x.batched: {unique_key: [a]}`
  fails to parse with a message naming `merge_key: [a]`.
- `smelt-core::config::smelt_yml_batched_block_names_every_declared_key` —
  `{safety_overrides: {...}, nondeterministic_columns: [c]}` names both replacements
  (`safety_overrides:` top-level, `columns.c.contract: plausible`), and an empty `batched: {}`
  still errors with the generic three-line fix-it.
- `smelt-core::config::merge_key_folds_into_incremental_config` — smelt.yml `merge_key: [event_id]`
  on a `refresh: incremental` + `grain: partition` model surfaces as
  `get_incremental(...).unique_key`; single-string sugar accepted.
- `smelt-core::config::merge_key_frontmatter_wins_over_smelt_yml` — via
  `get_incremental_with_metadata`.
- `smelt-core::config::merge_key_does_not_confer_identity` — a model declaring only `merge_key:`
  derives `Grain::Partition` (not the composed shape) and `get_unique_key_with_metadata` stays
  empty.
- `smelt-core::metadata::merge_key_parses_in_frontmatter` — frontmatter `merge_key:` folds into
  the internal `batched.unique_key` representation; a frontmatter `batched: {unique_key: [a]}`
  fix-it now names `merge_key: [a]`.
- `smelt-core::config::ephemeral_model_with_merge_key_is_refused` — the ephemeral/view
  "cannot have incremental configuration" checks that read `ModelConfig::batched` presence keep
  firing off the new signal.
- Standing: `cargo test -p smelt-cli --test example_diagnostics`,
  `cargo test -p smelt-cli --test incremental --features smelt-cli/duckdb` (the timeseries
  MERGE-dedup path must be unchanged), `cargo test -p smelt-lsp --test example_workspaces`.

## Tasks

1. Land the spec delta above (specs first, then docs-site).
2. `crates/smelt-core/src/config.rs`: replace `ModelConfig::batched` with a renamed always-erroring
   sentinel (`batched_retired: ()`, `rename = "batched"`, `skip_serializing`) whose
   `deserialize_with` reads the raw `serde_yaml::Value`, enumerates its keys, and builds the
   per-key fix-it (extend the phase-7 `reject_nondeterministic_columns` pattern).
3. Add `ModelConfig::merge_key: Option<Vec<String>>` (`opt_string_or_vec`); rename
   `fold_smelt_yml_safety_overrides` → `fold_smelt_yml_incremental_keys` and fold `merge_key` into
   the returned `PartitionGrainConfig::unique_key` alongside the existing safety-override fold.
4. `crates/smelt-core/src/metadata.rs`: add `ModelMetadata::merge_key` folded into
   `ModelMetadata::batched`'s `unique_key` (mirror `fold_top_level_safety_overrides`); retarget
   `batched_subblock_fixit_message`'s `unique_key` line to `merge_key:`.
5. `Config::get_incremental_with_metadata`: frontmatter `merge_key` wins wholesale over the
   smelt.yml one, matching the existing `batched:`-block precedence.
6. Repair `validate_model_configs`: the smelt.yml-side dual-declaration check for
   `batched.safety_overrides` is unreachable and goes; the ephemeral/view checks that keyed off
   `ModelConfig::batched` presence switch to "declares `merge_key:` or `safety_overrides:`".
7. Delete `PartitionGrainConfig::nondeterministic_columns_retired` **only if** `rg` shows no
   remaining user-YAML deserialization path into `PartitionGrainConfig`; if it is deleted, update
   `phases/07-check.sh`'s "retirement sentinel wired" check to point at the frontmatter-side
   refusal instead, with a one-line rationale comment.
8. `examples/timeseries/smelt.yml`: both `batched: {unique_key: [event_id]}` blocks →
   `merge_key: [event_id]`; rewrite the two explanatory comments against the new spelling.
9. Write `phases/09-check.sh`: no live `batched` field on `ModelConfig`; retirement sentinel wired;
   every spec/docs-site `batched`-as-smelt.yml-key mention paired with the `merge_key:`
   replacement; `merge_key` documented in `smelt-yml.md`; the timeless `Phase [A-Z0-9]` grep.
10. Write `phases/09-summary.md` (shipped / decisions / for the next planner / gates).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/09-check.sh`
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,7,8}-check.sh` (06 has two
  known pre-existing `gap_claims` failures — IP-02, MP-33 — owned by row 10)
- `bash .claude/scripts/verify-phase.sh` (must be ALL GREEN)
- `cargo test -p smelt-cli --test example_diagnostics --test incremental` and
  `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`refactor(config)!: retire the smelt.yml batched: sub-block in favour of top-level merge_key:`
