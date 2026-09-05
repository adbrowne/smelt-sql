# Phase 8 summary — docs sweep and closure

**Shipped.** Deleted the stale "specified and unimplemented" `diagnostics.md` bullet for
`PropertyDowngrade`/`PropertyDiffBaselineUnavailable`. Rewrote `property_diff.md` §References
onto the paths that actually exist (R1): `profile.rs`/`diff.rs`/`diff_render.rs` split out
correctly, `commands/explain_diff.rs` not `explain.rs`, `baseline.rs` not a "`workspace.rs`
export helper", every test file that exists added, `smelt-logical/tests/profile_diff.rs` (never
existed) dropped, and "Plans (history)" repointed at `docs/outcomes/20260905-property-diff/
outcome.md` rather than inventing `docs/plans/20260905-property-diff.md` (never written).
Fixed the Overview example's two defects (R2): recounted the summary line to match its five `▼`
lines, moved the `reason:` line off a `cell_technique` change (never reason-bearing) onto a
`refusal_added` line (one of the three dimensions `Change::reason` actually populates), aligned
dimension spellings and the header format with `diff_render::change_line`/`text_report`, and
added the invertibility qualifier to the `SUM`→`MAX` prose so it reads as a feature-class claim,
not a promise about `examples/timeseries`. Corrected `editor-features.md`'s inverted open-buffer
claim (R3): an unsaved edit doesn't *trigger* a refresh, but the next refresh from any cause reads
the buffer, not disk. Added four new Known Divergences bullets (state_downgrade has no example
fixture, no example fixture for a combiner-driven downgrade, the refresh coalescer's
trailing-rerun path is untested, plus the already-present lens-action divergence) and reconfirmed
the five pre-existing bullets are all still live. Bumped `last_reviewed` to 2026-09-06. Added the
ROADMAP completion entry above the August 24 entry, naming the surfaces, standing gates, and open
divergences. Added the missing hardening-baseline sign-off line for the phase-5 `smelt-cli
println 174→175` bump (R7).

**R4 — tower-lsp adjudication, with evidence.** Built two throwaway wire probes (deleted after
use; evidence preserved in `crates/smelt-lsp/CLAUDE.md`): a minimal `initialize`/`initialized`/
`didChangeWatchedFiles` round trip, and a replay of `property_diff_coalescing.rs`'s own scenario
(staged `examples/timeseries` git repo, 10 alternating `.git/HEAD`/`.git/refs/heads/main`
`FileEvent`s in one notification), both driven over the real `tower_lsp::Server::serve` +
`tokio::io::duplex` wire the other `property_diff_*` tests use, dumping the encoded wire body's
event count against `params.changes.len()` logged inside the handler. **Both probes delivered
all 10 events to the handler** — the phase-7 comment's claim ("only the first `FileEvent` of a
multi-event burst reaches the handler over the wire") did not reproduce. Verdict: **not a real
transport defect** — no spec Known Divergence warranted. Recorded the evidence and verdict in
`crates/smelt-lsp/CLAUDE.md`; softened `property_diff_coalescing.rs`'s file comment so it no
longer states the unproven claim as fact (the test's direct-call design stands on its own merits
regardless).

**Timeless-oracle sweep (task 7):** `grep -nE 'Phase [A-Z0-9]+'` over `property_diff.md`,
`diagnostics.md`, `cli.md`, `lsp.md`, and the four referenced `docs-site/` pages — zero real hits
(only the rule's own meta-description in each spec's front-matter callout). `docs/outcomes/` was
deliberately left untouched (R5) — phase vocabulary belongs there.

**`/smelt:validate property_diff`** run and persisted to
`docs/validations/2026-09-06-property_diff.md`. Zero unaddressed drift: every surface/semantics/
invariant item is ✅ or is an already-named Known Divergence (⚠️), never a silent ❌. `cargo test
--workspace` was not run in this session per the phase-8 gate note (another session was
building; the controller runs the full suite).

## The nine-criteria verification table

