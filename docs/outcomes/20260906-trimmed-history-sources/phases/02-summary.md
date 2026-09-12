# Phase 02 summary — the rolling-retention declaration: parse, validate, refuse

**Shipped:**
- `docs/specs/sources.md`: `retention` row now states the well-formedness rule (parseable
  non-zero interval, source must declare `timeseries:`); §Semantics 5 "Retention refusal"
  states the same rule in prose, framed as fail-loud discipline; `MalformedSource` row now
  names the three retention forms explicitly.
- `crates/smelt-core/src/sources.rs`: `RawSourceYaml::retention` is now `Option<String>`
  (was `Option<DataLatency>`) so `parse_source_yaml` — not serde's opaque `DataLatency`
  deserializer — catches a bad interval and can name `retention:` and the offending value.
  Three new `SourceError` variants: `RetentionUnparseable`, `RetentionZero`,
  `RetentionWithoutTimeseries`. All three fall through the existing `_ => MalformedSource`
  catch-all in `smelt-db/src/queries/project.rs` — no mapping change needed.
- `crates/smelt-core/tests/source_world_facts.rs`: four new tests (unparseable, zero,
  no-`timeseries:`, and a control that the well-formed shape still parses); extended
  `watermark_and_retention_parse` to add a `timeseries:` block (see Decisions).
- `crates/smelt-cli/tests/example_diagnostics/retention_diagnostics.rs` (new module,
  registered in `main.rs`): three `examples/broken/models/sources/retention_{bad_interval,
  zero,unclocked}.yml` fixtures, each asserted to produce exactly one `MalformedSource`
  diagnostic and nothing else.

**Decisions:**
- `examples/timeseries/models/sources/raw/events.yml` declared `retention: '400 days'`
  with no `timeseries:` block (only `watermark:` + `unique_key:`) — exactly the inert shape
  this phase refuses. Removed the `retention:` line rather than adding a `timeseries:`
  block: the source is deliberately an unclocked lookup (watermark + unique_key only), and
  adding a clock to make an unused declaration parse would change the fixture's actual
  shape for no benefit. Grepped the workspace first to confirm nothing depends on this
  source's `retention` value (`examples/timeseries/models/sources/raw/events.yml` is
  otherwise the only trimmed-source-adjacent fixture using `raw.events` — separate `raw.events`
  tables built ad hoc in unrelated test files like `state_posture.rs` are unaffected).
- `crates/smelt-core/src/sources.rs` grew past its large-file-ratchet baseline
  (1297 → 1336 lines). Reviewer sign-off: the growth is three cohesive error variants plus
  ~20 lines of validation logic in the file that already single-owns source YAML parsing —
  splitting it for a 39-line addition would fragment a coherent parser for no gain. Ran
  `.claude/scripts/large-file-check.sh --update`.

**For the next planner:**
- Phase 3 (reach vs. retention in the walk) can now assume a parsed `SourceInfo.retention`
  is always non-zero and paired with `timeseries.is_some()` — no defensive check needed
  against an inert bound.
- Nothing else surfaced needing a new phase row.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test` workspace including the large-file ratchet after baseline update,
  `example_diagnostics`).
- `cargo test -p smelt-core --test source_world_facts --quiet` — 26 passed.
- `cargo test -p smelt-cli --test example_diagnostics --quiet` — 127 passed, 1 ignored.
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet` — 1 passed.
- `cargo test -p smelt-core --test hardening_budget --quiet` — 5 passed; ratchets unmoved
  (the one flagged `unwrap` regression is `smelt-hardening-probe`, the gate's own synthetic
  test fixture, not production code touched by this phase).
