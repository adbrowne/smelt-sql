# Phase 2 summary — the contract + plan-matrix core, redrafted around typed deltas

**Shipped:**
- `docs/specs/incremental_models.md` §Semantics, lines 448–833 (386 lines) redrafted into
  `### Typed deltas and the algebraic ladder` (with `#### The equivalence invariant`, `#### The
  algebraic maintenance ladder`, `#### Decomposed state (rung 2) in keyed models`, `#### Validator,
  not chooser` as its children) + `### The contract lattice` + `### The plan matrix` (with `####
  Per-cell admission`) — 297 lines, a 23% reduction, all seven headings preserved verbatim so the
  ~100 `§"…"` citations across sibling specs, root `CLAUDE.md`, and six crates keep resolving.
- The "typed delta" term of art adopted per `phases/01-outline.md`'s terminology table row 2 (ad
  hoc "delta shape"/"dirt" phrasing replaced), cited to its owner
  (`model_properties.md` §"Output-delta shape").
- The rejected `<model>__state` separate-table alternative moved out of §Semantics into a new
  `## Design` paragraph (next to the sibling per-cell-addressing decisions), per the craft doc's
  "rejected alternatives belong in §Design" rule.
- `phases/02-claims.md` — a 51-claim inventory of the pre-edit range, and
  `phases/02-check.sh` — six executable red-green checks (structure, orphan-refs, claim-inventory
  fixture, diagnostic-code survival, budget, timeless-grep), committed alongside the redraft.

**Decisions:**
- Compression came from three sources, in order of yield: (1) dropping literal restatements (the
  plan matrix's closing paragraph no longer re-derives "Validator, not chooser"'s rule, just cites
  it); (2) tightening connective prose while preserving every strength word and diagnostic code
  verbatim; (3) reflowing prose to a consistent ~99–102 char width, which alone recovered ~20 lines
  of accumulated ragged-wrap slack with zero content change — the highest-yield, lowest-risk lever
  once semantic trimming hit diminishing returns. Two reflow passes broke an inline `S_H = { … }`
  math span across a line boundary; caught by manual diff review and fixed by hand, not by the
  automated checks (worth a `no code-span crosses newline` grep if this method is reused).
- Per the plan's explicit content rule, the interchangeability paragraph in `#### Per-cell
  admission` was intentionally compressed to a citation of `§"Validator, not chooser"` rather than
  restating "may change freshness, never observable bits at a fixed S" a second time.

**For the next planner:**
- The claim-inventory + adversarial-verify method (`docs/specs/CLAUDE.md` §"Large redrafts")
  worked as documented: an independent subagent grading all 51 claims found exactly one real
  regression (claim 21 — the `merge_into` loop / optional presentation view mechanism detail was
  dropped from the ladder-boundary sentence), fixed in this phase. Reuse the same claims file
  format for phases 3–7; each phase should produce its own `NN-claims.md` before editing.
- Phase 3 (write addressing + repair family) and phase 4 (shape profiles) inherit the same reflow
  technique if their budgets are tight — a Python `textwrap`-based per-paragraph reflow (skip code
  fences, tables, headings; treat numbered/bulleted items as hanging-indent paragraphs) at a fixed
  width close to the file's existing max line length. Manually re-check math/formula spans after
  reflowing, since `textwrap` doesn't know inline-code spans shouldn't break.
- Not done here (deliberately out of scope for phase 2): §Design's "axes compose; exclusivity is
  the recurring error" polemic (line ~2053, owned by phase 4) and the `nondeterministic_columns`/
  `batched.*`/`grain: key_per_partition` fossil removals (owned by phase 6) — both untouched.
- The local dev environment's `DUCKDB_LIB_DIR` was pointed at `/usr/local/lib`, which has no
  `libduckdb.so`; the actual install is user-local at `~/.local/lib/duckdb`. Worth fixing the
  shell profile so future phases don't rediscover this.

**Gates:**
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/02-check.sh` → all six PASS
  (structure, no_orphan_refs, claim_inventory, diagnostic_codes, budget [297 lines], timeless).
- `bash .claude/scripts/verify-phase.sh` (full) → PASS (fmt, clippy zero-warnings, `cargo test`
  workspace, `example_diagnostics`), after correcting `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/
  `LIBRARY_PATH` to `~/.local/lib/duckdb`.
- `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_models.md` → empty.
- Adversarial claim verification (independent subagent against `02-claims.md`) → 51/51 preserved
  after one fix.
