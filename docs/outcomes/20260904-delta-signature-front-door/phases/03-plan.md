# Phase 3 plan — the incremental-models guide opens on delta signatures

## Objective

Rewrite the front of `docs-site/docs/guide/incremental-models.md` so a reader meets the
**delta signature** — what the model emits, on the three-point shape scale, with its addressing —
before any shape, frontmatter or contract detail, and reaches the rest of the guide from there.
Advances success criterion 2 (signature-first guide, no four-corners framing under
`docs-site/docs/`) and pins both halves with a standing gate so the framing cannot regress.

## Spec delta

None. This phase is user-doc-only: the front door it writes already exists normatively in
`docs/specs/incremental_models.md` §Overview "Deltas — the unit everything is typed in",
§Overview "Delta signatures — what a relation emits", and §Surface "CLI" (the headline form).
The guide is being brought up to the spec, not ahead of it.

## Tests

New file `crates/smelt-cli/tests/docs_front_door.rs` (three tests, red before the rewrite):

1. `incremental_guide_first_section_introduces_delta_signatures` — in
   `guide/incremental-models.md`, the body of the **first** `##` section contains "delta
   signature" (case-insensitive), and the first occurrence of "signature" in the whole file
   precedes the first `## Configuration` heading. Red today: "signature" appears zero times.
2. `incremental_guide_front_door_headline_matches_real_explain_output` — the headline line
   inside the guide's first fenced `smelt explain` block is byte-identical to the corresponding
   first line of a real `smelt explain <model>` run against `examples/timeseries` (same
   harness shape as `explain_docs_freshness.rs`'s existing pins; reuse its helpers by copying
   the small `fenced_blocks`/run helpers rather than making the other file `pub`).
3. `no_four_corners_framing_under_docs_site` — zero matches for `four corners` / `four-corners`
   (case-insensitive) across `docs-site/docs/**/*.md`. Green today; this test is the ratchet
   that keeps criterion 2's second half true, and it is stated as such in a doc comment.

## Tasks

1. Write the three tests above; confirm 1 fails and 3 passes before editing any prose.
2. Run `smelt explain` over the `examples/timeseries` models and pick the front-door example:
   prefer a model whose headline is **not** `general` (a real `append-only within a window` or
   `keyed upsert` emitter). If every timeseries model degrades to `general`, use `daily_events`
   and let the prose teach the degradation honestly — `explain` naming the responsible construct
   *is* the feature. Do not invent a headline; paste real output.
3. Replace `## How it works` with a new first section (suggested `## What a model emits`) that,
   in this order: names a data delta; gives the three-point scale
   `append-only within a window ⊑ keyed upsert ⊑ general` in one short list; defines addressing
   (window / key set / whole-table); shows the chosen model's real headline in a fenced block;
   states that a model's signature is **derived from its SQL and its inputs', never declared**;
   and closes with forward pointers to the declared shape facts (§Configuration below), the
   [composed shape](#the-composed-shape-key-time), [contract relaxations](#contract-relaxations),
   [Rebuilding](#rebuilding) and [Schema Evolution](schema-evolution.md).
4. Demote the DELETE+INSERT mechanics: keep the three numbered steps, but move them under
   `## Running incremental models` as `### What a partition-shaped run does`, and reword the
   standing claim "smelt uses a DELETE+INSERT strategy" into what it actually is — the technique
   the derived maintenance plan assigns to a partition-shaped cell — cross-linking
   `smelt explain` as the way to see what a given model's plan chose.
5. Sweep the rest of the guide for prose that contradicts the new front door (any text implying
   the strategy is fixed rather than derived, or that grain is a declared mode). Fix in place;
   keep edits minimal — this phase is a front-door rewrite, not a whole-page rewrite.
6. **Preserve every inbound anchor.** These 11 anchors are linked from elsewhere in
   `docs-site/docs/` and must still resolve after the rewrite: `#configuration`,
   `#contract-relaxations`, `#conditional-writes`, `#enrichment-joins-and-dimension-updates`,
   `#form-b--explicit-wherejoin-interval-filters`, `#non-deterministic-columns`,
   `#observed-deltas-and-no-op-cascades`, `#self-referential-ordered-models`,
   `#steering-prefer--technique`, `#the-composed-shape-key-time`,
   `#the-reconciliation-ledger`. `#how-it-works` has no inbound link and may go.
7. Add the guide to §Further reading's neighbours only if a link is genuinely missing; otherwise
   leave that section alone.

## Verification

- `cargo test -p smelt-cli --test docs_front_door` — the new gate, all three green.
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage`
  — the existing doc gates still green. **Watch out:** `explain_docs_freshness.rs` locates its
  pinned guide excerpt with `text.find("$ smelt explain daily_events_enriched\n")` — the *first*
  occurrence. If the new front-door block uses that same prompt line it will steal the pin.
  Use a different model in the front door, or move the pinned block itself to the front.
- `rg -in "four.corners" docs-site/docs/` — no output.
- `rg -no "incremental-models\.md#[a-z0-9-]+" docs-site/docs/ docs/ | sed 's/.*#/#/' | sort -u`
  — every anchor listed in task 6 still present as a heading in the rewritten file.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.

## Commit message

`docs(incremental): the guide opens on delta signatures, not DELETE+INSERT`
