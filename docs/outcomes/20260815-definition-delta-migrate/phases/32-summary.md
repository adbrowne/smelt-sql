**Shipped:** Nothing — no code or spec change. This iteration only diagnosed a process gap.

**Decisions:** None on the underlying `retain_departed` design; that work is untouched.

**For the next planner:** Row 32 (and 33, same defect) was queued directly as `planned` by
the phase-31 close-out commit (`eb0894d0`) without a PLAN step ever writing
`phases/32-plan.md`. The IMPLEMENT contract expects a plan file to already exist for any
`planned` row; none exists here. Flipped the row back to `blocked` (see outcome.md "## Blocked",
2026-09-03 entry) rather than improvising a plan. The next PLAN-step iteration should either
write `phases/32-plan.md` (declaration parsing, oracle transform, probe emitter, and the
`ContractRetainDepartedInvalid` diagnostic for `retain_departed`, per decision record
`docs/research/20260816-open-questions-triage.md`) and flip the row to `planned`, or fold the
work into a later phase if scope shifts. Same check is needed for row 33.

**Gates:** None run — no implementation work was in scope for this iteration.
