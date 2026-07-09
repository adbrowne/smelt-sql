# Theoretical foundations for incremental-model eligibility and the monotonicity primitive

**Purpose.** External references and theoretical grounding for
`docs/research/20260701-expanding-incremental-eligibility.md`. That doc audits
when a smelt model can be refreshed *incrementally* (window-by-window over an
event-time column) instead of full-refresh, under the invariant **incremental ≡
full-refresh**. Its central missing analysis is a **monotonicity primitive**:
proving a projected `event_time` expression traces back *monotonically* to a
source partition/timestamp column, which licenses pushing a time-window
predicate toward the source. This file gathers the academic theory that (a)
names the exact class of queries for which incremental refresh is sound, (b)
gives the algebraic license for the pushdown, and (c) marks the provable limits
that force the design onto conservative static analysis plus declared
guarantees.

Every claim below is traceable to a cited source; each source was confirmed to
exist via search against at least one authoritative index (DOI resolver,
arXiv, dblp, or official venue). Items with no single canonical citation are
flagged explicitly.

---

## 1. Synthesis — what the theory says about smelt's approach

**The one-line result.** *Monotone* queries — those where enlarging the input
can only add output tuples, never retract one (`I ⊆ I′ ⟹ Q(I) ⊆ Q(I′)`) — are
**exactly** the class that can be maintained incrementally by streaming in new
facts, without ever recomputing from scratch. This is the deepest theoretical
connection underneath the whole eligibility audit. It is the standard database-
theory definition of monotonicity (Abiteboul–Hull–Vianu, *Foundations of
Databases*, 1995), and the **CALM theorem** sharpens "can be maintained add-only
/ coordination-free" to *iff monotone*: Hellerstein conjectured it (2010) and
Ameloot–Neven–Van den Bussche proved it for relational-transducer networks
(PODS 2011 / JACM 2013). smelt's window-by-window refresh *is* an add-only,
coordination-free computation over a growing event stream, so the queries for
which "incremental ≡ full-refresh" holds by construction are precisely the
monotone ones.

**The operator boundary is already the doc's boundary.** Positive relational
algebra — selection (σ), projection (π), join/product (⋈, ×), union (∪) — is
monotone; set difference/`EXCEPT` (negation), aggregation (`SUM`/`COUNT`/
`MIN`/`MAX`), and `DISTINCT`/`GROUP BY` deduplication are **non-monotone**,
because a new (or late-arriving) input row can change or retract a previously
emitted output. This is exactly the split the doc arrives at empirically: the
"transparent" bodies (project/filter/rename), single-stream `UNION ALL`, and
fact-⋈-lookup joins are the safe slice; aggregation-not-aligned-to-partition,
`DISTINCT`, `LIMIT`, cross-window window frames, and `EXCEPT` are the hazards.
The theory tells us this is not a coincidence — it is the monotone/non-monotone
frontier.

**The pushdown license and the eligibility proof are the same fact.** The doc's
eligibility argument is a *commutation* statement, `σ_event_time(Q(R)) =
Q(σ_event_time(R))`. That is precisely the classical precondition for
**predicate pushdown** in a query optimiser — selection commutes with
projection, distributes over union/intersection/difference, and pushes into the
argument(s) of a join it references (Garcia-Molina–Ullman–Widom, *Database
Systems: The Complete Book*, §16.2; the heuristic dates to the System R
optimiser, Selinger et al. 1979). So "is this model incrementalisable?" and "how
deep can the time filter be pushed toward the scan?" are one computation — which
is exactly the unification the doc proposes in Part 4.

**The scalar half of the primitive is order-preservation.** Pushing a *range*
predicate through a projected expression `f(x)` is sound exactly when `f` is a
**monotone (order-preserving)** function: if `f` is non-decreasing then
`a ≤ x < b ⟹ f(a) ≤ f(x) < f(b)`, so a window filter on the projection pulls
back to a window filter on the source column. This is the scalar-function analog
of relational monotonicity and is the operational core of min/max
"zone-map"/micro-partition pruning in real engines (Netezza zone maps, Snowflake
micro-partitions). `DATE_TRUNC`, `CAST`-widening, and `+ INTERVAL` are
order-preserving; general arithmetic on a timestamp may not be — precisely the
distinction the doc's monotonicity primitive must draw. (No single seminal paper
states the scalar lemma; it is standard order theory realised by the pruning
systems — flagged below.)

