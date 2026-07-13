# Web-analytics tutorial series (docs rewrite)

**Goal.** Replace `docs-site/docs/examples/web-analytics-maintenance.md` with a
multi-page tutorial that a reader with *no prior smelt exposure* can follow.
Target personas: (a) an analyst with SQL but little data-engineering
background, (b) a dbt/SQLMesh practitioner, (c) a PySpark pipeline builder.
Each should finish understanding: why consider smelt, that smelt genuinely
analyzes the query (derives bounds, refuses what it can't prove), and what
"deriving properties" means. Matter-of-fact tone; trade-offs acknowledged.

Docs: docs-only (no product code changes; `generate_tutorial.py` and its
freshness gate are the only code touched).

## Page map

New directory `docs-site/docs/examples/web-analytics/`; nav section
"Examples → Web Analytics Pipeline". The old page is deleted and redirected
(add `mkdocs-redirects` to docs-site deps).

1. `index.md` — **The problem.** Late/duplicated event feed, sessions,
   attribution. The two hand-built extremes (nightly full rebuild vs
   hand-written incremental with magic lookback numbers). Thesis: write the
   plain query; smelt derives the maintenance. Pipeline diagram, series map,
   run-it-locally setup.
2. `first-model.md` — **A first incremental model** (stage `01_first_model`).
   `timeseries:` frontmatter, `smelt build`, `smelt run --event-time-start/
   --event-time-end`, `smelt explain --show-sql`, the DELETE+INSERT pair,
   partitions as the unit of maintenance.
3. `late-data.md` — **Duplicates and late data** (stages `02_dedup_refused`,
   `03_late_data`). Adding the dedup QUALIFY triggers a real refusal
   (captured via `run --dry-run`); the override declares why it's safe.
   Adding the lateness filter makes `explain` read 3 days back — *derived
   from the filter*, not configured. Contrast: dbt `is_incremental()` +
   hand-chosen lookback; Spark equivalent.
4. `sessions.md` — **Sessionization and the cross-midnight backfill**
   (canonical workspace). Why any bounded sessionizer must cut; the
   clock-anchored rule; the payoff demo: a day-D run rewrites day D−1's
   partition (real emitted bounds, 2026-05-04 period). Condensed
   "alternative design" section: root-anchored `sessions_chained`, ordered
   (sequential) execution as the cost, never-idle comparison table.
5. `changing-things.md` — **Backfills, new columns, late updates**
   (canonical + stage `04_add_column`). (a) `backbuild --dry-run` chunked
   range backfill; (b) adding a column: automatic `ALTER TABLE ADD COLUMN`,
   NULL history, `backfill:`; (c) upstream data changed:
   `run --since-upstream --source … --landed …` propagating only affected
   partitions through `events_enriched`.
6. `taking-stock.md` — what you wrote vs what smelt derived (recap table);
   honest trade-offs vs dbt/SQLMesh/dataframe pipelines; where to go next.

Terminology rule: concepts introduced through the problem first; internal
names ("Form B") mentioned once with a link to
`guide/incremental-models.md#form-b--explicit-wherejoin-interval-filters`.
Never use "skew inversion" or "window-independent" without inline plain
definitions (they exist nowhere else in user docs).

## Tutorial stages

`examples/web_analytics/tutorial_stages/<nn>_<name>/` — each a complete,
minimal smelt project (own `smelt.yml`, sources, models), validated by the
real CLI. The canonical example is untouched and stays the endpoint; stages
inline the payload parsing (no `functions/`) so pages 2–3 carry no function
concepts; the sessions page introduces functions where they're unavoidable.

- `01_first_model` — sources + bronze + minimal `events_parsed` (casts,
  JSON projection, amplitude_id; no dedup, no lateness).
- `02_dedup_refused` — 01 + dedup `QUALIFY` without the safety override;
  exists to capture the real refusal message (`run --select … --dry-run`
  exits non-zero with the two-remedy error).
- `03_late_data` — 02 + `safety_overrides.allow_window_functions` +
  the 3-day lateness filter. Equivalent to canonical `events_parsed`
  modulo the payload-parse function.
- `04_add_column` — 03 + one new derived column, for the schema-evolution
  demo. Mechanism TBD during implementation: prefer an offline
  `smelt diff`-based block; execution-backed transcript (datagen tiny scale
  + duckdb CLI) only if diff output is insufficient, and then marked
  `no-ci-verify` so the freshness gate skips it.

## Generator & freshness gate

`generate_tutorial.py` becomes a multi-page renderer; the marker grammar is
extended and mirrored in `crates/smelt-cli/tests/tutorial_freshness.rs`
(which scans all series pages, including fenced blocks indented inside
`??? example` collapsibles):

```
<!-- smelt-generate: [@cwd=<rel-workspace>] [@render=full|skeleton|transcript]
     [@expect-error] [@no-ci-verify] <smelt argv…> -->
```

- `full` — current rendering, plus stripping `--` comment lines (keeping
  `-- trigger:` / `-- chunk` / `-- Would run:` structural markers). Prose
  carries the teaching; the annotated model sources remain the commented
  artifact.
- `skeleton` — the maintenance-window frame only (trigger lines, BEGIN,
  DELETE, INSERT INTO header, output-clamp WHERE, COMMIT) with the SELECT
  body elided to one marker line. Used inline; the `full` version sits in a
  collapsible directly below.
- `transcript` — verbatim stdout(+stderr with `@expect-error`) for
  `run --dry-run` / refusal captures.

Every embedded block stays real CLI output; `--check` covers all pages.

## Review process

Rounds of: 3 fresh-context Sonnet persona reviewers (analyst, dbt/SQLMesh,
PySpark) reading only the rendered pages; one Opus structure/flow editor
(pages + requirements, no repo context); one Opus technical-accuracy
reviewer (claims vs specs, commands vs CLI). Revise between rounds; stop
when persona rounds come back clean (budget 2–3 rounds).

## Status

- [x] Ground-truth verification (CLI flags, schema-evolution behavior,
      refusal message, freshness gate, datagen numbers)
- [x] Stage `01_first_model` scaffolded + `explain` verified
- [ ] Stages 02–04 + validation
- [ ] Generator multi-page rewrite + freshness-gate update
- [ ] Page drafts
- [ ] Review rounds + revisions
- [ ] Old-page redirect, nav update, final gates
