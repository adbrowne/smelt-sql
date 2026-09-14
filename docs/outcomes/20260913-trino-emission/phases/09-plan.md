# Phase 9 plan — The Trino type-oracle leg

## Objective

Answer criterion 11 by **landing** the leg, not deferring it: wire `TrinoOracle` (already a
`TypeOracle`, built in phases 5–6) into `type_property_tests.rs` alongside DuckDB/Spark/BigQuery,
route the `"trino"` backend key through `divergences.rs`, and register every divergence and
`Unknown` a live sweep actually surfaces. The deferral branch is not taken — the transport,
the refusal classifier and the Arrow map all exist, so a deferral here would be silence dressed
as a decision. One hole found while reading: an unmappable Trino type signature currently reaches
`classify_oracle_error` wearing the `Execution failed for 'trino':` prefix and is skipped as a
query refusal, i.e. smelt's own mapping gap silently counts as "the engine rejected this SQL".
That is the "unverified = passing" equivalence this outcome exists to deny, and phase 9 closes it.

## Spec delta (made first)

- `docs/specs/multi_backend.md` §"Output-schema type conformance" — append one paragraph: the
  type-property oracle (`cargo test -p smelt-db --test type_property_tests`) compares smelt's
  inferred type against **four** live oracles, Trino included via `/v1/statement` schema
  metadata; a tolerated difference is a registered `divergences.rs` entry keyed per backend and
  an `Unknown` is a registered `known_unknowns.rs` entry, the string-family rule being the only
  blanket leniency. State that a Trino column type smelt's Arrow map cannot decode is **fatal**
  to the leg, never a skipped case — the engine accepted the query, so the failure is smelt's.
- `docs/specs/multi_backend.md` §"CI tiering" table — add a row for the type-property Trino leg
  on the same tier as its other live Trino legs (per-PR on Trino-relevant paths, else labeled
  PR + nightly), and state the env gate: `SMELT_TRINO_URL` unset ⇒ the leg is absent (as Spark's
  and BigQuery's are), which is distinct from a reachable tier whose probes are skipped.
- `docs/specs/multi_backend.md` §References — `crates/smelt-oracle-testkit/src/trino_oracle.rs`
  and `type_property_tests.rs` (four oracles, not "Spark oracle").

## Tests (red-green, in order)

1. `prop_helpers::divergences::tests::finds_trino_divergence` — `find_divergence(smelt, actual,
   "trino", …)` resolves an entry whose `trino_type` matches. **Red today**: `find_divergence`'s
   `match backend` has no `"trino"` arm, so every Trino mismatch is unregisterable.
2. `prop_helpers::divergences::tests::unknown_backend_key_still_matches_nothing` — a bogus
   backend name resolves `None`; proves arm 1 didn't turn the match into a wildcard.
3. `smelt_oracle_testkit::error_class` case-table addition —
   `"trino oracle cannot map declared column type: …"` classifies `Fatal`, while the four
   existing `TRINO_REFUSALS` prefixes stay `QueryRefusal`.
4. `smelt_oracle_testkit::trino_oracle::tests::unmappable_declared_type_is_not_a_refusal` —
   `TypeOracle::query_types`' error for a signature `trino_type_to_arrow` rejects carries the
   new non-refusal prefix rather than `BackendError`'s `execution_failed` spelling.
5. `type_property_tests::trino_coverage_floor_tests::{below_floor_is_rejected,
   at_or_above_floor_is_accepted}` — pure unit tests over `check_trino_coverage_floor`, mirroring
   the BigQuery pair, so "the leg ran but compared nothing" fails.
6. `type_property_tests::prop_type_inference` (live) — with the tier up, the sweep runs the Trino
   leg, accumulates `TRINO_COLUMNS_COMPARED`, and asserts the floor once after the sweep.
7. The deterministic smoke tests that already fan out to Spark (join / multi-model / outer-CTE
   shapes, the six `if let Some(spark)` sites) each gain the matching Trino arm.

## Tasks

1. Write the spec delta above.
2. `divergences.rs`: add `pub trino_type: Option<DataType>` to `TypeDivergence`, fill `None` on
   all 27 entries (fill a real value only where a live probe measures one — never guessed, per
   the `bigquery_type` doc comment, which this field's doc mirrors), add the `"trino"` arm to
   `find_divergence`, and land tests 1–2.
3. `trino_oracle.rs`: in `TypeOracle::query_types`, map a `trino_type_to_arrow` failure to a new
   distinct message (`trino oracle cannot map declared column type: <sig>`) instead of passing
   `BackendError`'s refusal-shaped `Display` through; land test 4. Leave `ValueOracle` untouched.
4. `error_class.rs`: document + assert that the new prefix is **not** in `TRINO_REFUSALS`
   (test 3). No allow-list entry is added — `Fatal` is the default and that is the point.
5. `type_property_tests.rs`: add the `TRINO` `LazyLock<Option<TrinoOracle>>` (from
   `TrinoOracle::from_env`), `TRINO_COLUMNS_COMPARED`, `TRINO_COLUMN_COVERAGE_FLOOR` +
   `check_trino_coverage_floor` with its unit tests, the sweep arm, the post-sweep floor check
   in `prop_type_inference`, and the Trino arms in the deterministic smoke tests. Update the
   module doc's "DuckDB (always) and Spark (…)" sentence to name all four.
6. Bring the tier up (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`) and run the
   live sweep. **Calibrate the floor from the measured run**, as BigQuery's was — set it well
   under the observed count and record both numbers in the doc comment.
7. Triage every failure the live sweep reports: an inference bug is fixed; a genuine engine
   difference becomes a `trino_type` divergence entry with `status` and description; an
   `Unknown` becomes a `known_unknowns.rs` entry (markers keyed on the generating expression).
   Record the full list in the summary — a registered divergence is a finding, not a pass.
8. If the coordinator cannot be brought up, do **not** state types offline and do not weaken the
   leg: emit `<<PHASE_BLOCKED>>`. This phase has no offline fallback (phases 5–7's discipline).

## Verification

- `bash scripts/trino-up.sh && source scripts/trino-env.sh` then, live:
  `cargo test -p smelt-db --test type_property_tests --quiet 2>&1 | tail -40`
  (and once at `PROPTEST_CASES=512` to calibrate the floor; record the `COVERAGE[trino]` line).
- `cargo test -p smelt-oracle-testkit --quiet`, `cargo test -p smelt-db --test dialect_audit`
  (live, unchanged — proves the oracle change didn't disturb the schema/value legs).
- Tier **unexported**: `cargo test -p smelt-db --test type_property_tests --quiet` still green
  with the Trino leg simply absent, then `bash .claude/scripts/verify-phase.sh`
  (phases 3–8's pattern — the shared Iceberg catalog collides under full-workspace scheduling).
- No ratchet lowered: `dialect-gaps-baseline.txt`, `registry-migration-baseline.txt`,
  `parser-gaps-baseline.txt`, `hardening-baseline.txt` unchanged.

## Commit message

`test(types): add the live Trino leg to the type-property oracle with its divergence registry`