**The modern, constructive statement: DBSP.** The DBSP paper (Budiu–McSherry–
Ryzhyk–Tannen, VLDB 2023, Best Research Paper) gives a formal algebra that
incrementalises *any* relational query as `Q^Δ = D ∘ Q ∘ I` (differentiate ∘ Q ∘
integrate) and classifies operators by cost: **linear** operators (selection,
projection, map, union) satisfy `Q^Δ = Q` — the incremental version *is* the
original applied to the delta, cost proportional to the change; **bilinear**
operators (joins) obey a product rule; aggregation and non-monotone
recursion/negation need nested integration/differentiation. DBSP's linear-vs-
bilinear split is the precise, modern statement of the doc's "safe slice," and
the fact that linear operators commute with the delta is the same algebra that
licenses pushing the event-time window predicate to the source.

**The streaming analog: watermarks.** The Dataflow Model (Akidau et al., VLDB
2015) formalises a **watermark** as a monotone lower bound on the event times
still to arrive — a measure of event-time completeness that "only moves
forward." This is the streaming-theory mirror of smelt's monotonicity primitive
and its bounded-lookback reasoning: a window can be safely closed/emitted only up
to the completeness the watermark guarantees, which is exactly why late/out-of-
order data forces bounded lookback and why "incremental ≡ full" holds only within
that completeness bound.

**The provable limits (what the doc must acknowledge).** The system *cannot*
decide the invariant in general:

- **Query equivalence for full SQL is undecidable.** Trakhtenbrot's theorem
  (finite satisfiability of first-order logic is undecidable) yields, by standard
  reduction, undecidability of containment/equivalence for relational algebra
  with negation (`EXCEPT`, universal quantification). So "incremental plan ≡
  full-refresh plan" cannot be decided by comparing arbitrary models. (Even
  Datalog, though monotone, has undecidable equivalence — monotonicity buys
  incremental maintainability, *not* decidable equivalence.)
- **Monotonicity of an arbitrary function/UDF is undecidable.** By Rice's
  theorem, every non-trivial semantic property of a program is undecidable;
  "is this function order-preserving?" is one such property. So a projected
  `event_time` expression containing an opaque UDF or arbitrary arithmetic cannot
  be proven monotone automatically.
- **Where traction is regained.** Conjunctive-query containment/equivalence *is*
  decidable (NP-complete, Chandra–Merlin 1977), and monotonicity is soundly
  decidable over recognisable syntactic sub-classes: positive relational algebra
  (monotone by construction) and a whitelist of order-preserving scalar
  constructs (`DATE_TRUNC`, `CAST`-widening, `+ INTERVAL`, positive-constant
  arithmetic, and their compositions).

This is exactly the doc's open question "how much can be decided statically vs.
needs a declared guarantee," and the theory answers it crisply: **decide
monotonicity soundly but incompletely over a whitelist; require a user-declared
guarantee everywhere else; never push a filter you have not licensed.** The
primitive is necessarily a *sufficient-condition* analysis, not a complete
decision procedure — which matches the conservative posture the doc already
adopts in §4.6.

---

## 2. Annotated bibliography

### Area 1 — Incremental View Maintenance (IVM): the classic theory

**Blakeley, Larson & Tompa (1986), "Efficiently Updating Materialized Views."**
Proc. ACM SIGMOD 1986, pp. 61–71.
DOI: https://doi.org/10.1145/16894.16861 · dblp: https://dblp.org/rec/conf/sigmod/BlakeleyLT86.html
The founding IVM paper for select-project-join (SPJ) views. Its key contribution
to smelt's problem is the notion of an **irrelevant update**: a state-independent
syntactic condition under which a base-table change *cannot* affect the view, so
it can be filtered out before any recomputation. That is the theoretical ancestor
of the doc's time-window pushdown — proving changes outside a partition/time
range are irrelevant to a window licenses skipping them.
*Note: also indexed under DOI `10.1145/16856.16861` (SIGMOD Record 15(2)); same paper.*

