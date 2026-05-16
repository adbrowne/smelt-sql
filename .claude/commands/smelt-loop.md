---
description: Run the smelt app-build test loop — a sandboxed agent builds a smelt project from docs+skill, the result is validated, and a reviewer proposes skill improvements
argument-hint: [--tier small|medium|large] [--mode local|pypi] [--iterations N]
allowed-tools: [Bash, Read, Edit, Write, Agent, AskUserQuestion]
---

You are running an integration loop that tests **the smelt user experience end-to-end**. A subagent will try to build a smelt project from scratch using only documentation that ships with the smelt CLI plus a `smelt-app-builder` skill. Failures expose tool bugs, doc gaps, or skill gaps — each goes to a different fix path.

## Args

Parse `$ARGUMENTS` for these flags (all optional):
- `--tier <small|medium|large>` (default: `small`)
- `--mode <local|pypi>` (default: `local`; `pypi` is currently disabled)
- `--iterations <N>` (default: `1`)

### Tier overview

| Tier | Surface covered | Union table | Acceptance gate |
|---|---|---|---|
| `small` | Basic SQL models, seeds, smelt.ref | — | Row counts in 3 tables |
| `medium` | + smelt.define typed functions (Expr<T>) | — | Row counts + structural checks |
| `large` | + generator files (`generates: models`), YAML config loader, per-cohort union | `all_orders` | 9-row union = sum of per-cohort shipped counts |

### Large tier

The `large` tier requires the agent to build a workspace that exercises
**Phase E2 multi-model production**:

1. A YAML config (`configs/cohorts.yaml`) listing three cohorts keyed by country.
2. A generator file (`models/cohorts.gen.sql`) with `generates: models` frontmatter
   that loads the YAML via `smelt.config.load_yaml` and emits one `ModelDef` per
   cohort via `|> map(fn c => ModelDef {…})`, filtering shipped orders for each country.
3. A downstream union model (`models/all_orders.sql`) that references the three
   emitted models (`smelt.cohorts.us`, `smelt.cohorts.uk`, `smelt.cohorts.de`).
4. An acceptance test (`tests/cohort_count.test.sql`) verifying the union's
   row count equals the sum of per-cohort shipped order counts.

The acceptance gate in `validate.py` checks:
- `all_orders` (or `all_cohorts`) exists with 9 rows (3 per country × 3 countries).
- Per-country row counts match expectations (3 shipped per country).
- A generator file with `generates: models` frontmatter exists.
- A YAML config file exists under `configs/`.

The fixture's seed CSV (`seeds/raw_orders.csv`) has 12 orders (4 per country,
3 shipped + 1 cancelled each). Expected union after filtering shipped:
- US: 3 shipped rows
- UK: 3 shipped rows
- DE: 3 shipped rows
- Total: 9 rows

If no flags are passed, use the defaults.

## Per-iteration recipe

For each iteration `i` from 1 to `--iterations`:

### 1. Set up a fresh run directory

```bash
ts="$(date +%Y%m%d-%H%M%S)"
run_dir="$HOME/.smelt-test-runs/loop-${i}-${ts}"
bash tests/agent-loop/harness/setup_run.sh <tier> <mode> "$run_dir"
```

This creates `$run_dir/{project,artifacts}`, copies the skill and the fixture's `spec.md`, copies fixture seed CSVs into `project/seeds/`, sets up a `.venv`, installs smelt, and runs smoke checks (`smelt --help`, `smelt docs list`).

If setup fails, **stop the loop** — the harness is broken and we should fix that before running more iterations. Surface the error to the user.

### 2. Spawn the build subagent

Use the `Agent` tool with `subagent_type=general-purpose`. Pass the following prompt verbatim, substituting `$run_dir` with the absolute path:

```
You are building a smelt data project from scratch. Your working directory is <run_dir>/project.

ISOLATION RULES:
- Read only files under <run_dir>/. Specifically:
  * <run_dir>/spec.md — what to build
  * <run_dir>/skill.md — guidance on how to build smelt projects
  * <run_dir>/project/** — your own work
- Do NOT read any other path on the host. The smelt source tree is off-limits even though it exists on this machine.
- For documentation, use the smelt CLI: `smelt docs list` and `smelt docs show <topic>`. Do not read docs from anywhere else.

ENVIRONMENT:
- The smelt CLI is at <run_dir>/project/.venv/bin/smelt; activate the venv or call it directly.
- DuckDB is installed in the same venv for validation.
- Seed CSVs are already under project/seeds/ — do not regenerate them.

YOUR TASK:
1. Read <run_dir>/skill.md fully.
2. Read <run_dir>/spec.md fully.
3. Build the project: write smelt.yml, model files under models/, run `smelt build`, iterate until it succeeds.
4. Run validation: `<run_dir>/project/.venv/bin/python <repo>/tests/agent-loop/fixtures/<tier>/validate.py` — but the harness will also run this for you, so just confirm `smelt build` produces the required tables.
5. When done (passing or stuck), write <run_dir>/retro.md with these sections:
   ## confusion
   What was hard to figure out, and how you eventually resolved it (or didn't).
   ## doc_gaps
   What the docs *should* cover but don't. Cite the closest existing topic.
   ## skill_gaps
   What you wish was in skill.md — concrete, actionable additions.
   ## suspected_tool_bugs
   Behaviour that looked like a smelt bug. Include the exact command and unexpected output.

When finished, return a one-paragraph summary of what you built and what (if anything) you got stuck on.
```

