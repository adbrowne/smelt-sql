# Phase 1 — Spec closure delta

**Outcome:** `docs/outcomes/20260906-scd2-keyed-succession/outcome.md`
**Status:** planned
**Kind:** spec-only. No production code, no new Rust tests, no new diagnostic codes.

## Objective

The branch already carries the six `spec(scd2)` commits, so the succession grain is normatively
specified except for three surfaces later phases must build against. This phase pins exactly
those three — the tombstone ledger's physical shape, the `smelt explain` succession rendering,
and the contract-lattice posture — so phases 3/4/5/8 have an oracle instead of a judgement call.
Advances success criteria 3 (contract refusals), 4 (ledger DDL), 8 (explain), and 10 (closure).

## Spec delta

Three edits, spec-first, all normative. Write them in the timeless-oracle voice — no phase or
plan vocabulary in any spec body.

**(a) `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)"** — add a
**Physical shape** paragraph after the existing **Lifecycle** paragraph:

- The ledger is a **per-model sibling table**, never the shared `_smelt_ledger`. Rationale to
  state: neighbour lookup runs `LEAD`/`LAG` over the union of presented rows and ledger rows
  ordered by `t`, so `k` and `t` must be stored in the model's own column types; a shared
  VARCHAR-keyed table would force a cast into every neighbour lookup.
- Name is derived from the presented table: `<presented table>__tombstones`, in the model's own
  schema. `__tombstones` is a **reserved relation-name suffix**, on the same terms as the
  reserved `__` column suffix that §"Decomposed state (rung 2) in keyed models" already
  establishes.
- Columns are **exactly** `k ∪ {t}` — the classifier verdict's `key_cols` then `clock_col`, in
  that order, each in the model's own inferred type, each `NOT NULL`. Primary key `(k, t)`.
  No payload, no delete flag (every row is a delete by construction), no run-metadata column.
- Lifecycle is tied to the presented table: created with it, dropped with it, rebuilt in the
  same transaction on `--full-refresh` and `smelt repair`, and replaced wholesale by a skeleton
  change. (This restates nothing — the existing Lifecycle paragraph owns *when*; this owns
  *what relation*.)
- Also update the Residency cell of the "Tombstone ledger (succession grain)" row in
  `docs/specs/state.md` §"The state-structure inventory" to name the per-model sibling table.

**(b) `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report"** — add a
succession paragraph plus the `--json` keys. Pin exactly:

- Delta-signature headline `<shape>` for a succession model: `event history keyed by [<k>],
  event-addressed by (<k>, <t>)`. JSON `delta_signature`: `shape: "keyed_succession"`,
  `addressing: "event"`, `keys: [...]`, `axis: "<run axis>"`, `grain: "<label>"`. This is an
  enum-value addition, admissible under the append-stability rule (§Constraints item 5).
- Text block lines, in this order, each omitted when it has no value: `grain: succession`;
  `identity: (<k…>, <t>)`; `technique: succession-patch`; `run axis: <partition_column>
  (arrival-partitioned | event-time-partitioned)`; `clock: <event_time_column>`;
  `posture: re-run tolerant; order-independent but serial`; `pre-window filter: <sql>`;
  `internal state: tombstone ledger <table>__tombstones (<k…>, <t>) — not part of the model's
  public schema`.
- JSON: a per-model `succession` object with `key_columns`, `clock_column`, `run_axis`,
  `partitioning: "arrival"|"event_time"`, `lead_columns`, `lag_columns`, `delete_flag`,
  `pre_window_filter`, `tombstone_ledger: {table, columns}`, and the postures
  `rerun_tolerant: true`, `order_independent: true`, `concurrent: false`. An absent field is
  omitted, never `null`. A recorded downgrade renders through the existing `state_downgrade`
  path — no succession-specific downgrade rendering.

**(c) Contract-lattice posture** — no new lattice point (single-ownership rule).

- `docs/specs/incremental_shapes.md` §"Succession-grain constraints": add constraint **12**
  stating that `contract.frozen_horizon` and `contract.retain_departed` are refused on a
  succession model by the existing rules — `frozen_horizon` because it is admitted only on the
  partition grain (`ContractFrozenHorizonInvalid`, the message naming the succession grain),
  `retain_departed` because it is admitted only on a keyed shape consuming a `mutable_snapshot`,
  which this grain never does (`ContractRetainDepartedInvalid`) — and that `contract.deferral`
  is **admitted with unchanged semantics**, measuring frontier lag against the model's clock,
  which is grain-independent and which a succession model always carries.
- `docs/specs/incremental_models.md` §"Contract relaxations (`contract:`)": one sentence
  recording the same posture, so the lattice's own spec states it.

**(d) One residual bullet** in `incremental_shapes.md` §Known Divergences "The succession grain":
a hand-authored model whose derived table name ends in the reserved `__tombstones` suffix has no
dedicated collision diagnostic. Record it as behaviour ("collides silently"), not as a code to
add — this outcome's code budget is the twelve already specified. Do not touch any other
divergence bullet; phase 10 owns the rewrite.

## Tests

None. This phase is spec-only by construction: every surface it pins is *built* by a later
phase, and each of those phases carries the red-green tests that hold the spec to account —
phase 3 the two contract refusals, phase 4 the ledger DDL shape, phase 8 the explain text and
JSON fields. Writing an assertion here would have nothing to assert against.

## Tasks

1. Read §"The tombstone ledger (hidden state)" and §"Succession-grain constraints" in
   `incremental_shapes.md` in full before editing; the paragraphs must interlock, not restate.
2. Write spec delta (a) — the **Physical shape** paragraph, plus the `state.md` Residency cell.
3. Write spec delta (b) — the succession paragraph and `--json` keys in `cli.md`.
4. Write spec delta (c) — constraint 12 and the `incremental_models.md` sentence.
5. Write spec delta (d) — the one residual divergence bullet.
6. Grep every edited file for `Phase [A-Z0-9]` and plan vocabulary; the timeless-oracle rule
   applies to all of it.
7. Bump `last_reviewed` on each edited spec's header if that spec carries one.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must stay green; a spec-only phase touches no code,
  so a failure here means something unrelated leaked in.
- Manual consistency read: the ledger's column list in (a) matches the classifier verdict's
  `key_cols`/`clock_col` naming in `model_properties.md` §"Keyed-succession classification",
  and the explain fields in (b) name the same things the verdict carries.
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_shapes.md docs/specs/cli.md docs/specs/state.md docs/specs/incremental_models.md`
  returns nothing new.
- Confirm no new `DiagnosticCode` name appears anywhere in the delta.

## Commit message

`spec(scd2): pin the tombstone ledger's physical shape, explain rendering, and contract posture`
