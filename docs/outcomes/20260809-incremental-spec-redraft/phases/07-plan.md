# Phase 7 plan — retire `nondeterministic_columns`

## Objective

Delete the superseded `nondeterministic_columns` list form from the parser and the `smelt.yml`
surface, replacing it with the already-declared `columns.<c>.contract: plausible` per-column
equivalence contract as the *sole* surface, with a fail-loud fix-it for any caller still writing
the old key. Advances success criterion 2 (the accretion list is gone) and clears
`model_properties.md:395`'s gap bullet, which criterion 3 counts as a genuine gap only while the
parse path survives.

## Spec delta (spec-first — the implement step makes these edits before touching code)

- `docs/specs/models.md` — §"Constraint violations" table (`:244`) and the `batched:` retirement
  paragraph (`:249`), plus the §Known Divergences bullets at `:342` / `:344`: state that
  `nondeterministic_columns` no longer parses on *either* path; the `smelt.yml`
  `models.<name>.batched.nondeterministic_columns` key is a hard error whose fix-it names
  `columns.<c>.contract: plausible` in the model's `.sql` frontmatter (the contract has no
  `smelt.yml` spelling — say so). Delete the `:344` migration bullet: the migration is complete.
  `:342`'s `batched:`-still-alive sentence keeps its remaining two keys (row 9's work).
- `docs/specs/model_properties.md` — rows `:67` and `:100` lose their `nondeterministic_columns`
  `(superseded)` entries entirely; `:140` and `:310` restate the widening in terms of
  `columns.<c>.contract: plausible`. Delete the `:395` Known-Divergence gap bullet.
- `docs/specs/incremental_models.md` — §Surface catalogue entry and the `:2184` divergence
  mention drop the list form; add to §"Non-determinism and the payload rule" that a `plausible`
  contract on an event-time, partition or `unique_key` column is refused (the ported bar).
- `docs/specs/diagnostics.md` — update the `:139` fix-it wording, and register the new
  skeleton-position code (`PlausibleContractOnSkeletonColumn`).

## Tests (red-green)

1. `smelt-core::config::test_smelt_yml_batched_nondeterministic_columns_is_refused_with_fixit` —
   `models.x.batched.nondeterministic_columns: [inserted_at]` fails deserialization with a message
   naming `columns.inserted_at.contract: plausible` and the `.sql` frontmatter location.
2. `smelt-core::metadata::test_plausible_contract_on_event_time_column_is_error` (+ `_partition_`,
   + `_unique_key_`) — the skeleton-position bar ported from the deleted list-form validation
   (`metadata.rs:1064-1092`); each names the offending column and its role.
3. `smelt-core::metadata::test_plausible_contract_on_payload_column_validates` — green case.
4. `smelt-logical::rules::incremental::nondeterministic_admitted_via_column_contract_plausible` —
   `RANDOM() AS batch_id` is admitted when the model declares `columns.batch_id.contract:
   plausible` (replaces `model_with_nondeterministic_columns`'s admission cases).
5. `smelt-logical::rules::incremental::nondeterministic_rejection_names_column_contract` — the
   rejection message names `columns.<c>.contract: plausible`, never `batched.*`.
6. `smelt-logical::rules::incremental::run_clock_pinning_still_admitted_without_declaration` —
   the `NOW()` path is unchanged (guards against regressing the existing example workspace).
7. `smelt-cli --test example_diagnostics` — `examples/incremental_nondeterministic_columns` stays
   diagnostic-clean (it already declares `contract: plausible`; its comment header needs rewording).
8. `phases/07-check.sh` — red at HEAD, green after: no production `nondeterministic_columns`
   identifier outside the retirement diagnostic; no `nondeterministic_columns` in `docs/specs/` or
   `docs-site/` except retirement/fix-it text; timeless `Phase [A-Z0-9]` grep clean in the edited
   spec ranges.

## Tasks

1. Write `phases/07-check.sh` and confirm every check is red at HEAD.
2. Make the spec-delta edits above.
3. Replace `PartitionGrainConfig::nondeterministic_columns` (`smelt-core/src/config.rs:773`) with a
   retirement sentinel: a renamed, `skip_serializing`, always-erroring `deserialize_with` field so
   no consumer can read it and presence produces the fix-it message. Reuse the phrasing style of
   `batched_subblock_fixit_message` (`metadata.rs:743`).
4. Port the skeleton-position bar: delete `metadata.rs:1064-1092`'s list-form rule and add the
   equivalent validation over `columns.<c>.contract == Plausible` against `event_time_column`,
   `partition_column` and the effective `unique_key`; register the diagnostic code.
5. Rewire `check_nondeterminism` (`smelt-logical/src/rules/incremental.rs:~880-910`) to read the
   model's declared `plausible` column set instead of `inc_config.nondeterministic_columns`;
   update its doc comments (`:459`, `:654`, `:666`, `:711`) and both rejection messages.
6. Port the unit tests that build via `model_with_nondeterministic_columns` (`incremental.rs:1486`)
   to declare column contracts; delete the helper.
7. Sweep the remaining literal-`vec![]` construction sites and fixtures that name the field
   (`rule_diagnostics.rs`, `smelt-backend-duckdb`, `smelt-cli/src/explain.rs`, the `smelt-runtime`
   and `smelt-cli` test fixtures, `crates/smelt-cli/tests/e2e/incremental_nondeterministic_columns_e2e.rs`).
8. Update `docs-site/docs/reference/smelt-yml.md:248,255` and the
   `guide/incremental-models.md#non-deterministic-columns` anchor it links to.

## Do not cross

- The `batched:` block itself and its `unique_key` / `safety_overrides` keys — row 9.
- `grain: key_per_partition` and the dead `IncrementalStrategy` variants — row 8.
- Whole-file `§"…"` citation sweep — row 10. Fix only citations this phase's own edits dangle.
- No `columns:` block is added to `smelt.yml` (new behaviour, §Out of scope).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/07-check.sh` → all green.
- `phases/02-check.sh` … `06-check.sh` → still green.
- `bash .claude/scripts/verify-phase.sh` → ALL GREEN.
- `cargo test -p smelt-cli --test example_diagnostics` (covered by the bundled gate, called out
  because this phase edits an example workspace).

## Commit message

`refactor(config)!: retire nondeterministic_columns in favour of columns.<c>.contract: plausible`
