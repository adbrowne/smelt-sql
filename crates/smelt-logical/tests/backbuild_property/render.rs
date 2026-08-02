//! Renders `recipe.rs`'s structural `BeforeRecipe`/`EditRecipe` values to
//! actual SQL text (before/after definitions) plus the matching
//! `BackbuildInputs` — the one place recipes become the strings
//! `definition_diff`/`derive_backbuild_options` consume. `proptest` shrinks
//! the recipe, never this module's SQL text (the testkit's structural-
//! shrinking convention, `crates/smelt-maintenance-testkit/src/recipe.rs`).

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use smelt_logical::backbuild::{BackbuildInputs, SourceRef};

use super::recipe::{BeforeRecipe, EditRecipe, Shape, WhereKind};

/// One planned output column: its final name, the expression that computes
/// it, whether it is provably `NOT NULL`, and whether it is a
/// freshly-added column (vs. a pre-existing bare/renamed/rewritten one).
struct ColumnPlan {
    name: String,
    expr: String,
    not_null: bool,
    added_type: Option<&'static str>,
}

fn where_conjunct_sql(kind: WhereKind, widened: bool) -> String {
    match kind {
        WhereKind::AmountPositive => "o.amount > 0".to_string(),
        WhereKind::QtyPositive => "o.qty > 0".to_string(),
        WhereKind::StatusOpen => "o.status = 'open'".to_string(),
        WhereKind::TsHorizon => {
            // A bare fixed-width `'YYYY-MM-DD'` string literal, not a
            // `DATE '...'`-typed literal — the E4 range-predicate
            // recognizer (`classify.rs`) only matches a plain `column <op>
            // literal` shape (research §4 E4's ISO-8601 fixed-width
            // requirement; see `e4_nonpadded_date_literal_refuses`,
            // `crates/smelt-logical/tests/backbuild_options.rs`).
            if widened {
                "o.ts >= '2024-01-01'".to_string()
            } else {
                "o.ts >= '2025-01-01'".to_string()
            }
        }
    }
}

fn render_where(
    conjuncts: &[WhereKind],
    widened_ts: bool,
    extra: &[String],
    removed: Option<WhereKind>,
    joiner: &str,
) -> Option<String> {
    let mut parts: Vec<String> = conjuncts
        .iter()
        .filter(|k| Some(**k) != removed)
        .map(|k| where_conjunct_sql(*k, widened_ts))
        .collect();
    parts.extend(extra.iter().cloned());
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(joiner))
    }
}

