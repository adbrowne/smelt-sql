# Handoff — Databricks Free Edition dogfood spine

**Date:** 2026-09-13
**Worktree:** `/home/andrew/smelt-sql/.claude/worktrees/databricks-prod`
**Branch:** `worktree-databricks-prod`
**Outcome:** `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`

## Where we are

The `github_activity` pipeline (bronze→silver→gold→mart, 16 models) runs to completion on a
**third target**: a Databricks Free Edition workspace, reached through a first-class
`type: databricks` target rather than a hand-assembled Spark Connect URL. A full refresh and
eleven consecutive incremental windows (`2026-08-05` through `2026-08-15`) all reach 16/16
success. DuckDB and Databricks agree on every model except one already-registered,
already-bounded divergence; three consecutive full-refresh-oracle comparisons confirm the
equivalence invariant holds on this third engine. This document is the evidence bank for
criterion 9 of the outcome above and the punch-list input to a follow-on
`databricks-correctness` outcome, which this outcome deliberately does not create.

```
bash scripts/dbx-auth.sh              # per session: mints a 1h OAuth token
bash scripts/dbx-verify.sh            # reachability + out-of-scope-write refusal
bash scripts/dbx-dogfood-parity.sh    # DuckDB vs Databricks dual-target sweep
bash scripts/dbx-dogfood-oracle.sh    # Databricks incremental vs its own oracle
```

## Defects fixed in place (needed just to get a run to complete at all)

The outcome's brief was record-don't-fix, with one named exception: a defect blocking *every*
run gets fixed, everything else is recorded. Six defects crossed that line, each surfacing only
once the prior one cleared — same "next construct at the same failure point" shape each time:

1. **The fingerprint hash literally spelled `sha256(...)`, which does not exist on
   Spark/Databricks.** `crates/smelt-logical/src/maintenance/emit/fingerprint.rs` built every
   baseline-snapshot and row-fingerprint expression as dialect-unaware SQL text — correct on
   DuckDB and BigQuery (both accept `sha256`/`SHA256`), a runtime `[UNRESOLVED_ROUTINE]` on
   Spark, which spells it `sha2(expr, 256)`. Fixed by a single owner
   (`crates/smelt-logical/src/maintenance/emit/hash.rs`) every hash spelling under
   `src/maintenance/` now routes through, gated by a structural test
   (`hash_spelling_has_one_owner`) so a future hand-spelled `sha256(` under that tree fails the
   build. Blast radius before the fix: every model whose driving source needed an append-only
   baseline snapshot — 3 of 16 directly, 12 more downstream. (Model: `bronze.events`,
   `silver.actor_naming`, `silver.repo_naming`. Phase 6, 6b.)
2. **`DROP VIEW`/`DROP TABLE` didn't recognise Unity Catalog's own error text.**
   `SparkBackend::drop_view_if_exists`/`drop_table_if_exists` swallow a
   drop-type-mismatch error to support the "drop both defensively" bootstrap pattern for
   self-referential models, but only recognised the two message shapes vanilla OSS Spark
   returns. Databricks/Unity Catalog returns a third shape,
   `[DROP_COMMAND_TYPE_MISMATCH] Cannot drop a table with DROP VIEW`, which the string match
   missed — so a real, valid table from a prior run made every self-referential bootstrap model
   fail on the *second* run against an already-populated schema. Fixed with two pure, tested
   predicates (`is_table_not_view_error`, `is_view_not_table_error`) recognising all three
   shapes in both directions. (Model: `bronze.events`, `silver.actor_naming`,
   `silver.repo_naming`. Phase 6c.)
3. **A source-written, unbounded `CAST(x AS VARCHAR)` is rejected by Databricks —
   `DATATYPE_MISSING_SIZE`.** Spark/Databricks' SQL parser treats bare `VARCHAR`/`TEXT` as the
   set-only char/varchar family and refuses it as a cast target with no length; DuckDB and
   BigQuery both accept it unbounded. The output-boundary cast wrap already had per-dialect
   spelling for its own synthesized casts; a *source-written* cast had none. Fixed by giving
   `source_cast_type_sql` (`crates/smelt-dialect/src/type_conformance.rs`) the same per-dialect
   spelling the wrap already had, shared by both call sites so they cannot drift. (Model:
   `silver.actor_sessions`. Phase 6d.)