**Gupta, Mumick & Subrahmanian (1993), "Maintaining Views Incrementally"
(the counting algorithm).** Proc. ACM SIGMOD 1993, pp. 157–166.
DOI: https://doi.org/10.1145/170035.170066 · dblp: https://dblp.org/rec/conf/sigmod/GuptaMS93.html
Introduces the **counting algorithm**: each derived tuple carries a count of its
alternative derivations, so inserts/deletes apply incrementally and the view is
provably identical to a from-scratch recompute (smelt's ≡ invariant) under set
and bag semantics. It is the count machinery that lets *non-monotone* operators
(negation, `MIN`/`MAX` under deletion) be maintained safely — marking the price
of admitting operators beyond the monotone slice, and why the cheap slice is the
one that needs no such bookkeeping.

**Gupta & Mumick (1995), "Maintenance of Materialized Views: Problems,
Techniques, and Applications."** IEEE Data Engineering Bulletin 18(2):3–19.
Bulletin PDF: http://sites.computer.org/debull/95JUN-CD.pdf
The canonical **survey and taxonomy**, classifying view-maintenance along four
axes — the view class (SPJ, aggregation, recursion), the resources available
(base data vs. only view+deltas), the modification types, and whether a technique
works for all instances or only some. This is the standard vocabulary for stating
precisely *which views are incrementally maintainable and at what cost* — the
exact framing of an eligibility audit.

**Gupta & Mumick, eds. (1999), *Materialized Views: Techniques,
Implementations, and Applications*.** MIT Press. ISBN 9780262571227.
Publisher: https://direct.mit.edu/books/edited-volume/2853/
The definitive edited collection consolidating the survey plus the primary IVM
papers (counting, self-maintenance, aggregate views, warehouse maintenance). Use
as the single authoritative secondary citation for the field as a whole.

**Quass, Gupta, Mumick & Widom (1996), "Making Views Self-Maintainable for
Data Warehousing."** Proc. PDIS 1996, pp. 158–169.
Semantic Scholar: https://www.semanticscholar.org/paper/58677038cda0fc9d685a366ab097a71dc099c76d
Stanford InfoLab: http://infolab.stanford.edu/warehousing/publications.html
Defines **self-maintainability**: refreshing a view from deltas + the stored view
*without re-reading the source*, and shows that **key and referential-integrity
constraints** make an SPJ view self-maintainable. This is the formal justification
for smelt's "trace the projected `event_time` back to a source key/partition
column" primitive — constraint-derived provenance is exactly what bounds which
source rows a window can depend on and lets the predicate be pushed to the source.

### Area 2 — Monotonicity in query languages & the CALM theorem *(deepest connection)*

**Hellerstein (2010), "The Declarative Imperative: Experiences and Conjectures
in Distributed Logic."** ACM SIGMOD Record 39(1):5–19 (PODS 2010 keynote).
DOI: https://doi.org/10.1145/1860702.1860704 · PDF: https://dsf.berkeley.edu/papers/sigrec10-declimperative.pdf
States the original **CALM conjecture** — *Consistency And Logical Monotonicity*
— that a program has an eventually-consistent, coordination-free implementation
*iff* it is expressible in monotonic logic. This is the theoretical anchor for
"monotone ⇒ safe to compute by growing the input": smelt's window-by-window
refresh is an add-only, coordination-free computation, and CALM says the sound
queries for it are precisely the monotone ones.

**Ameloot, Neven & Van den Bussche (2011/2013), "Relational Transducers for
Declarative Networking" (the proof of CALM).** PODS 2011: 283–292; JACM 60(2), 2013.
arXiv: https://arxiv.org/abs/1012.2858 · dblp: https://dblp.org/rec/conf/pods/AmelootNB11.html
Gives the first **formal proof** of a precise CALM: the class of monotone queries
is *exactly* the class computable by coordination-free relational-transducer
networks. This supplies the "exactly" — monotonicity is not merely sufficient but
the sharp boundary — justifying smelt treating a monotone trace as the *licensing*
condition for incremental refresh rather than a loose heuristic. Outside the
monotone class, an add-only window strategy cannot in general preserve
"incremental ≡ full."

**Hellerstein & Alvaro (2020), "Keeping CALM: When Distributed Consistency Is
Easy."** Communications of the ACM 63(9):72–81.
DOI: https://doi.org/10.1145/3369736 · arXiv: https://arxiv.org/abs/1901.01930
The accessible synthesis, restating the result as the **CALM Theorem** and the
design discipline of "push coordination only to the non-monotone points of a
program." That is the direct analog of smelt's rule: push the window predicate
freely across monotone operators; fall back to full-refresh exactly where the
query leaves the monotone fragment. Best single citation for a non-specialist
reader.

**The monotone ↔ non-monotone operator boundary** *(textbook grounding).*
Abiteboul, Hull & Vianu, *Foundations of Databases* ("Alice Book"), 1995,
Ch. 4–5. Free HTML: http://webdam.inria.fr/Alice/
Standard definition: *Q* is **monotone** iff `I ⊆ I′ ⟹ Q(I) ⊆ Q(I′)`. Monotone =
positive relational algebra / unions of conjunctive queries (σ, π, ⋈, ∪, i.e.
positive-existential FO / negation-free Datalog); non-monotone = set
difference/`EXCEPT`, aggregation, and `DISTINCT`/`GROUP BY` (adding a row can
retract a prior output). This is the classification smelt's static analysis
implements, and the per-expression version — a monotone map preserves a
predicate's boundary — is exactly the doc's monotonicity primitive.

### Area 3 — Differential dataflow / DBSP *(most relevant modern theory)*

**McSherry, Murray, Isaacs & Isard (2013), "Differential Dataflow."** CIDR 2013.
PDF: https://www.cidrdb.org/cidr2013/Papers/CIDR13_Paper111.pdf · dblp: https://dblp.org/rec/conf/cidr/McSherryMII13.html
Generalises incremental computation from a single total order to a
**partially-ordered (multi-dimensional) set of versions**, representing
collections as **weighted difference streams (Z-sets)** and operating on
differences. Correctness = accumulated differences equal the from-scratch result
at every version. This is the model that makes *nested iteration* (recursion)
incrementally maintainable, extending the safe slice beyond classic SPJ +
aggregation.

**Murray, McSherry, Isaacs, Isard, Barham & Abadi (2013), "Naiad: A Timely
Dataflow System."** Proc. ACM SOSP 2013, pp. 439–455 (Best Paper).
DOI: https://doi.org/10.1145/2517349.2522738 · PDF: https://sigops.org/s/conferences/sosp/2013/papers/p439-murray.pdf
The systems substrate — **timely dataflow** — under differential dataflow: cyclic
dataflow with lattice-structured logical timestamps and lightweight progress
tracking, giving batch throughput with stream latency. Relevant as evidence that
differential IVM is practical, and as the origin of the timestamp-lattice
discipline smelt's monotonic event-time progress echoes.

**Budiu, McSherry, Ryzhyk & Tannen (2023), "DBSP: Automatic Incremental View
Maintenance for Rich Query Languages."** PVLDB 16(7):1601–1614 (VLDB 2023 Best
Research Paper).
DOI: https://doi.org/10.14778/3587136.3587137 · PDF: https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf · arXiv: https://arxiv.org/abs/2203.16684 · extended VLDBJ 2025: https://doi.org/10.1007/s00778-025-00922-y
The cleanest modern answer to "which operators are cheaply incrementalizable."
DBSP models computation as stream operators over abelian groups (Z-sets) with
mutually-inverse **integration `I`** (running sum) and **differentiation `D`**
(consecutive difference). The core result: for any query `Q`, the incremental
version is `Q^Δ = D ∘ Q ∘ I`, provably equal to recomputing `Q` (smelt's ≡
invariant), automatically and compositionally. Cost taxonomy:
- **Linear** (selection, projection, map, union): `Q^Δ = Q` — apply the original
  to the delta; cost ∝ change. This is the cheapest safe slice and corresponds
  exactly to the operators over which a time-window predicate commutes/pushes down.
