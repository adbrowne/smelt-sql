# Closure report — 2026-08-15 incremental-spec program

Baseline: commit `03a431f3` (`outcome(20260815-definition-delta-migrate): scaffold`), the first
commit of the program. Denominator: 80 `§Known Divergences`/`(Open Question)` bullets across the
four anchor specs (`definition_deltas.md` 7, `incremental_models.md` 25, `incremental_shapes.md`
32, `model_properties.md` 16), extracted and classified in phases 1-2, spot-checked in phase 3,
validated drift-free in phase 4. This report is the single checkable artifact success criterion 1
asks for; criteria 2-6 are each given their own section below with the evidence phases 1-4 already
established, re-cited rather than re-derived.

## Criterion 1 — full disposition table (80/80 IDs)

Disposition vocabulary: `closed <sha>` (verified absent from current spec text and confirmed in
repo/tests), `open` (still a real gap, reason given below), `drifted` (spec wording changed since
baseline; phase-3 Verdict of `accurate`/`relocated` follows in parentheses — spec text itself is
fine, no fix needed). Full context for every row (subsection, baseline line number, bullet prose)
is in `baseline-inventory.md`; this table restates disposition + reason/evidence only.

### definition_deltas.md (7)

| ID | spec | bullet lead-in | disposition | evidence |
|----|------|-----------------|-------------|----------|
| DD-01 | definition_deltas | The definition-delta synthesis layer is unwired. | closed 1c7bffea | - |
| DD-02 | definition_deltas | `smelt migrate` does not exist | closed 1c7bffea | - |
| DD-03 | definition_deltas | The atomicity rule is conditional in practice. | closed 129984d9 | - |
| DD-04 | definition_deltas | The conformance harness has no definition-edit step kind yet | closed 7b0bf461 | - |
| DD-05 | definition_deltas | No approval store exists | closed 0d5cb0e6 | - |
| DD-06 | definition_deltas | Open question — plan-hash scope. | closed c186b5fb | - |
| DD-07 | definition_deltas | The diagnostic name is narrower than its rule. | closed eab2aa72 | - |

### incremental_models.md (25)

