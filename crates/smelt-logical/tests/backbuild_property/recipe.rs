//! Structural (never-SQL-text) recipes for the generative backbuild
//! conformance gate (`docs/plans/20260802-backbuild-followups.md` Phase 7).
//! `BeforeRecipe` draws a before-definition shape over the fixed source pool
//! (`data.rs`); `EditRecipe` draws 1-3 structural edits that mirror the
//! research catalogue's technique axis. `compatible_edits`/
//! `compatible_adversarial_edits` are the "re-draw structurally rather than
//! emitting invalid SQL" gate the plan calls for: an edit incompatible with
//! the drawn `BeforeRecipe` (e.g. `AddAggregate` on an ungrouped model) is
//! never offered, so `proptest` never has to retry into validity — it only
//! ever samples from the compatible set. Rendering to actual SQL text lives
//! in `render.rs`; this module never builds a SQL string.

#![allow(dead_code)]

use proptest::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {
    CustomerId,
    Amount,
    Qty,
    Status,
    Ts,
}

impl Col {
    pub fn name(self) -> &'static str {
        match self {
            Col::CustomerId => "customer_id",
            Col::Amount => "amount",
            Col::Qty => "qty",
            Col::Status => "status",
            Col::Ts => "ts",
        }
    }

    pub fn sql_type(self) -> &'static str {
        match self {
            Col::CustomerId | Col::Amount | Col::Qty => "INTEGER",
            Col::Status => "VARCHAR",
            Col::Ts => "DATE",
        }
    }

    /// Whether this column's underlying `src_orders` source is nullable
    /// (`data.rs`'s DDL: only `status` is) — the fact `render.rs` consumes
    /// to build `BackbuildInputs::not_null_columns` truthfully.
    pub fn nullable(self) -> bool {
        matches!(self, Col::Status)
    }
}

pub fn all_cols() -> Vec<Col> {
    vec![Col::CustomerId, Col::Amount, Col::Qty, Col::Status, Col::Ts]
}

/// The single set of source-table constants both `data.rs` (staged DDL) and
/// `render.rs` (`BackbuildInputs::sources` facts) build from — one column
/// list feeds both a `CREATE TABLE` string and a `SourceRef`, so a name,
/// type, nullability, or unique-key edit here cannot desynchronize DDL from
/// declared facts (see this module's `Col`, which the `src_orders` schema
/// wraps around, and the two dimension schemas below).
pub mod schema {
    use super::{all_cols, Col};

    /// One column's DDL-relevant facts: name, SQL type, and whether it is
    /// declared `NOT NULL`.
    #[derive(Debug, Clone, Copy)]
    pub struct ColumnSpec {
        pub name: &'static str,
        pub sql_type: &'static str,
        pub not_null: bool,
    }

    /// A table's full schema: physical name, ordered columns, and unique
    /// key (single-column, for this fixed source pool).
    pub struct TableSchema {
        pub table_name: &'static str,
        pub columns: Vec<ColumnSpec>,
        pub unique_key: &'static str,
    }

    impl TableSchema {
        /// Renders this schema's `CREATE TABLE` statement — the only place
        /// DDL text is assembled from column facts, so `data.rs` never
        /// hand-writes a column list that could drift from `Col`/this
        /// module's dimension specs.
        pub fn create_table_sql(&self) -> String {
            let cols: Vec<String> = self
                .columns
                .iter()
                .map(|c| {
                    let suffix = if c.not_null { " NOT NULL" } else { "" };
                    format!("{} {}{suffix}", c.name, c.sql_type)
                })
                .collect();
            format!("CREATE TABLE {} ({});", self.table_name, cols.join(", "))
        }

