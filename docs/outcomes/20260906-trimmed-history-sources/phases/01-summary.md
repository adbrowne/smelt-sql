# Phase 01 summary — the equivalence quantifier for a trimmed source

**Decision (criterion 1, wholly; criterion 2's declared-vs-observed half):** `S` in the
equivalence invariant ranges over *all history ever scanned* — a trimmed `retention:` never
shrinks it. A partition that ages out of retention was still scanned, so the stored table
remains the answer of record over it; retention narrows only the *executable* `full_refresh`
oracle's replayable region. A recompute whose window reaches past the bound is refused
(`SourceRetentionExceeded`), never run to a smaller answer. `retention:` itself stays a
**declared, verified** narrowing world-fact (never warehouse-observed) and is **rolling**
(anchored to the current run, not a fixed date).

## Spec anchors touched

- `docs/specs/incremental_models.md` §"The equivalence invariant" — new paragraph
  "Trimmed history narrows replayability, never `S`" inserted after §"The replayability
  split", before §"Key departure follows the source posture".
- `docs/specs/sources.md`:
  - `retention` row in the §"Source YAML shape" key table — gained the rolling framing and
    the "declared, never observed" sentence.
  - §Semantics 5 ("Retention refusal") — same two additions, in prose.
  - §Design — new paragraph "Retention narrows replayability, not the equivalence
    invariant's quantifier" after "Trusted-replayable retention default", stating the
    rejected alternative (narrowing `S`) and why.

## Example reconciliation

`examples/github_activity/models/sources/raw/github_events.yml` and
`github_events_arrival.yml` both declared an inert `retention: '90 days'` against the
loader's real 45-day `partition_expiration_days` (per the spine's handoff,
`docs/handoffs/2026-09-08-github-activity-findings.md`). Both now declare `retention: '45
days'`, and `github_events.yml`'s stale comment block is rewritten to state the reconciled
fact. New test `github_activity_loader.rs::source_yaml_retention_matches_the_loader_expiration`
parses `README.md`'s documented `partition_expiration_days` and asserts both source YAMLs
match it exactly — red on 90-vs-45, green now.

## For the next planner (phase 2, the declaration)

- The declaration's *shape* (`retention: '<N> days'`, a bare interval string) does not
  change — this phase only sharpened its meaning and reconciled the fixture. Phase 2 should
  focus on: what a malformed rolling bound looks like (unparseable interval — already
  covered generically by `MalformedSource`'s "malformed `watermark`/`retention`" clause per
  `sources.md` §Surface's diagnostic table; check whether a *new* `examples/broken/`
  fixture is actually needed or whether the existing malformed-interval coverage already
  satisfies criterion 2, since `retention:`'s parse path is shared with `watermark:`).
- Criterion 3 (the walk): the bound to compare against a model's required look-back is the
  source's `retention:` interval as parsed into `smelt-core`'s existing interval type —
  nothing new was introduced here that the walk needs to know about beyond "it's rolling,
  so re-evaluate every run" (criterion 5's job, not the walk's).
- Not done here, deliberately out of this phase's scope per the plan: no maintenance
  mechanism, no walk change, no new diagnostic code, no docs-site page (lands with the
  declaration/refusal per the outcome's phase table).

## Gates

- `bash .claude/scripts/verify-phase.sh` — pass (see below for detail if truncated).
- `cargo test -p smelt-cli --test github_activity_loader --quiet` — 11 passed.
- `cargo test -p smelt-core --test source_world_facts watermark_and_retention_parse --quiet`
  — 1 passed.
- `cargo test -p smelt-cli --test example_diagnostics --quiet` — 126 passed, 1 ignored (no
  new diagnostics introduced by the YAML edit).
- No ratchet moved.