4. **`epoch_us` had no registry entry at all** (not merely un-emitted for Spark) —
   `examples/github_activity/functions/sessionize.sql` calls it, DuckDB has it natively, Spark
   has no equivalent name and no registry entry means no dialect check ever runs against it, so
   it reached the engine printed verbatim and failed `[UNRESOLVED_ROUTINE]`. Registered as
   `SqlFunction::EpochUs` in the `BuiltinRegistry` — `(Timestamp) -> BigInt`, DuckDB keeps the
   native name, Spark/Databricks emits `unix_micros({0})`, BigQuery emits `UNIX_MICROS({0})`.
   (Model: `silver.actor_sessions`. Phase 6e.)
5. **`LAG`/`LEAD` with an explicit window frame is refused by Spark.** DuckDB accepts a frame
   clause on an offset function; Spark's `lag`/`lead` must run over the implicit default frame,
   no `ROWS BETWEEN`/`RANGE BETWEEN` allowed. `sessionize.sql`'s two `LAG` calls both carry an
   explicit frame. Fixed with a new `RewriteId::ElideWindowFrame` verdict on `(SparkSql,
   Window)` and `(SparkSql, WholePartitionWindow)`, applied live at print time (not a
   pre-planned range list, because the calls live inside a `smelt.define` body inlined by
   textual re-parse at print time) — a named-window `LAG`/`LEAD` (frame on a shared `WINDOW w
   AS (...)` clause) is not eligible and still reaches Spark's own refusal rather than silently
   dropping a frame that mattered. (Model: `silver.actor_sessions`. Phase 6f.)
6. **A succession-grain model duplicated rows up to 7x on the incremental write path when its
   `TombstoneLedger` is unrealisable.** `silver.actor_naming` (`SuccessionPatch` technique)
   downgrades on Spark/Databricks because no `TombstoneLedger` exists there — but
   `resolve_live_succession_cell` treated any downgraded cell as "not live" and fell through to
   the generic `DeleteInsert` driver, which has no `(key, clock)` fold at all, producing up to
   7x duplication of `(actor_id, created_at)` pairs. The full-refresh oracle proved the
   duplication was confined to the incremental write path (a full refresh on the same engine
   did **not** reproduce it), which pointed at a dispatch bug rather than a model-SQL defect.
   Fixed in two steps: (a) the dispatch now stays live for a downgraded cell and forces the
   full-rebuild route instead of falling through; (b) that full-rebuild route
   (`rebuild_succession_state`) itself unconditionally required a `TombstoneLedger` too, so step
   (a) alone still refused live — closed by a new, single-owned, ledger-free emitter
   (`emit_succession_full_rebuild_ledgerless`) that writes the presented arm alone, no tombstone
   table, no clock-tie probe (resolved instead by the fold's own deterministic tie-break). Both
   fixes are `docs/specs/state.md` §"The degradation contract"'s normative behaviour now — the
   cost this contract trades for correctness on a no-ledger backend is a full-table rebuild every
   window rather than a window-forward patch. (Model: `silver.actor_naming`. Phases 9c, 9d
   (blocked), 9e, 9f.)

**Lesson that generalises beyond any one fix (9c → 9d → 9e):** a test that asserts a *dispatch
decision* is not a test that the *dispatched function succeeds*. Two of 9c's own offline tests
asserted the downgraded cell resolved live and was marked for full rebuild — and both passed —
while the function it routed to still refused live. Any future "route X to function Y" fix in a
maintenance driver should include at least one test that actually calls Y against a real
backend, not only one that confirms X routes to it.

## Defects recorded, not fixed (input to `databricks-correctness`)

In priority order — highest-value / most-real-user-affecting first:

1. **`gold.events_enriched`'s `current_repo_name` column diverges between DuckDB and
   Databricks by a small, bounded, unordered set** (9 rows at the smallest measured checkpoint,
   growing to 16 as more fixture days carry more repo renames). Root cause: this model's
   key-addressed model-edge cell is `KeyDiscovery::EnrichmentKeyed` (a value-enrichment join),
   whose ideal technique is `ColumnScopedMerge`. On a backend with no `MergeLedger` (Spark/
   Delta), availability resolution downgrades it to `DeleteInsert` — sacrificing the unwindowed,
   run-level heal a `ColumnScopedMerge` cell performs on every run. DuckDB has the ledger and
   performs that heal; Databricks does not and does not. Registered as
   `DivergenceBound::UnorderedColumnDivergence` in both the dual-target parity suite
   (`crates/smelt-cli/tests/github_activity_dual_target.rs`) and the equivalence-oracle suite
   (`crates/smelt-cli/tests/github_activity_dbx_oracle.rs`). Bound: the column may contain a
   different (but still valid, still currently-correct) `current_repo_name` value for a repo
   that was renamed since the last fully-healed run; row identity and every other column match
   exactly. **Candidate fix:** the 2026-09-13 Catalog Commits research note below may unlock a
   real `MergeLedger` on Databricks specifically, closing this at the root rather than bounding
   it. (Phases 7b, 8, 9b, 9c, 9f.)
2. **Every succession-grain model on a no-`TombstoneLedger` backend rebuilds from the whole
   source on every incremental window, not just the window.** This is now the *documented*,
   correct behaviour (`docs/specs/state.md` §"The degradation contract"), not a bug — but it is
   an O(source) cost per window rather than O(window), and the fixture's scale (tens of
   thousands of rows) hides it. Worth a real cost measurement at a larger scale before this
   trade-off is accepted as a permanent design point rather than a stopgap. (Phase 9e.)