- **Bilinear** (joins, product): product rule
  `(a×b)^Δ = a_prev×Δb + Δa×b_prev + Δa×Δb`.
- **Aggregation / non-monotone recursion / negation**: handled via nested
  `I`/`D` and fixed points — the frontier of automatic IVM.
DBSP's linear-vs-bilinear split is the precise theoretical statement of smelt's
"safe slice"; `Q^Δ = D∘Q∘I` with linear operators commuting with the delta is the
algebraic license behind pushing the event-time window predicate to the source.

### Area 4 — Predicate pushdown formal theory & monotone scalar expressions

**Selinger, Astrahan, Chamberlin, Lorie & Price (1979), "Access Path Selection
in a Relational Database Management System" (System R optimizer).** Proc. ACM
SIGMOD 1979, pp. 23–34.
DOI: https://doi.org/10.1145/582095.582099 · dblp: https://dblp.org/rec/conf/sigmod/SelingerACLP79.html
The foundational cost-based optimiser. It establishes applying restriction
(selection) predicates as early as possible against the cheapest access path —
the operational ancestor of predicate pushdown and the historical anchor for the
doc's claim that pushing σ toward the source is a standard, correctness-preserving
optimiser move.

**Garcia-Molina, Ullman & Widom (2008), *Database Systems: The Complete Book*
(2nd ed.), Ch. 16 §16.2 "Algebraic Laws for Improving Query Plans."** Pearson.
Author page: http://infolab.stanford.edu/~ullman/dscb.html
The authoritative textbook statement of the equivalence rules the eligibility
proof relies on: selection commutes with projection, distributes over
union/intersection/difference, and pushes into the join/product argument(s) it
references — exactly the `σ∘Q = Q∘σ` commutation, with "Pushing Selections" as the
named transformation. (Silberschatz–Korth–Sudarshan, *Database System Concepts*,
"Transformation of Relational Expressions," is an equivalent alternative source.)