| ID | spec | bullet lead-in | disposition | evidence |
|----|------|-----------------|-------------|----------|
| IM-01 | incremental_models | The scheduler does not yet consume delta signatures end to end. | open | Unimplemented scheduler-integration backlog, not a design question — the delta-signature model itself is fully decided and shipped; wiring the scheduler is unscheduled follow-on work outside this confirm-only program's scope (no new code). |
| IM-02 | incremental_models | `smelt explain` does not yet print the delta-signature headline | open | CLI-surface backlog item (display only); no product decision blocks it, just unscheduled implementation, and adding new CLI output is out of scope for a closure audit. |
| IM-03 | incremental_models | Per-cell `deferral` is not yet scheduled | closed 64f20a29 | - |
| IM-04 | incremental_models | `diff_patch` over the region `DeleteInsert` default has no runtime lowering | drifted (accurate) | Reworded: membership-sensitive case now lowered (64f20a29); plain windowed/partition-grain default still `backend_default` — real, still-open gap, current wording accurate. |
| IM-05 | incremental_models | Frontmatter-time grain checking has one narrow gap | closed 6badd725 | - |
| IM-06 | incremental_models | The write-pin equivalence factor is structural only | closed 6f8b8083 | - |
| IM-07 | incremental_models | An inadmissible write-*variant* pin has no pre-execution gate | closed 6f8b8083 | - |
| IM-08 | incremental_models | Observed-delta consumption is partial | closed a14a75c3 | - |
| IM-09 | incremental_models | No execution technique keys off a maintained-model creation cell | closed 6badd725 | - |
| IM-10 | incremental_models | Plan-consumer gaps | closed 242ec4e1 | - |
| IM-11 | incremental_models | Emission remainders | open | Named residual-emission backlog with no live consumer yet; not blocked by an undecided question, just unscheduled — tracked implicitly under "Proofs as product" Future Extension. |
| IM-12 | incremental_models | Locality and diagnostic residues on the maintenance-plan proofs | closed e885bd6e | - |
| IM-13 | incremental_models | The ledger's warehouse substrate is DuckDB-only (Open Question) | drifted (accurate) | Spark deferral decided (not undecided) per `docs/research/20260816-open-questions-triage.md`; DuckDB-only gap itself unchanged — accurately still open as a build gap, not a live decision. |
| IM-14 | incremental_models | Graph-layer gaps | drifted (accurate) | Narrowed by three closures (87df1dcc self-edges, 15b4073a key-addressed edge admission, acb5e66d `--select` scoping); only key-temporal-locality-vs-admission residue remains, current wording accurate. |
| IM-15 | incremental_models | Delta detection for `--since-upstream` is explicit-only in v1 | open | Deliberate v1 scope limit; automatic watermark-diffed `--since-upstream` is explicitly named in `definition-delta-migrate` §Out of scope / `incremental_models.md` §Future Extensions as future, undecided surface this program declined to design. |
| IM-16 | incremental_models | Straddle attribution without locality is scoped out of the ledger's v1 | open | Deliberate v1 scope limit (same boundary as IM-15); extending it requires deciding how straddle attribution composes with locality, a design question this program did not take on. |
| IM-17 | incremental_models | No out-of-band-edit tripwire (Open Question) | closed 2e1f6d19 | - |
| IM-18 | incremental_models | A proposed `on_column_add: backfill \| leave_null \| recompute` policy knob (Open Question) | closed 2e1f6d19 | - |
| IM-19 | incremental_models | The derived model-wide horizon is under construction | open | In-progress implementation backlog (not a live design question) — the horizon's derivation rules are decided, the code path is unfinished; unscheduled follow-on work. |
| IM-20 | incremental_models | Override-ladder reach (Open Question) | closed 7fc342d3 | - |
| IM-21 | incremental_models | docs-site coverage of the plan's CLI surface is partial (Open Question) | closed 41deabef | - |
| IM-22 | incremental_models | A group merged across two mutable inputs has no group-merge-provenance policy (Open Question) | closed 2e1f6d19 | - |
| IM-23 | incremental_models | `change_feed` sources never get an `UpstreamMutation` cell (Open Question) | drifted (accurate) | A cell now exists (outcome phase 28c); wording narrowed to "always re-derives from the full input" (fold consumption still missing) — the narrower gap is real and still open, current wording accurate. |
| IM-24 | incremental_models | `INTERSECT`/`EXCEPT` are unclassified set operations | closed 4f8b9c66 | - |
| IM-25 | incremental_models | Conditional-maintenance gaps | open | Narrowed to the one remaining `supports_fingerprint_sidecar` residue; an implementation gap in an already-decided design, not a live question — unscheduled follow-on work. |

### incremental_shapes.md (32)