fn select_sql(items: &[ColumnPlan], from_and_rest: &str) -> String {
    let list = items
        .iter()
        .map(|c| format!("{} AS {}", c.expr, c.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {list} {from_and_rest}")
}

/// Structural edit accumulation for the order-family (`Plain`/`Joined`)
/// shapes — one field per edit's effect, applied once over the base
/// projection (`recipe.rs`'s disjointness reasoning: each edit's target is
/// either a brand-new column or a column no other compatible edit also
/// targets, so accumulation order never matters here).
#[derive(Default)]
struct OrderMutations {
    added: Vec<(String, String, &'static str)>, // name, expr, sql type
    renamed: Option<(&'static str, &'static str)>,
    rewritten: BTreeMap<&'static str, String>,
    dropped: Option<&'static str>,
    join_added: Option<bool>, // Some(general)
    extra_where: Vec<String>,
    removed_where: Option<WhereKind>,
    widened_ts: bool,
}

fn apply_order_edits(edits: &[EditRecipe], before: &BeforeRecipe) -> OrderMutations {
    let mut m = OrderMutations::default();
    for edit in edits {
        match edit {
            EditRecipe::AddSelfDerived => m.added.push((
                "derived_total".to_string(),
                "o.order_id * 10".to_string(),
                "INTEGER",
            )),
            EditRecipe::AddPullthrough => {
                if let Some(c) = super::recipe::all_cols()
                    .into_iter()
                    .find(|c| !before.columns.contains(c))
                {
                    m.added.push((
                        c.name().to_string(),
                        format!("o.{}", c.name()),
                        c.sql_type(),
                    ));
                }
            }
            EditRecipe::AddJoin { general } => {
                m.join_added = Some(*general);
                if *general {
                    m.added.push((
                        "score_or_default".to_string(),
                        "COALESCE(d.score, -1)".to_string(),
                        "INTEGER",
                    ));
                } else {
                    m.added
                        .push(("region".to_string(), "d.region".to_string(), "VARCHAR"));
                }
            }
            EditRecipe::AddWindow => m.added.push((
                "rn".to_string(),
                "ROW_NUMBER() OVER (PARTITION BY status ORDER BY order_id)".to_string(),
                "BIGINT",
            )),
            EditRecipe::Rename => m.renamed = Some(("customer_id", "account_id")),
            EditRecipe::RewriteSelfDerived => {
                // Reads `order_id` (never itself the target of any
                // compatible edit — see `AddSelfDerived`'s doc comment) —
                // never `amount` itself: a D1 rewrite whose new expression
                // depends on the very column being changed is unprovable
                // (`d1_new_expr_reading_own_old_value_refuses`,
                // `backbuild_options.rs`) by construction, not a
                // classifier bug.
                m.rewritten
                    .insert("amount", "o.order_id + 1000".to_string());
            }
            EditRecipe::RewriteFromUpstream => {
                m.rewritten.insert("qty", "o.customer_id".to_string());
            }
            EditRecipe::TightenFilter => m.extra_where.push("o.status IS NOT NULL".to_string()),
            EditRecipe::LoosenFilter => {
                if before.where_conjuncts.len() == 1 {
                    m.removed_where = Some(before.where_conjuncts[0]);
                }
            }
            EditRecipe::WidenHorizon => m.widened_ts = true,
            EditRecipe::DropColumn => m.dropped = Some("ts"),
            EditRecipe::AddAggregate | EditRecipe::AddBranch | EditRecipe::RemoveBranch => {
                // Not compatible with the order family — `compatible_edits`
                // never offers these here.
            }
            EditRecipe::AdversarialGrainChange
            | EditRecipe::AdversarialInnerJoin
            | EditRecipe::AdversarialVolatileFn
            | EditRecipe::AdversarialOpaqueRewrite => {
                // The adversarial axis is rendered by `render_adversarial`,
                // never mixed into a technique-axis edit list.
            }
        }
    }
    m
}

fn base_columns(before: &BeforeRecipe, m: Option<&OrderMutations>) -> Vec<ColumnPlan> {
    let mut items = vec![ColumnPlan {
        name: "order_id".to_string(),
        expr: "o.order_id".to_string(),
        not_null: true,
        added_type: None,
    }];
    for col in super::recipe::all_cols() {
        if !before.columns.contains(&col) {
            continue;
        }
        if let Some(m) = m {
            if m.dropped == Some(col.name()) {
                continue;
            }
        }
        let (name, not_null) = match m.and_then(|m| m.renamed) {
            Some((from, to)) if from == col.name() => (to.to_string(), !col.nullable()),
            _ => (col.name().to_string(), !col.nullable()),
        };
        let rewritten = m.and_then(|m| m.rewritten.get(col.name()));
        let (expr, not_null) = match rewritten {
            // Every rewrite target this module emits reads a NOT NULL
            // upstream column (`amount * 2` over NOT NULL `amount`;
            // `o.customer_id`, itself NOT NULL) — provably NOT NULL by
            // construction of the rewrite, not merely inherited from `col`.
            Some(expr) => (expr.clone(), true),
            None => (format!("o.{}", col.name()), not_null),
        };
        items.push(ColumnPlan {
            name,
            expr,
            not_null,
            added_type: None,
        });
    }
    if before.shape == Shape::Joined {
        items.push(ColumnPlan {
            name: "region".to_string(),
            expr: "d.region".to_string(),
            not_null: false,
            added_type: None,
        });
    }
    items
}

fn order_from_clause(
    before: &BeforeRecipe,
    join_added: Option<bool>,
    widened_ts: bool,
    extra_where: &[String],
    removed_where: Option<WhereKind>,
) -> String {
    let mut from = "FROM src_orders o".to_string();
    if before.shape == Shape::Joined || join_added.is_some() {
        from.push_str(" LEFT JOIN src_dim d ON o.customer_id = d.dim_id");
    }
    if let Some(w) = render_where(
        &before.where_conjuncts,
        widened_ts,
        extra_where,
        removed_where,
        " AND ",
    ) {
        from.push_str(" WHERE ");
        from.push_str(&w);
    }
    from
}

fn not_null_columns_of(items: &[ColumnPlan]) -> BTreeSet<String> {
    items
        .iter()
        .filter(|c| c.not_null && c.added_type.is_none())
        .map(|c| c.name.clone())
        .collect()
}

/// Builds a [`SourceRef`] straight from a `recipe::schema::TableSchema` —
/// the same schema `data.rs` renders `CREATE TABLE` DDL from, so
/// `physical_name`/`unique_key`/`not_null_columns` can't drift from the
/// staged table's actual columns.
fn source_ref_from_schema(schema: &super::recipe::schema::TableSchema) -> SourceRef {
    SourceRef {
        physical_name: schema.table_name.to_string(),
        unique_key: Some(vec![schema.unique_key.to_string()]),
        not_null_columns: schema
            .not_null_column_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
    }
}

fn base_sources() -> BTreeMap<String, SourceRef> {
    let mut sources = BTreeMap::new();
    sources.insert(
        "o".to_string(),
        source_ref_from_schema(&super::recipe::schema::orders_schema()),
    );
    sources
}

fn with_dim_source(mut sources: BTreeMap<String, SourceRef>) -> BTreeMap<String, SourceRef> {
    sources.insert(
        "d".to_string(),
        source_ref_from_schema(&super::recipe::schema::dim_schema()),
    );
    sources
}

pub struct Rendered {
    pub before_sql: String,
    pub after_sql: String,
    pub inputs: BackbuildInputs,
}

fn render_order_family(before: &BeforeRecipe, edits: &[EditRecipe]) -> Rendered {
    let before_items = base_columns(before, None);
    let before_from = order_from_clause(before, None, false, &[], None);
    let before_sql = select_sql(&before_items, &before_from);

    let m = apply_order_edits(edits, before);
    let mut after_items = base_columns(before, Some(&m));
    for (name, expr, ty) in &m.added {
        after_items.push(ColumnPlan {
            name: name.clone(),
            expr: expr.clone(),
            not_null: false,
            added_type: Some(ty),
        });
    }
    let after_from = order_from_clause(
        before,
        m.join_added,
        m.widened_ts,
        &m.extra_where,
        m.removed_where,
    );
    let after_sql = select_sql(&after_items, &after_from);

    let mut sources = base_sources();
    if before.shape == Shape::Joined || m.join_added.is_some() {
        sources = with_dim_source(sources);
    }
    let added_column_types: BTreeMap<String, String> = m
        .added
        .iter()
        .map(|(name, _, ty)| (name.clone(), ty.to_string()))
        .collect();

    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.clone(),
        row_identity: Some(vec!["order_id".to_string()]),
        not_null_columns: not_null_columns_of(&after_items),
        added_column_types,
        sources,
    };

    Rendered {
        before_sql,
        after_sql,
        inputs,
    }
}

fn render_grouped(before: &BeforeRecipe, edits: &[EditRecipe]) -> Rendered {
    let where_sql = render_where(&before.where_conjuncts, false, &[], None, " AND ")
        .map(|w| format!(" WHERE {w}"))
        .unwrap_or_default();
    let before_sql = format!(
        "SELECT o.customer_id AS customer_id, count(*) AS n FROM src_orders o{where_sql} GROUP \
         BY o.customer_id"
    );

    let add_aggregate = edits.contains(&EditRecipe::AddAggregate);
    let after_sql = if add_aggregate {
        format!(
            "SELECT o.customer_id AS customer_id, count(*) AS n, SUM(o.amount) AS total_amount \
             FROM src_orders o{where_sql} GROUP BY o.customer_id"
        )
    } else {
        before_sql.clone()
    };

    let mut added_column_types = BTreeMap::new();
    if add_aggregate {
        added_column_types.insert("total_amount".to_string(), "HUGEINT".to_string());
    }

    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.clone(),
        row_identity: None,
        not_null_columns: ["customer_id"].into_iter().map(str::to_string).collect(),
        added_column_types,
        sources: BTreeMap::new(),
    };

    Rendered {
        before_sql,
        after_sql,
        inputs,
    }
}

fn render_branch(before: &BeforeRecipe, edits: &[EditRecipe]) -> Rendered {
    let first_branch = "SELECT o.order_id AS order_id, o.status AS status, 'a' AS src FROM \
                         src_orders o";
    let second_branch = "SELECT z.zone_id AS order_id, z.zone AS status, 'b' AS src FROM \
                          src_dim2 z";

    let (before_sql, after_sql) = match before.shape {
        Shape::SingleBranch => {
            let before_sql = first_branch.to_string();
            let after_sql = if edits.contains(&EditRecipe::AddBranch) {
                format!("{first_branch} UNION ALL {second_branch}")
            } else {
                before_sql.clone()
            };
            (before_sql, after_sql)
        }
        Shape::DoubleBranch => {
            let before_sql = format!("{first_branch} UNION ALL {second_branch}");
            let after_sql = if edits.contains(&EditRecipe::RemoveBranch) {
                first_branch.to_string()
            } else {
                before_sql.clone()
            };
            (before_sql, after_sql)
        }
        _ => unreachable!("render_branch called for a non-branch shape"),
    };

    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.clone(),
        row_identity: Some(vec!["order_id".to_string()]),
        not_null_columns: ["order_id"].into_iter().map(str::to_string).collect(),
        added_column_types: BTreeMap::new(),
        sources: BTreeMap::new(),
    };

    Rendered {
        before_sql,
        after_sql,
        inputs,
    }
}

