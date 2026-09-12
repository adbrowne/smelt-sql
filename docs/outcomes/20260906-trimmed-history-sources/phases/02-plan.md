# Phase 02 plan — the rolling-retention declaration: parse, validate, refuse

## Objective

Make `retention:` a well-formed-or-refused declaration rather than a field that
deserialises opportunistically. Advances success criterion 2: every malformed form of the
rolling bound refuses with a named `DiagnosticCode` (`MalformedSource`) whose message names
`retention:` and the offending value, each backed by an `examples/broken/` fixture. Also
clears the ground for criterion 3 — the walk may assume a parsed retention is non-zero and
clocked, so it never has to defend against an inert bound.

## Spec delta (made first, by the implement step)

`docs/specs/sources.md`:

- **`retention` row** (§"Source YAML shape" key table) — append the well-formedness rule:
  the value must be a parseable non-zero interval, and the source must declare
  `timeseries:`; a bound with no clock is inert (nothing to compare a reach against) and is
  refused rather than accepted and ignored.
- **§Semantics 5 "Retention refusal"** — one sentence in prose for the same three rules,
  framed as the fail-loud discipline: an unclocked or zero bound would silently license
  every replay it was written to forbid.
- **§"Diagnostic codes" `MalformedSource` row** — replace the bare "malformed `watermark`/
  `retention`" clause with the three named retention forms (unparseable interval, zero
  interval, no `timeseries:`).

No change to `incremental_models.md` — the quantifier is settled (phase 1).

## Tests (red → green)

`crates/smelt-core/tests/source_world_facts.rs`:

1. `retention_unparseable_interval_is_malformed` — `retention: 'banana'` returns a
   `SourceError` whose message contains `retention` and `banana` (today it fails as an
   opaque `YamlParse` naming the retired key `data_latency`).
2. `retention_zero_is_malformed` — `retention: '0 days'` refuses.
3. `retention_without_timeseries_is_malformed` — a clocked-fact-free source declaring
   `retention: '45 days'` refuses, message naming `timeseries:`.
4. `retention_with_timeseries_parses` — control: the `examples/github_activity` shape still
   parses to `DataLatency { 45 days }` (extend the existing parse test if one covers it).

`crates/smelt-cli/tests/example_diagnostics/` (new module `retention_diagnostics.rs`,
mirroring `external_step_diagnostics.rs`'s `assert_exactly_one` helper):

5. `broken_workspace_retention_fixtures` — each of the three new
   `examples/broken/models/sources/retention_*.yml` fixtures produces exactly one
   diagnostic, code `MalformedSource`, and no other `examples/broken` file regresses.

## Tasks

1. Write the spec delta above.
2. Add tests 1-4 red against the current parser.
3. In `crates/smelt-core/src/sources.rs`, change `RawSourceYaml::retention` to
   `Option<String>` so a bad interval is caught by smelt, not by serde, and parse it in
   `parse_source_yaml` via `DataLatency::parse`.
4. Add `SourceError::MalformedRetention { value: String, reason: &'static str }` (or three
   variants if that reads better) covering unparseable / zero / no-`timeseries:`; ensure it
   lands on `DiagnosticCode::MalformedSource` through `smelt-db`'s existing `_ =>` mapping
   in `queries/project.rs` (verify, do not assume).
5. Add the three `examples/broken/models/sources/retention_{bad_interval,zero,unclocked}.yml`
   fixtures, each minimal and each carrying a one-line `description:` of what is wrong.
6. Add test 5 and register the new module in `example_diagnostics/main.rs`.
7. Confirm no well-formed example regresses: `examples/timeseries` (`400 days`) and
   `examples/github_activity` (`45 days`) both declare `timeseries:` and stay green.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test source_world_facts --quiet`
- `cargo test -p smelt-cli --test example_diagnostics --quiet`
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet`
- Ratchets unmoved (no new production `unwrap`/`expect`).

## Commit message

`feat(sources): refuse a malformed rolling retention bound with MalformedSource`