**Hellerstein & Stonebraker (1993), "Predicate Migration: Optimizing Queries
with Expensive Predicates."** Proc. ACM SIGMOD 1993, pp. 267–276.
DOI: https://doi.org/10.1145/170036.170078 · PDF: https://15721.courses.cs.cmu.edu/spring2019/papers/23-optimizer2/hellerstein-sigmod1993.pdf
Shows predicate *placement* is a cost decision, not a free "always push down" —
predicates can be migrated up or down (including across joins) while preserving
equivalence. Relevant because the doc argues the time-window predicate is
*legally* migratable to the source only when a commutation precondition holds;
this is the formal framework for correctness-preserving predicate movement.
Follow-on: Hellerstein, "Practical Predicate Placement," SIGMOD 1994,
https://doi.org/10.1145/191843.191904.

**Min-max / zone-map pruning — monotone range-predicate pushdown to storage.**
The industrial realisation of "`a ≤ x < b` restricts which blocks must be
scanned," which combined with a monotone `f` gives `f(a) ≤ f(x) < f(b)` block
pruning:
- IBM Netezza **Zone Maps** — https://www.ibm.com/docs/en/netezza?topic=statistics-zone-maps
  (per-extent min/max for ordered numeric/date columns; range predicates skip
  extents outside `[min,max]`; canonical early zone-map system).
- Snowflake **Micro-Partitions & Clustering** — https://docs.snowflake.com/en/user-guide/tables-clustering-micropartitions
  (per-column min/max per 50–500 MB micro-partition; the optimiser prunes
  partitions whose `[min,max]` cannot satisfy the predicate — exactly the
  primitive smelt pushes a time-window filter down to).
