# Phase 15 summary — bank the DuckDB-half evidence

**Shipped:**
- `docs/handoffs/2026-09-08-github-activity-findings.md` — the interim findings handoff:
  the four measured root causes (succession naming-tie fold, enrichment freeze, oracle
  windowing gap, mart repair gap), the five `DIVERGENCE_REGISTRY` entries as a table, the
  latent unmeasured clock-tie gap, requirements handed to `external-dag-steps` and
  `trimmed-history-sources`, and a four-item punch-list for `bigquery-correctness`. Every
  claim traces to `phases/0{2,3,4,6,8,9}-summary.md`, the outcome's decision log, or
  `examples/github_activity/README.md` — nothing re-derived.
- 3 new tests in `crates/smelt-cli/tests/github_activity_oracle.rs`:
  `every_registry_entry_is_named_in_the_findings_handoff` (accept direction),
  `findings_handoff_names_no_unknown_relation` (reverse direction, parses the handoff's own
  markdown table), `findings_handoff_declares_its_interim_status` (interim marker + "phase
  16" pointer). All string-level, no DuckDB dependency, no new test target.
- Corrected the stale comment on `examples/github_activity/models/sources/raw/
  github_events.yml`'s `retention: '90 days'` field — it claimed to match the loader's
  trim (45 days) and did not; comment only, value left for `trimmed-history-sources`.
  (`github_events_arrival.yml` carries no such comment — nothing to fix there.)
- Decision-log pointers appended to `bigquery-correctness`, `external-dag-steps`, and
  `trimmed-history-sources`'s `outcome.md` naming the handoff's path and DuckDB-only scope.
- `examples/github_activity/README.md`'s "Trusting the numbers" section now points at the
  handoff.

**Decisions:**
- Handoff's divergence table uses the registry's own underscored relation names
  (`silver_repo_naming`, not `silver.repo_naming`) in backtick-quoted cells, specifically so
  `every_registry_entry_is_named_in_the_findings_handoff`'s substring match and
  `findings_handoff_names_no_unknown_relation`'s table parse both work off one literal
  string — no translation layer between the doc and the registry to drift.
- Did not restate `DIVERGENCE_REGISTRY`'s full predicate parameters (key columns, exact
  columns) in the handoff table — pointed at the registry as the single source of the exact
  shapes, per the plan's "cited by file" instruction, to avoid a second copy of the bound
  definitions going stale.

**For the next planner:**
- Phase 16 extends this same handoff with the live-BigQuery half once a GCP project and
  credential exist — same file, new sections, same interim marker removed on completion.
- Nothing new found this phase; it is a pure harvest of prior phases' findings. The
  punch-list, requirements, and latent gap are all as previously measured — no new
  divergence surfaced.

**Gates:**
- `cargo test -p smelt-cli --test github_activity_oracle` — 10 passed, 1 ignored
  (measurement-only, unchanged), 108.76s.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manual: every numeric claim in the handoff cross-checked against
  `phases/0{2,3,4,6,8,9}-summary.md` and the outcome's decision log; none uncited.
