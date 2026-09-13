# Phase 10 plan — bank the evidence

## Objective

Close **criterion 9** (evidence banked) and the `.gitignore` half of criterion 10 hygiene:
harvest every defect, divergence, missing emission verdict, unsupported construct and Free
Edition constraint the phases 4c–9f live runs surfaced into one committed handoff; update
`docs/specs/multi_backend.md` §Known Divergences from it; give `docs-site/` a Databricks
backend section; revise `docs/ROADMAP.md` item 11; and stop `.env` being committable. Entirely
offline — no workspace, no credential.

## Spec delta

`docs/specs/multi_backend.md` §"Known Divergences / Open Questions" (the *only* spec edit):

- **Replace** the entry "The Databricks capability matrix column is inherited, not
  independently verified" — it is now false. State instead which flags the live sweep
  exercised (Delta semantics, `merge_into`, column-scoped merge, `IS NOT DISTINCT FROM`,
  schema-evolution DDL) and which remain inherited from the Spark (Delta) column because no
  model in `examples/github_activity` reached them, naming the evidence
  (`phases/08-parity.json`, `phases/09b-equivalence.json`, dated 2026-09-13).
- **Add** an entry for `realisable_state_structures` realising nothing on Spark/Databricks:
  a succession cell is `state_downgraded` there, so it maintains by ledger-free full rebuild
  rather than window-forward patch — correct, but O(source) per window. Behaviour-phrased,
  linking this outcome.
- **Add** an entry for `gold.events_enriched`'s `UnorderedColumnDivergence` — the one
  difference both live sweeps still measure, its bound, and why it is registered rather than
  fixed.
- **Add** an entry for `gold.events_enriched`'s key-addressed model-edge cell being downgraded
  at plan-derivation time on Delta rather than realising a fingerprint sidecar (what phase 7b
  landed), if §"The fingerprint sidecar capability" does not already state it — check first,
  do not duplicate.

Each entry is behaviour-phrased, carries no phase vocabulary (timeless-oracle rule), and
points at `docs/handoffs/2026-09-13-databricks-findings.md`.

## Tests

New file `crates/smelt-core/tests/databricks_docs_freshness.rs`:

1. `docs_site_names_every_refused_databricks_key` — parse the key names out of
   `DATABRICKS_FOREIGN_KEYS` in `crates/smelt-core/src/config.rs` (regex over the source, not
   restated — the const is private) and assert the docs-site Databricks section names each one
   as refused. A new foreign key added without documenting it fails.
2. `docs_site_states_the_databricks_host_rule` — the section states `host` is required and
   bare (no scheme, no trailing slash), matching `validate_targets`' own diagnostic text.

New file `crates/smelt-cli/tests/databricks_findings_handoff.rs`:

3. `the_handoff_names_every_measured_divergence` — every relation in `08-parity.json` with a
   nonzero `duck_only`/`dbx_only`, and every relation in `09b-equivalence.json` with a nonzero
   `incr_only`/`oracle_only`, is named in `docs/handoffs/2026-09-13-databricks-findings.md`.
4. `the_handoff_names_every_excluded_model` — every entry of either report's
   `excluded_models` is named in the handoff, so a model dropped from a sweep is a recorded
   finding rather than a silent gap.
5. `the_spec_no_longer_claims_the_capability_column_is_unverified` — the stale
   "inherited, not independently verified" sentence is gone from `docs/specs/multi_backend.md`
   and the handoff is cited there.

## Tasks

1. Read every `phases/*-summary.md` from `04c` through `09f` plus `free-edition-facts.md`, and
   collect the findings (defect / divergence / unsupported construct / Free Edition fact),
   each with the model and statement that provoked it.
2. Write tests 3–5 red against the absent handoff.
3. Write `docs/handoffs/2026-09-13-databricks-findings.md`, following
   `docs/handoffs/2026-08-16-bigquery-backend.md`'s shape: where we are; what the live runs
   proved; the defects fixed in place to make a run complete at all (fingerprint hash
   dialect dispatch, `DROP_COMMAND_TYPE_MISMATCH`, source-written cast spelling, `epoch_us`,
   `LAG`/`LEAD` frame elision, the succession `state_downgraded` dispatch + ledger-free
   rebuild); the defects **recorded not fixed**; the registered divergences with their
   bounds; the Free Edition constraints that shaped the design; and an explicit "input to a
   follow-on `databricks-correctness` outcome" section listing the punch-list in priority
   order. Do **not** scaffold that outcome.
4. Green tests 3–4.
5. Apply the §Known Divergences spec delta; green test 5.
6. Write tests 1–2 red, then add a `### Databricks` section to
   `docs-site/docs/guide/targets.md` (sibling of `### DuckDB` / `### Spark` / `### BigQuery`,
   placed after BigQuery): the `type: databricks` target shape, `${ENV}`-only token with the
   literal-token hard error, the refused foreign keys, catalog/schema on Unity Catalog,
   serverless-only compute, Arrow-only loading, no cross-engine exchange, and a
   `#### Free Edition constraints` subsection from `free-edition-facts.md`. Green tests 1–2.
   Do not document the ambient-session form — that is phase 11's spec delta.
7. Revise `docs/ROADMAP.md` item 11: Databricks is now a distinct, live-verified backend
   (link the handoff and this outcome); what remains under the item is metrics-view
   compatibility / the Metrics DSL, native IVM (Enzyme), and paid-tier compute shapes.
8. Add `.env` to `.gitignore` (next to the existing local-runtime block, with a one-line
   comment naming it as the wizard library's default `ENV_FILE`); confirm no `.env` is
   currently tracked with `git ls-files .env`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test databricks_docs_freshness`
- `cargo test -p smelt-cli --test databricks_findings_handoff`
- `cargo test -p smelt-cli --test docs_front_door` (the docs-site front door still resolves)
- `cargo test -p smelt-cli --test github_activity_dual_target --test github_activity_dbx_oracle`
  (26/26 and 18/18 — unchanged by this phase)
- `git ls-files .env` prints nothing

## Commit message

`outcome(databricks-dogfood-spine): phase 10 banks the live findings, spec divergences and Databricks docs`
