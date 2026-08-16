//! The standing generative conformance gate for backbuild synthesis
//! (`docs/plans/20260802-backbuild-followups.md` Phase 7; oracle
//! `docs/research/20260802-backbuild-synthesis.md` §2, §6). The property the
//! whole module claims, made generative: for any generated model and any
//! generated edit, every backbuild option the classifier derives — and every
//! bounded composed selection — applied to a real DuckDB, is multiset-equal
//! to a full rebuild of the after-definition over the same staged inputs.
//!
//! `TestRunner::deterministic()` throughout — no wall-clock/randomness
//! outside `proptest`, mirroring `crates/smelt-cli/tests/
//! maintenance_conformance/gate.rs`'s loop idiom. Default case count is
//! [`DEFAULT_CASES`]; override with `SMELT_BACKBUILD_CASES`.
//!
//! Module map: `recipe.rs` draws structural (never-SQL-text)
//! `BeforeRecipe`/`EditRecipe` values over the fixed source pool; `render.rs`
//! turns a recipe into actual SQL text plus `BackbuildInputs`; `data.rs`
//! generates the fact-table rows staged alongside a small fixed dimension
//! fixture. `proptest` shrinks the recipe, never the rendered SQL.
//!
//! Known generative blind spot (2026-08-03 mutation audit, M5): ALTER
//! `DROP COLUMN` *ordering* (drops must run last, after every statement
//! that reads the dropped column) is unobservable here at any case count —
//! no composed script the generator can express both drops a column and
//! reads it, because edit combinations that would are correctly refused at
//! admission. Drop ordering is covered behaviorally by the conformance
//! suite only (`backbuild_conformance.rs::c1_dropped_column_drops_last`);
//! a regression there will not reproduce in this harness.

#[path = "backbuild_property/data.rs"]
mod data;
#[path = "backbuild_conformance/harness.rs"]
mod harness;
#[path = "backbuild_property/recipe.rs"]
mod recipe;
#[path = "backbuild_property/render.rs"]
mod render;

use duckdb::Connection;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::backbuild::{
    assemble, definition_diff, derive_backbuild_options, BackbuildInputs, ComparableDiff,
    ConjunctDiff, DefinitionDiff, SelectListDiff, Selection, SetOpDiff, SkeletonCause,
    SkeletonDiff, SourceRef, Technique,
};

/// Default deterministic case count — small enough to keep the gate on par
/// with `maintenance_conformance`'s per-target budget, large enough (with
/// `recipe.rs`'s weighted shape mix) to reach every [`Technique`] variant at
/// least once (`admission_rate_stays_above_floor`'s coverage guard).
const DEFAULT_CASES: usize = 24;

