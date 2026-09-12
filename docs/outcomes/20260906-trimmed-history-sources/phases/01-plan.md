# Phase 01 — Settle the equivalence-invariant quantifier for a trimmed source

**Outcome:** `docs/outcomes/20260906-trimmed-history-sources/outcome.md`
**Spec anchors:** `docs/specs/incremental_models.md` §"The equivalence invariant";
`docs/specs/sources.md` §"Source YAML shape", §Semantics 5 ("Retention refusal"),
§"Design rationale"
**Advances:** success criterion 1 (wholly), criterion 2 (the declared-vs-observed half of it)

## Objective

Write down, in the spec, whether `full_refresh(inputs ∈ S)` for a source whose retained
history is bounded and moving forward is taken over *retained* history or over *all history
that ever existed*, with the reasoning — and settle §Open question 4 (is the bound a
smelt-visible declaration or a property smelt merely observes) in the decision log. This is
spec-and-decision work; no maintenance mechanism is built here. It also closes the one
concrete inconsistency the spine handed over: `examples/github_activity`'s inert
`retention: '90 days'` against its loader's real 45-day `partition_expiration_days`.

## The decision to write (derived, not invented)

Both halves follow from structure already in the specs; the phase's job is to state them
explicitly and give the reasoning, not to re-open them.

1. **Quantifier: over all history ever processed. Trimming does not shrink `S`.**
   `S` is defined as "source rows or partitions the runs have *scanned*"
   (§"The equivalence invariant"). A partition that has aged out of the source was still
   scanned, so it stays in `S`; retention is a **replay** bound, not a membership bound —
   which is exactly how `sources.md` already types the slot ("replay bound",
   `retention:`). The consequence to state plainly: trimming makes the *executable* oracle
   unavailable over the departed region, it does not make the stored answer wrong. This is
   the existing **replayability split** applied to a bound that moves, not a new carve-out.
2. **Therefore: stored state is the answer of record over trimmed history.** A recompute
   whose window reaches past the bound would produce a strictly *smaller* answer than the
   invariant demands, so it is refused (`SourceRetentionExceeded`) rather than being
   re-declared correct by narrowing the quantifier. The scaffold's feared outcome — "under
   'all history that ever existed' every maintained model over a trimmed source is
   permanently degraded" — does not follow: steady-state maintenance reads only forward, so
   only a model whose *reach* exceeds the bound, or an explicit backfill/full refresh, is
   affected. This is what keeps the outcome small.
3. **Retention is rolling, and that is the novelty.** `retention: '45 days'` denotes a bound
   anchored to the current run, advancing on its own — so admissibility is a property of the
   run, not of the code, and must be re-evaluated per run (criterion 5's mechanism; not built
   here).
4. **Open question 4: declared, verified — never observed.** `retention:` is a world-fact
   declaration under `sources.md` §Semantics "The trust rule", and it is a **narrowing**
   declaration (it licenses smelt to stop trusting replay), so it is admissible only paired
   with a verification mechanism. smelt never probes a warehouse to discover a retention
   bound. A declaration that over-claims (longer than the producer actually keeps) is the
   unsafe direction and is what the paired mechanism must catch.

## Spec delta (made by the implement step, before any code)

- `docs/specs/incremental_models.md` §"The equivalence invariant" — add a short paragraph
  after **The replayability split**, titled for the trimmed case, stating decisions 1–2 and
  their reasoning, and pointing at `sources.md`'s `retention:` slot. Timeless-oracle rule
  applies: no phase or outcome vocabulary in the body.
- `docs/specs/sources.md` — `retention:` row in the key table and §Semantics 5 gain the
  **rolling** framing (decision 3) and the explicit "declared, never observed" sentence
  (decision 4); §"Design rationale" records why the quantifier is not narrowed.
- No `docs-site/` change in this phase — the user-facing page lands with the declaration
  and refusal (later phase), per the outcome's phase table.

## Tests

- `crates/smelt-cli/tests/github_activity_loader.rs::source_yaml_retention_matches_the_loader_expiration`
  — new; parses `partition_expiration_days` from `README.md` (reuse the existing helper in
  `ddl_declares_day_partitioning_and_the_documented_retention_bound`) and asserts both
  `models/sources/raw/github_events.yml` and `github_events_arrival.yml` declare exactly that
  many days of `retention:`. **Red today** (90 vs 45); green once the YAMLs are corrected.
- `crates/smelt-core/tests/source_world_facts.rs::watermark_and_retention_parse` — unchanged;
  run to confirm the interval parse this phase relies on still holds.

## Tasks

1. Write the `incremental_models.md` §"The equivalence invariant" paragraph (decisions 1–2).
2. Write the `sources.md` edits (decisions 3–4) in the key table, §Semantics 5, and
   §"Design rationale".
3. Add the red test above; watch it fail on 90-vs-45.
4. Correct `retention:` to `'45 days'` in both `examples/github_activity/models/sources/raw/`
   YAMLs, and rewrite the stale comment block in `github_events.yml` (lines ~29-34) so it
   states the reconciled fact rather than deferring it to this outcome.
5. Append the two decisions (quantifier; declared-not-observed) to the outcome's
   `## Decision log`, dated, each with its one-line reasoning.
6. Write `phases/01-summary.md`: the decision as written, the exact spec anchors touched, and
   anything the walk phase (criterion 3) must know about how the bound is spelled.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_loader --quiet 2>&1 | tail -20`
- `cargo test -p smelt-core --test source_world_facts --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20` (the example
  YAML edit must not introduce a diagnostic)
- No ratchet should move; if one does, stop and report rather than update a baseline.

## Commit message

`spec(sources): settle the equivalence quantifier for a trimmed-history source`
