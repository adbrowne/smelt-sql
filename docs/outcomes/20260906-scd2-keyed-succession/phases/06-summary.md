# Phase 6 summary — BLOCKED

## Shipped

Nothing kept in the tree. All exploratory test code was reverted
(`git checkout --` on `crates/smelt-cli/tests/maintenance_conformance/probes.rs`);
the working tree is byte-identical to the pre-phase commit.

## Decisions

- Did not force either planned test to pass by asserting on the wrong error
  or by silently widening scope to add unplanned runtime wiring. The plan's
  own task list forbade re-implementing/re-testing the generic probe
  mechanism at the unit level, and adding production dispatch wiring
  unplanned, with no unit tests of its own, would be worse than blocking.
- Did not weaken the mutation-control test's assertion to accept whatever
  error currently surfaces (`SuccessionClockTie`) — that would document a
  coincidental side effect, not the probe obligation the plan and the spec
  actually describe.

## For the next planner

**Root finding:** the append-only posture probe's live dispatch
(`crate::source_probes::{append_only_posture_probes, dispatch_and_record_append_only_postures}`)
is only called from the ordinary keyed/partition-grain path in
`crates/smelt-runtime/src/execute/project/mod.rs` (two call sites, both
inside `match plan.incremental`). The succession-patch dispatch block
(`resolved_grain().is_none()` branch, same file, ~line 2280) never calls
either function and hardcodes `probes: Vec::new()` on the model's
`ModelRunRecord`. Concretely: a succession model's declared
`mutation_profile: append_only` source posture is not verified at runtime
today — neither the "late append is an observation" leniency nor the
"genuine mutation still fails" obligation fires. A mutation that happens to
also collide on `(key, clock)` with an already-presented row incidentally
trips `SuccessionClockTie` instead; a mutation that doesn't (e.g. one not
re-read by the currently-driven window) passes with no probe at all.

This is real, unscoped production work — not a test-writing task — so it
needs its own phase before phase 6 can be attempted as written. See the two
candidate options recorded in `outcome.md` §Blocked (wire the dispatch into
`execute_succession_maintenance`, or re-scope phase 6 to a documented
divergence and defer the wiring). Either way this is criterion-7 work; it
cannot be left out of the outcome.

Everything else phase 6 pre-verified checked out fine on inspection: the
count-gated fingerprint predicate
(`AppendOnlyBaselinePartition::check_fingerprint`,
`crates/smelt-logical/src/maintenance/emit/probes.rs`) and the
count-increase/late-append classification in
`dispatch_and_record_append_only_postures` are both correctly implemented —
they are just never reached for a succession-grain model.

## Gates

Not run to completion — the phase produced no code to gate. The one
ad-hoc check performed (`cargo test -p smelt-cli --test
maintenance_conformance succession_late_append_into_closed_partition_is_re_presented`
and the mutation-control counterpart) was exploratory, on since-reverted
code, and is not part of the committed tree.