fn case_count() -> usize {
    std::env::var("SMELT_BACKBUILD_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Bound on the per-case composed-selection product (research §6
/// "Conformance harness"; plan Phase 7 "cap the product; log/count what the
/// cap drops — no silent truncation").
const PRODUCT_CAP: usize = 8;

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

/// Every atom-option-index combination up to `cap`, plus the true (uncapped)
/// product size — an odometer over `sizes`, stopping once `cap` combinations
/// have been emitted. Never silent: callers compare the returned total
/// against `cap` themselves.
fn bounded_product(sizes: &[usize], cap: usize) -> (Vec<Vec<usize>>, usize) {
    if sizes.is_empty() {
        return (vec![Vec::new()], 1);
    }
    let total = sizes.iter().product::<usize>().max(1);
    let mut combos = Vec::new();
    let mut idx = vec![0usize; sizes.len()];
    loop {
        combos.push(idx.clone());
        if combos.len() >= cap || combos.len() >= total {
            break;
        }
        let mut pos = sizes.len();
        loop {
            if pos == 0 {
                break;
            }
            pos -= 1;
            idx[pos] += 1;
            if idx[pos] < sizes[pos] {
                break;
            }
            idx[pos] = 0;
            if pos == 0 {
                break;
            }
        }
    }
    (combos, total)
}

/// Tracks whether every [`Technique`] variant has been exercised at least
/// once — plain `bool` fields (not a `HashSet`: `Technique` derives neither
/// `Hash` nor `Ord`) keyed by an exhaustive `match` in [`Self::record`], so
/// a new `Technique` variant fails this file to compile until it is wired
/// in here too.
#[derive(Default)]
struct TechniqueTally {
    full_refresh: bool,
    self_derived_add: bool,
    rename: bool,
    self_derived_rewrite: bool,
    upstream_pullthrough: bool,
    join_update_from: bool,
    join_scalar_subquery: bool,
    predicate_tighten_delete: bool,
    horizon_extension_insert: bool,
    filter_loosen_insert: bool,
    union_branch_insert: bool,
    discriminated_branch_delete: bool,
    aggregate_backfill: bool,
    window_backfill: bool,
    column_drop: bool,
}

impl TechniqueTally {
    fn record(&mut self, t: Technique) {
        match t {
            Technique::FullRefresh => self.full_refresh = true,
            Technique::SelfDerivedColumnAdd => self.self_derived_add = true,
            Technique::Rename => self.rename = true,
            Technique::SelfDerivedColumnRewrite => self.self_derived_rewrite = true,
            Technique::UpstreamPullthrough => self.upstream_pullthrough = true,
            Technique::JoinEnrichmentUpdateFrom => self.join_update_from = true,
            Technique::JoinEnrichmentScalarSubquery => self.join_scalar_subquery = true,
            Technique::PredicateTightenDelete => self.predicate_tighten_delete = true,
            Technique::HorizonExtensionInsert => self.horizon_extension_insert = true,
            Technique::FilterLoosenInsert => self.filter_loosen_insert = true,
            Technique::UnionBranchInsert => self.union_branch_insert = true,
            Technique::DiscriminatedBranchDelete => self.discriminated_branch_delete = true,
            Technique::AggregateColumnBackfill => self.aggregate_backfill = true,
            Technique::WindowColumnBackfill => self.window_backfill = true,
            Technique::ColumnDrop => self.column_drop = true,
        }
    }

    fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        macro_rules! check {
            ($field:ident, $name:literal) => {
                if !self.$field {
                    missing.push($name);
                }
            };
        }
        check!(full_refresh, "FullRefresh");
        check!(self_derived_add, "SelfDerivedColumnAdd");
        check!(rename, "Rename");
        check!(self_derived_rewrite, "SelfDerivedColumnRewrite");
        check!(upstream_pullthrough, "UpstreamPullthrough");
        check!(join_update_from, "JoinEnrichmentUpdateFrom");
        check!(join_scalar_subquery, "JoinEnrichmentScalarSubquery");
        check!(predicate_tighten_delete, "PredicateTightenDelete");
        check!(horizon_extension_insert, "HorizonExtensionInsert");
        check!(filter_loosen_insert, "FilterLoosenInsert");
        check!(union_branch_insert, "UnionBranchInsert");
        check!(discriminated_branch_delete, "DiscriminatedBranchDelete");
        check!(aggregate_backfill, "AggregateColumnBackfill");
        check!(window_backfill, "WindowColumnBackfill");
        check!(column_drop, "ColumnDrop");
        missing
    }
}

/// `generated_options_match_full_rebuild_oracle` (plan Phase 7 TDD list):
/// the main gate. `N` seeded cases, each drawing a `BeforeRecipe` + 1-3
/// `EditRecipe`s over the fixed source pool; stage sources with generated
/// data; parse both renders; `definition_diff` → `derive_backbuild_options`;
/// verify the `FullRefresh` baseline once, then every admissible composed
/// selection in the bounded per-atom option product via `verify_script`
/// against a fresh staged copy; when any atom's option set is empty, assert
/// `assemble` returns no targeted script and every refusal carries a
/// non-empty atom + reason.
#[test]
fn generated_options_match_full_rebuild_oracle() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let case_strat = recipe::arb_case();
    let data_strat = data::arb_order_rows();

    let mut capped_cases = 0usize;
    let mut verified_scripts = 0usize;

    for i in 0..n {
        // Reserved slots (see `recipe::GUARANTEED_EDITS`) draw a
        // deterministic single-edit case that makes each rare technique's
        // coverage true by construction; later slots stay fully
        // `arb_case`-generative. Draws no bits from `runner` in the
        // reserved-slot arm, so unreserved-slot cases still get the same
        // draws they always did.
        let (before, edits) = match recipe::guaranteed_case(i) {
            Some(case) => case,
            None => case_strat.new_tree(&mut runner).unwrap().current(),
        };
        let rows = data_strat.new_tree(&mut runner).unwrap().current();

        let rendered = render::render(&before, &edits);
        let conn = Connection::open_in_memory().expect("duckdb");
        harness::stage_inputs(&conn, &data::full_staging_sql(&rows));

        let diff = definition_diff(&parse(&rendered.before_sql), &parse(&rendered.after_sql));
        assert!(
            !diff.is_noop(),
            "case {i}: recipe {before:?} + edits {edits:?} rendered a no-op diff \
             (before={:?}, after={:?})",
            rendered.before_sql,
            rendered.after_sql
        );

        let options = derive_backbuild_options(&diff, &rendered.inputs);

        harness::verify_option(
            &conn,
            "t",
            &rendered.before_sql,
            &rendered.after_sql,
            &options.full_refresh,
        );

        let sizes: Vec<usize> = options.atoms.iter().map(|a| a.options.len()).collect();
        if sizes.contains(&0) {
            let selection = Selection::Targeted {
                atom_choices: vec![0; options.atoms.len()],
            };
            let script = assemble(&options, &selection);
            assert!(
                script.is_empty(),
                "case {i}: an atom with no admissible option must yield an empty targeted \
                 script (partial application is never offered), got {script:?}"
            );
            for atom in &options.atoms {
                if atom.options.is_empty() {
                    assert!(
                        !atom.inadmissible.is_empty(),
                        "case {i}: an atom with no options must carry >=1 named refusal: \
                         {atom:?}"
                    );
                    for r in &atom.inadmissible {
                        assert!(
                            !r.atom.is_empty() && !r.reason.is_empty(),
                            "case {i}: refusal must name a non-empty atom and reason: {r:?}"
                        );
                    }
                }
            }
        } else if !options.atoms.is_empty() {
            let (combos, total) = bounded_product(&sizes, PRODUCT_CAP);
            if total > PRODUCT_CAP {
                capped_cases += 1;
                eprintln!(
                    "case {i}: bounded product capped ({total} combos > cap {PRODUCT_CAP}) — \
                     verifying the first {PRODUCT_CAP}"
                );
            }
            for combo in &combos {
                let selection = Selection::Targeted {
                    atom_choices: combo.clone(),
                };
                let script = assemble(&options, &selection);
                assert!(
                    !script.is_empty(),
                    "case {i}: combo {combo:?} over atoms {:?} composed to an empty script",
                    options.atoms
                );

                // Rerun-safety leg (mutation-audit finding 1,
                // `docs/handoffs/2026-08-03-backbuild-property-test-review.md`):
                // when every atom's chosen option in this combo claims
                // `rerun_safe: true`, apply the composed script *twice*
                // against a fresh before-table before the oracle check —
                // makes `rerun_safe` a tested claim rather than a
                // self-reported, never-exercised field. Otherwise, the
                // single-application leg the harness always ran.
                let combo_rerun_safe = combo.iter().enumerate().all(|(atom_idx, &opt_idx)| {
                    options.atoms[atom_idx].options[opt_idx].rerun_safe
                });
                if combo_rerun_safe {
                    harness::build_before(&conn, "t", &rendered.before_sql);
                    for stmt in &script {
                        conn.execute_batch(stmt).unwrap_or_else(|e| {
                            panic!("case {i}: apply backbuild script statement (1st pass) `{stmt}`: {e}")
                        });
                    }
                    for stmt in &script {
                        conn.execute_batch(stmt).unwrap_or_else(|e| {
                            panic!("case {i}: apply backbuild script statement (2nd rerun-safety pass) `{stmt}`: {e}")
                        });
                    }
                    harness::assert_matches_full_rebuild(&conn, "t", &rendered.after_sql);
                } else {
                    harness::verify_script(
                        &conn,
                        "t",
                        &rendered.before_sql,
                        &rendered.after_sql,
                        &script,
                    );
                }
                verified_scripts += 1;
            }
        }
    }

    assert!(
        verified_scripts > 0,
        "N={n} deterministic sample verified zero composed scripts — generator/derivation \
         regression"
    );
    eprintln!(
        "generated_options_match_full_rebuild_oracle: N={n}, verified_scripts=\
         {verified_scripts}, capped_cases={capped_cases}"
    );
}

