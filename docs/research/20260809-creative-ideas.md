# Creative ideas on top of smelt

**Date:** 2026-08-09
**Status:** Brainstorm — divergent ideation, deliberately unfiltered for feasibility. Nothing here is committed work; promising items should graduate to their own research doc, then a spec.

## Why this doc

smelt has accumulated a set of unusual assets that most data tooling doesn't have:

1. **A property-proof substrate** — the composition walk derives *proven* facts about a model (key/grain, functional dependencies, event-time monotonicity, determinism, filter distributivity, bounded reach, aggregate algebra) rather than trusting declarations.
2. **A pure maintenance plan** — per-cell technique assignment as inspectable data, with a generative conformance gate proving `incremental_state == full_refresh` against a real engine.
3. **A semantic output fingerprint** — a provable "this change cannot alter the output" oracle.
4. **A real compiler pipeline** — error-recovery parser, Salsa incrementality, typed meta-language, LSP — over SQL itself, not a template layer.
5. **Differential conformance machinery** — parser/type-inference verified against live DuckDB in both directions, shrink-only ledgers, mutation-tested gates.

Most of the ideas below are answers to: *what else do these assets buy, beyond "a better dbt"?*

Ideas are grouped by theme. Each has a one-line pitch, a note on which asset it leverages, and a rough effort/leverage feel (S/M/L effort; ★–★★★ leverage). Cross-references to existing research are noted where an idea extends prior thinking.

---

## Theme A — The property proofs as a product surface

The walk's verdicts are currently internal fuel for maintenance admission. They are also *facts about the user's data model* that no other tool can state with confidence.

### A1. `smelt prove` — a property report card ★★★ (S)
CLI command that prints, per model, every property the walk established and — crucially — the *disproof site* for every property it couldn't: "not per-key constant because `updated_at` flows from a non-FD column at line 14." The maintenance plan already carries this; surfacing it as a first-class report turns refusal diagnostics into a teaching tool. Cheap because it's a rendering layer over existing data.

### A2. Property assertions in frontmatter — `must_hold:` ★★★ (S)
Let a model declare `must_hold: [deterministic, key_grain(user_id)]` and fail compilation when the walk can't prove it. This is a *semantic regression test with zero test code* — today a teammate can silently break the property that made a model incrementalizable, and the first symptom is a technique downgrade. Turns the proof substrate into a contract surface.

### A3. Proof-diff in CI / PR comments ★★★ (M)
A GitHub action that runs the walk on base and head and comments the property delta: "this PR demotes `daily_revenue` from window-forward merge to full refresh (lost: filter distributivity at stg_orders.sql:22); estimated cost delta: +40 min/day." The maintenance plan is pure data, so diffing two plans is trivial; the hard part is cost annotation (see D2). This is the "engineer controls planning" differentiator made visible at review time.

### A4. Property-aware autocomplete and hover in the LSP ★★ (M)
Hover on a model ref shows its proven grain, settle bound, and determinism verdict. Completion inside a `GROUP BY` warns when the chosen keys break downstream locality proofs. The LSP already has the Salsa queries; this is wiring verdicts into hover content.

### A5. "Why is this a full refresh?" — interactive refusal explorer ★★ (M)
Extend the Maintenance Atlas artifact pattern into the CLI/LSP: a `smelt explain --why <model>` that walks the refusal chain interactively, showing at each hop what declaration or rewrite would unlock the next technique rung. The refusal data is already structured (named diagnostics with remedies); this is a navigation UX over it.

### A6. Property-preserving refactoring engine ★★★ (L)
LSP code actions that rewrite SQL *proven* not to change the model's properties (and, with the fingerprint, not its output): extract CTE, push a filter through a join the walk knows is distributive, split a model at a grain boundary. Every refactor ships with the proof obligation it discharged. This is the long-promised "refactor with correctness receipts" from the project's founding pitch, now actually buildable because the proofs exist. Extends `2026-04-05-lsp-refactorings.md`.

### A7. Disproof-guided model linter ★★ (M)
A lint pass that flags *near-miss* patterns: "this `COALESCE(MAX(col), -1)` refuses once-write, but `COALESCE(MAX(col))` plus a default downstream admits it." The keyed-frontier work catalogued exactly these narrow-vs-wide spelling pairs; a linter is the cheapest way to teach them at edit time rather than at refusal time.

### A8. Property algebra playground ★ (S)
A docs-site interactive page (or artifact) where users compose synthetic operators and watch properties fold through the walk — an educational tool that doubles as a spec-conformance visualisation. The Maintenance Atlas showed there's appetite for this form.

---

## Theme B — The fingerprint as a platform primitive

`output_fingerprint` proves "same output" — an equivalence oracle. Equivalence oracles are rare and general.