| ID | spec | bullet lead-in | disposition | evidence |
|----|------|-----------------|-------------|----------|
| IS-01 | incremental_shapes | One classification call site reads the outer SQL body | closed 98393e25 | - |
| IS-02 | incremental_shapes | The window-function batch-safety check runs on unexpanded outer SQL | closed 98393e25 | - |
| IS-03 | incremental_shapes | Per-source clamp observability is partly emitted (Open Question) | closed 3a6e995a | - |
| IS-04 | incremental_shapes | Per-column `data_latency` is unimplemented | open | Unimplemented backlog feature; the per-column-vs-per-model latency model is already decided (per-model exists), extending it to per-column is unscheduled work, not an open design question. |
| IS-05 | incremental_shapes | Non-deterministic row-set-membership or grouping is out of scope | open | Deliberate scope exclusion (a correctness boundary, not a gap to close) — admitting non-deterministic grouping into the incremental contract is a product decision this program declined to make. |
| IS-06 | incremental_shapes | CTE-only `event_time_column` references are not yet detected | closed 5b434b0c | - |
| IS-07 | incremental_shapes | Schema evolution on the partition grain is largely a definition delta now (Open Question) | open | Full behavior depends on how definition-delta schema-evolution semantics (still evolving, IM-19/IM-25 residues) interact with the partition grain specifically; needs further design this program didn't do. |
| IS-08 | incremental_shapes | The `smelt.metric()` interaction is unspecified (Open Question) | drifted (accurate) | OQ decided 2026-08-16 (refuse the combination) but unimplemented: `PartitionGrainForbidsMetrics` refusal has no classifier/diagnostic yet — a real, still-open build gap, current wording accurate. |
| IS-09 | incremental_shapes | Per-`ModelDef` overrides for generator-emitted models are not part of the closed field set in v1. | closed 7fc38775 | - |
| IS-10 | incremental_shapes | `g_run >= g_part` auto-coarsening is not implemented (Open Question) | drifted (accurate) | OQ decided 2026-08-16 (reject with suggestion) and landed, but the rejection doesn't yet name the coarsened window — a narrower still-open gap, current wording accurate. |
| IS-11 | incremental_shapes | Monotone-integer `partition_column` has no end-to-end run | closed 0cd4cbf4 | - |
| IS-12 | incremental_shapes | A window-forward keyed run with no event-time window silently full-refreshes instead of refusing | closed af20dfe3 | - |
| IS-13 | incremental_shapes | The once-write classifier has no nullability route around the fallback case | open | Implementation gap in an already-decided classifier design; unscheduled follow-on work, not a live question — named in the out-of-scope spot-check as still missing. |
| IS-14 | incremental_shapes | A re-run-tolerant keyed model keeps no ledger at all unless additive-graded (Open Question) | closed af20dfe3 | - |
| IS-15 | incremental_shapes | Snapshot-reconcile admits at most one unclocked source in the FROM clause (Open Question) | drifted (relocated) | Moved to §Future Extensions "Multi-source snapshot-reconcile"; `KeyedSnapshotPostureUnsupported` still refuses ≥2 unclocked sources — decision recorded 2026-08-16 to defer widening admission, not to fix; still honestly framed as undecided-future. |
| IS-16 | incremental_shapes | `KeyedRetractableContribution` has no implementation (Open Question) | closed 7871e97d | - |
| IS-17 | incremental_shapes | `safety_overrides:` on a key-addressed model is not a hard error | closed af20dfe3 | - |
| IS-18 | incremental_shapes | The reconciliation ledger's fold is transactional on DuckDB only (Open Question) | drifted (accurate) | Same DuckDB-only gap, bold lead-in reworded with an added clause about the ledger table's existence. This is the bullet `20260815-keyed-grain-residue` phase 3 ("Transactional ledger fold on every shipped backend") is about; that phase is `blocked` and its own decision log honestly states the criterion "deliberately left unmet" — no false closure claim, so criterion 6 does not fire (see §Criterion 6). Not `residue`: the owning outcome never claimed this closed. |
| IS-19 | incremental_shapes | `smelt explain` prints neither the per-column guarantee ledger nor the derivable forward reach (Open Question) | open | "Proofs as product" is a named Future Extension (`smelt prove`, guarantee-summary surface) this program's out-of-scope list explicitly declines to design; printing the ledger/reach needs that surface decided first. |
| IS-20 | incremental_shapes | Key temporal locality route 2 admits only a declared functional dependency (Open Question) | open | Named in `definition-delta-migrate` §Out of scope: widening admission beyond a declared FD is a new-admission-width product decision this program explicitly declined to make. |
| IS-21 | incremental_shapes | Locality machinery gaps | open | Named in the out-of-scope spot-check (route 3 DuckDB-binder limit; granularity/recurrence-precedence underdetermined) — resolving the precedence rule is a design decision not yet made, not an implementation-only gap. |
| IS-22 | incremental_shapes | The derived execution postures are internal, and one of the three is not derived at all | drifted (accurate) | `smelt explain` now prints `Execution postures:` (`20260815-keyed-grain-residue` phase 4, `crates/smelt-cli/src/explain.rs:770-808`); wording narrowed to the still-unbuilt "order-independence is not yet acted on" optimization — real, still-open, current wording accurate. |
| IS-23 | incremental_shapes | The generative conformance pool cannot stage NULL payloads (Open Question) | closed f2b412e0 | - |
| IS-24 | incremental_shapes | Locality open questions (Open Question) | drifted (relocated) | Moved to §Future Extensions "Deletion-adjacent locality relaxations"; same three sub-gaps (recurrence-bound slice pruning, granularity-equality relaxation, slice-scoped deletion) — decision 2026-08-16 to defer rather than implement, still honestly framed as undecided-future. Note: an earlier phase-1/2-planning decision-log entry mislabeled the transactional-fold bullet as `IS-24`; that bullet is actually `IS-18` (see above) — corrected in phase 2, recorded here for the record. |
| IS-25 | incremental_shapes | The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped | drifted (accurate) | Bold lead-in reworded to "The pattern-function template file does not exist"; same unshipped gap, still present. |
| IS-26 | incremental_shapes | Driver granularity is `day`/`week` only (Open Question) | drifted (relocated) | Moved to §Future Extensions "Wider driver granularities"; still day/week only, still honestly framed as undecided-future. |
| IS-27 | incremental_shapes | `--auto` staleness fidelity for all-invertible models is conservative in v1 (Open Question) | drifted (relocated) | Moved to §Future Extensions "Exact `--auto` staleness…"; same gap, still honestly framed as undecided-future. |
| IS-28 | incremental_shapes | Self-referential keyed models are rejected (Open Question) | drifted (relocated) | Moved to §Future Extensions "Self-referential keyed models"; still rejected, still honestly framed as undecided-future. |
| IS-29 | incremental_shapes | Run-pinning alignment is deferred (Open Question) | drifted (accurate) | OQ decided 2026-08-16 (both grains run `NOW()`/`CURRENT_*` as-is, no pinning); current bullet describes the residual divergence ("still rejected in keyed models") with a decision record instead of as an open question — accurate. |
| IS-30 | incremental_shapes | Key deletion is unresolved beyond retention | closed 2e1f6d19 | - |
| IS-31 | incremental_shapes | Ladder rungs 3–4 remain specified ahead of this profile's use of them | open | Named in the out-of-scope spot-check (`definition-delta-migrate` §Out of scope: "Ladder rungs 3-4, group-rung retraction, bounded-domain multiset") — which techniques implement rungs 3-4 semantics is a design decision this program declined to make. |
| IS-32 | incremental_shapes | The `key_per_partition` grain derives no plan | open | Named in the out-of-scope spot-check ("key_per_partition missing plan derivation") — the plan-derivation strategy for this grain is undecided, not merely unimplemented against a settled design. |