/// `admission_rate_stays_above_floor` (plan Phase 7 TDD list): generator
/// health — over the seeded run, the fraction of cases yielding at least one
/// targeted option stays above a floor, and a per-technique coverage tally
/// shows every [`Technique`] variant exercised at least once at the default
/// case count. Classification-only (no DuckDB) — fast enough to run its own
/// independent sample.
#[test]
fn admission_rate_stays_above_floor() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let case_strat = recipe::arb_case();

    let mut tally = TechniqueTally::default();
    // Generative-only counters (see the floor rationale below): the first
    // `recipe::guaranteed_slot_count()` slots are deterministic
    // always-admitting cases (`recipe::guaranteed_case`), not proptest draws.
    // Folding them into the admission rate makes the floor decorative at the
    // default case count (most of the 24 slots trivially admit regardless of
    // generator health) — count them separately from the generative slots
    // that `arb_case` actually draws.
    let guaranteed_slots = recipe::guaranteed_slot_count();
    let mut generative_total = 0usize;
    let mut generative_admitted = 0usize;

    for i in 0..n {
        // See the matching comment in `generated_options_match_full_rebuild_oracle`.
        let (before, edits) = match recipe::guaranteed_case(i) {
            Some(case) => case,
            None => case_strat.new_tree(&mut runner).unwrap().current(),
        };
        let rendered = render::render(&before, &edits);

        let diff = definition_diff(&parse(&rendered.before_sql), &parse(&rendered.after_sql));
        assert!(
            !diff.is_noop(),
            "case {i}: recipe {before:?} + edits {edits:?} rendered a no-op diff"
        );

        let options = derive_backbuild_options(&diff, &rendered.inputs);
        tally.record(options.full_refresh.technique);

        let case_admitted = options.atoms.iter().any(|a| !a.options.is_empty());
        if case_admitted {
            for atom in &options.atoms {
                for opt in &atom.options {
                    tally.record(opt.technique);
                }
            }
        }
        if i >= guaranteed_slots {
            generative_total += 1;
            if case_admitted {
                generative_admitted += 1;
            }
        }
    }

    // The floor is measured over the *generative* slots only (`i >=
    // guaranteed_slots`), never the whole sample: `recipe::guaranteed_case`
    // reserves the first `guaranteed_slots` slots (the single-edit
    // `GUARANTEED_EDITS` list plus the combined both-tighten-variants slot)
    // for deterministic always-admitting cases, so a floor computed
    // over all `n` slots stays comfortably passed even if `arb_case`'s
    // generative arm admits nothing at all — decorative, not a real
    // generator-health gate. Measured directly against this harness:
    // generative-arm admission rate is 1.00 at N=24 and
    // 277/287 = 0.965 at N=300 — `arb_case`'s weighted shape/edit mix almost
    // always lands on a `BeforeRecipe`/`EditRecipe` pair with at least one
    // admissible option, so the floor has plenty of room. 0.80 is chosen
    // with ~15-20 points of headroom below both observed rates: high enough
    // to catch a real regression (e.g. a technique's precondition
    // accidentally narrowed to never compose, or a compatible-edit list
    // drifting out of sync with the classifier), comfortably below the
    // ~0.97-1.00 the generator produces today so normal shrinking/seed
    // churn doesn't flake it.
    if generative_total > 0 {
        let generative_rate = generative_admitted as f64 / generative_total as f64;
        assert!(
            generative_rate >= 0.80,
            "generative-arm admission rate {generative_rate:.2} over {generative_total} \
             generative cases (N={n} total, {guaranteed_slots} guaranteed) fell below the \
             80% generator-health floor ({generative_admitted}/{generative_total} admitted)"
        );
    }

    let missing = tally.missing();
    assert!(
        missing.is_empty(),
        "N={n} deterministic sample never exercised these Technique variants: {missing:?} — \
         tune recipe.rs's generators (a technique that never admits fails loudly here, not \
         silently green)"
    );
}