- Ziauddin et al. (2017), "Dimensions Based Data Clustering and Zone Maps,"
  PVLDB 10(12) — http://www.vldb.org/pvldb/vol10/p1622-ziauddin.pdf (peer-reviewed
  formal treatment of zone maps and clustering-driven pruning).

**The scalar monotonicity lemma** *(flagged: no single canonical citation).*
"If `f` is monotone non-decreasing then `a ≤ x < b ⟹ f(a) ≤ f(x) < f(b)`, so a
range predicate on `f(x)` pulls back to a range predicate on `x`" is standard
order theory, operationalised by the zone-map / micro-partition systems above.
`DATE_TRUNC`, `CAST`-widening, and `+ INTERVAL` are order-preserving and preserve
min/max ordering, so pruning/pushdown stays valid; general timestamp arithmetic
need not be. Cite the algebraic-equivalence sources (System R, Complete Book) for
the "predicate commutes with the operator" half and this order-theory lemma (with
the pruning systems as mechanism) for the "predicate commutes with the scalar
expression" half. This is the exact formal content of smelt's monotonicity
primitive.

### Area 5 — Theoretical limits (undecidability / intractability)

**Chandra & Merlin (1977), "Optimal Implementation of Conjunctive Queries in
Relational Data Bases" (the decidable island).** Proc. ACM STOC 1977, pp. 77–90.
DOI: https://doi.org/10.1145/800105.803397
Conjunctive-query (CQ) containment and equivalence are **decidable — NP-complete**
— via the homomorphism theorem, and every CQ has a unique minimal form. This
delimits *where* an equivalence check like "incremental plan ≡ full-refresh plan"
is even decidable: restricted to the conjunctive (monotone, negation-and-
aggregation-free) fragment it is decidable (if NP-hard). It is the theoretical
justification for a system to *restrict* the class it will incrementalise
automatically, and to demand a declared guarantee outside that fragment.

**Trakhtenbrot's theorem — undecidability of equivalence for full relational
algebra/SQL.** B. A. Trakhtenbrot (1950); modern treatment in Libkin, *Elements
of Finite Model Theory* (Springer 2004) and Abiteboul–Hull–Vianu (1995).
Overview: https://en.wikipedia.org/wiki/Trakhtenbrot%27s_theorem
Derivation of query-containment undecidability: https://pages.cs.wisc.edu/~paris/cs838-s16/lecture-notes/lecture2.pdf
Finite satisfiability/validity of first-order logic is **undecidable**; by
standard reduction, containment/equivalence for full relational algebra/calculus
(SQL with negation/`EXCEPT`, universal quantification) is undecidable. **The hard
wall:** smelt cannot in general decide "incremental plan ≡ full-refresh plan" by
comparing arbitrary SQL. It must restrict to a decidable fragment, rely on
structural monotonicity-preserving rewrites whose soundness is proven once, or
accept a user-declared guarantee — i.e., a *sufficient-condition* static analysis,
not a complete decision procedure. (Even monotone Datalog has undecidable
equivalence: monotonicity buys incremental maintainability, not decidable
equivalence.)

**Rice's theorem — undecidability of monotonicity for arbitrary functions/UDFs.**
H. G. Rice (1953), Trans. AMS 74(2):358–366.
DOI: https://doi.org/10.1090/S0002-9947-1953-0053041-6 · Overview: https://en.wikipedia.org/wiki/Rice%27s_theorem
Every non-trivial semantic property of programs is undecidable; "is this function
order-preserving?" is one such property, so **detecting whether an arbitrary
function/UDF (or Turing-expressive query) is monotone is undecidable.** This
directly explains why smelt cannot *automatically prove* monotonicity of an
arbitrary projected `event_time` expression containing opaque UDFs or arbitrary
arithmetic. The engine must fall back to (a) a whitelist of known order-preserving
constructs or (b) a user-declared monotonicity annotation — the crisp "decided
statically vs. declared" split the doc needs.