### model_properties.md (16)

| ID | spec | bullet lead-in | disposition | evidence |
|----|------|-----------------|-------------|----------|
| MP-01 | model_properties | Several declared proofs have no consumer wired yet. | open | "Proofs as product" (`smelt prove`, `must_hold:`, proof-diff) is a named Future Extension this program's out-of-scope list explicitly declines to design; wiring consumers ahead of that surface decision is premature. |
| MP-02 | model_properties | `EffectiveWindow` and `BoundResult` remain two separate walks (Open Question). | open | Merging the two walks is an architecture decision (single-walk unification) this program did not take on; still framed as an open question in the spec. |
| MP-03 | model_properties | The composition walk is not yet the sole source of every property. | open | Ongoing walk-migration backlog (mechanical port of remaining ad hoc scans onto `analysis/walk.rs`); not a live design question, just unscheduled work. |
| MP-04 | model_properties | Declared source lateness reaches no live scan today (Open Question) | open | Wiring lateness declarations into the live scan path needs a decision on where in the walk it composes; deferred, still framed as open. |
| MP-05 | model_properties | `cumulative.rs`'s whole-SQL window-function admission scan is not yet classified onto the walk | open | Walk-migration backlog (same class as MP-03) — a known leaf-classifier-vs-walk classification task, not a live question. |
| MP-06 | model_properties | `INTERSECT`/`EXCEPT` are unclassified for filter distribution | drifted (accurate) | Scope narrowed by 4f8b9c66 (per-set-operation-arm mutation-sensitivity classification, built independently); the filter-distribution gap itself is still open, current wording reflects the narrower scope accurately. |
| MP-07 | model_properties | Additive-only model-diff can't detect a semantic change under an unchanged expression (Open Question) | open | Stricter diff semantics trade completeness against false-positive rate; the tradeoff is undecided, named as an open question this program did not resolve. |
| MP-08 | model_properties | A keyed-grain output poses no partition-locality question | closed 43a25731 | - |
| MP-09 | model_properties | `MaintenanceSkeletonColumnAdded` is not yet surfaced as an LSP/CLI diagnostic ahead of a run | closed dec07e11 | - |
| MP-10 | model_properties | Skeleton-source closure v1 is restricted to non-aggregating enrichment scopes (Open Question) | open | Widening admission beyond non-aggregating scopes is a new-admission-width product decision, explicitly the kind of choice this program's boundary (§"The outcome") declined to make unilaterally. |
| MP-11 | model_properties | Only one maintenance-cell route consults a declared-RI closure today | open | Migration backlog (extending RI-closure consultation to other routes); not blocked on a decision, just unscheduled. |
| MP-12 | model_properties | Fingerprint projection (P4) has no consumer yet | open | Same "Proofs as product" boundary as MP-01 — premature to wire a consumer ahead of that undecided future surface. |
| MP-13 | model_properties | The append-only posture probe does not consult declared lateness | open | Implementation gap in an already-decided probe design; unscheduled follow-on work, not a live question. |
| MP-14 | model_properties | `SourceUniqueKeyViolated` remains the one probe-registry row with no emitter at all (Open Question) | open | Whether to implement this emitter or accept the coverage gap permanently is undecided; named as an open question this program did not resolve. |
| MP-15 | model_properties | Output-delta shape is derived, typed onto propagation edges, and acted on by dirt propagation, but the keyed dirt-set remains symbolic. | open | Related to the "Lattice v2" (per-column-group freshness) Future Extension explicitly declined by this program's boundary; making the dirt-set concrete needs that wider admission decided first. |
| MP-16 | model_properties | The grammar boundary between `columns.<c>.contract` and a future column `tests:` block is deliberately deferred (Open Question) | open | Explicitly named as a deliberately deferred grammar-boundary product decision — the spec itself frames it as undecided, this program did not adjudicate it. |