/// `adversarial_edits_always_refuse_or_verify` (plan Phase 7 TDD list): the
/// adversarial edit axis (grain change, non-`LEFT` join add, volatile
/// function, opaque predicate rewrite) never crashes and never yields an
/// unverified script — each case either refuses by name or its script
/// passes the oracle.
#[test]
fn adversarial_edits_always_refuse_or_verify() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let case_strat = recipe::arb_adversarial_case();
    let data_strat = data::arb_order_rows();

    for i in 0..n {
        let (before, edit) = case_strat.new_tree(&mut runner).unwrap().current();
        let rows = data_strat.new_tree(&mut runner).unwrap().current();

        let rendered = render::render_adversarial(&before, edit);
        let conn = Connection::open_in_memory().expect("duckdb");
        harness::stage_inputs(&conn, &data::full_staging_sql(&rows));

        let diff = definition_diff(&parse(&rendered.before_sql), &parse(&rendered.after_sql));
        assert!(
            !diff.is_noop(),
            "case {i}: adversarial recipe {before:?} + edit {edit:?} rendered a no-op diff"
        );

        let options = derive_backbuild_options(&diff, &rendered.inputs);

        // `AdversarialVolatileFn`'s after-definition contains `RANDOM()` —
        // by construction not oracle-verifiable even via `FullRefresh`
        // (re-evaluating `after_sql` for the comparison view draws a
        // *different* random value than the table's own materialization).
        // Research §2 "Determinism caveat": a volatile after-expression is
        // surfaced (refused), never asserted equal to a second, necessarily
        // different draw. This axis's whole point for that one variant is
        // "the added column refuses" — assert exactly that, and skip both
        // the `FullRefresh` and composed-script oracle checks.
        if edit == recipe::EditRecipe::AdversarialVolatileFn {
            assert!(
                options.atoms.iter().any(|a| !a.inadmissible.is_empty()),
                "case {i}: AdversarialVolatileFn must refuse (a volatile added expression is \
                 never admissible), got {:?}",
                options.atoms
            );
            continue;
        }

        harness::verify_option(
            &conn,
            "t",
            &rendered.before_sql,
            &rendered.after_sql,
            &options.full_refresh,
        );

        let selection = Selection::Targeted {
            atom_choices: vec![0; options.atoms.len()],
        };
        let script = assemble(&options, &selection);

        if script.is_empty() {
            // Refused by name — the adversarial edit's whole point.
            assert!(
                options.atoms.iter().any(|a| !a.inadmissible.is_empty()),
                "case {i}: adversarial edit {edit:?} refused with an empty script but no atom \
                 carries a named reason: {:?}",
                options.atoms
            );
        } else {
            // The classifier admitted something anyway — still must be
            // oracle-correct, never a silently wrong plan.
            harness::verify_script(
                &conn,
                "t",
                &rendered.before_sql,
                &rendered.after_sql,
                &script,
            );
        }
    }
}