Capture the subagent's transcript / final message and write it to `$run_dir/transcript.md`.

### 3. Run the eval

```bash
bash tests/agent-loop/harness/eval.sh "$run_dir" || true
```

The script writes `$run_dir/eval.json` with `{passed, failed, total, failures}`. Read it. Don't fail the loop on a failed eval — that's the *interesting* case the reviewer needs to look at.

### 4. Decide if the reviewer should run

The reviewer is worth running if **any** of these are true:
- `eval.json` has `failed > 0`
- `retro.md` exists and any section under it is non-empty
- The build subagent's summary mentions getting stuck

If none of those: print "iteration $i passed cleanly, no review needed" and continue to the next iteration.

### 5. Spawn the reviewer subagent

Use the `Agent` tool with `subagent_type=general-purpose`. Prompt:

```
You are reviewing a test run of an agent that built a smelt project using only docs and a skill. Read these inputs:

- <run_dir>/transcript.md — what the build agent did
- <run_dir>/retro.md — the build agent's structured retrospective
- <run_dir>/eval.json — pass/fail per acceptance check
- <repo>/.claude/skills/smelt-app-builder/SKILL.md — the current skill

For each issue you can identify, classify it as exactly ONE of:
- TOOL_BUG: the smelt CLI / runtime did something wrong. Do NOT edit the skill for these. Note them in review_notes.md as "draft issue: ..." with reproduction steps.
- DOCS_GAP: the user-facing docs are missing or wrong. Do NOT edit the skill for these. Note them in review_notes.md as "draft docs PR: ..." with the page that should change and what should be added.
- SKILL_GAP: knowledge that legitimately belongs in the skill (gotcha, idiom, sequencing tip). Propose a SKILL.md edit.

For SKILL_GAP fixes only, write a unified diff against SKILL.md to <run_dir>/proposed_skill_diff.patch. Each diff hunk must be justified in 1-2 sentences in <run_dir>/review_notes.md alongside its issue category.

Constraints on skill edits:
- Keep SKILL.md under 250 lines. If it grows past that, propose moving older detail into references/<file>.md instead.
- Don't restate what the docs already say; the skill is for non-obvious workflow guidance.
- Don't add fixes for things classified as TOOL_BUG or DOCS_GAP.

When done, return a 5-line summary: counts in each category, and whether you produced a skill diff.
```

### 6. Surface the diff to the user

If `$run_dir/proposed_skill_diff.patch` exists and is non-empty:

1. Show the user the diff (read it and print it).
2. Use `AskUserQuestion` with options:
   - **Apply diff** — `cd <repo> && git apply <run_dir>/proposed_skill_diff.patch`
   - **Reject** — copy the patch to `<repo>/tests/agent-loop/regression/cases/rejected-<ts>.patch` for later analysis; do not modify SKILL.md
   - **Edit first** — open the patch in the user's editor (`$EDITOR`); then prompt again
3. Apply only on explicit Apply. If `git apply` fails, print the error and let the user decide.

Also surface the `review_notes.md` content — `TOOL_BUG` and `DOCS_GAP` items are valuable even when there's no skill diff.

### 7. Print iteration summary

```
Iteration $i / $iterations  (run_dir: $run_dir)
  eval: <passed>/<total> passed
  retro: <"yes" | "empty">
  classifications: <T tool_bug, D docs_gap, S skill_gap>
  skill diff: <"applied" | "rejected" | "n/a">
```

## Stop conditions

End the loop early if any of these hold:
- The user rejects three skill diffs in a row (likely a bad direction).
- Three consecutive iterations pass cleanly with no retro signal (loop has converged on this fixture; run a different tier).
- `setup_run.sh` failed (harness broken).

## After the loop

Print a final summary:
- Total iterations run
- Pass rate across iterations
- Skill diffs applied vs. rejected
- Aggregate counts of TOOL_BUG / DOCS_GAP findings (these are follow-up work for the user)

Tell the user where the run dirs live (`~/.smelt-test-runs/`) so they can inspect transcripts and eval output. Each run keeps the agent's full project (smelt.yml, models/, seeds/, the built `.duckdb`) at `<run_dir>/project/` — nothing is auto-deleted, and `~/.smelt-test-runs/latest` symlinks to the most recent setup. To browse the most recent build: `ls ~/.smelt-test-runs/latest/project/`.
