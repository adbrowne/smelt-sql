# Phase 3 claim inventory — lines 752–1115 of `docs/specs/incremental_models.md` (pre-edit)

Diagnostic codes present in this range (exact list, grep-verified):
`MaintenanceNoAdmissibleTechnique`, `MaintenanceWriteAddressingRefused`,
`MaintenanceWritePatternUnavailable`, `MaintenanceRepairKeysNotDiscoverable`,
`MaintenanceRepairSliceUnbounded`, `ContractLateArrivalOutsideHorizon`,
`MaintenanceScanUnbounded`, `MaintenanceSkeletonColumnAdded`.

## `### Per-cell write addressing` (752–824)

1. (754-755) Every cell derives its physical write from the write-pattern set, which is an open
   registry, not a closed enum.
2. (758) The known patterns today: region DELETE+INSERT, keyed MERGE, column-scoped MERGE,
   in-place UPDATE, full rebuild, diff_patch, and more.
3. (761-763) **The available-addressings rule**: a write mechanism is admitted for a cell iff
   `available = (declared contract facts) × (trigger/changed-input need) × (equivalence
   invariant) × (backend capability)`.
4. (765-766) The first three factors are structural; the fourth is the target engine's capability
   registry (owned by `architecture.md`).
5. (768-769) Keyed MERGE / column-scoped MERGE / in-place UPDATE require a declared `unique_key`.
6. (770) Region DELETE+INSERT requires a declared partition axis (`timeseries:`).
7. (771) A bare lookup (identity, no clock) has no region → only keyed merge or full rebuild.
8. (772-775) A bare partition table (clock, no identity) has no identity → only region rewrite or
   full rebuild; gaining keyed dimension-change addressing requires declaring `unique_key`, which
   makes it the composed clock-and-identity shape; declaring identity is load-bearing (admits
   keyed writes), never a dedup footnote.
9. (777) A cell with no admissible write mechanism is refused `MaintenanceNoAdmissibleTechnique`,
   naming the cell.
10. (779-780) Addressing is how a row is found, not how far the statement ranges; keyed MERGE
    does not make the statement table-wide.
11. (781-783) When the output also declares a `timeseries:` axis, the write stays bounded to the
    affected partitions: the changed-input delta resolves to touched partitions first, and keyed
    MERGE is emitted per partition (or with a partition predicate) against just those.
12. (783-786) A genuinely window-free keyed write (one whole-table MERGE) is reached only when the
    cell provably cannot be bounded to a partition set; that unboundedness is itself a derived
    per-cell fact, fail-loud, never a default.
13. (786-788) Partition-scoping is orthogonal to the addressing corner: region and keyed writes
    alike ride the partition pruning the plan computes.
14. (790-791) `maintenance.cells[].write` names the write mechanism per cell; a pin is validated
    against the equivalence invariant for its cell.
15. (791-792) A pin refused with `MaintenanceWriteAddressingRefused` when the addressing cannot
    uphold the equivalence invariant.
16. (792-794) A pin refused with `MaintenanceWritePatternUnavailable` when the name is
    unrecognised or the target backend cannot execute it.
17. (794) The pin selects among admissible mechanisms; it never widens the admissible set.
18. (796-819) Worked example: `order_facts` (composed clock+identity, joins mutable dimension)
    plans to three cells — creation/backfill on orders as region DELETE+INSERT, mutation on
    customers as column-scoped keyed MERGE scoped to touched partitions.
19. (821-823) One model, three cells, two addressings; neither verdict is declared; pinning either
    is validated, not trusted.

### `#### The write-pattern set is open (and partly backend-provided)` (825–874)

20. (827-831) The named patterns are the ones understood today; the set grows (partition/atomic
    swap, copy-on-write vs merge-on-read, `MERGE...WHEN NOT MATCHED BY SOURCE` prune,
    staged-upsert, predicate-targeted UPDATE by non-key predicate, incremental MV refresh,
    engine-specific primitives); the durable contract is the admission function, not the
    enumeration.
21. (833-837) The invariant is the admission function, not the enum: a new pattern is admitted by
    declaring which contract facts it requires and discharging the equivalence proof obligation
    for the cells it serves; grain stays derived, cost model ranks whatever the rule admits; a new
    mechanism can never be less correct than the ones it joins, because the equivalence gate is
    the price of entry.