/// `stale_upstream_documents_precondition_generatively` (plan Phase 7 TDD
/// list): one deterministic case (not `proptest`-drawn — the precondition's
/// edge is a single fixed scenario, not a generator target) mutating a
/// staged source between `build_before` and script application for an
/// upstream-reading option, asserting the *documented divergence* (research
/// §2 "Why the precondition is load-bearing"): an upstream-read script bakes
/// in current upstream state, so a precondition violation makes the result
/// diverge from a full rebuild against the current inputs on the untouched
/// sibling column — the contract's edge, tested, not just stated.
#[test]
fn stale_upstream_documents_precondition_generatively() {
    let conn = Connection::open_in_memory().expect("duckdb");
    harness::stage_inputs(
        &conn,
        "CREATE TABLE src_orders (order_id INTEGER NOT NULL, customer_id INTEGER NOT NULL, \
         amount INTEGER NOT NULL, qty INTEGER NOT NULL, status VARCHAR, ts DATE NOT NULL);
         INSERT INTO src_orders VALUES
           (1, 1, 10, 2, 'open', DATE '2025-06-01'),
           (2, 2, 20, 3, 'closed', DATE '2025-06-02');",
    );

    let before_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id FROM \
                       src_orders o";
    let after_sql = "SELECT o.order_id AS order_id, o.customer_id AS customer_id, o.amount AS \
                      amount FROM src_orders o";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut sources = std::collections::BTreeMap::new();
    sources.insert(
        "o".to_string(),
        SourceRef {
            physical_name: "src_orders".to_string(),
            unique_key: Some(vec!["order_id".to_string()]),
            not_null_columns: ["order_id"].into_iter().map(str::to_string).collect(),
        },
    );
    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.to_string(),
        row_identity: Some(vec!["order_id".to_string()]),
        not_null_columns: ["order_id", "customer_id"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        added_column_types: [("amount".to_string(), "INTEGER".to_string())]
            .into_iter()
            .collect(),
        sources,
    };
    let options = derive_backbuild_options(&diff, &inputs);
    let option = &options.atoms[0].options[0];
    assert!(
        option.reads_upstream,
        "expected an upstream-read option to exercise the precondition edge: {option:?}"
    );

    // `T_old` is built while the precondition (`T_old == eval(before, I)`)
    // still holds.
    harness::build_before(&conn, "t", before_sql);

    // The precondition is now violated: `src_orders` changes after `t` was
    // built.
    conn.execute_batch(
        "UPDATE src_orders SET customer_id = 999, amount = 12345 WHERE order_id = 1",
    )
    .expect("mutate upstream after build_before");

    for stmt in &option.statements {
        conn.execute_batch(stmt)
            .unwrap_or_else(|e| panic!("apply upstream-read backfill `{stmt}`: {e}"));
    }

    // The backfilled column is upstream-read — it bakes in the *current*
    // (post-mutation) upstream state.
    let amount_now =
        harness::text_column(&conn, "SELECT amount::VARCHAR FROM t WHERE order_id = 1");
    assert_eq!(amount_now, vec!["12345".to_string()]);

    // The untouched sibling column still reflects the *stale* build.
    let customer_now = harness::text_column(
        &conn,
        "SELECT customer_id::VARCHAR FROM t WHERE order_id = 1",
    );
    assert_eq!(customer_now, vec!["1".to_string()]);

    // Demonstrate the actual divergence: a full rebuild against the
    // *current* inputs disagrees with `t` on the sibling column — exactly
    // the edge research §2 documents ("diverges exactly when the contract
    // says it may").
    let rebuilt_customer = harness::text_column(
        &conn,
        &format!("SELECT customer_id::VARCHAR FROM ({after_sql}) AS rebuilt WHERE order_id = 1"),
    );
    assert_eq!(rebuilt_customer, vec!["999".to_string()]);
    assert_ne!(
        customer_now, rebuilt_customer,
        "t must diverge from a full rebuild against current inputs — the documented edge of \
         the §2 precondition, not a correctness bug in the script"
    );
}

/// Substrate-unification Phase 5 (`docs/plans/20260808-substrate-unification.md`):
/// the left-join enrichment admission (`classify.rs`'s
/// `admit_added_left_join`, now delegating its at-most-one-match proof to
/// `analysis::join_shape::fan_out` + `analysis::functional_dependency::
/// functional_dependency_verdict`) and that same shared FD verdict function,
/// called independently over the equivalent `Cardinality`, agree on one
/// join shape — for both an exact key match (admitted) and a key mismatch
/// (refused). One verdict, two consumers, proven to coincide rather than
/// asserted by inspection.
#[test]
fn fd_verdict_shared() {
    use smelt_logical::analysis::functional_dependency::functional_dependency_verdict;
    use smelt_logical::analysis::join_shape::{fan_out, JoinContext};

    let before_sql = "SELECT o.order_id AS order_id FROM orders o";
    let after_sql = "SELECT o.order_id AS order_id, d.name AS dim_name FROM orders o LEFT JOIN \
                      dims d ON o.order_id = d.order_id";
    let after_file = parse(after_sql);
    let after_stmt = after_file.select_stmt().expect("select");
    let join = after_stmt
        .from_clause()
        .expect("from clause")
        .joins()
        .next()
        .expect("one join");

    for (unique_key, expect_admitted) in [(vec!["order_id"], true), (vec!["customer_id"], false)] {
        let diff = definition_diff(&parse(before_sql), &after_file);
        assert!(!diff.is_noop());

        let mut sources = std::collections::BTreeMap::new();
        sources.insert(
            "d".to_string(),
            SourceRef {
                physical_name: "dims".to_string(),
                unique_key: Some(unique_key.iter().map(|s| s.to_string()).collect()),
                not_null_columns: unique_key.iter().map(|s| s.to_string()).collect(),
            },
        );
        let backbuild_inputs = BackbuildInputs {
            table: "t".to_string(),
            after_sql: after_sql.to_string(),
            row_identity: None,
            not_null_columns: std::collections::BTreeSet::new(),
            added_column_types: [("dim_name".to_string(), "TEXT".to_string())]
                .into_iter()
                .collect(),
            sources,
        };
        let options = derive_backbuild_options(&diff, &backbuild_inputs);
        let atom = &options.atoms[0];
        let admission_admits = atom.options.iter().any(|o| {
            matches!(
                o.technique,
                Technique::JoinEnrichmentUpdateFrom | Technique::JoinEnrichmentScalarSubquery
            )
        });
        assert_eq!(
            admission_admits, expect_admitted,
            "unique_key {unique_key:?}: admission verdict {atom:?}"
        );

        // The independent verdict, computed the same way
        // `admit_added_left_join` now does: declared `unique_key` feeds a
        // `JoinContext`, `fan_out` proves the join's cardinality against it,
        // and `functional_dependency_verdict` turns that into a
        // constant/not-constant verdict.
        let mut ctx = JoinContext::new();
        for k in &unique_key {
            ctx = ctx.with_unique_key("d", k);
        }
        let cardinality = fan_out(&join, &ctx);
        let fd_verdict = functional_dependency_verdict(Some(cardinality), false);
        assert_eq!(
            fd_verdict.is_constant(),
            expect_admitted,
            "unique_key {unique_key:?}: FD verdict {fd_verdict:?} disagrees with the admission \
             outcome"
        );
    }
}