## Criterion 2 — owning-outcome closure claims independently confirmed absent from current spec text

`keyed-grain-residue` and `partition-grain-residue` each claim to close specific baseline bullets.
Both sets are verified `closed`/absent from `incremental_shapes.md` §Known Divergences in the
current (HEAD) text, per `check-classification.sh`'s presence/absence check against
`current-inventory.tsv` (regenerated with `bash extract-baseline.sh HEAD`):

- `partition-grain-residue` claims: IS-01, IS-02, IS-06, IS-09, IS-11 (partition-grain classifier
  fixes) — all `closed`, all absent from current `incremental_shapes.md` §Known Divergences
  (`check-classification.sh` line-by-line grep confirms no `closed`/`residue` row's baseline
  lead-in survives verbatim in `current-inventory.tsv`).
- `keyed-grain-residue` claims: IS-12, IS-14, IS-16, IS-17, IS-23, IS-30 (keyed-grain classifier
  and ledger fixes), plus IS-22 (execution-posture printing, `drifted`/`accurate` — the print
  landed, the residual optimization gap is correctly still described) — all closure claims for
  fully-closed IDs are absent from current §Known Divergences; the one bullet this outcome did
  *not* close (IS-18, transactional ledger fold) was never claimed closed (see §Criterion 6).

`check-classification.sh` enforces this mechanically for all 35 `closed` + 0 `residue` rows across
all four specs, not just the two owning outcomes' claims; re-run in phase 4 and again this phase —
green both times.

## Criterion 3 — out-of-scope spot-check

Full per-item table is `baseline-inventory.md` §"Out-of-scope spot-check" (25 items from
`20260815-definition-delta-migrate` §Out of scope, each checked: still present, still tagged/framed
as claimed, behaviour still missing). Result: 24/25 `accurate`, 1 stale **in the out-of-scope
prose itself, not in any anchor spec**: the `docs/plans/20260704-model-updates.md` item's claim
that D1-D3's fate is "unclear ... individually" no longer holds for D3 (`refresh:
materialized_view`, now fully specified and shipped as `docs/specs/materialized_view.md`); D1
(`latest_value`)/D2 (`versioned`) remain genuinely absent and unclear. No spec bullet claims D3
undecided, so nothing needed fixing under the spec-only-if-trivial rule — recorded as a footnote,
not a new row or `## Blocked` entry (no product decision needed; D3 is already decided and
shipped).

