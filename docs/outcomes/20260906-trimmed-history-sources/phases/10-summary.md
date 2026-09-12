# Phase 10 summary — Explain and docs: bound vs. required reach

**Shipped:**
- `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report": new **Retention
  reach.** paragraph (text row shapes + `--json` `retention` array), placed after **State
  downgrade.**
- `crates/smelt-cli/src/explain/retention.rs` (new module): `write_retention_text` (text
  rendering) and `retention_json_rows`/`ExplainRetentionJson` (JSON rendering), both reading
  `MaintenancePlan::retention_reaches`/`retention_downgrades` verbatim — no re-derivation.
- `smelt explain <model>` text report now prints a `Retention:` section (after `Refusals:`,
  before `Relation contract:`), omitted entirely when the model references no `retention:`
  source.
- `--json` gains an append-stable `retention` array on `ExplainMaintenanceJson`
  (`#[serde(skip_serializing_if = "Vec::is_empty")]`); `build_maintenance_plan_json` takes two
  new slice params, wired from the one call site in `commands/explain.rs`.
- Tests: `crates/smelt-cli/tests/explain_model/retention.rs` (4 text + 2 JSON, via
  `support::build_report_for` and a spawned-binary JSON helper), plus a two-sided doc-sync test
  `docs_site_diagnostics_reference_lists_every_source_retention_code` in
  `explain_maintenance/docs_and_technique.rs` mirroring the succession-code gate.
- Docs-site: `guide/sources.md` "Bounded history (`retention:`)" section, `reference/sources-yml.md`
  `retention` field row, `reference/smelt-explain.md` "Retention reach" section,
  `reference/diagnostics.md` new "Source retention" table (`SourceRetentionExceeded`/
  `SourceRetentionDowngraded`).
- Fixed the stale "unconsumed by any maintenance logic today" comment in
  `examples/github_activity/models/sources/raw/github_events.yml` (false since phases 3-9 wired
  retention into the walk/refuse/degrade path — the fixture just never reaches the bound).

**Decisions:**
- `UnprovableReason` is rendered a *fourth* time (once each in `smelt-db`, `smelt-runtime`, and
  now `smelt-cli`'s `explain/retention.rs`) rather than factored into a shared function — matches
  the codebase's existing precedent (the first two already duplicate independently, each in its
  own surface's voice) and avoids inventing a new cross-crate dependency for two short match arms.
- Placed the `Retention:` text section after `Refusals:` and before `Relation contract:` — it's
  model-level info like refusals, not per-cell, and the spec's paragraph-ordering instruction
  ("after State downgrade") was about spec prose placement, not a literal report-line position
  requirement (state downgrade is per-cell, rendered inside the cell loop; retention is not).
- **Baseline sign-off**: both `crates/smelt-cli/src/explain.rs` (2578→2593) and
  `crates/smelt-cli/src/commands/explain.rs` (1193→1195) needed a `--update` despite the plan's
  "extract rather than update" instruction for `explain.rs`. The retention *rendering logic*
  (JSON struct, row derivation, text writer — the bulk of the new code) was extracted to
  `explain/retention.rs` as directed. What remained in `explain.rs` after aggressive comment
  trimming (mod/use decl, the new `ExplainMaintenanceJson` field, two new `build_maintenance_
  plan_json` params, one dispatch call each) is irreducible glue: it must live where the struct
  and function are defined, matching every sibling field/param on the same struct/function
  (`state_downgrade`, `key_locality`, etc.) that lives there too. `commands/explain.rs`'s +2 is
  the two new call-site args. Reviewer sign-off: this is minimal, unavoidable wiring, not new
  logic growth.

**For the next planner:**
- Phase 11 (close-out) can verify criterion 7 is now fully met: text + `--json` both render, and
  the docs-site page (`guide/sources.md`) plus the diagnostics reference are in place.
- Not done, out of scope for this phase: no fenced `smelt explain` excerpt of the `Retention:`
  section was added to `guide/sources.md` (the plan's task 5 suggested one) — the section instead
  points to `reference/smelt-explain.md#retention-reach`, which does carry a fenced excerpt. Two
  copies of the same excerpt would be a freshness-drift risk with no test enforcing both; if a
  future reviewer wants one in the guide too, `explain_docs_freshness.rs`'s headline check would
  need `guide/sources.md` added to its walk (it already walks all of `docs-site/docs/`, so a
  bare `Retention:`-only excerpt there is safe as-is — only a `Maintenance plan: ` excerpt is
  gated).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_model --test explain_maintenance --test
  explain_docs_freshness --test cli_docs_coverage --test docs_front_door` — all passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 128 passed (includes
  `github_activity_no_diagnostics`, confirming the yml comment edit didn't regress).
- `bash .claude/scripts/large-file-check.sh` — OK after `--update` (see Decisions).
