# Review: `docs/specs/incremental_models.md` (post-redraft consistency pass)

**Date:** 2026-07-22
**Scope:** consistency, wording, and correctness review of `docs/specs/incremental_models.md`
(2,517 lines, as of `last_reviewed: 2026-07-22`), including the lint checks prescribed by
`docs/specs/CLAUDE.md` and cross-checks against sibling specs and the implementation.
**Status of findings:** proposed only — nothing below has been applied. Items in §A need a
direction decision before wording; §B and §C are mechanical once approved.

**Overall verdict.** The spec is in strong shape. The banned-vocabulary grep is clean and
162 of 168 `§"…"` references resolve. The substantive findings cluster around the composed
key+time corner (A1–A3) plus a few scoping/precedence ambiguities (A4–A7); the rest is
reference hygiene and wording.

Line numbers refer to the file as of this review.

---

## A. Substantive — need a direction decision

### A1. Route 1 (key-embedded) contradicts the `key_per_partition` grain rule

The biggest finding. Three passages say `partition_column ∈ unique_key` ⇒ derived
`grain: key_per_partition`, a *different* grain the key profile does not maintain:

- the four-corners table (line 55);
- §Surface "Grain is a derived label" (line 139): "The single fact `partition_column ∈
  unique_key` is what distinguishes the trajectory … from a keyed lookup whose key has a
  fixed home slice";
- the key-grain rule at line 245: "`grain: key_per_partition` is a **different grain**, not a
  sub-declaration".

But locality **route 1** (line 1418) is defined as "`partition_column` is a `unique_key`
column" and is presented as an admission route for a `grain: key` model — and
`KeyedGroupByContainsPartitionColumn` (line 375) offers "declaring `timeseries:` on the model
to stay `grain: key`" as a fix for exactly that shape. Since §Surface also requires
`unique_key` to restate the GROUP BY (line 241), any model written per the spec that
GROUP-BYs its partition column derives `key_per_partition` and refuses
(`MaintenanceUnsupportedGrain`, per §Known Divergences) — making route 1 unreachable as
specified.

**Implementation evidence.** `derive_grain` (`crates/smelt-core/src/config.rs`) maps
partition-in-declared-key to `KeyPerPartition`; `crates/smelt-db/src/queries/maintenance.rs`
refuses that grain at plan derivation. The route-1 conformance recipes
(`crates/smelt-maintenance-testkit/src/render.rs`) square the circle only through the
"narrow gap" recorded in §Known Divergences: they assert `grain: key` and deliberately
declare **no** top-level `unique_key:` ("a keyed output's `unique_key` is derived from its
own `GROUP BY`, never declared") — which itself contradicts §Surface's restate rule.
`establish_locality` (`crates/smelt-logical/src/maintenance/locality.rs`) then reads the
GROUP-BY-derived key, so route 1 fires.

**Decision needed.** Either:

- (a) `partition_column ∈ key` on a locality-admitted output stays `grain: key`, and
  `key_per_partition` needs a different discriminator (the current formula
  `(clock?, identity?, partition_column ∈ key?)` cannot distinguish the two shapes); or
- (b) route 1 belongs to the trajectory grain, and the route list, the
  `KeyedGroupByContainsPartitionColumn` fix-it text, and the conformance family get re-homed.

Whichever way, this spec, `models.md` (line 130 four-corner table, line 132, line 134), and
`derive_grain` should end up saying the same thing.

### A2. The outer output-clamp is specified over two different columns

§"Execution model" step 2 (line 986) injects
`WHERE partition_column >= out_start AND partition_column < out_end` over the **derived
output window** — this matches the implementation (`crates/smelt-runtime/src/execute.rs`
passes `partition_col` to `inject_time_filter`). But §"Event-time outer-visibility"
(line 1172) says the clamp injects
`WHERE event_time_column >= start AND event_time_column < end`. When the two columns differ
(a skewed Form B model), these are different filters over different windows.

Note the *check* in code (`crates/smelt-logical/src/rules/rule_diagnostics.rs`,
`check_event_time_injectable`) does guard `event_time_column`, while `partition_column`
output-visibility is separately guaranteed by `MalformedTimeseries`
(`timeseries.md` rule 1; consumed at line 1260).

**Proposed fix:** reword the §"Event-time outer-visibility" opening so it does not restate
the clamp formula with a different column (e.g. "the outer output-clamp (§"Execution model"
step 2) requires its clamp column to be accessible at the outermost SELECT…") — but confirm
which column the visibility requirement is really about before wording.

### A3. Derived posture 2 (order-independence) omits the additive-fold family

The column-family catalogue (line 285) marks additive fold **Order-independent: yes**
(`+`/`xor` commute), but posture 2 (line 1313) says only "the extremal/lattice and proven
once-write families qualify" — as written, any model with a `SUM` column is forced into
sequential window application. One of the two is wrong; the posture-2 enumeration looks like
the slip (additive folding is order-independent; the ledger's never-fold-twice obligation is
orthogonal to ordering).

### A4. Interval-versioning soft-close-on-absence is unscoped — wrong for window-forward

§"Deletion handling" (line 1671) treats "a key present in the store but absent from the
incoming set" as a retraction, and even names the window-forward event time to stamp it with
("the run's window boundary for a window-forward feed"). Under window-forward consumption,
absence from a run window just means "no updates this window" — soft-closing on absence
would close every key not touched in each window.

**Proposed fix:** scope retraction-on-absence to **snapshot-diff only**; state that a
window-forward/CDC feed signals deletion only via explicit delete events (which the
section's last sentence already says).

### A5. The `LIMIT` safety check vs `safety_overrides`

The safety-checks preamble (line 1139) says "Each is individually disabled via
`safety_overrides.allow_<check>: true`", but the LIMIT row says "never". The implementation
has `allow_limit` (and `allow_distinct`) in `crates/smelt-core/src/config.rs`.

**Decision needed:** if `allow_limit` is honoured, the LIMIT row should read "never by
proof; escape hatch `allow_limit`"; if LIMIT is meant to be un-waivable, that is an
implementation divergence to record in §Known Divergences and the preamble needs a
carve-out. The DISTINCT row should also name its hatch (or none) consistently with the
window-functions row, which spells its hatch explicitly.

### A6. `max_lookback` vs `allow_full_scan` disagree between two statements

The `MaintenanceScanUnbounded` table row (line 353) implies `allow_full_scan` excuses a
`max_lookback` violation ("…(or exceeds a declared `max_lookback`) **and** no
`allow_full_scan` acceptance exists"); §"Partition-local maintenance" (line 789) says
`max_lookback` "additionally refuses a derived span wider than the operator's stated
expectation" — unconditionally. Pick one (or declare the combination of both keys on one
source a configuration error).

### A7. Running-example `key_recurrence: '7 days'` is not the sources.md grammar

Line 100 declares `key_recurrence: '7 days'` on `sources.raw_events`, but `sources.md`
defines `key_recurrence` as a structured `{key: […], window: '…'}` block, with
`MalformedSource` firing on a missing `key`/`window`.

**Proposed fix:** `key_recurrence: { key: [event_id], window: '7 days' }`.

---

## B. Reference / lint fixes (mechanical once approved)

1. **Line 2246:** `(smelt-yml.md §"Layer split")` — there is no spec named `smelt-yml.md`
   (the spec is `smelt_yml.md`, which has no such section); the "Layer split" note lives in
   the *docs-site* page `docs-site/docs/reference/smelt-yml.md`. Better: cite `models.md`
   §Known Divergences, which records the surviving `batched:` override path.
2. **Line ~718:** `model_transforms.md §"widened scan + exact clamp"` — the actual name
   there is "Two-layer widened-scan + exact output clamp" (a catalogue row, not a heading).
   Align the name.
3. **Prose-only diagnostic codes missing from the §Diagnostics table** (craft rule: codes
   mentioned in prose must appear in the table): `MaintenanceUnsupportedGrain` (line 2206),
   `MaintenanceGranularityMismatch` (line 2170), `KeyedSnapshotPostureUnsupported`
   (line 2286) — all real codes catalogued in `diagnostics.md`. Add rows. `MalformedTimeseries`
   (line 1262) is owned by `timeseries.md` — add with an ownership note or exempt cross-spec
   codes explicitly.
4. **Stale inbound references from sibling specs** (heading-rename sweep debt from the
   redraft; "Heading names are API"): `models.md` cites §"Partition-grain frontmatter (in
   `.sql` files)" (twice) and §"What the composed shape **uniquely** enables";
   `planner_integration.md` cites §"Functions inside partition-grain **model** bodies";
   `materialized_view.md` cites §"The composition contract" — none exist any more. Fixes
   land in those files, but the redraft owes the sweep. (References to bold paragraph labels
   — "Upstream model edges", "Observed deltas on model edges", "Grain is a derived label",
   "Group convergence", "Failure mode" — do resolve to existing bold labels; fine if that
   convention is accepted, otherwise promote those labels to headings.)
5. **Minor:** the `MaintenanceReachNotDerivable` table row (line 352) cites only admission
   obligation 4; §"The graph layer" (line 874) also fires it for an upstream-model edge with
   no derivable clock. Extend the row text.

---

## C. Wording / craft

1. **Line 137:** "Text anywhere in this corpus that treats 'partitioned' and 'keyed' as
   mutually exclusive alternatives is wrong and is corrected against this section" — the
   arguing-with-past-drafts tone `docs/specs/CLAUDE.md` bans, and the same doctrine is
   restated in §Design (line 1739, "reviewers should treat one-or-the-other phrasing … as a
   defect"). Keep one calm statement (preference: the Design paragraph, which is the
   decision record) and reduce the other to a cross-reference.
2. **Line 1596:** "`--event-time` run windows" → the flags are
   `--event-time-start`/`--event-time-end` everywhere else.
3. **`prefer` enums differ** between `maintenance.defaults.prefer` (includes `auto`,
   line 148) and `cells[].prefer` (no `auto`, line 152). If intentional (absence = auto),
   add a clause saying so; otherwise align.
4. **Line 2426:** stray spaces in `maintenance_plan_conformance :: coverage_matrix_is_inhabited`.

---

## Lint results (for the record)

- `rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]'` over the spec:
  **clean**.
- Internal `§"…"` reference resolution (prefix-matching against headings, line-wrap and
  backtick aware): **168 references, 6 unresolved**, all external — the two
  `model_transforms.md` names (B2; one is a bold paragraph lead that exists verbatim, one a
  renamed catalogue row) and four path/naming issues (B1, plus full-path research/docs-site
  cites the checker cannot see; verified by hand: `docs/DESIGN.md` §"Incremental Table
  Builds", `07-example-catalogue.md` §"Coverage matrix", and docs-site
  `incremental-models.md` §"The composed shape (key + time)" all exist).
- Reverse direction (sibling specs → this spec): the stale citations listed in B4.

## Suggested application order

B and C plus A4, A5 (documenting `allow_limit`), A6, and A7 are safe to apply once a
direction is picked; A1–A3 need an intent ruling first. A1's resolution should be made
consistently across this spec, `models.md`, and `derive_grain` — and probably deserves its
own small spec diff + plan, since it changes what the key-grain §Surface requires
(`unique_key` restatement vs GROUP-BY-derived identity).
