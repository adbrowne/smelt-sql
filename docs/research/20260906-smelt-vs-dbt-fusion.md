# smelt and dbt Fusion: Two Compilers, Two Questions

**Date:** 2026-09-06
**Status:** Research — comparative analysis, not a plan
**Audience:** anyone deciding what smelt is *for*, given that dbt Labs shipped a Rust SQL compiler first

---

## Abstract

dbt Fusion and smelt independently reached the same architectural conclusion: the
transformation layer should be a compiler with a real understanding of SQL, not a string
templater. Both are written in Rust, both build a typed IR of every model, both ship an LSP.
The convergence is strong evidence the premise is right.

They then pointed that compiler at different questions. **Fusion's analysis serves the
authoring loop** — is this SQL valid, where did this column come from, how fast can I parse
10,000 models. **smelt's analysis serves the maintenance loop** — is this incrementally
maintained table provably equal to what a full refresh would have produced. The distinction
that follows is not "which is better"; it is that Fusion's analysis is *advisory* and smelt's
is *load-bearing*, and almost every other difference — including the enormous maturity gap —
is downstream of that one choice.

---

## 1. The shared claim, and what it actually bought

Both projects claim to replace Jinja-blind execution with SQL comprehension. Both deliver a
dialect-aware parser, cross-model type propagation, column lineage, and editor diagnostics.
Fusion reports parsing a 10,000-model project up to 30× faster than dbt Core, having escaped
the Python GIL.

The 30× number is the first thing to discount, and dbt's own community does. Parsing and
compilation are typically 5–15% of a run's wall clock; the remaining 85–95% is the warehouse
executing SQL that Fusion does not rewrite. Fusion accelerates the cheap half. Its *cost*
story therefore does not rest on Rust at all — it rests entirely on state-aware orchestration
(§3), which is a separate mechanism with separate risks.

The second thing to discount is that "SQL comprehension" is a single feature you either have
or don't. In Fusion it is a three-position dial. `baseline`, the default, emits every finding
as a **warning** and lets invalid SQL run. `strict` — the mode that actually performs type
inference, column-level lineage, and function-signature checking — requires `dbt login`.
And a model using a custom materialization is **silently downgraded to `off`**, a downgrade
that cascades to every downstream dependent regardless of their own configuration.

So: the analysis is real, it is gated behind authentication, and it is defeasible by one
legacy macro halfway up your DAG.

## 2. Advisory analysis versus load-bearing analysis

This is the load-bearing distinction of the whole comparison.

In Fusion, nothing in the runtime *depends* on the analysis being correct. Fusion analyses
your model, tells you what it found, and then hands the warehouse the SQL you wrote. A wrong
inference produces a spurious warning or a missing one. The blast radius of an analysis bug
is developer annoyance.

In smelt, the analysis *licenses a physical rewrite*. The entire incremental subsystem rests
on one equation:

```
incremental_state(S) == full_refresh(inputs ∈ S)
```

A proof — event-time monotonicity, faithful-fold conditions, partition alignment,
mutation-sensitivity, skeleton-source closure — is what permits smelt to substitute a cheap
maintenance statement for a full rebuild. A wrong proof is not an annoying warning. It is a
wrong table that looks correct. The blast radius is silent data corruption.

Three consequences fall directly out of that asymmetry, and they explain nearly everything
else about the two codebases:

**smelt must be fail-closed.** Every proof rejects by default on a construct it cannot decide;
an unprovable shortcut is refused with a diagnostic rather than approximated. Fusion, by
contrast, *can* degrade to a warning, and does — because degrading costs it nothing.

**smelt must carry oracles Fusion doesn't need.** A differential parser gate against live
DuckDB in both directions, a type oracle compared to warehouse schemas exactly, a generative
conformance gate that drives typed model recipes through the real pipeline and asserts the
result equals a full-refresh oracle after every run step. These exist because "we think the
analysis is right" is not an acceptable answer when the analysis rewrites your data.

**smelt is far smaller, and structurally so.** Advisory analysis can ship against 100% of a
dialect on day one by warning where it is unsure. Load-bearing analysis can only ship the
subset it has proven. This is why Fusion covers the warehouse market and smelt covers DuckDB
and Spark. It is not a resourcing gap alone; it is the price of the stronger contract.

## 3. Where Fusion's capability actually stops: incremental correctness

Fusion's answer to cost is state-aware orchestration: fingerprint model code and data state,
skip models whose inputs haven't changed. Marketed as building "only models that will actually
produce a different result."

The mechanism underneath is a freshness signal — `loaded_at_field` or `loaded_at_query` on the
upstream. dbt's own documentation states the hole plainly: when `loaded_at` reflects an *event*
timestamp rather than an ingestion timestamp, a late-arriving record does not advance it, and
state-aware orchestration **may not trigger a rebuild** even though the model's lookback window
would have included those rows. The operator is told to "make sure your freshness logic aligns
with that window" — i.e. correctness under late data is delegated back to the human, in YAML,
where it can silently drift from the SQL it is supposed to describe.

Meanwhile Fusion's celebrated SQL comprehension is not applied to this question at all.
`is_incremental()` blocks remain hand-written Jinja that Fusion renders but does not verify.
`microbatch` — dbt's own abstraction over the hand-written filter — is not implemented in
Fusion; models using it error out, and dbt's stated ambition is to "one day back them with
SQL comprehension." That day has not arrived.

