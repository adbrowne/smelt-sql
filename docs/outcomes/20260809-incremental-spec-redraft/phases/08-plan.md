# Phase 8 plan — retire declared `grain: key_per_partition` and the dead `IncrementalStrategy` variants

## Objective

Close the two remaining fossils in criterion 2: `grain: key_per_partition` leaves the **declared**
surface (fail-loud on any frontmatter / `smelt.yml` spelling, naming the two facts that derive it
and `grain: key` as the closest supported shape), and `IncrementalStrategy::{Append,
InsertOverwrite}` — unreachable dead variants, since `resolve_strategy` always returns
`DeleteInsert` — are deleted along with their dispatch arms and their three spec restatements.
Advances criteria 2 and 3 (the `InsertOverwrite` dead-code Known-Divergence bullet disappears).

**Scope boundary.** `Grain::KeyPerPartition` survives as a *derived* label: `derive_grain` still
produces it for clock + identity with `partition_column ∈ unique_key`, `smelt explain` still names
it, and `MaintenanceUnsupportedGrain` still refuses it at plan derivation. Only the *declaration*
is retired. Implementing a KPP plan is new behaviour (§Out of scope). Do not touch the `batched:`
sub-block's `unique_key`/`safety_overrides` (row 9) or run whole-file citation sweeps (row 10). The
backend trait methods `insert_into_from_query` / `insert_overwrite` stay — they are the capability
that would admit those strategies later, and Spark/testkit implement them.

## Spec delta (spec-first — make these edits before the code)

1. `docs/specs/incremental_models.md`
   - `:1231-1235` — the strategy-resolution paragraph: `DeleteInsert` is the only strategy; drop
     `Append`/`InsertOverwrite` from the enum listing and drop the "`Append` is unreachable until
     gated on …" clause, replacing it with one sentence naming the backend capability
     (`insert_into_from_query` / `insert_overwrite`) as the future admission point.
   - `:1761` and `:1908` — the two restatements of the same `Append`-unreachable clause: delete.
   - `:2105` — Known Divergences bullet on the production-unreachable `InsertOverwrite` dead code:
     delete (the code is gone).
   - `:2148` — the `grain: key_per_partition` divergence bullet: rewrite as the *derived*-label gap
     (declaring it is refused; the derived label refuses at plan derivation), keeping the tracking
     link.
   - `:136`, `:143`, `:53`, `:285` — the check-only-assertion surface line must stop offering
     `key_per_partition` as a writable value; the derived-label prose stays.
2. `docs/specs/models.md` — `:75` (`grain` frontmatter row), `:129`/`:131` (derivation table + the
   check-only paragraph), `:133`, and the `:343` Known-Divergences sentence: the writable assertion
   set becomes `partition | key`; `key_per_partition` is derived-only and its declaration is a hard
   error. Follow the wording pattern phase 7 used for the retired sub-block.
3. `docs/specs/diagnostics.md` — document the declaration refusal in prose next to the
   `MaintenanceUnsupportedGrain` row (`:503`), the way the `batched.nondeterministic_columns`
   refusal was documented; no new `DiagnosticCode` (this is a config-parse error, as in phase 7).

## Tests (red → green)

- `smelt-core::config` `declared_grain_key_per_partition_is_retired` — `serde_yaml`
  `grain: key_per_partition` errors, message names both facts (`timeseries:` clock,
  `partition_column ∈ unique_key`) and `grain: key`.
- `smelt-core::config` `declared_grain_partition_and_key_still_parse` — the two surviving values
  round-trip; `Serialize`/`Display` for `KeyPerPartition` are unchanged (derived-label output).
- `smelt-core::tests::refresh_axis` — convert the existing agreeing-assertion test (`:760-784`) to
  assert `derive_grain` still yields `KeyPerPartition` from the facts with **no** `grain:` written.
- `smelt-core::tests::source_world_facts` (`:512`) — same conversion if it declares rather than
  derives.
- `smelt-cli::example_diagnostics::timeseries_broken_key_per_partition_emits_unsupported_grain`
  and `smelt-cli::explain_model::key_per_partition_shows_unsupported_grain_refusal_not_keyed_cells`
  — unchanged assertions, but the fixture now *derives* KPP (see task 4); they must stay green,
  proving the derived path is untouched.
- `smelt-core::config` `incremental_strategy_append_and_insert_overwrite_are_gone` — extend the
  existing `merge`-rejection test: `"append"` and `"insert_overwrite"` no longer deserialize.
- `smelt-cli::incremental::strategies` — the four `Append`/`InsertOverwrite` dispatch tests are
  deleted; replace with one direct-call test per surviving trait method
  (`insert_into_from_query`, `insert_overwrite`) so the backend capability keeps DuckDB coverage.

## Tasks

1. Spec edits above (all three files) first.
2. `Grain::deserialize` (`crates/smelt-core/src/config.rs:136-152`) — reject `key_per_partition`
   with the retirement message; the `Invalid grain:` fallback lists only `partition`/`key`. Leave
   the variant, `Serialize`, `Display`, and `derive_grain` intact.
3. Sweep declared-grain sites the rejection now breaks: `crates/smelt-core/tests/refresh_axis.rs`,
   `crates/smelt-core/tests/source_world_facts.rs`, any `examples/**` frontmatter (`rg -n
   'grain: key_per_partition' examples`).
4. `examples/timeseries_broken_key_per_partition/models/trajectory.sql` — drop the `grain:` line,
   add top-level `unique_key: [device_id, event_date]` so `partition_column ∈ key` derives
   `KeyPerPartition`; reword the comment to name the derived label, not the declaration.
5. Delete `IncrementalStrategy::{Append, InsertOverwrite}` (`config.rs:668-672`); collapse the
   dispatch `match` in `crates/smelt-backend/src/lib.rs:263-273` to the single `DeleteInsert` arm
   and `strategy_label` in `crates/smelt-cli/src/helpers.rs:21-26`.
6. Update the doc comments that name the removed variants:
   `crates/smelt-runtime/tests/statement_parity.rs:3938-3943`,
   `crates/smelt-cli/tests/incremental/main.rs`, `smelt-backend/src/lib.rs::resolve_strategy`.
7. docs-site behaviour-accuracy edits (terminology sweep stays row 10):
   `reference/smelt-yml.md:160,172`, `guide/sql-models.md:164`,
   `guide/materializations.md:113`, `reference/timeseries.md:86,93` — the writable `grain:` set is
   `partition | key`; `key_per_partition` is a derived label with no execution path.
   `reference/state.md:16,61` and `reference/cli.md:318` mention the *derived* label only — leave.
8. Write `phases/08-check.sh` (pattern: `07-check.sh`): no writable `key_per_partition` in spec or
   docs-site surface tables; the retirement message present in `config.rs`; zero `Append`/
   `InsertOverwrite` `IncrementalStrategy` references in `crates/` outside historical `docs/plans`;
   the `:2105` KD bullet gone; timeless grep clean over the touched files.
9. Write `phases/08-summary.md`.

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/08-check.sh`
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,6,7}-check.sh` (still green)
- `bash .claude/scripts/verify-phase.sh` (full gate)
- `cargo test -p smelt-cli --test example_diagnostics` and
  `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`refactor(config)!: retire declared grain: key_per_partition and the dead Append/InsertOverwrite strategies`