/// Render a technique-axis case (`recipe::arb_case`'s output).
pub fn render(before: &BeforeRecipe, edits: &[EditRecipe]) -> Rendered {
    match before.shape {
        Shape::Plain | Shape::Joined => render_order_family(before, edits),
        Shape::Grouped => render_grouped(before, edits),
        Shape::SingleBranch | Shape::DoubleBranch => render_branch(before, edits),
    }
}

/// Render exactly one adversarial edit (`recipe::arb_adversarial_case`'s
/// output) — always a `Plain`/`Joined`/`Grouped` `before` per
/// `arb_before_for_adversarial`.
pub fn render_adversarial(before: &BeforeRecipe, edit: EditRecipe) -> Rendered {
    let before_items = base_columns(before, None);
    let before_not_null = if before.shape == Shape::Grouped {
        ["customer_id"].into_iter().map(str::to_string).collect()
    } else {
        not_null_columns_of(&before_items)
    };
    let before_from = order_from_clause(before, None, false, &[], None);
    let before_sql = if before.shape == Shape::Grouped {
        let where_sql = render_where(&before.where_conjuncts, false, &[], None, " AND ")
            .map(|w| format!(" WHERE {w}"))
            .unwrap_or_default();
        format!(
            "SELECT o.customer_id AS customer_id, count(*) AS n FROM src_orders o{where_sql} \
             GROUP BY o.customer_id"
        )
    } else {
        select_sql(&before_items, &before_from)
    };

    let after_sql = match edit {
        EditRecipe::AdversarialGrainChange => {
            // Only offered for Plain/Joined (never Grouped, which is
            // already grouped) — replaces the whole projection with a
            // freshly grouped one: a grain change (G1), refused outright.
            let where_sql = render_where(&before.where_conjuncts, false, &[], None, " AND ")
                .map(|w| format!(" WHERE {w}"))
                .unwrap_or_default();
            format!(
                "SELECT o.customer_id AS customer_id, count(*) AS n FROM src_orders o{where_sql} \
                 GROUP BY o.customer_id"
            )
        }
        EditRecipe::AdversarialInnerJoin => {
            let mut from = "FROM src_orders o".to_string();
            if before.shape == Shape::Joined {
                from.push_str(" LEFT JOIN src_dim d ON o.customer_id = d.dim_id");
            }
            // A fresh alias (`d2`) avoids colliding with `Joined`'s own
            // pre-existing `d` alias — this edit's job is a join-
            // multiplicity change (G2: non-`LEFT`), not an alias clash.
            from.push_str(" JOIN src_dim d2 ON o.customer_id = d2.dim_id");
            if let Some(w) = render_where(&before.where_conjuncts, false, &[], None, " AND ") {
                from.push_str(" WHERE ");
                from.push_str(&w);
            }
            if before.shape == Shape::Grouped {
                format!(
                    "SELECT o.customer_id AS customer_id, count(*) AS n {from} GROUP BY \
                     o.customer_id"
                )
            } else {
                select_sql(&before_items, &from)
            }
        }
        EditRecipe::AdversarialVolatileFn => {
            if before.shape == Shape::Grouped {
                let where_sql = render_where(&before.where_conjuncts, false, &[], None, " AND ")
                    .map(|w| format!(" WHERE {w}"))
                    .unwrap_or_default();
                format!(
                    "SELECT o.customer_id AS customer_id, count(*) AS n, RANDOM() AS flag FROM \
                     src_orders o{where_sql} GROUP BY o.customer_id"
                )
            } else {
                let mut items = before_items;
                items.push(ColumnPlan {
                    name: "flag".to_string(),
                    expr: "RANDOM()".to_string(),
                    not_null: false,
                    added_type: Some("DOUBLE"),
                });
                select_sql(&items, &before_from)
            }
        }
        EditRecipe::AdversarialOpaqueRewrite => {
            let w = render_where(&before.where_conjuncts, false, &[], None, " OR ")
                .expect("adversarial opaque rewrite requires >=2 where conjuncts");
            let mut from = "FROM src_orders o".to_string();
            if before.shape == Shape::Joined {
                from.push_str(" LEFT JOIN src_dim d ON o.customer_id = d.dim_id");
            }
            from.push_str(" WHERE ");
            from.push_str(&w);
            if before.shape == Shape::Grouped {
                format!(
                    "SELECT o.customer_id AS customer_id, count(*) AS n {from} GROUP BY \
                     o.customer_id"
                )
            } else {
                select_sql(&before_items, &from)
            }
        }
        other => panic!("render_adversarial called with a non-adversarial edit: {other:?}"),
    };

    let mut sources = base_sources();
    if before.shape == Shape::Joined {
        sources = with_dim_source(sources);
    }
    let mut added_column_types = BTreeMap::new();
    if matches!(edit, EditRecipe::AdversarialVolatileFn) {
        added_column_types.insert("flag".to_string(), "DOUBLE".to_string());
    }

    let inputs = BackbuildInputs {
        table: "t".to_string(),
        after_sql: after_sql.clone(),
        row_identity: Some(vec!["order_id".to_string()]),
        not_null_columns: before_not_null,
        added_column_types,
        sources,
    };

    Rendered {
        before_sql,
        after_sql,
        inputs,
    }
}
