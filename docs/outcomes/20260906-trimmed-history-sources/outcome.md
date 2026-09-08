# Outcome: A source whose history is bounded and moving forward refuses or degrades, never silently under-reads

**Created:** 2026-09-06
**Status:** active
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"Trimmed-history sources", §"Trimmed history versus SCD2 lifetime", §Open questions 3 and 4
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/model_properties.md`; `docs/specs/state.md` §"The degradation contract"; `docs/specs/diagnostics.md`

## The outcome

A source can declare that its retained history is **bounded, with the bound moving
forward** as old partitions age out. smelt reasons about that bound the way it reasons
about any other model property: a model whose maintenance needs to read further back than
the source still retains is refused at analysis time with a named code, or degraded with a
recorded downgrade — never silently computed over the history that happens to survive.
Because the bound moves on its own, a model that was admissible last month can stop being
admissible with no change to the code; smelt says so when that happens instead of quietly
returning a smaller answer. The equivalence invariant's quantifier is settled explicitly
for such sources, so `full_refresh(inputs ∈ S)` has one meaning rather than two.

## Success criteria (checkable)

1. **The quantifier is decided.** `docs/specs/incremental_models.md` states whether
   `full_refresh(inputs ∈ S)` for a trimmed source is taken over *retained* history or
   over *all history that ever existed*, with the reasoning. This is the research doc's
   §Open questions 3 and it decides everything downstream — it is settled in the spec
   before any code.
2. **Declaration.** A source declares its retention bound, and the declaration says
   whether the bound is smelt-visible (declared) or merely observed (§Open questions 4 —
   settled in the decision log). Malformed forms refuse with a named `DiagnosticCode` and
   an `examples/broken/` fixture; `diagnostics_catalogue` green.
3. **Reach vs. retention, in the walk.** The comparison between a model's required
   look-back and its source's retained bound is produced by the composition walk in
   `crates/smelt-logical/src/analysis/walk.rs` — not by an ad hoc scan — per the property
   composition walk rule; `cargo test -p smelt-logical --test walk_coverage` green.
4. **Refuse or degrade, never silent.** A model whose reach exceeds the retained bound
   either refuses with a named code or takes a recorded downgrade through the existing
   degradation contract; a test covers each case, and a test asserts no path computes a
   smaller answer without one or the other.
5. **The bound moving is an event.** A source whose bound advances past what an admitted
   model needs is detected on a run and surfaced — the model's admission is re-evaluated
   against the current bound, not the bound at authoring time. A test moves the bound
   forward under a previously-admissible model and asserts the refusal or downgrade fires.
6. **Conformance.** `crates/smelt-maintenance-testkit` gains a trimmed-retention
   `SourceRecipe` whose bound advances between run steps, and
   `cargo test -p smelt-cli --test maintenance_conformance` exercises it against the
   oracle the criterion-1 decision defines. Seeded sample green.
7. **Explain and docs.** `smelt explain` renders the retained bound and the model's
   required reach against it (text and `--json`); a docs-site page documents the
   declaration, the refusal, and the degradation; `cli_docs_coverage` green.
8. **Gates green.** `bash .claude/scripts/verify-phase.sh`, `walk_coverage`,
   `statement_parity`, `execute_parity`, `maintenance_conformance`; ratchets unmoved.

## Out of scope

- The succession grain's interaction with retention (research doc tension 2 — a dimension
  outliving its source's retention). That is a genuine interaction and it is deliberately
  deferred: it cannot be designed before `20260906-scd2-keyed-succession` exists. This
  outcome must leave the quantifier decision (criterion 1) stated clearly enough for that
  work to build on, and nothing more.
- Enforcing or implementing retention — smelt reasons about a bound someone else applies.
- Backfilling history a source no longer retains.
- Any change to the contract lattice's declared points.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Settle and spec the equivalence-invariant quantifier for a trimmed source (retained history vs. all history), with reasoning — this decides the rest | done |
| 2 | The rolling-retention declaration: spec + `smelt-core` parse/validation of a moving bound, malformed forms refused with a named `DiagnosticCode` and an `examples/broken/` fixture | done |
| 3 | Reach vs. retention in the composition walk: `analysis/walk.rs` produces the required-look-back vs. retained-bound verdict, no ad hoc scan; `walk_coverage` green | planned |
| 4 | Refuse or degrade, never silent: wire the verdict to a named refusal or a recorded downgrade through the degradation contract, plus the no-silent-under-read test | pending |
| 5 | The bound moving is an event: admission re-evaluated against the current bound on every run, with a test that advances the bound under a previously-admissible model | pending |
| 6 | Conformance: a trimmed-retention `SourceRecipe` in `smelt-maintenance-testkit` whose bound advances between run steps, driven through `maintenance_conformance` against the phase-1 oracle | pending |
| 7 | Explain and docs: `smelt explain` renders bound vs. required reach (text and `--json`); docs-site page for the declaration, refusal and degradation; `cli_docs_coverage` green | pending |
| 8 | Close-out: verify every success criterion's evidence at HEAD, all gates green, ratchets unmoved | pending |

## Decision log

- 2026-09-09 (phase 3 planning): **the verdict is a fold over the walk's output, in a new
  module, not a new transfer function.** The required look-back the criterion names is
  already the composition walk's own product (`derive_model_bounds`'s `BoundResult`, walked
  via `QueryTree` in `analysis/source_bounds.rs` — `analysis/walk.rs` named in criterion 3
  is now the `analysis/walk/` module). So the comparison needs no second walk: it is a pure
  map over that verdict against `BoundContext`'s declared retention, and it inherits series
  composition (stacked frames *add*) for free — a test pins exactly that, since a whole-text
  max-merge would under-derive the reach and wrongly admit. It lands in a new
  `analysis/retention_reach.rs` rather than inside `source_bounds.rs`, which sits at its
  3367-line large-file ratchet baseline. The verdict also carries a `window_age` term
  (`required_lookback = before + window_age`) so the rolling re-evaluation phase 5 needs is
  built in from the start rather than retrofitted. Phase table unchanged — the phase-2
  summary surfaced no work needing a new row.

- 2026-09-09 (phase 2 implement): **an inert retention in `examples/timeseries` was
  removed, not made well-formed.** `examples/timeseries/models/sources/raw/events.yml`
  declared `retention: '400 days'` on a source with no `timeseries:` — exactly the shape
  this phase refuses. Rather than adding a `timeseries:` block to make it parse, the
  `retention:` line was deleted: the source is deliberately an unclocked lookup
  (`watermark:` + `unique_key:` only), and nothing in the workspace reads its retention
  value. `crates/smelt-core/src/sources.rs` also grew past its large-file-ratchet baseline
  (1297 → 1336); reviewer sign-off: three cohesive error variants plus ~20 lines of
  validation in the file that already single-owns source YAML parsing, baseline updated
  via `.claude/scripts/large-file-check.sh --update`.
- 2026-09-08 (phase 2 planning): **a malformed rolling bound is refused, and an inert one
  counts as malformed** — phase 2 refuses `retention:` when the interval is unparseable,
  when it is zero, and when the source declares no `timeseries:`. Reasoning: the first is
  ordinary fail-loud parsing (today it escapes as an opaque serde `YamlParse` naming the
  retired `data_latency` key); the last two are the same failure this outcome exists to
  prevent, one level up — a bound with no clock has no reach to be compared against, so
  accepting it would silently license every replay it was written to forbid. No new
  `DiagnosticCode`: `MalformedSource` already names the retention clause in
  `sources.md` §"Diagnostic codes". Phase table otherwise unchanged — the phase-1 summary
  surfaced no work needing a new row.

- 2026-09-08 (phase 1 implement): **quantifier settled: over all history ever processed,
  never narrowed by retention.** `docs/specs/incremental_models.md` §"The equivalence
  invariant" now states it directly: a partition that ages out of `retention:` was still
  scanned, so it stays in `S` and the stored table remains the answer of record over it —
  retention narrows the *executable* `full_refresh` oracle's replayable region, never the
  quantifier. A recompute reaching past the bound is refused (`SourceRetentionExceeded`)
  rather than run to produce a smaller answer. Reasoning: this is the existing
  replayability split applied to a bound that moves, and it is what keeps the feature
  small — only a model whose reach exceeds the bound, or an explicit backfill, is ever
  affected; steady-state forward-only maintenance never is.
- 2026-09-08 (phase 1 implement): **retention is declared, verified, never observed** —
  `docs/specs/sources.md`'s `retention:` row and §Semantics 5 now say so explicitly.
  Reasoning: `retention:` is a narrowing world-fact under the existing trust rule (it
  licenses smelt to stop trusting replay over a region), so it follows the same
  discipline as every other narrowing declaration — trusted only paired with a
  verification mechanism, never discovered by probing the warehouse, with over-claiming
  (longer than the producer actually keeps) as the unsafe direction the mechanism must
  catch. `retention:` is also **rolling** (anchored to the current run, advancing on its
  own) rather than a fixed calendar date, because that is the physical fact real
  warehouses expose (BigQuery `partition_expiration_days`, Delta's retention window).
- 2026-09-08 (phase 1 implement): reconciled `examples/github_activity`'s inert
  `retention: '90 days'` to the loader's real 45-day `partition_expiration_days` on both
  `github_events.yml` and `github_events_arrival.yml`, per the spine's findings handoff;
  new red-then-green test
  `crates/smelt-cli/tests/github_activity_loader.rs::source_yaml_retention_matches_the_loader_expiration`
  guards the two staying in sync.
- 2026-09-08 (phase 1 planning): **table reshaped from the placeholder** into rows 2-8, one
  per remaining success criterion, ordered declaration → walk verdict → refuse/degrade →
  bound-movement → conformance → explain/docs → close-out. Nothing left the outcome; the
  spine's handoff requirement (reconciling the example's inert `retention: '90 days'` against
  the loader's real 45-day `partition_expiration_days`) is folded into phase 1 rather than
  deferred, since it is a doc-level fact the quantifier decision settles.

- 2026-09-08 (bigquery-dogfood-spine phase 15): **the interim findings handoff now
  exists** at `docs/handoffs/2026-09-08-github-activity-findings.md`, covering the
  DuckDB half only ("Requirements handed to `20260906-trimmed-history-sources`" section) —
  the loader's 45-day `partition_expiration_days` bound and its derivation, and the
  inert, now-corrected-comment-but-still-inconsistent `retention: '90 days'` field on both
  `examples/github_activity/models/sources/raw/github_events{,_arrival}.yml` that this
  outcome must reconcile. The live-BigQuery half lands in that outcome's phase 16.
- 2026-09-06 (scaffold): **deliberately short**, for the same reason as
  `20260906-external-dag-steps` — the declaration's shape depends on what the spine's
  loader actually retains and on whether that retention is a smelt-visible declaration or
  an observed property (§Open questions 4). The phase-1 planner completes the table from
  the spine's findings handoff.
- 2026-09-06 (scaffold): criterion 1 is ordered first on purpose. The quantifier question
  is not a detail — under "retained history" a trimmed source is ordinary and the feature
  is small; under "all history that ever existed" every maintained model over a trimmed
  source is permanently degraded. The answer changes the size of this outcome by an order
  of magnitude, so nothing else is planned until it is written down.
- 2026-09-06 (scaffold): tension 2 (SCD2 lifetime beyond source retention) is out of scope
  here but is *not* dropped — it is named in Out of scope with the dependency that blocks
  it, so the succession work inherits it rather than losing it.

## Blocked

(none)
