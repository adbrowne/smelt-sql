# Phase 6 plan — Rewrite Known Divergences (both specs) as genuine gap lists

## Objective

Rewrite `## Known Divergences / Open Questions` in `docs/specs/incremental_models.md` (340 lines,
27.7k chars, 60 bullets) and `docs/specs/model_properties.md` (34 lines, 27.7k chars — the
"3,000-char bullets") so each bullet states one live gap, what is missing, and its tracking link —
no landed-work narrative, no settled decision restated as a divergence. Advances criterion 3
(gap-only divergence lists in both specs) and the "seven proofs" half of criterion 4.

## Scope boundaries (do not cross)

- **No fossil removal.** `nondeterministic_columns`, `batched.*`, dead `IncrementalStrategy`
  variants, `grain: key_per_partition` stay named where the current text names them; row 7 removes
  them from parser/config surface. This phase only reshapes their *divergence bullets* into
  gap-first form (e.g. `model_properties.md:351`'s "ratified decision K3" label becomes a plain gap
  bullet: the list form still parses; tracked by row 7's removal).
- **No editing outside the two `## Known Divergences / Open Questions` sections.** The whole-file
  `§"…"` citation sweep is row 8's; fix only citations *inside* these two ranges.
- **No new behaviour.** A gap discovered while inventorying is recorded as a gap bullet, never
  fixed here.

## Spec delta

This phase *is* the spec edit; no user-visible behaviour changes, so no separate delta. Both
sections keep their existing structure: `incremental_models.md`'s three `###` subsections
(`The contract, plan, and graph layer`, `The partition grain`, `The key grain`) survive verbatim;
`model_properties.md`'s section stays flat.

## Tests

Red-green via `phases/06-check.sh` (model on `05-check.sh`; every check must be observed **red at
HEAD** before the redraft):

- `structure` — the three `###` headings survive verbatim, in order, inside
  `incremental_models.md`'s section; `model_properties.md`'s section stays flat.
- `no_landed_narrative` — zero occurrences, in either section, of landed-work vocabulary:
  `is now built`, `are now built`, `is built as`, `are built`, `now wired`, `now unified`,
  `Both triples are landed`, `is landed`, `All seven`, `landed phase`, `remain(s) unconsumed`
  as a build-status report. (Red today: dozens.)
- `no_seven_proofs` — `rg 'All seven|seven .*proofs'` over both spec bodies returns nothing
  (`incremental_models.md:2164`, `model_properties.md:325,327` today).
- `bullet_budget` — no top-level bullet in either section exceeds 1,200 characters (dissolves the
  3,000-char bullets; `model_properties.md:320,325,331,333` are 3–5k today).
- `section_budget` — `incremental_models.md`'s section ≤ 150 lines; `model_properties.md`'s
  ≤ 8,000 characters.
- `gap_claims` — every row of `phases/06-claims.md` marked `keep` has its `rg` anchor present in the
  redrafted text; every row marked `drop` has its landed-work anchor *absent*. Fixture-style, like
  phases 2–5.
- `gap_shape` — every top-level bullet in both sections opens with a bolded gap statement
  (`- **…**`) and contains either a tracking link (`docs/plans/`, `docs/outcomes/`,
  `docs/research/`, or a `§`-cross-ref to a sibling spec's Known Divergences) or the literal
  `(Open Question)`.
- `timeless` — `Phase [A-Z0-9]` in either section only ever on a line that also carries a
  `docs/plans/` or `docs/outcomes/` link (the spec rule's sole tolerance).
- `orphan_refs` — every `§"…"` citation inside the two sections resolves to a real heading in its
  target file (range-scoped, per phase 5's precedent).
- `no_split_code_spans` — no backtick span broken across a line wrap inside the two sections.

## Tasks

1. Inventory both sections into `phases/06-claims.md`: one row per distinct claim — id, one-line
   statement, `rg` anchor, verdict `keep` (live gap) / `drop` (landed-work narrative, no gap
   inside it) / `merge:<id>` (duplicate of another gap). Splitting the giant
   `model_properties.md` bullets means reading each clause and asking "is this a gap, or a build
   report?" — a clause that names something *missing* is always `keep`.
2. Write `phases/06-check.sh`; run it at HEAD and record which checks are red.
3. Redraft `incremental_models.md`'s section: each `keep` becomes a bullet of the form *bold gap
   statement → one or two sentences of what is missing and why it is not unsound → tracking link*.
   Drop landed-work preambles wholesale (the `deferral` bullet's ~15-line "both triples are landed"
   recital; "Proof-layer residues"' build recital; "Emission remainders"' history), keeping only
   their residual gap sentences.
4. Redraft `model_properties.md`'s section: dissolve each 3–5k-char bullet into several short gap
   bullets (`bounded_domain:` has no consumer; `functional_dependency_verdict_over_vector` and the
   once-write enrichment transform are unconsumed; expression-position subquery scopes are not
   walk-enumerated; `cumulative.rs`'s `OVER(` scan is unclassified; the append-only probe ignores
   declared lateness; `SourceUniqueKeyViolated` has no emitter; …). Every "is now built" recital
   with no residual gap is deleted — that history lives in git and §References.
5. Rewrite the two "seven proofs" bullets gap-first with the count dropped (only the keyed-grain
   locality residue and the `MaintenanceSkeletonColumnAdded`-not-surfaced gap are live).
6. Fix any `§"…"` citation *inside* the two ranges that the redraft dangles (e.g. cross-refs to
   `§Known Divergences "Proof-layer residues"` if that bullet's title changes).
7. Adversarial verify: dispatch an independent subagent to grade every `keep` row against the
   redrafted text (`preserved` / `weakened` / `lost`) plus a second question — does any `drop` row
   hide a live gap? Restore every `lost`, every hidden gap, and every high-value `weakened`.
   Require 0 lost before proceeding.
8. Write `phases/06-summary.md` (shipped / decisions / gates / for-the-next-planner, per phase 5's
   shape).

## Verification

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/06-check.sh` → all PASS.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5}-check.sh` → still green
  (this phase's cross-reference retargets can reach into their ranges' citations).
- `bash .claude/scripts/verify-phase.sh` (needs `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH`/`LIBRARY_PATH`
  set to `~/.local/lib/duckdb`).
- Adversarial-verify report recorded in the summary: 0 lost, 0 hidden gaps in `drop` rows.

## Commit message

`docs(incremental-spec): rewrite Known Divergences in both specs as gap-first lists`
