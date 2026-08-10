# Phase 4 plan — shape profiles redraft + the composed key+time composition table

## Objective

Redraft `docs/specs/incremental_models.md` §Semantics lines 1195–1851 (`### Shape profiles`
through `### Interactions`, 657 lines) into ≤ 330 lines, keeping every normative claim. The
composed key+time corner — today three prose sections making overlapping claims
(`#### Key temporal locality (the time-partitioned output)`,
`#### What the composed shape enables`, `#### Key-grain output shape`) — is stated as **one
composition table** plus its local machinery. Advances criterion 3 (no restatement essays) and
the outcome statement's "substantially reduced length, reads as always designed this way";
touches criterion 4 only as a passive grep.

## Spec delta

Structural redraft only — no user-visible behaviour change, so no spec-delta-first step beyond
the redraft itself. Two boundaries the implementer must **not** cross:

- The dead `IncrementalStrategy` variants (`Append`, `InsertOverwrite`, current spec line ~1266
  `#### Strategy enum (backend-internal)`) are owned by phase 7's fossil removal. Phase 4
  preserves those claims verbatim-in-substance; it does not delete them.
- `grain: key_per_partition` mentions in this range (if any) likewise stay; phase 7 retires them.

## Heading rule (blast-radius checked this phase)

Preserve every existing heading string in the range **verbatim** unless it has zero `§"…"`
citations outside `docs/plans/`, `docs/research/`, and `docs/outcomes/`. Verified by `rg` while
planning:

- `Key temporal locality (the time-partitioned output)` — cited ~15× across
  `crates/smelt-logical/src/maintenance/locality.rs`, `propagate.rs`, `smelt-cli/src/explain.rs`,
  and docs-site. **Preserve verbatim** (both the bare and parenthesized forms are cited).
- `Key-grain output shape` — cited by `crates/smelt-cli/tests/example_diagnostics.rs:2713`.
  **Preserve verbatim** (may be demoted to a `####` child).
- `What the composed shape enables` — only citation is `docs/plans/20260722-…` (historical).
  **Free to absorb** into the composition table.

Every other heading in the range keeps its exact string; consolidation happens by demotion under
umbrella headings (the phase-2 precedent), never by renaming.

## Tests

Executable checks in `phases/04-check.sh`, modelled on `03-check.sh` (reuse it verbatim where the
check is range-independent). Confirm each is RED on the unedited spec where it can be, before
editing:

1. `structure` — the range's expected heading list, in order, at the expected levels (red before).
2. `no_orphan_refs` — every `§"…"` reference in the corpus (crates, docs/specs, docs-site, root
   `CLAUDE.md`) that names a heading of `incremental_models.md` resolves to a live heading.
3. `claim_inventory` — `phases/04-claims.md` exists and every claim ID it lists is accounted for.
4. `diagnostic_codes` — the set of diagnostic codes named in the range is unchanged pre/post.
5. `budget` — range ≤ 330 lines (red before: 657).
6. `timeless` — `rg 'Phase [A-Z0-9]|Historical name|pre-cut|ratified|category error'` empty over
   the whole file.
7. `no_split_code_spans` — no backtick span broken across a wrap (phase-3-inherited hazard).
8. `composition_table` — the composed key+time corner is expressed as exactly one table; the
   three overlapping prose sections' bullet lists are gone.

## Tasks

1. Inventory every normative claim in lines 1195–1851 into `phases/04-claims.md` (numbered, with
   source line ranges and the diagnostic-code list at the top) **before** editing. Parallelise
   across subagents by subsection group.
2. Write `phases/04-check.sh`; run it and record which checks are red on the unedited spec.
3. Redraft `### Shape profiles` intro (~15 lines) and `### The partition grain`: keep the
   composition table; trim `#### Execution model` / `#### Strategy enum` to what is **not**
   derivable from the emitters themselves, per the phase-1 outline; keep all other headings.
4. Redraft `### The key grain`: build the single composed key+time composition table — rows are
   the capabilities (propagation admissibility, key→partition dirt projection exactness, no-op
   write-elimination bound, settle-bound × observed-delta composition, consumer-visible output
   shape, merge-target scan pruning); columns are *bare keyed* / *composed keyed+time*, where it
   binds (graph layer / statement emission), and the route dependency (routes 1–2 exact vs route
   3 widened). Locality's structural preconditions and three routes stay as prose under the
   preserved `#### Key temporal locality (the time-partitioned output)` heading.
5. Redraft `### Interactions` to the outline's short cross-shape note (~10 lines).
6. Run an **independent adversarial-verify subagent** over all inventoried claims
   (preserved / weakened / lost / strengthened). Restore everything not `preserved` — budget a
   second restore pass, per the phase-3 lesson; never close the budget by cutting content again.
7. Re-run `04-check.sh` green, then the full gate.
8. Write `phases/04-summary.md` (shipped / decisions / for-the-next-planner / gates).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/04-check.sh` → all eight PASS.
- Adversarial claim verification → 100% `preserved` after restores; report the first-pass grade.
- `bash .claude/scripts/verify-phase.sh` → PASS (set `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/
  `LIBRARY_PATH` to `~/.local/lib/duckdb`).

## Commit message

`docs(incremental-spec): redraft the shape profiles around one composed-corner table`
