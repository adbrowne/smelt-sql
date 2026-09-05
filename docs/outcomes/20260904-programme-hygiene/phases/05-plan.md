# Phase 05 plan — correct `daily_events_status`'s `ColumnScopedMerge` overclaim

## Objective

`examples/timeseries/models/daily_events_status.sql` and
`models/sources/raw/user_status.yml` assert that MP11's horizon-clamped column-scoped
`MERGE` (F15) fires for this model's `UpstreamMutation` cells. It does not: both cells
derive `Technique::DeleteInsert`. Correct the three comment blocks to the derived
technique and drop the `docs/TODO.md` bullet that records the gap. Serves success
criterion 5's intent (timeseries fixture comments state the technique the fixture
actually derives) — the sibling half the phase-04 summary surfaced — and keeps
criterion 6's verify-phase green.

Docs-only: comment text in two example files plus one TODO bullet. No crate changes,
no `.sql` body, frontmatter *keys*, or YAML *values* touched — only comments.

## Ground truth (confirmed at plan time)

`cargo run -q -p smelt-cli -- explain daily_events_status --project-dir examples/timeseries`:

- `{event_id, event_type, user_id}` on `UpstreamMutation{raw.user_status}` →
  `RecomputeRegion` / `DeleteInsert`, region key `WholeRow`, `locality: partition_local`,
  scan clamp `source=raw.user_status column=changed_at before/after=Seconds(86400)`.
- `{status}` on the same trigger → identical.
- `{*}` on `Backfill` → `RecomputeRegion`/`DeleteInsert`, **not** partition_local
  (`raw.events` is unclocked).

So the `PartitionLocal::Yes` / genuine-`ScanClamp` / clocked-dimension contrast with
`daily_events_enriched.sql` is **true and must be preserved** — it is why this fixture
exists. Only the "column-scoped `MERGE`" / MP11 / F15 /
`execute_column_scoped_merge` dispatch sentences are false.

## Spec delta

None. No user-visible behaviour changes; the specs already describe per-cell admission
correctly (`docs/specs/incremental_models.md` §"Per-cell admission").

## Tests

No new tests. The claim under repair is a comment, and its oracle is `smelt explain`,
run as a verification gate below. `crates/smelt-cli/tests/explain_model.rs` already
pins this model's plan shape and must stay green.

## Tasks

1. Read the derived plan first: run the `smelt explain` command in Verification and
   keep its output to hand while editing.
2. `examples/timeseries/models/daily_events_status.sql` frontmatter comment (~:8-20) —
   keep the `WholeRow` row-identity explanation and the retired-`batched.unique_key`
   note; replace "MP11's horizon-clamped column-scoped `MERGE`" with the derived
   `RecomputeRegion` corner written via `Technique::DeleteInsert` over the clamped
   region, citing `docs/specs/incremental_models.md` §"Per-cell admission".
3. Same file, body comment (~:22-33) — keep the clocked-dimension / `ScanClamp` /
   `PartitionLocal::Yes` contrast with `daily_events_enriched.sql` verbatim in meaning;
   replace the "MERGE ... is wired to dispatch through" clause with what the clamp
   actually buys: the region recompute reads only a clamped window of
   `raw.user_status` instead of the accepted-full-scan corner. State the technique is
   `DeleteInsert`.
4. `examples/timeseries/models/sources/raw/user_status.yml` (~:9-16 and the
   `unique_key:` comment ~:20-24) — the `ScanClamp`/`PartitionLocal::Yes` paragraph is
   accurate, leave it; rewrite the `unique_key:` rationale, which today says the key is
   declared so "the mutation-driven column-scoped MERGE (MP11) can prove its join does
   not fan out". Say what is true: the key licenses the non-fan-out proof the region
   recompute needs, and the derived technique today is `DeleteInsert` — no
   column-scoped `MERGE` is reached for this model.
5. Do not use the literal substrings `MP11`, `F15`, `ColumnScopedMerge`,
   `column-scoped`, `column_scoped` in the rewritten comments — the verification `rg`
   below expects zero hits in both files (phase 4's summary flagged this exact trap).
6. `docs/TODO.md` — delete the whole `**`daily_events_status.sql`'s comments also
   overclaim `ColumnScopedMerge`**` bullet (~:489-504), now closed. Leave every other
   bullet, including the dangling-"composed shape" one, untouched.
7. If any *other* claim in these two files turns out false while editing (e.g. a
   `smelt explain` field that disagrees), do not fix it here: record it as a fresh
   `docs/TODO.md` bullet and note it in the summary.

## Verification

- `cargo run -q -p smelt-cli -- explain daily_events_status --project-dir examples/timeseries`
  — every rewritten claim must match this output.
- `rg -n 'MP11|F15|ColumnScopedMerge|column.scoped|column_scoped' examples/timeseries/models/daily_events_status.sql examples/timeseries/models/sources/raw/user_status.yml`
  — empty.
- `rg -n "daily_events_status.sql.s comments also overclaim" docs/TODO.md` — empty.
- `cargo test -p smelt-cli --test explain_model` — green (this model's plan shape is pinned there).
- `bash .claude/scripts/verify-phase.sh` — green.

## Commit message

`docs(programme-hygiene): correct daily_events_status's overclaimed column-scoped MERGE to the derived DeleteInsert`