22. (838-843) Concretely: a dimension-mutation cell could one day be served by an UPDATE that
    locates rows through the join key rather than the output's `unique_key`, partition by
    partition — admitted by declaring the facts it needs (a proven FD from join key to repaired
    columns) and discharging equivalence; today's registry serves that cell with keyed column
    MERGE.
23. (844-849) The pattern set is backend-relative: admission carries backend capability as its
    fourth factor via the backend's capability registry (`architecture.md`); a pattern the target
    cannot execute is not a candidate; this is where backend-specific optimisations are
    contributed rather than special-cased in the planner, and prevents silent dependence on a
    primitive only one engine has.
24. (850-853) The `write:` pin is an open, fail-loud vocabulary resolved against the registry, not
    a sealed enum; an unrecognised pin, or one naming a pattern the target backend cannot provide,
    is refused with a diagnostic — never silently downgraded.
25. (855-858) `diff_patch`: computes candidate rows for a slice, diffs against the slice's stored
    state, writes only the difference — inserting absent rows, updating rows whose compared
    columns differ, deleting stored rows absent from a complete candidate set.
26. (858-861) `diff_patch` contract facts required: a declared `unique_key` for insert/update legs,
    and change comparability (`model_properties.md` §"Change comparability") over written columns
    for the update leg.
27. (861-865) The delete leg additionally requires slice completeness (candidate set provably
    contains every row that should exist in the slice, same premise as the repair family's
    correctness argument); not admitted without it; lacking completeness the pattern degrades
    explicitly to insert+update, stated as reduced-capability admission, not a silently dropped
    delete leg.
28. (866-868) `diff_patch` is graded Idempotent (a second run against unchanged input diffs to
    empty) — what makes it the reconciliation and drift-repair write.
29. (868-870) The slice a `diff_patch` write restricts to is the candidate's own slice (affected-key
    set for a per-group recompute, a partition region for a windowed one) — not tied to a
    partition axis.
30. (872-873) Backends execute registered patterns; they never author maintenance-statement text
    (owned by §"Statement emission (single owner)").

## `### The repair family` (875–957)

31. (877-878) A non-invertible combiner refuses reprocessing outright when a merged window's input
    changes; full refresh is the only universally correct fallback for it.
32. (878-884) The repair family narrows that refusal for one common case: when the change is a
    retraction or mutation whose affected output keys are provably finite
    (`model_properties.md` §"Affected-key discovery"), the plan recomputes only those groups from
    a bounded input slice; it is the targeted-write refinement of recompute-a-region, landing in
    the column-scoped re-derivation corner of the 2×2.
33. (884-886) Like a region recompute it supersedes and resets the ledger for the keys it rewrites
    (§"Per-cell admission", interchangeability).
34. (888-891) Why correct: recomputing key set `K` over an input slice provably containing every
    row contributing to any `k ∈ K` reproduces `full_refresh` restricted to `K`; keys outside `K`
    are untouched and stay bit-identical.
35. (891-892) The equivalence invariant holds cell-wide as a consequence: written keys equal the
    full-refresh oracle restricted to `K`, unwritten keys equal it trivially.
36. (892-896) The load-bearing premise is slice completeness: the input slice a per-group recompute
    reads must provably contain every row that can contribute to a key in `K`; reuses key temporal
    locality (§"Key temporal locality"), not a new proof.
37. (898-900) A repair cell is admitted only when three obligations discharge; two are reused from
    §"Per-cell admission"'s numbered list, one is new.
38. (902-903) Obligation reused — derivable group key (obligation 6, "well-defined groups"): the
    walk's grain names the groups a repair recomputes.