| # | How verified | Evidence | Verdict |
|---|---|---|---|
| 1 | `PropertyProfile` field list read against §"The property profile"; gate run | `crates/smelt-logical/src/analysis/profile.rs`; `property_profile_parity` 3/3 | **Met** |
| 2 | `diff_purity` (no I/O); exhaustive `Dimension`/`ChangeKind` matches (no wildcard) | `crates/smelt-logical/tests/diff_purity.rs`; `diff.rs` unit tests | **Met** |
| 3 | Baseline module in `smelt-core`; repo-state test; exit-2 mapping | `cargo test -p smelt-core --test baseline` 20/20 | **Met, wording corrected** — Phase 4 reworded "a thin `smelt-db` profile query" to the actual owner (`smelt-runtime`'s `build_model_diagnostics`), because `smelt-db` cannot depend on `smelt-runtime`. The correction is justified (a real layering constraint, not a convenience rewrite) and is recorded in the outcome's criterion-3 text itself — graded against the corrected wording, not silently regraded against the original |
| 4 | Flags + exclusivity + both fixtures | `property_diff_cli` 16/16 incl. the join-induced-downgrade and maintenance-lost fixtures | **Met, example corrected** — Phase 5 replaced the original `SUM`→`MAX` fixture plan with a row-identity-breaking join, because `examples/timeseries`'s only combiner-sensitive cell is a `NewData` fold over an append-only source, which never needs invertibility (Decision log, 2026-09-05). The correction is a verified fact about the feature's sensitivity, not a workaround, and is recorded in both the outcome's criterion-4 text and the Decision log — graded against the corrected fixture, not the disproven original |
| 5 | JSON schema exact match; flags documented | `property_diff_cli.rs` schema test; `cli_docs_coverage` 4/4 | **Met** |
| 6 | Marker + `<details open>` + workflow present | `property_diff_ci_docs` 3/3; `.github/workflows/property-diff.yml`; `ci.md` | **Met** |
| 7 | Lens/diagnostic parity with CLI JSON, proven non-vacuous | `property_diff_parity` 35/35 (sabotage-run-proven, phase 7) | **Partially met** — the lens and `PropertyDowngrade` diagnostic are fully delivered and parity-gated, but the spec's promised lens *action* ("Executing the lens opens the text report… in the editor's output channel") has no emission site in any editor; executing the lens is a no-op everywhere today. Recorded as a Known Divergence in `property_diff.md`. This is the one criterion this closure does **not** claim as fully met (ruling R6) |
| 8 | This phase's own tasks | `docs/validations/2026-09-06-property_diff.md`; ROADMAP diff; this file | **Met** |
| 9 | Staged gate run, all green; hardening sign-off present | Gate block below; `.claude/hardening-baseline.txt` sign-off line added | **Met** |

## Gate results (staged, this session)

`cargo fmt --all -- --check` clean; `clippy-gate.sh` clean on both feature sets;
`diagnostics_catalogue` 1/1; `property_profile_parity`+`property_diff_cli`+`property_diff_ci_docs`
+`cli_docs_coverage` 3+3+16+4 green; `diff_purity`+`walk_coverage` 1+8 green;
`baseline`+`hardening_budget` 20+4 green; `execute_parity`+`profile_workspace` 4+3 green;
`property_diff_parity`+`property_diff_coalescing`+`property_diff_refresh`+`property_diff_overlay`
+`example_workspaces` 35+1+1+1+2 green; `example_diagnostics` 121/122 (1 pre-existing ignore);
`mkdocs build --strict` clean (pre-existing unrelated INFO-level anchor notices only).
`cargo test --workspace` deferred to the controller per this phase's gate note.

## Outcome status

Phase 8 flipped to `done` in the phase table. The outcome's `**Status:**` field is left as
`active`, not `done` — per ruling R6, criterion 7 is only partially delivered (the lens action
is a no-op in every editor), and the plan's own instruction is to leave the status for the
controller to rule on rather than self-grade a partial delivery as complete. Every other
criterion (1–6, 8, 9) is genuinely met, with criteria 3 and 4 met against text that was honestly
corrected mid-outcome (R8) rather than against their original, since-disproven wording.
