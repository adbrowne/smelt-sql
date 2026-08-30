# Handover: Registry Dialect Emission

Use this prompt to start a new session that executes the plan.

---

## Prompt (copy below the line)

---

Execute the implementation plan at `docs/plans/20260823-registry-dialect-emission.md` using the `subagent-driven-development` skill.

**Key context:**

- The plan has 13 phases (0–12), mostly sequential. Read the "Execution workflow" section at the bottom of the plan for model selection and parallelism rules.
- Branch: `registry-dialect-emission` (create from `main` if it doesn't exist)
- Verification gate between phases: `bash .claude/scripts/verify-phase.sh`
- Each phase has a verbatim commit message in its final step — use it exactly.
- Phases 10+11 (Spark/BigQuery legs) can run in parallel. Everything else is serial.

**Model assignments (Bedrock):**

- Implementer (most phases): DeepSeek V3.2 — it's cheap and the plan has complete code/specs
- Implementer (Phases 4, 6, 12): Claude Sonnet — multi-file integration work
- Task reviewer: DeepSeek V3.2
- Fix-loop escalation (round 4+): Claude Opus 4
- Final whole-branch review: Claude Opus 4

**What's already done:** Nothing. Start from Phase 0.

**Constraints to carry into every dispatch:**

1. Red-green TDD: failing test before implementation, every phase
2. `verify-phase.sh` must pass before commit
3. No silent fallbacks — `Unsupported` emits a diagnostic
4. `smelt-oracle-testkit` is dev-dependency only, no `[[bin]]`, no hardening-baseline row
5. Dialect slugs are exactly `duckdb`/`spark`/`postgres`/`bigquery` — no alternatives
6. Atomic commits per phase using the plan's verbatim `Commit.` message

Begin.