39. (904-905) Obligation reused — bounded per-group read footprint (obligation 4, "bounded
    reach"): the key→input-slice reach is derived (key-temporal-locality route) or
    declared-and-checked.
40. (906-910) New obligation 7 — affected-key discovery: the changed input's delta names a finite
    key set (`model_properties.md` §"Affected-key discovery"); a sound over-approximation is
    admissible (costs extra recomputation, never correctness); an under-approximation is never
    admissible (a missed key leaves stale state).
41. (912-914) All three obligations are fail-closed: any one unprovable refuses the repair family
    by name for that cell; it never widens to a whole-table repair; the refusal names which
    obligation failed (`MaintenanceRepairKeysNotDiscoverable` / `MaintenanceRepairSliceUnbounded`).
42. (916-919) Obligation 7 over a `mutable_snapshot` source: no tombstone/change history, so a key
    whose entire window contribution was deleted between runs leaves no row for a current-source
    scan to select — under obligation-7-forbidden under-approximation.
43. (919-923) For this posture the affected-key relation is instead the group-grain fingerprint
    sidecar diff (`sources.md` §"The fingerprint sidecar" — "Partition grain"): one row per output
    group key, so a vanished group still surfaces via "sidecar row with no matching source key".
44. (923-927) This discovery read is unbounded by the cell's `ScanClamp` — a clamped rescan
    compared against the sidecar's full stored digests would flag every group outside the clamp as
    spuriously changed, degrading to whole-table repair every run; the per-group recompute itself
    stays bounded by the discovered key set per obligation 4.
45. (928-934) An absent or stale-stamped comparandum cannot distinguish a vanished group from one
    that never existed, so the affected set widens further to every currently-observed group plus
    every group already present in stored output — a sound over-approximation, distinct from the
    admission-refusal rule; it degenerates to a whole-table repair for that one run and self-heals
    once the sidecar is refreshed.
46. (935-937) An append-only source (no native deletion) keeps the ordinary clamped current-source
    scan — the group-grain sidecar is scoped to the one posture that needs it.
47. (939-942) Ledger grading: per-group recompute is graded Idempotent for the keys in its slice,
    exactly like a region recompute; re-running reproduces the same state and resets any additive
    ledger record for those keys rather than folding a second time.
48. (944-946) Repair over a decomposed combiner: the fold path materialises hidden `__`-marked
    state columns alongside presented ones; a repaired group's candidate must carry them too or
    the write's implicit column list mismatches the physical table.
49. (947-950) The repair candidate is the model's own state-augmented projection, identical to the
    projection the fold's own create/merge path materialises: raw model SQL widened with the state
    columns' own `per_partition_expr`s before compilation, the same widening
    `execute_windowed_keyed`/`execute_snapshot_reconcile` already apply for the ordinary fold.
50. (951) A stateless column family widens to nothing — a no-op for every combiner admitted before
    decomposed state existed.
51. (952-956) A `diff_patch` write over a decomposed repair extends its change-suppression
    predicate to compare hidden state columns alongside presented compared columns: a group whose
    presented value is unchanged but whose state moved is still rewritten (suppressing that write
    would leave stale state behind a correct-looking value) — strictly less suppression than
    presented-only comparison, sound by construction.

## `### Windowed maintenance and the horizon` (958–1039)

52. (960-961) Maintenance runs over a bounded input window by default — a full scan is the
    surfaced fallback, not the baseline.
53. (961-965) A run reasons about two windows, always with `scan ⊇ write`: the write window
    (partitions/keys written this run) and the scan window (input rows read to produce that write
    window correctly).
54. (966-969) The scan window is bounded where the model carries a `timeseries:` clock:
    input-delta discovery is window-forward, only the new window (plus lookback) is read, stored
    state stands in for history; without a clock the source can only be snapshot-diffed, so the
    scan degrades to a full read (`models.md` §"Input-consumption axis").
55. (969-971) Scan windowing is orthogonal to output addressing: a clocked key-addressed model
    still windows its scan even though its write reaches back by key outside that window.
56. (971-975) Bounding the scan never weakens the invariant: the engine evaluates the model, joins
    included, over the widened scan window, and the write is clamped to the exact write window
    (`model_transforms.md` §"widened scan + exact clamp") — join optimisation stays with the
    engine rather than smelt hand-computing minimal deltas.
57. (977-979) The horizon (partition grain only) is a write-eligibility clamp — a bound on which
    partitions a run may write to: the far edge of the maintained window, past which inputs are
    no longer folded in.
58. (979-982) The horizon is derived, never trusted from a declaration: the clamp bounds are
    computed from the model's own reach (lookback, window frames, join contribution —
    `model_properties.md`); a declared horizon smaller than the true reach would drop rows that
    should have been rewritten.
59. (982-984) A modeller may declare a horizon ceiling (`horizon_ceiling: '30 days'`): smelt warns
    at compile time when the derived horizon would exceed it; the clamp always uses the derived
    value.
60. (986-989) Because the derived clamp is the model's SQL, a late arrival landing after its
    natural partition passed the horizon is silently excluded from the maintenance run at the
    default point, not diagnosed: smelt cannot fail loud on a row it never scans.
