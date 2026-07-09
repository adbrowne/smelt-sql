//! `JoinContext`/`fan_out` composite-key coverage (catalog cell G-10;
//! `docs/plans/20260707-maintenance-plan-impl.md` Phase MP10).
//!
//! A genuine COMPOSITE unique key — e.g. `(user_id, dt)`, jointly but not
//! individually unique — must prove `Cardinality::OneToOne` when the
//! equi-join matches every column of the declared key-set. Matching only a
//! strict SUBSET of a declared composite key must NOT prove one-to-one: that
//! is the key hazard this phase's review checklist calls out, and it must
//! fail closed to `OneToMany`.

use smelt_logical::analysis::join_shape::{fan_out, Cardinality, JoinContext};

fn parse_join(sql: &str) -> smelt_parser::JoinClause {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select stmt");
    let from = select.from_clause().expect("from clause");
    let join = from.joins().next().expect("join clause");
    join
}

#[test]
fn two_column_key_proves_one_to_one() {
    let join = parse_join("SELECT * FROM events e JOIN dims d ON e.k1 = d.k1 AND e.k2 = d.k2");
    let ctx = JoinContext::new().with_composite_unique_key("d", &["k1", "k2"]);
    assert_eq!(fan_out(&join, &ctx), Cardinality::OneToOne);
}

#[test]
fn partial_key_match_is_fan_out() {
    let join = parse_join("SELECT * FROM events e JOIN dims d ON e.k1 = d.k1");
    let ctx = JoinContext::new().with_composite_unique_key("d", &["k1", "k2"]);
    assert_eq!(fan_out(&join, &ctx), Cardinality::OneToMany);
}

#[test]
fn three_column_key_all_matched_proves_one_to_one() {
    let join = parse_join(
        "SELECT * FROM events e JOIN dims d ON e.k1 = d.k1 AND e.k2 = d.k2 AND e.k3 = d.k3",
    );
    let ctx = JoinContext::new().with_composite_unique_key("d", &["k1", "k2", "k3"]);
    assert_eq!(fan_out(&join, &ctx), Cardinality::OneToOne);
}

#[test]
fn matching_superset_of_declared_key_still_proves_one_to_one() {
    // The join equi-matches k1, k2, AND k3, but only (k1, k2) is declared as
    // the unique key — a superset match still proves one-to-one.
    let join = parse_join(
        "SELECT * FROM events e JOIN dims d ON e.k1 = d.k1 AND e.k2 = d.k2 AND e.k3 = d.k3",
    );
    let ctx = JoinContext::new().with_composite_unique_key("d", &["k1", "k2"]);
    assert_eq!(fan_out(&join, &ctx), Cardinality::OneToOne);
}

#[test]
fn single_column_convenience_still_composes_with_composite_declarations() {
    // A source with both a single-column key and a composite key declared:
    // matching either fully proves one-to-one.
    let join_single = parse_join("SELECT * FROM events e JOIN dims d ON e.id = d.id");
    let join_composite =
        parse_join("SELECT * FROM events e JOIN dims d ON e.k1 = d.k1 AND e.k2 = d.k2");
    let ctx = JoinContext::new()
        .with_unique_key("d", "id")
        .with_composite_unique_key("d", &["k1", "k2"]);
    assert_eq!(fan_out(&join_single, &ctx), Cardinality::OneToOne);
    assert_eq!(fan_out(&join_composite, &ctx), Cardinality::OneToOne);
}

#[test]
fn undeclared_composite_key_fails_closed() {
    // No key declared at all for `d` — must stay OneToMany even though the
    // join matches two columns.
    let join = parse_join("SELECT * FROM events e JOIN dims d ON e.k1 = d.k1 AND e.k2 = d.k2");
    let ctx = JoinContext::new();
    assert_eq!(fan_out(&join, &ctx), Cardinality::OneToMany);
}