## Criterion 4 — `/smelt:validate` drift reports

Four full-spec sweeps, `docs/validations/2026-09-04-<slug>-closure.md`:

| Spec | Drift found | Disposition |
|------|-------------|-------------|
| `definition_deltas` | 0 | - |
| `incremental_models` | 0 | - |
| `incremental_shapes` | 1 (stale §References Code path: `windowing.rs` no longer holds `PartitionAxis`/`resolve_scan_window`, moved to `analysis/partition_axis.rs`/`analysis/source_bounds.rs`) | fixed this phase (phase 4) |
| `model_properties` | 1 (Surface + §Semantics "Event-time monotonicity trace" hadn't caught up to the shipped `Offset::Integer` variant, commits `98393e25`/`cc75fe58`) | fixed this phase (phase 4) |

Both findings were doc/wording drift, fixed inline in phase 4 (no new phase row, no `## Blocked`
entry needed). Every already-flagged-open bullet from `baseline-inventory.md` was cross-checked and
correctly *not* re-litigated as drift, per phase 4's rule that a bullet the spec already flags
open/relocated is not itself drift. `check-validations.sh` (all four reports present, all seven
required sections, no undispositioned ❌ line) — green.

## Criterion 5 — standing-gate suite (this phase's run)

| Gate | Command | Verdict |
|------|---------|---------|
| Full local gate | `bash .claude/scripts/verify-phase.sh` | PASS (fmt-check, clippy zero-warnings both feature sets, cargo test workspace, example_diagnostics all green) |
| Maintenance conformance | `cargo test -p smelt-cli --test maintenance_conformance` | PASS (75 passed; 0 failed) |
| Statement parity | `cargo test -p smelt-runtime --test statement_parity` | PASS (33 passed; 0 failed) |
| Walk coverage | `cargo test -p smelt-logical --test walk_coverage` | PASS (4 passed; 0 failed) |
| Execute parity | `cargo test -p smelt-runtime --test execute_parity` | PASS (4 passed; 0 failed) |

All five gates run in the foreground in this phase (not cited from an earlier phase), per the
phase-5 plan.

## Criterion 6 — no false-closure residue

Phase 2 classified all 80 baseline bullets with **zero** `residue` verdicts: every bullet an owning
outcome's decision log claims closed was independently confirmed against the repo (code, tests, or
a landed decision record), not taken on the outcome's own say-so.

The one candidate case flagged at phase 1 — `IS-18` (the reconciliation ledger's transactional
fold, DuckDB-only), which `20260815-keyed-grain-residue` phase 3 ("Transactional ledger fold on
every shipped backend") is blocked on — turned out **not** to be `residue`: that outcome's own
decision log honestly states the criterion is "deliberately left unmet" rather than claiming
closure. Since no outcome falsely claims IS-18 closed, success criterion 6 does not fire and no
owning outcome needs to be reopened. IS-18 is correctly classified `drifted`/`accurate` in
§Criterion 1 above — a real, still-open, honestly-described gap.

## Summary

All six success criteria are met:

1. 80/80 IDs enumerated above with disposition and (for all 29 `open` rows) a stated reason —
   `check-closure-report.sh` green.
2. Both owning outcomes' closure claims independently confirmed absent from current spec text.
3. All 25 out-of-scope items spot-checked; one prose-only staleness found and footnoted (no spec
   edit needed).
4. All four `/smelt:validate` sweeps report 0 drift after phase 4's two inline fixes;
   `check-validations.sh` green.
5. All five standing gates PASS, run fresh in this phase.
6. Zero false-closure residue; the one blocked-outcome bullet (IS-18) never claimed closure, so no
   reopen is needed.

**Outcome status: done.**