This is the capability gap, stated precisely: **Fusion can tell you your incremental model's
SQL is well-typed. It cannot tell you your incremental model is right.** Nothing in its
architecture is aimed at that question.

smelt's architecture is aimed at almost nothing else. The modeller declares at most two facts
about the *output* — a clock (`timeseries:`) and an identity (`unique_key:`) — and everything
physical is derived: which maintenance technique runs per cell, what each run scans, how writes
locate stored rows, what bookkeeping exists. Delta signatures (shape × addressing) are typed
per column group and composed through the DAG, so a chain of models can be maintained end to
end with the degradation point named. Declarations may only *widen* what proofs admit, never
substitute for a proof, and a narrowing declaration is admissible only if a runtime probe
exists that can falsify it before any write commits. Relaxations of the invariant are not
ad hoc: they are a closed lattice of declared points (`frozen_horizon`, `deferral`,
`retain_departed`), each single-owned as a schema, an oracle transform, and a probe emitter.

There is no equivalent in Fusion, and — importantly — no partial version of it. You cannot add
a correctness contract to a system whose runtime executes user-authored SQL unchanged. That is
a redesign, not a feature.

## 4. Where smelt's capability stops

Honesty in both directions, or this is marketing too.

**Ecosystem.** dbt has packages, adapters for every warehouse, a semantic layer, catalog
integrations, hosted orchestration, and a decade of installed base. smelt has none of these
and no migration path — the no-Jinja decision that makes smelt's analysis *total* also means
smelt cannot run a single existing dbt model. Fusion keeps Jinja and pays for it with
introspection escape hatches that defeat its own analysis; smelt refuses to pay and forfeits
the market that Jinja built.

**Backend reach.** DuckDB and Spark are real. BigQuery is a partial sweep with an open gap
ledger. The PostgreSQL emission dialect was retired in September 2026 as unverified. The
README's "cross-engine deployment — DuckDB, Spark, Postgres, etc." overstates the present.

**The headline differentiator is unbuilt.** Cross-model optimisation — fusing two transparent
models into one execution unit — is listed in `planner_integration.md` as future work, and
cross-model column lineage (the "eclipse" analysis) is an open roadmap item. On this axis
Fusion, which has shipped column-level lineage, is ahead. The README's "learning from history"
is aspirational and should be read as such.

**Maturity.** Every spec in `docs/specs/` except `architecture.md` is marked `experimental`.
This is a one-person project. Fusion is a funded product with a migration story for tens of
thousands of projects.

## 5. Conclusion

The two systems are not competing implementations of the same idea. Fusion is a fast, correct
*front end* to a pipeline whose back end is unchanged: it validates SQL, then executes exactly
what you wrote, and saves money by skipping work on a freshness heuristic the operator is
responsible for aligning. smelt is a *back end* proposition: it treats a maintained table as
state under an equivalence contract and derives the physical plan that upholds it, refusing
when it cannot prove the shortcut safe.

The honest summary for each direction:

- If you have a dbt project, Fusion is strictly better than what you have, and smelt is not
  available to you at any price.
- If your pain is that your incremental models are quietly wrong and nobody can prove
  otherwise, Fusion does not address it, and its roadmap addresses it only as an ambition.

smelt's defensible claim is not "faster" or "no Jinja." It is that **the analysis is
load-bearing** — proofs license rewrites, unprovable cases refuse, and the equivalence
invariant is checked by a generative conformance oracle rather than asserted in a blog post.
That claim is worth exactly as much as the proof coverage behind it, which is why the coverage
ratchets, not the messaging, are the thing to watch.

---

## Sources

- [Meet the dbt Fusion Engine — dbt Developer Blog](https://docs.getdbt.com/blog/dbt-fusion-engine)
- [A new concept: static analysis — dbt Developer Hub](https://docs.getdbt.com/docs/fusion/new-concepts)
- [About state-aware orchestration — dbt Developer Hub](https://docs.getdbt.com/docs/deploy/state-aware-about)
- [Setting up state-aware orchestration — dbt Developer Hub](https://docs.getdbt.com/docs/deploy/state-aware-setup)
- [About microbatch incremental models — dbt Developer Hub](https://docs.getdbt.com/docs/build/incremental-microbatch)
- [Build `microbatch` incremental models into Fusion — dbt-labs/dbt-fusion#12](https://github.com/dbt-labs/dbt-fusion/issues/12)
- [dbt Core v2 is here: still open source — dbt Developer Blog](https://docs.getdbt.com/blog/dbt-core-v2-is-here)
- [dbt Licensing FAQ — dbt Labs](https://www.getdbt.com/licenses-faq)
- [dbt Fusion and dbt 2.0 Explained — Datacoves](https://datacoves.com/post/dbt-fusion)
- [dbt Fusion: A First Look and Hands-On Review — Hiflylabs](https://hiflylabs.com/blog/2025/6/27/dbt-fusion-first-look)
- smelt: `docs/specs/incremental_models.md`, `docs/specs/model_properties.md`, `docs/specs/architecture.md`, `docs/specs/planner_integration.md`, `docs/ROADMAP.md`, `CLAUDE.md`