/// Phase 5 reviewer finding 1: a *composite* declared `unique_key` must be
/// registered with `join_shape::JoinContext` as one composite key-set
/// (`with_composite_unique_key`), never one `with_unique_key` call per
/// column — the latter registers each column as an *independently*
/// sufficient single-column key, so a join keyed on only *part* of a
/// 2-column composite key would be falsely proven `OneToOne` (at-most-one-
/// match unsound). `dims` declares a composite `unique_key`
/// `["region_id", "dept_id"]`; equating only `region_id` in the ON clause
/// must refuse, equating both must admit.
#[test]
fn fd_verdict_shared_composite_key() {
    let before_sql = "SELECT o.region_id AS region_id, o.dept_id AS dept_id FROM orders o";

    let partial_after_sql = "SELECT o.region_id AS region_id, o.dept_id AS dept_id, d.name AS \
                              dim_name FROM orders o LEFT JOIN dims d ON o.region_id = \
                              d.region_id";
    let full_after_sql = "SELECT o.region_id AS region_id, o.dept_id AS dept_id, d.name AS \
                           dim_name FROM orders o LEFT JOIN dims d ON o.region_id = d.region_id \
                           AND o.dept_id = d.dept_id";

    for (after_sql, expect_admitted) in [(partial_after_sql, false), (full_after_sql, true)] {
        let diff = definition_diff(&parse(before_sql), &parse(after_sql));
        assert!(!diff.is_noop());

        let mut sources = std::collections::BTreeMap::new();
        sources.insert(
            "d".to_string(),
            SourceRef {
                physical_name: "dims".to_string(),
                unique_key: Some(vec!["region_id".to_string(), "dept_id".to_string()]),
                not_null_columns: ["region_id", "dept_id"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            },
        );
        let backbuild_inputs = BackbuildInputs {
            table: "t".to_string(),
            after_sql: after_sql.to_string(),
            row_identity: None,
            not_null_columns: std::collections::BTreeSet::new(),
            added_column_types: [("dim_name".to_string(), "TEXT".to_string())]
                .into_iter()
                .collect(),
            sources,
        };
        let options = derive_backbuild_options(&diff, &backbuild_inputs);
        let atom = &options.atoms[0];
        let admission_admits = atom.options.iter().any(|o| {
            matches!(
                o.technique,
                Technique::JoinEnrichmentUpdateFrom | Technique::JoinEnrichmentScalarSubquery
            )
        });
        assert_eq!(
            admission_admits, expect_admitted,
            "ON clause equating only part of a composite unique_key must never be proven \
             at-most-one-match: after_sql={after_sql:?}, atom={atom:?}"
        );
    }
}

/// Substrate-unification Phase 5 (`docs/plans/20260808-substrate-unification.md`):
/// the G1-vs-G2 catalogue label is driven by `diff.rs`'s own structured
/// `SkeletonCause`, not by `classify.rs` lowercased-`.contains`-scanning the
/// free-English `reason` string. Hand-construct a `DefinitionDiff` whose
/// skeleton change is structurally a grain change (`cause:
/// SkeletonCause::GrainChanged`) but whose `reason` prose happens to mention
/// "join" (a plausible real message: a `GROUP BY` over a column literally
/// named `join_key`) — the pre-Phase-5 string scan would misclassify this as
/// G2 (join-multiplicity change) because it checked `.contains("join")`
/// after failing to find the *fixed* substring `"group by"` in a
/// hand-written reason that omits it; the structured classifier is immune
/// to the reason text's own wording by construction.
#[test]
fn skeleton_reason_structured() {
    let diff = DefinitionDiff::Comparable(Box::new(ComparableDiff {
        select_list: SelectListDiff::Diffed {
            added: Vec::new(),
            dropped: Vec::new(),
            changed: Vec::new(),
            unchanged: Vec::new(),
            before_order: Vec::new(),
            after_order: Vec::new(),
        },
        where_clause: ConjunctDiff::Diffed {
            added: Vec::new(),
            removed: Vec::new(),
            unchanged: Vec::new(),
        },
        skeleton: SkeletonDiff::Changed {
            reason: "the grouping over join_key changed".to_string(),
            cause: SkeletonCause::GrainChanged,
        },
        set_ops: SetOpDiff::NotApplicable,
    }));

    let options = derive_backbuild_options(&diff, &inputs("t", "SELECT 1"));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert_eq!(atom.inadmissible.len(), 1, "{:?}", atom.inadmissible);
    assert!(
        atom.inadmissible[0].reason.starts_with("G1"),
        "expected the structural GrainChanged cause to win a G1 label regardless of 'join' \
         appearing in the reason prose, got: {:?}",
        atom.inadmissible[0].reason
    );
}

fn inputs(table: &str, after_sql: &str) -> BackbuildInputs {
    BackbuildInputs {
        table: table.to_string(),
        after_sql: after_sql.to_string(),
        row_identity: None,
        not_null_columns: std::collections::BTreeSet::new(),
        added_column_types: std::collections::BTreeMap::new(),
        sources: std::collections::BTreeMap::new(),
    }
}

/// Substrate-unification Phase 3 (`docs/plans/20260808-substrate-unification.md`):
/// the F2 branch-removal discriminator proof must recognise a *typed*
/// literal (`DATE '…'`) branch tag, not just a bare `NUMBER`/`STRING`
/// token — the walk's `is_constant_literal` already does (walk_hardening.rs
/// `constant_literal_rejects_function_call`); backbuild's own
/// `bare_literal` did not. This is the phase's single named
/// behaviour change, in the accepting direction: today this refuses (no
/// `DiscriminatedBranchDelete` option), fixed it admits one whose
/// statement equality-predicate literal is the removed branch's own typed
/// tag.
#[test]
fn typed_literal_branch_discriminator() {
    let before_sql = "SELECT id, DATE '2026-01-01' AS src FROM events_a UNION ALL SELECT id, \
                       DATE '2026-02-01' AS src FROM events_b";
    let after_sql = "SELECT id, DATE '2026-01-01' AS src FROM events_a";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::RemovedSetOpBranch { index: 0 }
        ),
        "expected a RemovedSetOpBranch atom, got {:?}",
        atom.change
    );
    assert_eq!(
        atom.options.len(),
        1,
        "expected the typed-literal branch tag to be recognised as a discriminator: {atom:?}"
    );
    let option = &atom.options[0];
    assert_eq!(option.technique, Technique::DiscriminatedBranchDelete);
    assert_eq!(option.statements.len(), 1, "{:?}", option.statements);
    assert_eq!(
        option.statements[0],
        "DELETE FROM t WHERE src = DATE '2026-02-01'"
    );
}