        /// Declared `NOT NULL` column names — consumed by `render.rs` to
        /// build `SourceRef::not_null_columns` truthfully (the same fact
        /// `create_table_sql` renders as `NOT NULL`).
        pub fn not_null_column_names(&self) -> Vec<&'static str> {
            self.columns
                .iter()
                .filter(|c| c.not_null)
                .map(|c| c.name)
                .collect()
        }
    }

    /// `src_orders(order_id INTEGER NOT NULL /*unique*/, customer_id INTEGER
    /// NOT NULL, amount INTEGER NOT NULL, qty INTEGER NOT NULL, status
    /// VARCHAR, ts DATE NOT NULL)` — `order_id` (the row identity, never a
    /// [`Col`] recipes select over) plus every [`Col`] with its declared
    /// type/nullability.
    pub fn orders_schema() -> TableSchema {
        let mut columns = vec![ColumnSpec {
            name: "order_id",
            sql_type: "INTEGER",
            not_null: true,
        }];
        columns.extend(all_cols().into_iter().map(|c: Col| ColumnSpec {
            name: c.name(),
            sql_type: c.sql_type(),
            not_null: !c.nullable(),
        }));
        TableSchema {
            table_name: "src_orders",
            columns,
            unique_key: "order_id",
        }
    }

    /// `src_dim(dim_id INTEGER NOT NULL /*unique*/, region VARCHAR, score
    /// INTEGER)`.
    pub fn dim_schema() -> TableSchema {
        TableSchema {
            table_name: "src_dim",
            columns: vec![
                ColumnSpec {
                    name: "dim_id",
                    sql_type: "INTEGER",
                    not_null: true,
                },
                ColumnSpec {
                    name: "region",
                    sql_type: "VARCHAR",
                    not_null: false,
                },
                ColumnSpec {
                    name: "score",
                    sql_type: "INTEGER",
                    not_null: false,
                },
            ],
            unique_key: "dim_id",
        }
    }

    /// `src_dim2(zone_id INTEGER NOT NULL /*unique*/, zone VARCHAR)`.
    pub fn dim2_schema() -> TableSchema {
        TableSchema {
            table_name: "src_dim2",
            columns: vec![
                ColumnSpec {
                    name: "zone_id",
                    sql_type: "INTEGER",
                    not_null: true,
                },
                ColumnSpec {
                    name: "zone",
                    sql_type: "VARCHAR",
                    not_null: false,
                },
            ],
            unique_key: "zone_id",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A bare projection over `src_orders o`, no join/group/branch.
    Plain,
    /// `GROUP BY o.customer_id` — `columns` is unused (the grouped
    /// projection is fixed: `customer_id`, `count(*) AS n`).
    Grouped,
    /// A pre-existing `LEFT JOIN src_dim d ON o.customer_id = d.dim_id`,
    /// with `d.region AS region` already pulled through.
    Joined,
    /// A single `UNION ALL` branch (`src_orders` only) with a literal
    /// discriminator — `columns` is unused (fixed: `order_id`, `status`,
    /// `src`).
    SingleBranch,
    /// Two `UNION ALL` branches (`src_orders` + `src_dim2`) already present.
    DoubleBranch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhereKind {
    AmountPositive,
    QtyPositive,
    StatusOpen,
    TsHorizon,
}

impl WhereKind {
    fn requires(self) -> Col {
        match self {
            WhereKind::AmountPositive => Col::Amount,
            WhereKind::QtyPositive => Col::Qty,
            WhereKind::StatusOpen => Col::Status,
            WhereKind::TsHorizon => Col::Ts,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BeforeRecipe {
    pub shape: Shape,
    /// Bare pull-through columns beyond the always-present `order_id`
    /// (ignored for `Grouped`/`SingleBranch`/`DoubleBranch`, which have a
    /// fixed projection).
    pub columns: Vec<Col>,
    pub where_conjuncts: Vec<WhereKind>,
}

/// One structural edit — mirrors the research catalogue's technique axis
/// (research §4). `is_adversarial` variants are drawn only by
/// `compatible_adversarial_edits`/the dedicated adversarial test, never
/// mixed into the main gate's technique-axis draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRecipe {
    /// B1: a new column computed from stored `amount`/`qty`.
    AddSelfDerived,
    /// B3: a new column that is a bare pull-through of a not-yet-selected
    /// order column, requiring a fresh upstream read.
    AddPullthrough,
    /// B4: a newly added `LEFT JOIN src_dim d`, pulling either a bare
    /// column (`general: false`) or a general expression around one
    /// (`general: true`, `COALESCE(d.score, -1)`).
    AddJoin { general: bool },
    /// B5: a new aggregate column at the unchanged `GROUP BY` grain.
    AddAggregate,
    /// B6: a new window-function column over stored columns.
    AddWindow,
    /// B2: rename `customer_id` to `account_id`.
    Rename,
    /// D1: rewrite `amount`'s own expression to `o.amount * 2`.
    RewriteSelfDerived,
    /// D2: rewrite `qty`'s own expression to `o.customer_id` (an upstream
    /// column not currently stored under that name) — routes through the
    /// same `UpstreamPullthrough` technique B3 uses.
    RewriteFromUpstream,
    /// E1: add a `status IS NOT NULL` conjunct.
    TightenFilter,
    /// E1: add a `status = 'open'` conjunct — unlike [`Self::TightenFilter`]'s
    /// `IS NOT NULL` (which never itself evaluates NULL for a NULL `status`
    /// row), an equality conjunct evaluates NULL, not just FALSE, for such a
    /// row, so `emit_predicate_delete`'s `IS NOT TRUE` three-valued-logic
    /// wrapping is actually exercised (mutation-audit finding 2a,
    /// `docs/handoffs/2026-08-03-backbuild-property-test-review.md`).
    TightenFilterStatusOpen,
    /// E2: remove the (sole) existing conjunct.
    LoosenFilter,
    /// E4: widen the existing `ts` horizon conjunct.
    WidenHorizon,
    /// F1: add a second `UNION ALL` branch.
    AddBranch,
    /// F2: remove one of two existing `UNION ALL` branches.
    RemoveBranch,
    /// C1: drop the `ts` column.
    DropColumn,
    /// Adversarial: replace the whole projection with a freshly grouped
    /// one — a grain change (G1), never admissible.
    AdversarialGrainChange,
    /// Adversarial: add a non-`LEFT` join — a join-multiplicity change
    /// (G2), never admissible.
    AdversarialInnerJoin,
    /// Adversarial: add a column computed with `RANDOM()` — a volatile
    /// added expression, never admissible.
    AdversarialVolatileFn,
    /// Adversarial: rewrite a multi-conjunct `WHERE` from `AND` to `OR` —
    /// an opaque (non-conjunctive) predicate rewrite, never admissible.
    AdversarialOpaqueRewrite,
}

fn has(cols: &[Col], c: Col) -> bool {
    cols.contains(&c)
}

/// Every non-adversarial (technique-axis) edit compatible with `before` —
/// never empty for any `BeforeRecipe` this module can draw (see the
/// per-shape reasoning in each arm below), so `arb_case`'s
/// `subsequence(compatible, 1..=k)` never underflows.
pub fn compatible_edits(before: &BeforeRecipe) -> Vec<EditRecipe> {
    let mut out = Vec::new();
    let cols = &before.columns;
    match before.shape {
        Shape::Plain | Shape::Joined => {
            // `AddSelfDerived` reads only `order_id` — always present and
            // never itself the target of `Rename`/`RewriteSelfDerived`/
            // `RewriteFromUpstream`/`DropColumn`, so it never depends on a
            // column another compatible edit might simultaneously change
            // in the same diff (a dependency on a *changed* column would
            // drop out of the unchanged-representative set B1's proof
            // needs, and correctly refuse to resolve it) — unconditionally
            // compatible.
            out.push(EditRecipe::AddSelfDerived);
            if all_cols().iter().any(|c| !has(cols, *c)) {
                out.push(EditRecipe::AddPullthrough);
            }
            if before.shape == Shape::Plain {
                out.push(EditRecipe::AddJoin { general: false });
                out.push(EditRecipe::AddJoin { general: true });
                if has(cols, Col::Status) {
                    out.push(EditRecipe::AddWindow);
                }
            }
            if has(cols, Col::CustomerId) {
                out.push(EditRecipe::Rename);
            }
            if has(cols, Col::Amount) {
                out.push(EditRecipe::RewriteSelfDerived);
            }
            if has(cols, Col::Qty) {
                out.push(EditRecipe::RewriteFromUpstream);
            }
            // `TightenFilter` targets `status` (not `qty`) so it never
            // competes with `RewriteFromUpstream`'s own `qty`-rewrite for
            // the same column's "unchanged representative" status in the
            // same diff (E1's requalification needs its conjunct's
            // dependency to still resolve as *unchanged*).
            // Both E1 tighten-on-`status` variants are offered together:
            // `arb_case` drawing them in the same diff yields two added
            // conjuncts on the same (unchanged) column — two independent E1
            // atoms, each requalified on its own. This combination is what
            // originally exposed the `Expr::as_binary` child-first-cast
            // parser bug (three-conjunct AND chains mis-split; fixed
            // 2026-08-07, pinned by `backbuild_options.rs::
            // e1_three_conjunct_and_chain_with_duplicate_splits_and_requalifies_fully`),
            // so drawing it generatively is regression coverage, not a
            // hazard. `TightenFilterStatusOpen`'s own added conjunct text
            // (`status = 'open'`) is still withheld whenever `before`
            // already carries a `StatusOpen` conjunct: combined with
            // `LoosenFilter` removing that same solitary conjunct it
            // exactly cancels an existing predicate and renders a no-op
            // diff, and `compatible_edits` cannot see which other edits a
            // `subsequence` draw will pair it with.
            if has(cols, Col::Status) {
                out.push(EditRecipe::TightenFilter);
                if !before.where_conjuncts.contains(&WhereKind::StatusOpen) {
                    out.push(EditRecipe::TightenFilterStatusOpen);
                }
            }
            if before.where_conjuncts.len() == 1
                && before.where_conjuncts[0] != WhereKind::TsHorizon
            {
                out.push(EditRecipe::LoosenFilter);
            }
            if has(cols, Col::Ts) {
                if before.where_conjuncts.contains(&WhereKind::TsHorizon) {
                    out.push(EditRecipe::WidenHorizon);
                }
                out.push(EditRecipe::DropColumn);
            }
        }
        // `AddAggregate` is unconditionally compatible with `Grouped` —
        // never empty.
        Shape::Grouped => out.push(EditRecipe::AddAggregate),
        // `AddBranch`/`RemoveBranch` are unconditionally compatible with
        // their own shape — never empty.
        Shape::SingleBranch => out.push(EditRecipe::AddBranch),
        Shape::DoubleBranch => out.push(EditRecipe::RemoveBranch),
    }
    out
}

/// Every adversarial edit compatible with `before` (research §2 "we never
/// create a wrong plan" from the refusal side) — used only by
/// `adversarial_edits_always_refuse_or_verify`, never mixed with
/// `compatible_edits`'s technique-axis draws. Never empty for `Plain`/
/// `Joined`/`Grouped` (`AdversarialInnerJoin`/`AdversarialVolatileFn` are
/// unconditional); branch shapes have no adversarial edit defined and are
/// filtered out by `arb_before_for_adversarial`.
pub fn compatible_adversarial_edits(before: &BeforeRecipe) -> Vec<EditRecipe> {
    let mut out = Vec::new();
    match before.shape {
        Shape::Plain | Shape::Joined | Shape::Grouped => {
            out.push(EditRecipe::AdversarialInnerJoin);
            out.push(EditRecipe::AdversarialVolatileFn);
            if before.shape != Shape::Grouped {
                out.push(EditRecipe::AdversarialGrainChange);
            }
            if before.where_conjuncts.len() >= 2 {
                out.push(EditRecipe::AdversarialOpaqueRewrite);
            }
        }
        Shape::SingleBranch | Shape::DoubleBranch => {}
    }
    out
}

fn arb_columns() -> impl Strategy<Value = Vec<Col>> {
    proptest::sample::subsequence(all_cols(), 4..=5)
}

/// Each compatible `WhereKind` is included independently with 65%
/// probability (rather than a uniform-random *subset size*) — biased high
/// enough that a rare, single-conjunct-gated edit (`WidenHorizon` needs
/// `TsHorizon` to be *present*) is reliably drawn often enough across
/// [`DEFAULT_CASES`] for `admission_rate_stays_above_floor`'s per-technique
/// coverage guard.
///
/// `LoosenFilter`'s precondition is narrower than "any subset" reaches
/// reliably: it needs *exactly one* non-`TsHorizon` conjunct
/// (`compatible_edits`'s `where_conjuncts.len() == 1` gate), which the
/// independent-inclusion draw above only hits by chance (≈8% per eligible
/// case) — too rare to land at least once in [`DEFAULT_CASES`] draws. A
/// dedicated 30%-weighted "forced singleton" arm makes that precondition
/// directly reachable instead of hoping the general distribution wanders
/// into it, without disturbing the general-shape coverage the 65% arm gives
/// every other conjunct combination.
fn arb_where_for(columns: Vec<Col>) -> impl Strategy<Value = Vec<WhereKind>> {
    let mut candidates = Vec::new();
    for kind in [
        WhereKind::AmountPositive,
        WhereKind::QtyPositive,
        WhereKind::StatusOpen,
        WhereKind::TsHorizon,
    ] {
        if columns.contains(&kind.requires()) {
            candidates.push(kind);
        }
    }
    let non_ts: Vec<WhereKind> = candidates
        .iter()
        .copied()
        .filter(|k| *k != WhereKind::TsHorizon)
        .collect();
    let general = {
        let candidates = candidates.clone();
        proptest::collection::vec(
            prop_oneof![13 => Just(true), 7 => Just(false)],
            candidates.len(),
        )
        .prop_map(move |flags| {
            candidates
                .iter()
                .zip(flags)
                .filter_map(|(k, include)| include.then_some(*k))
                .collect::<Vec<_>>()
        })
        .boxed()
    };
    let has_ts = candidates.contains(&WhereKind::TsHorizon);
    if non_ts.is_empty() && !has_ts {
        general
    } else {
        let mut arms: Vec<(u32, proptest::strategy::BoxedStrategy<Vec<WhereKind>>)> =
            vec![(4, general)];
        if !non_ts.is_empty() {
            arms.push((
                4,
                proptest::sample::select(non_ts)
                    .prop_map(|k| vec![k])
                    .boxed(),
            ));
        }
        if has_ts {
            // Forced-Ts arm: `WidenHorizon`'s precondition
            // (`where_conjuncts.contains(TsHorizon)`) needs `TsHorizon`
            // present at all, not exactly-one — a dedicated arm that always
            // includes it (plus each other eligible conjunct independently)
            // makes that reachable directly rather than relying on the
            // general arm's per-conjunct 65% draw alone.
            let rest: Vec<WhereKind> = candidates
                .iter()
                .copied()
                .filter(|k| *k != WhereKind::TsHorizon)
                .collect();
            let forced_ts =
                proptest::collection::vec(any::<bool>(), rest.len()).prop_map(move |flags| {
                    let mut v: Vec<WhereKind> = rest
                        .iter()
                        .zip(flags)
                        .filter_map(|(k, include)| include.then_some(*k))
                        .collect();
                    v.push(WhereKind::TsHorizon);
                    v
                });
            arms.push((2, forced_ts.boxed()));
        }
        proptest::strategy::Union::new_weighted(arms).boxed()
    }
}

fn arb_plain_or_joined(joined: bool) -> impl Strategy<Value = BeforeRecipe> {
    arb_columns().prop_flat_map(move |cols| {
        let cols_for_map = cols.clone();
        arb_where_for(cols).prop_map(move |wh| BeforeRecipe {
            shape: if joined { Shape::Joined } else { Shape::Plain },
            columns: cols_for_map.clone(),
            where_conjuncts: wh,
        })
    })
}

/// `Grouped`'s fixed projection ignores `columns`, but its `FROM src_orders
/// o` still exposes every [`Col`] to a `WHERE` conjunct — so `where_conjuncts`
/// is drawn over [`all_cols`] rather than the (unused, empty) `columns` field.
/// Previously always empty (mutation-audit finding 2b: B5's re-aggregation
/// subquery carrying the model's `WHERE` clause,
/// `docs/handoffs/2026-08-03-backbuild-property-test-review.md`) — a
/// hardcoded empty `WHERE` here meant the generator could never render a
/// `Grouped` case whose `AddAggregate` script had a `WHERE` clause to drop.
fn arb_grouped() -> impl Strategy<Value = BeforeRecipe> {
    arb_where_for(all_cols()).prop_map(|wh| BeforeRecipe {
        shape: Shape::Grouped,
        columns: Vec::new(),
        where_conjuncts: wh,
    })
}

fn arb_single_branch() -> impl Strategy<Value = BeforeRecipe> {
    Just(BeforeRecipe {
        shape: Shape::SingleBranch,
        columns: Vec::new(),
        where_conjuncts: Vec::new(),
    })
}

fn arb_double_branch() -> impl Strategy<Value = BeforeRecipe> {
    Just(BeforeRecipe {
        shape: Shape::DoubleBranch,
        columns: Vec::new(),
        where_conjuncts: Vec::new(),
    })
}

/// Every `BeforeRecipe` shape the main gate draws from.
pub fn arb_before_recipe() -> impl Strategy<Value = BeforeRecipe> {
    prop_oneof![
        4 => arb_plain_or_joined(false),
        2 => arb_plain_or_joined(true),
        2 => arb_grouped(),
        1 => arb_single_branch(),
        1 => arb_double_branch(),
    ]
}

/// `Plain`/`Joined`/`Grouped` only (`compatible_adversarial_edits`'s
/// domain) — used by the dedicated adversarial test.
pub fn arb_before_for_adversarial() -> impl Strategy<Value = BeforeRecipe> {
    prop_oneof![
        2 => arb_plain_or_joined(false),
        2 => arb_plain_or_joined(true),
        1 => arb_grouped(),
    ]
}

/// `AddWindow` (B6, `Technique::WindowColumnBackfill`) composed in the
/// *same* script with a row-set-changing sibling atom
/// (`TightenFilter`/`PredicateTightenDelete`,
/// `LoosenFilter`/`FilterLoosenInsert`, `WidenHorizon`/
/// `HorizonExtensionInsert` — every `EditRecipe` this module offers whose
/// admitted technique deletes or inserts rows) is a legitimate generated
/// case, not a masked one: `derive_backbuild_options`'s cross-atom guard
/// (`apply_b6_row_set_change_guard` in
/// `crates/smelt-logical/src/backbuild/classify.rs`) strips B6 from the
/// window atom's options and records a named refusal whenever the diff also
/// carries a row-set-changing atom, so the main gate's own "an atom with no
/// admissible option yields an empty targeted script plus a named refusal"
/// branch (`generated_options_match_full_rebuild_oracle`) exercises this
/// combination directly — no generator-side exclusion needed.
pub fn arb_case() -> impl Strategy<Value = (BeforeRecipe, Vec<EditRecipe>)> {
    arb_before_recipe().prop_flat_map(|before| {
        let compatible = compatible_edits(&before);
        debug_assert!(
            !compatible.is_empty(),
            "compatible_edits must never be empty: {before:?}"
        );
        let max_k = compatible.len().min(3);
        let before_for_map = before.clone();
        proptest::sample::subsequence(compatible, 1..=max_k)
            .prop_map(move |edits| (before_for_map.clone(), edits))
    })
}

/// The technique-axis edits whose `compatible_edits` precondition is itself
/// narrow (an exact-shape `WhereKind`/grouping/branch gate, not just "some
/// column present"): a uniform `subsequence` draw over a 6-11-entry
/// compatible list picks any *particular* one of them only ~1/3 of the time,
/// and two of them landing in the *same* case can make the WHERE-diff
/// classifiers correctly refuse to compose (e.g. `TightenFilter` +
/// `WidenHorizon` together — research §4's "multiple simultaneous removed
/// conjuncts", not a classifier bug; verified directly against
/// `derive_backbuild_options` while diagnosing this list). Chasing that
/// combinatorics with per-edit inclusion probabilities against a single
/// fixed [`proptest::test_runner::TestRunner::deterministic`] seed is
/// whack-a-mole — nudging one edit's odds reshuffles the RNG stream every
/// later case draws from. [`guaranteed_case`] sidesteps it: reserve the
/// first `GUARANTEED_EDITS.len()` case slots for a single deterministic
/// `(BeforeRecipe, [edit])` pair each — still built from the same
/// `BeforeRecipe`/rendering path the rest of the gate uses, still
/// oracle-verified like every other case, just not proptest-drawn — so
/// `admission_rate_stays_above_floor`'s per-[`super::Technique`] coverage
/// tally is true by construction instead of by chance. The remaining case
/// slots stay fully [`arb_case`]-generative for shape/composition/data
/// diversity.
pub const GUARANTEED_EDITS: &[EditRecipe] = &[
    EditRecipe::AddSelfDerived,
    EditRecipe::Rename,
    EditRecipe::RewriteSelfDerived,
    EditRecipe::AddPullthrough,
    EditRecipe::AddJoin { general: false },
    EditRecipe::TightenFilter,
    EditRecipe::TightenFilterStatusOpen,
    EditRecipe::WidenHorizon,
    EditRecipe::LoosenFilter,
    EditRecipe::AddBranch,
    EditRecipe::RemoveBranch,
    EditRecipe::AddAggregate,
    EditRecipe::AddWindow,
    EditRecipe::DropColumn,
];

/// The `BeforeRecipe` a [`GUARANTEED_EDITS`] entry needs to be compatible
/// (see `compatible_edits`'s per-edit preconditions) — deliberately minimal
/// per edit (empty `where_conjuncts` unless the edit's own precondition
/// needs one) so the case stays a *single*-edit diff, never colliding with
/// another edit's own conjunct.
fn guaranteed_before(edit: EditRecipe) -> BeforeRecipe {
    let all = all_cols();
    match edit {
        // A non-empty `WHERE` here (rather than `Vec::new()`) makes the B5
        // WHERE-carry property (mutation-audit finding 2b) a guaranteed,
        // every-run assertion rather than one only the generative arm might
        // draw.
        EditRecipe::AddAggregate => BeforeRecipe {
            shape: Shape::Grouped,
            columns: Vec::new(),
            where_conjuncts: vec![WhereKind::AmountPositive],
        },
        EditRecipe::AddBranch => BeforeRecipe {
            shape: Shape::SingleBranch,
            columns: Vec::new(),
            where_conjuncts: Vec::new(),
        },
        EditRecipe::RemoveBranch => BeforeRecipe {
            shape: Shape::DoubleBranch,
            columns: Vec::new(),
            where_conjuncts: Vec::new(),
        },
        // `StatusOpen` (not `AmountPositive`) — `AmountPositive`'s `amount >
        // 0` is NOT NULL and 3VL-blind; `status = 'open'` evaluates NULL for
        // a NULL-status row (boundary row 92), so removing it exercises the
        // E2 complement-form `IS NOT TRUE` wrapping's NULL-semantics case
        // deterministically, at the default case count (mutation-audit
        // finding 3, `docs/handoffs/2026-08-03-backbuild-property-test-review.md`).
        EditRecipe::LoosenFilter => BeforeRecipe {
            shape: Shape::Plain,
            columns: all,
            where_conjuncts: vec![WhereKind::StatusOpen],
        },
        EditRecipe::WidenHorizon => BeforeRecipe {
            shape: Shape::Plain,
            columns: all,
            where_conjuncts: vec![WhereKind::TsHorizon],
        },
        // `Joined` (though `compatible_edits` now offers this variant for
        // `Plain` too) — keeps the guaranteed single-edit slot exercising
        // the joined rendering path, complementing the combined
        // both-tighten-variants slot's `Plain` before.
        EditRecipe::TightenFilterStatusOpen => BeforeRecipe {
            shape: Shape::Joined,
            columns: all,
            where_conjuncts: Vec::new(),
        },
        EditRecipe::AddPullthrough => BeforeRecipe {
            shape: Shape::Plain,
            columns: all.into_iter().filter(|c| *c != Col::Ts).collect(),
            where_conjuncts: Vec::new(),
        },
        _ => BeforeRecipe {
            shape: Shape::Plain,
            columns: all,
            where_conjuncts: Vec::new(),
        },
    }
}

/// The deterministic case for [`GUARANTEED_EDITS`] slot `i` (see
/// [`GUARANTEED_EDITS`]'s doc comment) — `None` once `i` runs past the
/// reserved slots, so callers fall back to [`arb_case`].
/// Total reserved deterministic slots: one per [`GUARANTEED_EDITS`] entry
/// plus the combined both-tighten-variants slot [`guaranteed_case`] serves
/// at the index just past the list.
pub fn guaranteed_slot_count() -> usize {
    GUARANTEED_EDITS.len() + 1
}

pub fn guaranteed_case(i: usize) -> Option<(BeforeRecipe, Vec<EditRecipe>)> {
    // One multi-edit slot beyond the single-edit list: both E1 tighten
    // variants in one diff over a `before` that ALREADY carries the
    // `StatusOpen` conjunct — rendering the after-WHERE as a
    // three-conjunct AND chain whose trailing conjunct duplicates the
    // pre-existing one. This is byte-for-byte the composition that exposed
    // the `Expr::as_binary` child-first-cast parser bug (2026-08-07): the
    // duplicate lets the conjunct diff pair the before-conjunct against
    // the *trailing* duplicate, so a mis-split `(a AND b)` subtree
    // surfaces as a single added conjunct and E1 emits half-requalified
    // invalid SQL. A shorter chain or a duplicate-free one degrades to a
    // refusal instead and detects nothing (verified by mutation
    // re-injection). Deterministic so the case runs every time, not only
    // when `arb_case` draws the pair.
    //
    // `TightenFilterStatusOpen` is deliberately drawn OUTSIDE
    // `compatible_edits` here: the generative arm withholds it for a
    // `StatusOpen`-bearing before only because a co-drawn `LoosenFilter`
    // could cancel the predicate into a no-op diff — a fixed slot with no
    // `LoosenFilter` cannot hit that, and the duplicate-conjunct rendering
    // is exactly the point.
    if i == GUARANTEED_EDITS.len() {
        let before = BeforeRecipe {
            shape: Shape::Plain,
            columns: all_cols(),
            where_conjuncts: vec![WhereKind::StatusOpen],
        };
        let edits = vec![
            EditRecipe::TightenFilter,
            EditRecipe::TightenFilterStatusOpen,
        ];
        debug_assert!(
            compatible_edits(&before).contains(&EditRecipe::TightenFilter),
            "combined tighten slot's before must offer TightenFilter: {before:?}"
        );
        return Some((before, edits));
    }
    let edit = *GUARANTEED_EDITS.get(i)?;
    let before = guaranteed_before(edit);
    debug_assert!(
        compatible_edits(&before).contains(&edit),
        "guaranteed_before({edit:?}) must make it compatible: {before:?}"
    );
    Some((before, vec![edit]))
}

/// One case for the adversarial test: a `BeforeRecipe` plus exactly one
/// adversarial edit.
pub fn arb_adversarial_case() -> impl Strategy<Value = (BeforeRecipe, EditRecipe)> {
    arb_before_for_adversarial().prop_flat_map(|before| {
        let candidates = compatible_adversarial_edits(&before);
        debug_assert!(
            !candidates.is_empty(),
            "compatible_adversarial_edits must never be empty: {before:?}"
        );
        let before_for_map = before.clone();
        proptest::sample::select(candidates).prop_map(move |edit| (before_for_map.clone(), edit))
    })
}
