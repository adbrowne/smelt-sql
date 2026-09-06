# Token usage review — outcome-hygiene worktree, 24h window (2026-09-06)

Ad hoc review of `.claude/usage-log.jsonl` in `.claude/worktrees/outcome-hygiene` (branch
`outcome-loop-20260904-programme-hygiene`) for the 24h window ending 2026-09-06T02:00Z, prompted
by a request to find savings — repeated per-session relearning, oversized tool results, and so
on. Not tied to a spec or plan; captured here so the findings aren't lost before acting on them.

## Method

`bash .claude/scripts/usage-summary.sh --since <24h-ago>` failed outright: `usage-log.jsonl:97`
holds two log entries concatenated onto one line (a partial-write race), which breaks every `jq`
pass over the file. Found it with a line-by-line `json.loads` sweep in Python (`jq` alone doesn't
report which line breaks), wrote a cleaned copy dropping that one line, and reran the summary
script plus ad hoc `jq` aggregations against the clean copy. The corrupted line is a hygiene
finding in its own right — see below.

## Headline numbers (24h)

- 48 distinct sessions; 2086 `Bash` / 591 `Edit` / 549 `Read` / 41 `Write` tool calls
- 80 `outcome-implement` + 62 `outcome-plan` headless steps (outcome-loop.sh event names, not
  the autonomy-loop's `headless-iter` — the two loops tag usage differently)
- **Total spend ≈ $335** across the window ($248.60 implement, $86.73 plan)
- `Edit` tool responses alone total **55.7 MB** returned in 24h, vs 4.5 MB for `Bash` and 2.5 MB
  for `Read` combined

## Finding 1 — cost is turn-count-driven, not call-count-driven

Per-`outcome-implement`-step cost tracks `num_turns`, not the amount of new work done:

- median step: 58 turns, $1.68
- worst 5 of 80 steps account for **27%** of implement spend
- single worst step: 308 turns, 67 min, **$19.92**, with 80.97M cache-read tokens against only a
  few thousand fresh input tokens for that step

Aggregate cache-read across all 80 implement steps is **923M tokens**, vs 11.7K fresh input
tokens. Essentially all of the $248.60 implement spend is re-reading an already-accumulated,
ever-growing transcript from cache on every subsequent turn, not paying for new information. A
step's cost curve is dominated by how large its transcript has already grown by the time it hits
turn *N*, which is set by what got embedded into the transcript early — see Finding 2.

## Finding 2 — a handful of oversized source files are what's growing those transcripts

`Edit` on a large file returns the full post-edit file content every time. Six files account for
most of the 55.7 MB of `Edit` response bytes in the window:

| file | size | edits (24h) | max single `Edit` response |
|---|---|---|---|
| `smelt-types/src/signatures.rs` | 351 KB | 29 | 363 KB |
| `smelt-runtime/src/execute.rs` | 332 KB | 11 | 333 KB |
| `smelt-cli/tests/maintenance_conformance/gate.rs` | 288 KB | 10 | 292 KB |
| `smelt-runtime/src/maintenance_driver.rs` | 276 KB | 18 | 281 KB |
| `smelt-db/src/lib.rs` | 212 KB | 12 | 219 KB |
| `smelt-logical/src/maintenance/emit.rs` | 200 KB | 13 | 204 KB |

`signatures.rs` alone was `Edit`-ed by 6 different sessions in the window, each starting cold: a
fresh session `Read`s the 350 KB file once, then every subsequent `Edit` re-echoes ~350 KB back
into that session's own transcript. Inside one long-running implement step, a handful of edits to
one of these files is what pushes the step into the 150–300-turn range where Finding 1's
cache-read cost curve turns steep — the file size compounds with turn count, it doesn't just add
to it once.

This also means the fix isn't "make Edit calls smaller" (nothing to do about a legitimate
targeted edit to one of these files) — it's that these files are too large for the access
pattern the outcome loop puts on them. They're hit repeatedly, by independent cold-start
sessions, over the life of a multi-day programme.

## Finding 3 — real per-session relearning, distinct from Findings 1–2

The DuckDB env-detection snippet from `CLAUDE.md` appears **~95+ times** verbatim across tool
calls in the window:

```bash
for d in /usr/local/lib "$HOME/.local/lib/duckdb"; do [ -e "$d/libduckdb.so" ] && export DUCKDB_LIB_DIR="$d" && break; done
export LD_LIBRARY_PATH="$DUCKDB_LIB_DIR:$LD_LIBRARY_PATH"
```

Cheap per occurrence (a few hundred bytes), but it's pure waste in the sense the user asked
about: every fresh headless session rediscovers and re-executes something that's a fixed fact
about the machine, because `outcome-loop.sh` spawns `claude --print` without the mise-managed
env already exported into that process. This is the same shape of cost as
[[project_duckdb_env_setup]] describes for interactive sessions, just paid once per headless
iteration instead of once per shell.

## Finding 4 — log integrity

`usage-log.jsonl` (2.5 MB / 7215 lines) has at least one corrupted line from a partial write
(`log-tool-call.sh` presumably interleaved two writes). `usage-summary.sh` has no tolerance for a
malformed line — one bad line fails every `jq` pass in the script, silently producing parse
errors that could be mistaken for "no data" if not read carefully (as happened here). The file
also has no rotation; nothing bounds its growth over a long-running loop.

## Recommendations, in expected-impact order

1. **Split the mega-files.** `signatures.rs`, `execute.rs`, `gate.rs`, `maintenance_driver.rs`,
   `lib.rs`, `emit.rs` are all 200–350 KB and all in the hot-edit list. This is the biggest lever
   in the data above — it directly shrinks both the per-`Edit` echo (Finding 2) and the
   transcript-growth curve that drives cache-read cost (Finding 1).
2. **Add a turn/cost circuit-breaker to `outcome-implement` steps.** Steps past ~150 turns show
   clearly diminishing output per dollar. `outcome-loop.sh` already has `ITER_COST_WARN`; a
   companion cap that force-stops a step and hands the remainder to a fresh phase (rather than
   letting one session run to 300 turns) would cap the tail directly.
3. **Export `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH` once in `outcome-loop.sh`**, before invoking
   `claude --print`, instead of relying on every spawned session to rediscover the fallback
   snippet. Small, free, immediate.
4. **Make `usage-summary.sh` tolerant of malformed lines** (skip-and-count rather than hard-fail)
   and add periodic rotation/truncation for `usage-log.jsonl` so the script stays usable on a
   long-running loop.

## Not investigated

- Whether `outcome-plan` steps (median cost not computed; total $86.73 / 62 steps) show the same
  turn-count-driven shape as implement steps — plausible given the same architecture, not
  confirmed here.
- Whether the six hot files are hot because the *outcome* itself concentrates work there, or
  because of how phases are being sliced by the plan step — i.e. whether Recommendation 1 (split
  the files) and a plan-phase-slicing fix are substitutes or complements.
- Cost attribution by individual outcome/phase name — the `outcome-*` events don't carry which
  outcome or phase was running, only `iter`; cross-referencing would need the per-iteration log
  files under wherever `outcome-loop.sh` writes `${log}`.