3. **Databricks Connect's `DatabricksSession` builder path has no persistent-session
   option in this tooling.** Every `scripts/dbx-query.sh` call and every internal query the
   Python adapter issues opens a brand-new session; nothing keeps a warm session across calls.
   Cost: ~4.5s of session-bootstrap latency per call, paid every single time. A latency-sensitive
   caller (many small queries in a loop) would need a design change — holding one session open
   across calls — not a config tweak. (Phases 4c, 5.)
4. **`scripts/dbx-key.sh` stores the workspace host WITH its `https://` scheme, but the
   `type: databricks` target's `host:` field requires a bare hostname.** Worked around for
   phase 6 by exporting a second, derived `SMELT_DBX_HOSTNAME` (scheme + trailing slash
   stripped) from `scripts/dbx-dogfood-env.sh` (phase 6b) — this closes the immediate blocker
   but leaves the wizard's own stored value in the scheme-bearing form; a future provisioning
   pass could instead store (or additionally export) the bare form at mint time so no derived
   variable is needed.
5. **`INVALID_HANDLE.SESSION_CLOSED` fires as a `UserWarning` on `adapter.close()` calls made
   shortly after a prior session's teardown.** Confirmed a Free Edition serverless-session
   lifecycle artifact (session teardown can happen on the order of single-digit seconds after
   the last statement), not a smelt defect — every call still returned correct data every time
   it was observed (phases 4c, 5, 6, 9b). Recorded as a Free Edition fact below, not a fix
   target.
6. **The dialect-audit's live cross-engine probe framework has no mechanism to inject an
   explicit window frame into a derived `Position::Window` probe.** `ElideWindowFrame` (finding
   5 above) is verified by targeted unit and integration tests and this outcome's own live runs,
   but not by the standing generative cross-engine audit — a future `RewriteId` needing the same
   "frame variant" treatment will hit the same gap. (Phase 6f.)
7. **Run report `completed_at`/`duration_ms` were unpopulated (`null`/`0`) on the three W1–W3
   incremental-window reports** (phase 7), then populated correctly on the next three (W4–W6,
   phase 7b) with no code change in between. Not investigated — likely coincidental to a prior
   run's in-flight failure leaving the reporter's finalize step unreached, not a real gap. Flag
   for a future timing-based assertion that depends on these fields.

## An open design question, not yet triaged (not a defect)

