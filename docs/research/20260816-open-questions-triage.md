# Open-questions triage — incremental specs (2026-08-16)

The decision doc for the delta-signature closure programme's decision track
(`docs/handoffs/2026-08-16-delta-signature-closure-programme.md`). Each item below is an
open question tagged across the incremental specs, stated so it can be read without the
spec open, with a recommendation where one is defensible. **Mark up inline: write
`Decision:` under each item (or just "agree").** Every decision lands as a spec diff first,
then graduates into the residue outcomes.

## A. Keys and deletion

**1. Departed-key deletion / retention policy.** Keyed incremental models never delete
today: a key that disappears from the source stays in the target unchanged, forever, and
nothing in the model contract says so. There is no tombstone, no opt-in hard delete, and
this same gap blocks maintaining aggregates under deletes and consuming change feeds that
carry delete events. **Recommend:** make retention the first new contract feature — a
declared per-model retention policy (`keep forever` as the explicit default matching
today's behaviour; opt-in hard delete; tombstone marking) — prioritised ahead of all other
contract-relaxation work, since two other questions below depend on it. This grows the
keyed-residue outcome's scope.
Decision: **deletion is derived from the source posture, not declared.** Default behaviour
preserves full-refresh equivalence: append-only upstream → keys never depart, retain
everything (already correct); mutable snapshot upstream → anti-join delete of departed keys
each reconcile (removes the current "departed keys are retained" carve-out); windowed scan
over a mutable upstream → whole-region recompute (or refusal), since departure isn't
observable from the window and there is no tombstone; change feed with delete events →
apply the delete (unblocks item 4b). "Keep departed keys" stops being the silent default
and becomes an explicit opt-in relaxation for users who want history — an SCD-flavoured
contract point, declared, explain-visible, with its own oracle transform. No migration
concern: there are no production uses yet, so the snapshot-reconcile behaviour change
(targets start losing departed keys) ships without a compatibility path.

**2. Self-referential keyed models** (`state = state + delta − decay` patterns, where the
model reads its own previous output). Rejected today. Admitting them needs an explicit
design separating "input" from "carried state" — without it, the full-refresh oracle that
backs the correctness guarantee doesn't exist. **Recommend:** keep rejecting; revisit only
after retention (deletion of carried state interacts directly). No spec change now beyond
confirming the rejection is intentional.
Decision: defer — keep rejecting; re-triage after item 1's posture-derived deletion lands.

**3. Deletion-adjacent locality relaxations.** Several smaller deferred questions (pruning
which key-slices a snapshot-style model rescans; allowing a daily-grain driver to feed
weekly output partitions; slice-scoped deletion) all interact with how deletion is
contracted. **Recommend:** defer the lot until item 1 is decided; then re-triage.
Decision: defer — re-triage once item 1's posture-derived deletion is in the spec.

## B. Change data coming in

**4. Change-feed sources.** A source declared as a change feed currently gets no
incremental treatment — any change forces full re-derivation of consumers, and (unlike
every other mutable source kind) it doesn't even get the standard "upstream mutated" repair
handling. Two separable calls: (a) give change-feed sources the same upstream-mutation
repair treatment as other mutable sources — **recommend yes now** (consistency, small);
(b) real fold machinery that consumes the feed's inserts/updates/deletes incrementally —
**recommend defer until retention (item 1) lands**, because delete events need a contract
home first.
Decision: agree

**5. Joining multiple snapshot-style sources.** A keyed model reconciling against a full
snapshot may have at most one un-clocked source in its FROM clause; joining two refuses
loudly. Widening needs a proven multi-source scan design. **Recommend:** keep the loud
refusal; widen only when a real workload hits it.
Decision: agree - add to further extensions so we don't lose track of this

## C. Run windows and scheduling

**6. Finer-than-partition run windows.** Asking for an hourly run window on a
day-partitioned model hard-rejects today. The alternatives are auto-coarsening (silently
widen the request to a day) or rejecting with a suggested corrected window. **Recommend:
reject-with-suggestion.** Silent widening recomputes more than the operator asked for and
undermines "the engineer controls planning"; a precise error with the coarsened window
spelled out costs one message.
Decision: agree

**7. `NOW()` / current-timestamp handling in keyed models.** Partition-grain models pin
these functions to a fixed timestamp at compile time so reruns are reproducible; keyed
models instead reject them outright. **Recommend:** unify on pinning — same rule
everywhere is easier to learn, and the mechanism already exists.
Decision: both current behaviours are wrong — allow `NOW()`/`CURRENT_*` to run as-is
everywhere (no compile-time pinning in partition models, no rejection in keyed models),
and amend the equivalence rule: **the promise is scoped to deterministic outputs** — where
two full refreshes would themselves disagree, no equivalence is promised. Mechanically:
the walk's existing per-column determinism verdict exempts time-dependent columns from
the conformance oracle's comparison, `smelt explain`'s per-column guarantees state the
exemption explicitly, and diff/suppression-style techniques that rely on
recompute-equality exclude (or refuse on) volatile columns via the same verdict so they
don't see permanent phantom drift.

**8. Driver granularities.** The scheduling driver understands day and week only; month
(and eventually hour) are inherited limitations for every consumer. **Recommend:** widen
on demand, month first when a workload needs it; not programme work now.
Decision: yes - add to future extensions

**9. `--auto` staleness precision.** For fully idempotent keyed models, `--auto` currently
over-approximates what's stale (safe, sometimes wasteful); exact staleness needs
delta-history machinery from a later ladder rung. **Recommend:** accept the conservatism;
defer.
Decision: agre - add to future extensions

**10. Merge history for idempotent keyed models.** The merge ledger is only written for
models where re-folding would double-count; a fully idempotent keyed model keeps no record
of which windows it has merged, so `--auto` has nothing to consult. Nothing is incorrect,
just blind. **Recommend:** once the ledger moves into the warehouse (state-residency
outcome, running now), write it for all keyed models — one small table buys staleness
visibility everywhere. Fold into the keyed-residue outcome.
Decision: agree - do this automaitaclly if state is supported in the project

## D. State and bookkeeping

**11. Opting out of smelt-authored tables in the warehouse.** The state doctrine puts
correctness-critical bookkeeping (the merge ledger) in the user's warehouse. A user who
forbids any tool-authored objects in the target schema has no knob. **Recommend:** a
project-level setting that forces the documented degradation path project-wide (recompute
instead of ledger-dependent techniques, recorded and visible in `smelt explain`) — not
per-table granularity. Honest and simple; refuse loudly where a declared contract can't be
honoured without state.
Decision: agree

**12. A transactional ledger for Spark.** The warehouse-resident ledger is DuckDB-only;
on other backends ledger-dependent models will take the recorded downgrade path once
state-residency lands. **Recommend:** don't build the Spark ledger until a real
Spark-targeted incremental workload demands it; the downgrade path makes absence safe and
visible.
Decision: yes - let's put this future extensions

**13. `.smelt/` storage format.** Local run state is JSON files today; an embedded
database was sketched for when the environment/snapshot store grows. **Recommend:** stay
with JSON; decide when the virtual-environments work actually lands.
Decision: agree

**14. Detecting out-of-band table edits.** If someone manually edits a target table
between runs, smelt doesn't notice. A digest tripwire would catch it at a per-run cost.
**Recommend:** don't build it — rare, self-inflicted, and expensive to check; record as an
explicit non-goal so the question stops reopening.
Decision: agree

**15. Sub-day interval bookkeeping.** Run-state coverage intervals are keyed by calendar
date, while models routinely filter at hourly/second boundaries. **Recommend:** move
interval keys to full timestamps as part of the scheduler outcome (it's rebuilding the
scheduler's currency anyway — cheapest moment to do it).
Decision: agree

## E. Definition changes and policy knobs

**16. A per-model "what happens when a column is added" knob** (backfill / leave null /
recompute). **Recommend: drop it.** `smelt migrate`'s per-column-group verdict (being
wired in the definition-delta outcome) already answers this case-by-case; a standalone
knob would be a second, drifting answer.
Decision: agree

**17. Values merged from two mutable inputs.** When one grouped output value draws on two
different sources that can both mutate, there's no declared policy for which repair
applies. **Recommend:** force full recompute of the affected region — the conservative
default every comparable rule already takes; revisit only if it proves expensive in
practice.
Decision: agree

## F. Surface and ergonomics

**18. Pattern helpers (`smelt.latest`, `smelt.once`, `smelt.current`).** Each keyed
pattern is reachable today only via its hand-written SQL spelling. Ship the helpers as
built-in functions, or as a shipped template file users import? **Recommend:** shipped
template file first — no parser/registry surface commitment, trivially promotable to
built-ins later if they stick.
Decision: agree

**19. Metrics × incremental time filtering.** How metric expansion composes with the time
filters injected into partition-grain models is unspecified. **Recommend:** refuse the
combination loudly for now (a named diagnostic), and spec the composition when metrics
work resumes — don't let it fail quietly in the meantime.
Decision: agree

**20. docs-site CLI coverage audit.** One divergence bullet says "docs coverage of the
maintenance CLI surface is partial" without enumerating what's missing. **Recommend:**
not a product decision — enumerate the residue once inside an outcome close-out phase,
document or drop each item, delete the open question.
Decision: agree

## Not product decisions (triaged out)

These carry "(Open Question)" tags in the specs but need engineering, not a product call;
they route to the residue outcomes as scope: per-source clamp observability in
`smelt explain --json` and editor hovers; the specified-but-never-produced retraction
diagnostic (rides item 1); the declared-dependency-only locality route; NULL payloads in
the generative conformance pool; ledger fold transactionality on non-DuckDB backends
(subsumed by the state-residency outcome and item 12).