/// Guard against the unification widening too far: a branch tag that is a
/// function call (`CURRENT_DATE`, not a constant literal) must never be
/// recognised as a discriminator by either layer — this should already pass
/// today and stays passing after the delegation.
#[test]
fn function_call_never_discriminates() {
    let before_sql = "SELECT id, CURRENT_DATE AS src FROM events_a UNION ALL SELECT id, \
                       CURRENT_DATE AS src FROM events_b";
    let after_sql = "SELECT id, CURRENT_DATE AS src FROM events_a";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let options = derive_backbuild_options(&diff, &inputs("t", after_sql));
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "a CURRENT_DATE branch tag is not a constant literal and must never be admitted as a \
         discriminator: {atom:?}"
    );
}

/// Substrate-unification Phase 5 (`docs/plans/20260808-substrate-unification.md`):
/// B1 must admit an added column that is derivable from a stored column
/// *through a CTE rename* — not just from a stored column referenced by the
/// exact same qualifier/raw-name text the outer SELECT list itself uses.
/// The CTE renames `order_id` to `id`; the outer SELECT then re-renames `id`
/// to `order_identifier`, so the representative's own (bare) raw name `id`
/// and its stored output name `order_identifier` differ. A bare dependency
/// on `id` (inside the added `doubled` column) is only provably the same
/// stored data once both the representative and the dependency are chased
/// to their shared base-relation leaf (`orders.order_id`) — the flat
/// `(qualifier, raw_name)` triple match `resolve_representative` used before
/// this phase cannot see that, because it requires a *bare* dependency's raw
/// name to equal the representative's own output name, which only ever
/// holds when the outer SELECT does not itself re-rename. The WITH prefix is
/// byte-identical on both sides (`diff.rs` refuses to chase a *changed* CTE
/// section outright) — only the outer SELECT list changes.
#[test]
fn provenance_chases_renames() {
    let with_prefix = "WITH cte AS (SELECT order_id AS id, amount FROM orders)";
    let before_sql = format!("{with_prefix} SELECT id AS order_identifier, amount FROM cte");
    let after_sql =
        format!("{with_prefix} SELECT id AS order_identifier, amount, id * 2 AS doubled FROM cte");

    let diff = definition_diff(&parse(&before_sql), &parse(&after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", &after_sql);
    backbuild_inputs
        .added_column_types
        .insert("doubled".to_string(), "INTEGER".to_string());

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        matches!(
            &atom.change,
            smelt_logical::backbuild::AtomicChange::AddedColumn { name } if name == "doubled"
        ),
        "expected an AddedColumn atom for 'doubled', got {:?}",
        atom.change
    );
    let b1 = atom
        .options
        .iter()
        .find(|o| o.technique == Technique::SelfDerivedColumnAdd)
        .unwrap_or_else(|| {
            panic!(
                "expected B1 to admit 'doubled' via the CTE-rename-chased representative for \
                 'id': {atom:?}"
            )
        });
    assert_eq!(
        b1.statements,
        vec![
            "ALTER TABLE t ADD COLUMN doubled INTEGER",
            "UPDATE t SET doubled = order_identifier * 2",
        ],
        "{:?}",
        b1.statements
    );
}

/// Phase 5 reviewer finding 2: the lineage fallback must refuse rather than
/// falsely unify two different join legs of a *self-join*. `orders` is
/// joined to itself as `o1`/`o2`; the stored representative `id1` comes
/// from `o1.id`, and the added column `id2` reads `o2.id` — a *different*
/// row's column, even though both chase to the identical
/// `LeafColumn{relation: "orders", column: "id"}` (same base table, same
/// column name). Matching them as "the same stored data" purely off that
/// shared leaf would be the exact C2 self-read hazard the flat
/// qualifier-match rule was written to prevent — B1 must refuse `id2`
/// (no representative resolves it), not admit a self-read `UPDATE` that
/// silently substitutes `o1`'s stored value for `o2`'s.
#[test]
fn self_join_leaves_are_not_unified() {
    let before_sql =
        "SELECT o1.id AS id1, o2.amount AS amt2 FROM orders o1 JOIN orders o2 ON o1.id = o2.id";
    let after_sql = "SELECT o1.id AS id1, o2.amount AS amt2, o2.id AS id2 FROM orders o1 JOIN \
                      orders o2 ON o1.id = o2.id";

    let diff = definition_diff(&parse(before_sql), &parse(after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", after_sql);
    backbuild_inputs
        .added_column_types
        .insert("id2".to_string(), "INTEGER".to_string());

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "a self-join must never let 'id2' (o2.id) be admitted via a representative sourced \
         from a *different* join leg (o1.id) just because they share a base-relation leaf: \
         {atom:?}"
    );
}

/// Phase 5 reviewer round 2, finding: the self-join guard must fire when
/// the self-join is *hidden inside a CTE body* the outer scope only sees as
/// one `Cte` alias — a scope-local "count Table aliases in the current
/// scope's own FROM tree" check never sees this, because the outer scope's
/// own `aliases` map has exactly one entry (`cte`), not two. `cte`'s own
/// body self-joins `orders` as `o1`/`o2`; the outer scope only ever
/// references `cte`'s already-projected columns. `added2` reads `cte`'s
/// `hidden_id2` (itself `o2.id`) while the representative for `id1` is
/// `cte`'s `id1` (`o1.id`) — the identical hazard as
/// `self_join_leaves_are_not_unified`, one CTE hop removed. Ambiguity must
/// be a property of `cte`'s own lineage entries (set when `cte`'s body is
/// walked) and survive being read back out through the outer scope's
/// `Cte` resolution, not something a scope-local recount could ever catch.
#[test]
fn self_join_ambiguity_survives_a_cte_hop() {
    let with_prefix = "WITH cte AS (SELECT o1.id AS id1, o2.amount AS amt2, o2.id AS hidden_id2 \
                        FROM orders o1 JOIN orders o2 ON o1.id = o2.parent_id)";
    let before_sql = format!("{with_prefix} SELECT id1, amt2 FROM cte");
    let after_sql = format!("{with_prefix} SELECT id1, amt2, hidden_id2 AS added2 FROM cte");

    let diff = definition_diff(&parse(&before_sql), &parse(&after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", &after_sql);
    backbuild_inputs
        .added_column_types
        .insert("added2".to_string(), "INTEGER".to_string());

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    assert!(
        atom.options.is_empty(),
        "a self-join hidden inside a CTE body must still refuse 'added2' (cte.hidden_id2, \
         itself o2.id) rather than admit it via a representative sourced from cte.id1 \
         (o1.id) — a self-read UPDATE that would substitute o1's value for o2's: {atom:?}"
    );
}

/// Phase 5 reviewer round 2, guard-of-the-guard: an *unrelated* CTE that
/// itself self-joins `orders` must never poison a *different*, clean CTE's
/// own lineage — ambiguity is scoped per lineage entry (built once, per
/// CTE body's own FROM tree), not global to the model. `dirty` self-joins
/// `orders` and is declared alongside `clean` (which references `orders`
/// only once) but the outer scope only ever reads from `clean` — `doubled`
/// must still be admitted via the ordinary CTE-rename-chased representative
/// path (`provenance_chases_renames`'s shape), proving `dirty`'s own
/// ambiguity never leaks into `clean`'s lineage entries.
#[test]
fn self_join_ambiguity_does_not_poison_an_unrelated_cte() {
    let with_prefix = "WITH clean AS (SELECT o.amount AS amt FROM orders o), dirty AS (SELECT \
                        x1.id AS xid1, x2.id AS xid2 FROM orders x1 JOIN orders x2 ON x1.id = \
                        x2.parent_id)";
    let before_sql = format!("{with_prefix} SELECT amt AS amount FROM clean");
    let after_sql = format!("{with_prefix} SELECT amt AS amount, amt * 2 AS doubled FROM clean");

    let diff = definition_diff(&parse(&before_sql), &parse(&after_sql));
    assert!(!diff.is_noop());

    let mut backbuild_inputs = inputs("t", &after_sql);
    backbuild_inputs
        .added_column_types
        .insert("doubled".to_string(), "INTEGER".to_string());

    let options = derive_backbuild_options(&diff, &backbuild_inputs);
    assert_eq!(options.atoms.len(), 1, "atoms: {:?}", options.atoms);
    let atom = &options.atoms[0];
    let b1 = atom
        .options
        .iter()
        .find(|o| o.technique == Technique::SelfDerivedColumnAdd)
        .unwrap_or_else(|| {
            panic!(
                "an unrelated self-joined CTE ('dirty') must not stop B1 admitting 'doubled' \
                 via 'clean's own (unambiguous) rename-chased representative: {atom:?}"
            )
        });
    assert_eq!(
        b1.statements,
        vec![
            "ALTER TABLE t ADD COLUMN doubled INTEGER",
            "UPDATE t SET doubled = amount * 2",
        ],
        "{:?}",
        b1.statements
    );
}