**Databricks' Catalog Commits (GA, 2026) may unlock a real cross-table transaction on
Delta/Unity-Catalog specifically**, which is exactly the primitive `realisable_state_structures`'
`MergeLedger`/`ReconciliationLedger`/`TombstoneLedger` doc comments cite as permanently absent
on Spark ("per-table atomicity and no cross-table transaction, so a ledger write and its data
write cannot be made atomic"). Catalog Commits makes Unity Catalog the commit coordinator for
Delta and explicitly supports multiple SQL statements across multiple UC-managed Delta tables
as one atomic commit. This is Databricks/Unity-Catalog-specific, not a property of generic
Spark-on-Delta-Lake-OSS — the `SqlDialect::SparkSQL` arm covers both today, so unlocking this
needs either a Databricks-specific capability flag or a confirmed UC-only distinction, never a
blanket flip of the `SparkSQL` row. Unconfirmed: whether Free Edition specifically has Catalog
Commits available/enabled, and whether smelt's current single-Spark-Connect-session transaction
model can drive a coordinated commit from the client side at all. If it holds, it would close
finding 1 above at the root (a real `MergeLedger`) rather than merely bounding it, and would
remove finding 2's O(source)-per-window cost for succession-grain models too. **Flagged for the
next spec/plan pass to triage — not investigated or acted on in this outcome.**

## Registered divergences, with their bounds

| Divergence | Bound | Registry |
|---|---|---|
| `gold_events_enriched.current_repo_name` | `UnorderedColumnDivergence` — value may differ for a renamed repo since the last fully-healed run; row identity and every other column match exactly | `github_activity_dual_target.rs`, `github_activity_dbx_oracle.rs` |

No other relation of the 16-model set diverges at any measured checkpoint. `08-parity.json` (11
days replayed) and `09b-equivalence.json` (three consecutive incremental-vs-oracle checkpoints,
`2026-08-13/14/15`) both confirm this — see `phases/08-parity.md`/`phases/09b-equivalence.md`
for the per-relation tables.

## Free Edition constraints that shaped the design

Full measured detail: `docs/outcomes/20260912-databricks-dogfood-spine/free-edition-facts.md`.
Summary of what shaped design decisions rather than just being trivia:

- **Serverless-only, Unity-Catalog-mandatory.** No host-visible warehouse path, no format
  choice — this is why `warehouse`/`format` are hard-refused keys on a `databricks` target
  rather than tolerated-but-ignored, and why `catalog`/`schema` (not `database`) are the
  addressing fields.
- **Credential: service-principal OAuth M2M**, confirmed by JWT structure (three
  dot-separated base64url segments), not a `dapi…` personal access token.
- **One-hour OAuth token lifetime**, refreshed only by a human-gated `dbx-auth.sh` (needs a
  gpg passphrase). Every live phase in this outcome had to budget around this — a headless
  phase that runs longer than the token's remaining life fails mid-run, not gracefully.
- **Session-per-invocation, no warm-session reuse** in this dogfood tooling (see finding 3
  above) — ~4.5s bootstrap cost per call.
- **`INVALID_HANDLE.SESSION_CLOSED` warnings are cosmetic**, not failures (see finding 5).
- **No fixed storage GB cap** — governed by an account-wide fair-usage policy instead
  (exceeding it suspends compute, does not delete data).
- **Max 5 concurrent job tasks per account; one SQL warehouse capped at `2X-Small`.**

## Next steps, in priority order (for a follow-on `databricks-correctness` outcome)

1. Triage the Catalog Commits question above — confirm Free Edition availability, decide scope,
   spec the capability split before touching `Technique` downgrade logic again.
2. If Catalog Commits does not pan out, treat finding 1 (`gold.events_enriched`) as a permanent,
   registered divergence and finding 2 (O(source)-per-window succession rebuild) as an accepted
   cost — both are already correctly bounded and documented.
3. Give the dialect-audit's live probe framework a way to inject an explicit window frame
   (finding 6), closing the one `RewriteId` this outcome shipped with no standing cross-engine
   coverage.
4. Consider having the provisioning wizard export (or store) a bare-hostname form directly
   (finding 4), removing the derived `SMELT_DBX_HOSTNAME` indirection.
5. A persistent-session option for the dogfood query tooling (finding 3), if a future phase
   needs many queries in a tight loop.