**Decidable restricted classes — where conservative static analysis regains
traction** *(guidance, no single canonical paper).*
Although general monotonicity detection is undecidable (Rice), monotonicity is
statically decidable — *soundly but incompletely* — over recognisable syntactic
sub-classes: **positive relational algebra / UCQs** (monotone by construction; no
negation, aggregation, or `DISTINCT`-induced retraction) and **order-preserving
scalar constructs** (`DATE_TRUNC`, `CAST`-widening, `+ INTERVAL`, positive-constant
arithmetic, and compositions, certified by structural induction over a whitelist).
The CALM "oblivious transducer" characterisation (Ameloot et al.) is the
model-theoretic version of this recognisable class. This is the constructive
resolution: certify a monotone trace over the whitelist ⇒ license the window
pushdown; otherwise require an annotation or full-refresh.
*Illustrative practitioner writeup (not peer-reviewed): Robert Fink, "On
Monotonicity in Relational Databases and Service-oriented Architecture," Palantir
Engineering, 2018 — https://medium.com/palantir/on-monotonicity-in-relational-databases-and-service-oriented-architecture-90b0a848dd3d*

### Area 6 — Timestamp/watermark monotonicity in streaming theory

**Akidau et al. (2015), "The Dataflow Model: … Unbounded, Out-of-Order Data
Processing."** PVLDB 8(12):1792–1803.
DOI: https://doi.org/10.14778/2824032.2824076 · PDF: http://www.vldb.org/pvldb/vol8/p1792-Akidau.pdf
The defining modern treatment separating **event time** from **processing time**
and formalising **watermarks** as a (generally heuristic) monotone lower bound on
the event times still to arrive — a completeness measure that "only moves
forward." This is the streaming analog of smelt's monotonicity primitive: a
watermark licenses closing/emitting a window (bounded-lookback correctness)
precisely because event time advances monotonically. It grounds why incremental
refresh over an event-time window equals full-refresh only up to watermark
completeness, and why bounded lookback is required for late/out-of-order data.

**Akidau et al. (2013), "MillWheel: Fault-Tolerant Stream Processing at Internet
Scale."** PVLDB 6(11):1033–1044.
DOI: https://doi.org/10.14778/2536222.2536229 · PDF: http://www.vldb.org/pvldb/vol6/p1033-akidau.pdf
Introduces **low watermarks** as a system primitive: a low watermark of *t* at a
stage guarantees all records with event timestamps ≤ *t* have been received, and
is maintained **monotonically increasing** and propagated downstream. This is the
concrete "event_time only moves forward" invariant the doc invokes for
bounded-lookback safety — the completeness signal that makes window-by-window
incremental computation sound.

**Begoli et al. (2021), "Watermarks in Stream Processing Systems: Semantics and
Comparative Analysis of Apache Flink and Google Cloud Dataflow."** PVLDB
14(12):3135–3147.
DOI: https://doi.org/10.14778/3476311.3476389 · PDF: http://www.vldb.org/pvldb/vol14/p3135-begoli.pdf
The most explicit *formal* treatment of watermark semantics: defines a watermark
as a monotonic function of event-time progress and analyses completeness
guarantees across two production systems. Best citation for a precise, formal
definition of watermark monotonicity underpinning "bounded lookback ⟹ incremental
≡ full-refresh within the watermark."

---

## 3. Verification notes

- All peer-reviewed entries confirmed against at least one authoritative index
  (DOI resolver, arXiv, dblp, or official PVLDB/venue PDF). The four VLDB
  streaming/IVM anchors (DBSP, Dataflow, MillWheel, Begoli) and the optimiser
  anchors (System R, Predicate Migration) have verified DOIs and open PDFs.
- **Flagged — no single canonical citation:** the scalar order-preservation lemma
  (monotone `f` pushes range predicates) is standard order theory; cite it via the
  zone-map/micro-partition systems plus the algebraic-equivalence textbooks, not a
  seminal paper.
- **Flagged — not peer-reviewed:** the Palantir "On Monotonicity" blog is
  illustrative only.
- **Minor duplicate DOI:** Blakeley–Larson–Tompa (1986) appears under both
  `10.1145/16894.16861` (proceedings) and `10.1145/16856.16861` (SIGMOD Record
  15(2)) — the same paper.