61. (989-991) Surfacing lateness is a model-author concern, not a maintenance guarantee, unless the
    model opts into the frozen horizon contract-lattice point (§"The contract lattice"), which
    turns this into a checked, diagnosed condition (`ContractLateArrivalOutsideHorizon`).
62. (992-995) The available pattern for the default point: fold the late row into the current
    partition (re-stamping its partition time) carrying a lateness/validity flag, and let a
    data-quality check raise on the flagged rows.
63. (997-999) The key grain has no write-eligibility clamp: a `grain: key` run merges every delta
    row it scans, into whatever key it names, however old (§"No write-eligibility clamp").
64. (999-1000) A derived forward reach is still computed and reported for observability, but it
    never gates admission and never bounds a write.
65. (1000-1004) This is deliberate: keyed write work is proportional to delta size regardless of
    key age, so a write clamp buys nothing for correctness — and would silently drop scanned
    inputs, the one thing the invariant forbids. Settled-key GC / a bounded working set is
    deferred optimisation that must ship together with late-fact accounting if ever introduced.
66. (1006-1008) Three pruning categories, one principle: only proofs prune; a declared bound is
    admitted only checked (fail-loud on violation); no unproven bound ever refuses a write.
67. (1010-1012) Category 1 — target-scan slice pruning (read-side): rows the write provably cannot
    touch are removed from the merge's read of stored state; licensed by key-temporal-locality
    proofs or the transactionally-checked recurrence declaration (§"Key temporal locality").
68. (1013-1024) Category 2 — no-op write elimination (write-side): a maintenance write is skipped
    iff the row's applied effect is proven to be the identity per row by evaluation (exact
    `IS DISTINCT FROM` over every column that can differ under the cell's trigger; comparing only
    the mutation-sensitive group is sound because other groups are proven insensitive); suppression
    may never skip evaluating a scanned input; a compared column must be a pure function of the
    processed inputs — a column that legitimately varies run to run (`contract: plausible`,
    run-pinned `NOW()`) is incomparable, and a cell containing one refuses the conditional
    technique, fail-closed.
69. (1023-1024) At fixed `S` the suppressed and unconditional variants produce identical state —
    interchangeable in the strongest sense of §"Per-cell admission", a cost-model/`prefer`/
    `technique` matter.
70. (1025-1028) `model_transforms.md` catalogues two physical realisations: change-suppressed MERGE
    (matched-arm `IS DISTINCT FROM` predicate, dialect-split on unmatched-by-source side) and the
    staged-candidate conditional DELETE+INSERT (merge-less realisation for a backend without
    MERGE), both licensed by region row identity plus per-column change comparability.
71. (1029-1030) Category 3 — write-eligibility clamps: forbidden on the key grain; derived-only on
    the partition grain (the horizon).
72. (1032-1034) Categories 1–2 preserve the invariant bit-for-bit at fixed `S`; category 3 bounds
    which inputs enter `S` at all. A suppressed write is the write-side dual of slice pruning
    (the proof is the per-row equality just evaluated), never a clamp.
73. (1035-1038) Two catalogued transforms read a derived forward reach without being write clamps:
    the dimension-driven horizon-bounded MERGE (scan/recompute bound on the enrichment recompute,
    not the write) and the horizon settled-delay/tail-rewrite mechanism (partition-grain
    forward-reach machinery); both in `model_transforms.md`.

## `### Partition-local maintenance (the K8 guardrail)` (1040–1051)

74. (1042-1043) A cell's per-`(cell × source)` locality verdict is the partition-locality
    projection (`model_properties.md` owns the proof, including the cross-axis predicate
    requirement).
75. (1043-1046) This section owns the policy consuming the verdict: emitted maintenance SQL must
    carry the partition predicate on both the scan and the merge/overwrite target — a bound stated
    only on a non-partition column is one the storage layer cannot prune by.
76. (1046-1049) Under the default `scan_bounds` (`require: partition_local`, `on_violation:
    error`), a non-local cell refuses (`MaintenanceScanUnbounded`) unless the source carries
    `allow_full_scan: true`; `max_lookback` additionally refuses a derived span wider than the
    operator's stated expectation.
77. (1049-1050) The guardrail never modifies a clamp — it only refuses or warns (§Surface
    "Maintenance overrides").

## `### Statement emission (single owner)` (1052–1082)