### B1. Semantic `git bisect` for data bugs ★★★ (M)
`smelt bisect --model X --predicate "count(*) where amount < 0"` — binary-search commit history, but *skip every commit the fingerprint proves output-equivalent for X*, so the search runs over the handful of commits that could have changed the data. Turns O(n) rebuild-per-commit debugging into O(log k) where k = output-affecting commits.

### B2. Fingerprint-keyed CI cache — "test only what changed" ★★★ (M)
CI that rebuilds and data-tests only models outside the eclipse of the diff. This is backbuild change-detection (roadmap #3.5) applied to CI rather than environments — same analyser, different consumer. Plausibly the single highest-ROI application: every data team's CI is either wastefully full or unsoundly partial today.

### B3. Cross-repo model deduplication radar ★★ (M)
Fingerprint every model across an organisation's smelt projects and report structural near-duplicates ("these 3 teams maintain the same sessionisation, modulo column names"). The fingerprint's canonical form is the dedup key; near-miss requires a distance metric over the canonical CST — start with exact-match-after-renaming, which is cheap.

### B4. Provable view/table interchange ★★ (M)
Let the planner freely swap a model between VIEW and TABLE materialisation when the fingerprint machinery plus cost model says it's safe and profitable — the user declared *what*, the planner picks the cheapest *how*, per environment. dbt makes this a manual config; smelt can make it a proven optimisation.

### B5. Semantic deploy gates — "promote iff provably equal or explicitly acked" ★★ (M)
Promotion (roadmap VE #3) extended with a policy: an environment may auto-promote when every changed model is fingerprint-equal (pure refactor), and requires human ack listing the *actual* output-affected set otherwise. Change-management as a proof artifact rather than a checklist.

### B6. Fingerprint-addressed result store (Nix for data) ★★★ (L)
Generalise the VE snapshot store into a content-addressed cache: any run, anywhere (CI, laptop, prod), can reuse any table whose (fingerprint, input-versions) key matches. Laptops hydrate from prod's cache; CI never rebuilds what a teammate already built. This is the Nix/Bazel remote-cache model applied to relational data — the fingerprint is precisely the missing derivation hash. Big, but it's the natural terminus of the VE roadmap line.

---

## Theme C — The maintenance plan beyond batch SQL

The plan is pure data mapping cells to techniques. Nothing about it is DuckDB-specific — or even batch-specific.

### C1. Streaming lowering of the same plan ★★★ (L)
A cell whose technique is a keyed monoid fold or window-forward merge is *already* a streaming operator description. Lower the same maintenance plan to a streaming runtime (Arroyo/RisingWave/Flink SQL, or a bespoke DuckDB micro-batch loop) — same models, same proofs, latency becomes a deployment knob. The equivalence invariant (`incremental_state == full_refresh`) is exactly the correctness contract streaming IVM systems struggle to state, and smelt gets it per-cell with named refusals for what can't stream. This is the strongest "beyond dbt" differentiator in the list; extends `20260726-beyond-ivm-differentiation.md`.

### C2. A micro-batch daemon: `smelt serve` ★★ (M)
The cheap version of C1: a resident process that watches sources (or receives webhooks), runs the maintenance plan on arrival, and exposes settle-bound-aware freshness endpoints ("`daily_revenue` is complete through 14:00"). The settle bound derived by the composed-axes work is precisely the freshness statement dashboards need and never have.

### C3. Maintenance-plan export as a portable artifact ★★ (S)
`smelt plan --format json` emitting the full per-cell plan (techniques, clamps, ledger grading, propagation edges) as a stable schema. Third parties — orchestrators, catalogs, cost tools, C1 above — consume the plan without linking smelt. The purity invariant means this is serialisation, not new analysis.

### C4. Backfill choreographer ★★ (M)
A first-class `smelt backfill --model X --range ...` that uses propagation edges + partition alignment to compute the *minimal* upstream read set and downstream repair set for a historical correction, orders the work, and (via the ledger) makes interrupted backfills resumable. Backfill is the operation practitioners fear most; the plan already contains everything needed to make it boring.

### C5. Late-data SLO monitor ★ (M)
The ledger knows what windows were processed when; sources have declared/observed lateness. Emit a per-model "watermark risk" metric — how often data arrived after its window was folded — and alert when a declared `key_recurrence`/lateness bound is empirically violated. Turns declared bounds into monitored contracts.

---

## Theme D — Cost, observability, operations

### D1. Run ledger → built-in cost/lineage warehouse ★★ (M)
Every run already flows through one pipeline (`execute_project`). Persist per-statement timings, rows written, technique chosen, bytes scanned into a `smelt_meta` schema *as smelt models* — dogfooding: the observability mart ships as a smelt project reading the ledger. Users get `SELECT * FROM smelt_meta.slowest_cells` for free.

### D2. Learned cost model feeding the technique ladder ★★ (M)
`smelt bakeoff` measures techniques offline; D1 measures them in production. Close the loop: the override ladder's default preference becomes informed by observed per-cell timings, with `smelt explain` printing "chose suppression: 12 observed runs averaged 3.1× faster." Keeps the ladder inspectable (it's still the same pure resolution) while making the default smart.

### D3. `smelt doctor` — workspace health sweep ★ (S)
One command bundling: models silently on full-refresh that could incrementalise (A7's near-misses), unused sources, grain mismatches across refs, missing data tests on maintained models, fingerprint-unstable (nondeterministic) models. Each finding links its remedy. Mostly assembly of existing diagnostics.

### D4. Chaos gate for maintenance: fault-injected conformance ★★ (M)
Extend the generative conformance harness to kill runs mid-statement (between DELETE and INSERT, mid-ledger-write) and assert the recovery path still converges to the full-refresh oracle. The July memory notes ledger folds are transactional on DuckDB only — this gate is what makes non-transactional backends honest. A "Jepsen for incremental models" story is also excellent marketing.

### D5. Data-test placement advisor ★ (M)
Given the propagation edges, compute where a data test is *most informative* (closest to the fault origin that still covers the invariant) and flag redundant tests fully implied by upstream tests plus proven properties (a `unique` test downstream of a proven key-grain is a no-op). Tests become part of the algebra rather than folklore.

---

## Theme E — The language and compiler reused elsewhere

The parser/type-inference/LSP stack is a general SQL frontend with unusual rigor. It has value outside the transformation framework.

### E1. Standalone `smelt-sql-analyzer` crate/wasm ★★ (M)
Publish the parser + type inference + lineage as an embeddable library (Rust crate + wasm build). Every BI tool, notebook, and internal SQL platform wants "parse this SELECT, tell me output columns/types/lineage" and currently gets sqlglot's best-effort. The DuckDB-differential conformance gates are the credibility story: this analyzer's claims are *tested against the engine*. Extends `20260719-crates-publishing.md`.

### E2. LSP-over-dbt — the Trojan horse ★★★ (M)
The dbt-adapter research (`20260529-dbt-incremental-adapter.md`) already sketched this: run smelt's LSP over a dbt project's compiled artifacts, giving dbt users goto-def, type-on-hover, and property diagnostics with zero migration. It's the lowest-friction adoption funnel imaginable — the editor experience *is* the demo.

### E3. `smelt fmt` — the canonical SQL formatter with semantic guarantee ★★ (S)
A formatter whose CI-checkable claim is "formatting is fingerprint-identity" — the only SQL formatter that can *prove* it didn't change your query. The printer already exists for the fidelity gate.

### E4. Typed notebook kernel ★★ (M)
A Jupyter kernel where each cell is a smelt model: cells get typed schemas on definition (before execution), refs across cells resolve with goto-def, and re-running a cell invalidates exactly the Salsa-computed downstream cells. Incremental computation over notebook cells is exactly Salsa's shape.

### E5. SQL semantic diff as a standalone tool ★★ (M)
`smelt sqldiff old.sql new.sql` → "output-equivalent" / "differs: column `x` nullability, rows where `region IS NULL`". Usable outside smelt projects entirely — code review of raw SQL anywhere. The fingerprint plus typed-column comparison is 80% of it.

### E6. Query-to-model decompiler ★ (M)
Paste a 400-line analyst query; smelt proposes a decomposition into staged models at grain boundaries the walk identifies, naming each stage by its keys. Onboarding tool: turns the "migrate our mess" objection into a feature.

### E7. Meta-language as a general config compiler ★ (L)
The typed meta-language (Phases A–G) is a small typed functional language with YAML interop. Spun loose, it competes with CUE/Dhall for typed-config use cases — probably a distraction, recorded for completeness.

---

## Theme F — Multi-backend and planning ambitions

### F1. Cross-engine plan splitting with proven cut points ★★★ (L)
The founding differentiator #4: run staging on DuckDB against Parquet, marts on Databricks — but let the *planner* choose the cut using partition alignment + delta-shape proofs to guarantee the handoff (Parquet exchange, per `feedback_parquet_exchange`) preserves the equivalence invariant across the seam. The conformance harness extends naturally: run the split plan, compare to single-engine oracle.

### F2. DuckDB-as-accelerator mode ★★ (M)
For a warehouse-resident project, smelt transparently mirrors small upstream tables into local DuckDB and runs dev iterations there, using the fingerprint to know when local results are trustworthy vs. must hit the warehouse. Dev-loop latency is the practitioner pain; this is "Turbopack for dbt."

### F3. Backend conformance kit ★★ (M)
Package the differential gates (parser fidelity, type oracle, statement parity, maintenance conformance) as a harness any new backend must pass — `smelt backend-cert postgres` printing a scorecard. Makes the Postgres backend (roadmap #8) and community backends a checklist rather than a research project, and makes "supported backend" a *defined* term.

### F4. Optimisation-rule marketplace ★ (L)
User-authored planner rules (the public rule API) shared as packages with mandatory property-preservation proofs — a rule ships with the walk facts it requires and preserves, and the harness property-tests it on install. Long-horizon; depends on the planner API stabilising.

### F5. Adaptive technique selection per run ★ (M)
Let technique choice consult run-time facts the plan marks as data-dependent — the G2 count-preservation probe generalised: "merge if delta < 5% of table, else rebuild partition." The plan stays pure (it emits the *decision tree*); the runtime evaluates the probe. Roadmap already gestures here (#3.7).

---

## Theme G — Generative machinery reused

The conformance harness — typed recipe generation, staged execution, oracle comparison — is a general "generate realistic pipelines and check an invariant" engine.

### G1. `smelt datagen` as a user-facing product ★★ (S)
Grow the internal datagen into: given a project's sources + declared/proven properties, generate schema-correct, referentially-consistent, distribution-plausible fixture data with controllable edge-case density (late rows, redeliveries, key collisions — exactly what the recipes already model). Every data team hand-rolls this badly; smelt derives it from artefacts it already has.

### G2. Pipeline fuzzing as a service for user projects ★★ (M)
Run the conformance harness over the *user's* models instead of synthetic recipes: stage their sources with generated adversarial data, drive their real plan, diff against full refresh. "Your pipeline, property-tested" — catches the redelivery bug before production does. This is the maintenance-conformance gate sold outward.

### G3. Regression corpus distillation ★ (S)
Auto-shrink any conformance failure to a minimal model+data pair and file it as a pinned test (the BIT_XOR hazard case, automated). Internal tooling, but it compounds every future gate.

### G4. Mutation testing for user data tests ★ (M)
Apply the cargo-mutants pattern (per the mutation-campaign memory) to *user pipelines*: mutate their SQL (flip a join type, off-by-one a window), check whether any of their data tests notices. Reports test-suite blind spots the same way the internal campaign attributed gate coverage.

---

## Theme H — Demos, teaching, ecosystem

### H1. "Incremental IQ" public benchmark ★★ (M)
A published suite of maintenance scenarios (late data, redelivery, key churn, SCD2 succession, window overlap) scoring tools on *correctness under adversarial ingestion*, with smelt's per-scenario proofs/refusals shown alongside dbt/SQLMesh behaviour. Benchmarks shape discourse; nobody benchmarks incremental *correctness* today because nobody else can pass it.

### H2. The Maintenance Atlas, productised ★ (S)
The loved plan-prover explorer artifact regenerated per-release and published on the docs site as the canonical "what does smelt prove?" reference — already has a regen pipeline per memory; make it a release step.

### H3. Live "watch the proof" tutorial mode ★ (M)
`smelt tutorial` scaffolds the web-analytics project and, as the user edits models, streams walk verdicts live ("you just added `ORDER BY` inside the aggregate — determinism lost, here's why"). The web-analytics tutorial series infrastructure is the substrate.

### H4. SCD2 / sessionisation / dedup pattern library with proofs ★★ (S)
Ship the hard patterns (the SCD2 succession research, clock-vs-root sessions, keyed dedup) as importable, property-annotated model templates — each arrives with its admission matrix already satisfied and its refusal boundaries documented. Patterns-with-receipts is a category no snippet library occupies.

---

## A rough short-list

If forced to pick five by leverage-per-effort, aligned with the current roadmap spine (fingerprint wiring → VE → backbuild):

1. **B2 — fingerprint-keyed CI** (the eclipse analyser's killer app; falls out of roadmap #3.5)
2. **A1/A2 — `smelt prove` + `must_hold:`** (cheap, unique, makes the proof substrate a daily-driver surface)
3. **E2 — LSP-over-dbt** (adoption funnel; research already done)
4. **C2 — settle-bound freshness daemon** (turns derived bounds into an operational feature nobody else has)
5. **D4 — chaos conformance gate** (closes a known honesty gap and is a story worth telling)

With **C1 (streaming lowering)** and **B6 (content-addressed data cache)** as the two long-horizon bets worth keeping warm in research docs.

## References

- `docs/ROADMAP.md` — current priority spine
- `docs/research/20260726-beyond-ivm-differentiation.md` — differentiation framing this doc extends
- `docs/research/20260529-dbt-incremental-adapter.md` — E2 groundwork
- `docs/research/20260601-virtual-environments.md`, `docs/specs/output_fingerprint.md` — Theme B substrate
- `docs/research/20260802-backbuild-synthesis.md` — B2/eclipse analysis
- `docs/research/20260719-crates-publishing.md` — E1 groundwork
