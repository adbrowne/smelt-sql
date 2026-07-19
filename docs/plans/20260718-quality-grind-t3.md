# Plan: Quality Grind Tier 3 — ratified decisions (comma-joins, P7d close-out)

**Date**: 2026-07-18
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) §"Constraints & Invariants" item 13 (dialect conformance); [`docs/specs/meta_config_loading.md`](../specs/meta_config_loading.md) (close-out only — surface already landed)
**Spec diff**: none for the meta close-out (P7d already implemented and spec'd, `meta_config_loading.md` Deltas). Comma-join semantics ratified 2026-07-18 (master D-QG-2): comma-separated FROM items are cross joins.
**Master**: [`docs/plans/20260718-quality-grind.md`](20260718-quality-grind.md)
**Tracking PR / branch**: `worktree-roadmap_todo`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

Same conventions as [`20260718-quality-grind-t1.md`](20260718-quality-grind-t1.md),
including the ledger-phase convention (re-verify entries reproduce before coding; close
entries; both compat gates re-run). The decisions in this plan are **ratified** (master
§"Tier 3 — decision queue") — do not re-open them; if implementation surfaces a genuine
contradiction, mark the phase `blocked` with the evidence.

---

## Context

Two items graduated from the master's decision queue on 2026-07-18. (1) Comma-join
semantics: Andrew ratified `FROM a, b` = cross join — the largest DuckDB-relevant
external-ledger category (25 entries), parked by the 2026-07-12 triage as
inference-semantics territory. (2) The P7c Map-loader decision resolved itself: the
chosen direction (wire Map consumption) was found already landed as P7d (postfix
`MAP_METHOD_CALL` on loader calls, `meta_eval.rs` lowering, green `tenants.sql`); what
remains is stale-state close-out.

## Scope

### In scope
- Comma-join: grammar/AST/printer + join-topology semantics (cross join) + ledger closure.
- P7d close-out: stale comment + stale `docs/TODO.md` §P7c section + spec Deltas check.

### Explicitly deferred
- ON-join `SELECT *` right-side expansion — master D-QG-3, deferred (stays pinned). A
  comma-join phase must NOT change how `SELECT *` expands across ON-joined refs.
- `NOT GLOB` (DuckDB itself rejects it), `smelt bakeoff` (D-QG-4, parked).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | `feat(parser): comma-separated FROM items parse as comma-joins` | 2026-07-19 |
| 2     | done    | `feat(types): comma-joins classified as cross joins with oracle-verified schema expansion` | 2026-07-19 |
| 3     | done    | `docs: close out stale P7c section and meta_config_e2e comment (Map API landed as P7d)` | 2026-07-19 |

---

### Phase 1: comma-join grammar + AST + printer

**Goal.** `FROM a, b` (and mixed forms like `FROM a, b JOIN c ON …`, `FROM t "alias"`
comma style) parse into the existing join CST with a comma-join marker; printer
round-trips to DuckDB-executable SQL. Close the `implicit_cross_join_comma_syntax`
ledger entries that are grammar-only failures.

**Pre-conditions.** None.

**TDD tests to write first.**
- Parser unit tests: `comma_join_two_tables`, `comma_join_three_tables`,
  `comma_join_mixed_with_explicit_join`, `comma_join_with_aliases` — clean parse, CST
  shape assertions, print round-trip.
- Differential seed line: `SELECT * FROM (VALUES (1)) t(a), (VALUES (2)) s(b)` (or an
  equivalent DuckDB-accepted form) so the fidelity direction covers it.

**Implementation shape.** In `crates/smelt-parser/src/parser/select.rs`, after the first
table ref, accept `,` followed by another table ref as a join clause carrying a
comma-join marker (distinct token/flag on the JOIN node so Phase 2 can classify it —
follow how NATURAL/USING joins are marked). Printer emits the comma form back (fidelity:
do not rewrite to `CROSS JOIN` text). Guard against regressing the SELECT-item comma
parse and trailing-comma support.

**Critical files.**
- `crates/smelt-parser/src/parser/select.rs`, `ast.rs` (JoinClause accessor), `printer.rs`, ledger, seed corpus.

**Docs touched.** None (Phase 2 carries the docs).

**Review checklist:**
- [ ] CST shape reuses the join-clause machinery (no parallel comma-list structure)
- [ ] Print fidelity: comma form preserved, DuckDB executes it
- [ ] Grammar-only ledger entries closed; both compat gates green
- [ ] No inference/schema changes in this phase

**Commit.** `feat(parser): comma-separated FROM items parse as comma-joins`

### Phase 2: comma-join semantics — cross-join classification + schema expansion

**Goal.** `JoinClause::join_type()` classifies a comma-join as a cross join (ratified
D-QG-2); schema expansion and nullability treat it as such (all columns from both sides,
no nullability change), verified against the DuckDB oracle. Close the remaining
(semantics-blocked) ledger entries.

**Pre-conditions.** Phase 1.

**TDD tests to write first.**
- `crates/smelt-db` integration tests: `comma_join_schema_both_sides` (`SELECT * FROM a, b`
  infers a's then b's columns — oracle-compare against DuckDB), `comma_join_where_filter_types`
  (the classic `FROM a, b WHERE a.x = b.x` form type-checks), nullability oracle case
  (cross join introduces no nullability).
- Regression pin: `on_join_star_current_behavior_left_side_only` stays green untouched
  (comma-join expansion must not alter ON-join behavior — D-QG-3 is deferred).

**Implementation shape.** `JoinClause::join_type()` in `crates/smelt-parser/src/ast.rs`
returns Cross for the comma marker; `row_extensions` wildcard expansion in
`crates/smelt-db/src/queries/schema.rs` covers comma-joined refs like other join-shared
expansion (no dedup — cross join shares no columns structurally; duplicate *names*
across operands follow whatever the existing multi-ref FROM handling does today — do not
invent new duplicate-name semantics, that is D-QG-3).

**Critical files.**
- `crates/smelt-parser/src/ast.rs`, `crates/smelt-db/src/queries/schema.rs`, `crates/smelt-db/src/type_inference/` (table-ref resolution), ledger.

**Docs touched.**
- `docs-site` SQL-dialect/models page if it enumerates supported join forms (timeless wording).

**Review checklist:**
- [ ] Oracle-compared schema (column order + types exact) for the two-table case
- [ ] ON-join pinned behavior untouched; NATURAL adjacent-operand limitation not worsened
- [ ] All 25 category entries closed or individually re-ledgered with an honest note
- [ ] Property oracles re-run (`type_property_tests`, `nullability_property_tests`)

**Commit.** `feat(types): comma-joins classified as cross joins with oracle-verified schema expansion`

### Phase 3: P7d close-out — stale state hygiene

**Goal.** Retire the stale artifacts left behind by the already-landed Map-consumption
surface (P7d): the out-of-date comment in `crates/smelt-cli/tests/e2e/meta_config_e2e.rs`
(~L25–28, still claiming `tenants` is on `KNOWN_UNBUILDABLE` — it is not, and builds
clean), and the entire stale `docs/TODO.md` §"P7c (diagnostic-parity) — PAUSED" section
(the decision it asks for is moot; record the resolution). Verify
`docs/specs/meta_config_loading.md` Deltas and the docs-site Map pages describe the
landed surface (explorer report says they do — confirm, don't rewrite).

**Pre-conditions.** None (independent of Phases 1–2).

**TDD tests to write first.** None (comment/docs hygiene). Verification:
`cargo test -p smelt-cli --test example_diagnostics` and the `meta_config_e2e` suite
green before and after.

**Implementation shape.** Replace the stale e2e comment with the current truth (tenants
builds via the Map API); rewrite the TODO §P7c section to a short dated resolution note
("resolved by P7d, commit `ab22f990`: option (B) was implemented — Map API postfix calls
on loader results; `meta_config` builds clean"). Cross-check `maps.md`,
`config-loaders.md`, `reference.md` claims against the closed 5-method API
(`entries/keys/values/get/has`) — fix only genuine staleness.

**Critical files.**
- `crates/smelt-cli/tests/e2e/meta_config_e2e.rs` (comment only), `docs/TODO.md`, docs-site meta-language pages (only if stale).

**Docs touched.** As above; timeless wording.

**Review checklist:**
- [ ] No behavioural claims written without re-verification against current code
- [ ] TODO section replaced by a resolution note, not silently deleted
- [ ] Both meta test suites green

**Commit.** `docs: close out stale P7c section and meta_config_e2e comment (Map API landed as P7d)`

---

## Blocked phases

(Append dated entries here; never stop-the-line.)

## Deferred during implementation

(Append-only.)

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-parser-compat --test external_corpus` — `implicit_cross_join_comma_syntax` category empty (or honestly re-ledgered)
- `cargo test -p smelt-db --test type_property_tests` + `--test nullability_property_tests` green
- `cargo test -p smelt-cli --test example_diagnostics` green