78. (1054-1057) The physical statements a run executes for a cell (region DELETE+INSERT pair,
    keyed fold MERGE, column-scoped MERGE, in-place UPDATE, first-run `CREATE TABLE...AS`) are
    produced by pure emitter functions in the maintenance layer (`smelt-logical`) — the
    statement-level counterpart of "one derivation, many consumers".
79. (1057-1061) An emitter is a pure function from plain data (target table, region literals, key
    columns, combiner-rendered set expressions, the compiled/clamped SELECT body, a dialect tag)
    to an ordered statement group plus its transactional requirement (a paired DELETE+INSERT is
    one transaction: a failed INSERT rolls back its DELETE).
80. (1061-1064) Backends execute emitted statements (connections, transactions, blocking dispatch)
    and never author maintenance-statement text; dialect differences (e.g. `MERGE...UPDATE SET *`
    needing a full-row source projection versus an explicit column-list `SET`) live in the
    emitters as dialect-keyed variants.
81. (1066-1069) Three deliberate exclusions, all warehouse-resident bookkeeping owned per dialect
    by `smelt-state`, each interleaved transactionally with the write it describes but not itself
    a maintenance statement.
82. (1070) Exclusion 1 — the reconciliation ledger's DDL/DML (§"The frontier record
    (reconciliation ledger)").
83. (1071) Exclusion 2 — the observed-output-delta record (§"The graph layer").
84. (1072-1076) Exclusion 3 — the fingerprint sidecar's own storage (table DDL, digest-refresh
    upsert, GC delete, `sources.md` §"The fingerprint sidecar"), except the sidecar's diff query
    IS emitter-authored (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`), since
    which source keys count as "changed" is a derived maintenance-relevant comparison.
85. (1078) Non-maintenance SQL (introspection, seed loading, schema-evolution DDL) is outside this
    rule.
86. (1079-1081) Single ownership is what makes maintenance SQL observable: the same emitters serve
    execution, the conformance equivalence gates, and `smelt explain --show-sql`, so printed SQL
    cannot drift from executed SQL.

## `### The definition-change trigger` (1083–1115)

87. (1085-1086) A model gaining output fields is a trigger of its own kind: the added group's
    processed-input vector is `∅` over every existing region, and its backfill advances
    `∅ → current`, touching only the new group.
88. (1087-1089) The classification of an added field (`SkeletonAdd` / `PureBackfill` /
    `UpstreamRederive`) is the definition-change column classification proof
    (`model_properties.md`); this section owns the plan-level policy each maps to.
89. (1091-1093) `SkeletonAdd` (identity/grouping/dedup/ordering) is a grain change, refused as a
    column backfill (`MaintenanceSkeletonColumnAdded`) — the honest plan is a recompute,
    effectively a new model.
90. (1094-1096) `PureBackfill` lands in the 2×2's targeted-write column as an in-place UPDATE (no
    upstream read); `UpstreamRederive` lands there as a column-scoped MERGE, keyed where the
    source is keyed, inheriting each read source's partition-locality verdict unchanged.
91. (1097-1099) Fields added together factor by shared mutation-sensitivity, one backfill op per
    group; the backfill of a newly-added group is always full-input, even for a column whose
    ongoing algebra folds — there is no prior state of that column to fold onto.
92. (1100-1103) Group convergence: a field co-sensitive with an existing group still instantiates
    at `∅` and forms its own catch-up group; mid-catch-up, a delta folds into the sibling group
    but is refused on the new group's unbackfilled regions (never fold ahead of the entry); the
    groups merge only once the new group's processed vector equals its sibling's over every
    region.
93. (1104-1109) The backfill is atomic with the column's own migration: a `PureBackfill` field's
    physical column and its backfilled values are created by the SAME statement group as the
    schema migration that adds the column — never a separately-dispatched write that could observe
    the column already added but not yet backfilled; the backfill's UPDATE is folded into the
    migration's `ADD COLUMN` statement group before it executes, the same mechanism a declared
    `backfill:`/`default:` frontmatter directive already used.
94. (1110-1114) A group failure (transactional-DDL backend) leaves neither the physical column nor
    the saved deployed-schema snapshot changed, so the next run's diff still sees the column
    missing and retries the whole migration+backfill together — there is no window in which the
    deployed-schema snapshot can outrun the column's real values (cross-ref §Known Divergences for
    the one case this does not cover).
