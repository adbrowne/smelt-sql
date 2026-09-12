use super::*;
use crate::parser::parse;
use crate::SyntaxKind;

/// Round-trip helpers: parse text → find CST node → cast to AST → check text equality.
fn round_trip_text(node: &SyntaxNode) -> String {
    node.text().to_string()
}

#[test]
fn ast_wrappers_for_record_constructs_round_trip() {
    // 1. SmeltRecordDecl round-trip
    {
        let src = "smelt.record SourceEntry = { name: Text, age: Integer }";
        let parse = parse(src);
        assert!(
            parse.errors.is_empty(),
            "SmeltRecordDecl: unexpected parse errors: {:?}",
            parse.errors
        );
        let decl = parse
            .syntax()
            .descendants()
            .find_map(SmeltRecordDecl::cast)
            .expect("must find SmeltRecordDecl node");
        // Name round-trip
        assert_eq!(
            decl.name().as_deref(),
            Some("SourceEntry"),
            "SmeltRecordDecl::name() must return 'SourceEntry'"
        );
        // Body round-trip: CST → AST → CST text equality
        let body = decl.body().expect("SmeltRecordDecl must have a body");
        assert!(
            round_trip_text(body.syntax()).contains("name: Text"),
            "SmeltRecordDecl body text must contain 'name: Text'"
        );
        // Fields
        let field_names: Vec<_> = decl.fields().filter_map(|f| f.name()).collect();
        assert_eq!(field_names, vec!["name", "age"]);
    }

    // 2. RecordLiteral round-trip
    {
        let src = "SELECT smelt.foo({a: 1, b: 'x'}) FROM t";
        let parse = parse(src);
        assert!(
            parse.errors.is_empty(),
            "RecordLiteral: unexpected parse errors: {:?}",
            parse.errors
        );
        let lit = parse
            .syntax()
            .descendants()
            .find_map(RecordLiteral::cast)
            .expect("must find RecordLiteral node");
        // Text round-trip
        let lit_text = round_trip_text(lit.syntax());
        assert!(
            lit_text.contains("a: 1") && lit_text.contains("b: 'x'"),
            "RecordLiteral text must contain field entries, got: {}",
            lit_text
        );
        // Fields
        let field_names: Vec<_> = lit.fields().filter_map(|f| f.name()).collect();
        assert_eq!(field_names, vec!["a", "b"]);
    }

    // 3. RecordTypeInline round-trip
    {
        let src = "smelt.define foo(cfg: { name: Text, count: Integer }) AS (cfg)";
        let parse = parse(src);
        assert!(
            parse.errors.is_empty(),
            "RecordTypeInline: unexpected parse errors: {:?}",
            parse.errors
        );
        let inline = parse
            .syntax()
            .descendants()
            .find_map(RecordTypeInline::cast)
            .expect("must find RecordTypeInline node");
        // Text round-trip
        let text = round_trip_text(inline.syntax());
        assert!(
            text.contains("name: Text"),
            "RecordTypeInline text must contain 'name: Text', got: {}",
            text
        );
        // Fields
        let field_names: Vec<_> = inline.fields().filter_map(|f| f.name()).collect();
        assert_eq!(field_names, vec!["name", "count"]);
    }

    // 4. MapMethodCall round-trip
    {
        let src = "smelt.define foo(m: Map<Text, Integer>) AS (m.entries())";
        let parse = parse(src);
        assert!(
            parse.errors.is_empty(),
            "MapMethodCall: unexpected parse errors: {:?}",
            parse.errors
        );
        let call = parse
            .syntax()
            .descendants()
            .find_map(MapMethodCall::cast)
            .expect("must find MapMethodCall node");
        // Method name
        assert_eq!(
            call.method_name().as_deref(),
            Some("entries"),
            "MapMethodCall::method_name() must return 'entries'"
        );
        // Text round-trip: CST → AST → CST text equality
        let call_text = round_trip_text(call.syntax());
        assert!(
            call_text.contains("entries"),
            "MapMethodCall text must contain 'entries', got: {}",
            call_text
        );
    }
}

/// `as_binary` on a node that is itself a `BINARY_EXPR` must wrap that
/// node, never a same-kind first child. `a AND b AND c` parses
/// left-associatively as `(a AND b) AND c`, and AND operands are bare
/// `BINARY_EXPR` children (no `EXPRESSION` wrapper) — so the inner
/// `a AND b` subtree has a `BINARY_EXPR` first child (the comparison
/// `a`), and a child-first cast silently returns that comparison,
/// dropping `b` from every recursive `as_binary` walk.
#[test]
fn as_binary_on_nested_and_chain_wraps_self_not_first_child() {
    let src = "SELECT 1 FROM t WHERE x = 1 AND y IS NOT NULL AND z = 2";
    let parsed = parse(src);
    assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
    let file = File::cast(parsed.syntax()).expect("file");
    let stmt = file.select_stmt().expect("stmt");
    let top = stmt
        .where_clause()
        .and_then(|w| w.expression())
        .expect("where expr");

    let top_bin = top.as_binary().expect("top is binary");
    assert_eq!(top_bin.operator().as_deref(), Some("AND"));
    let left = top_bin.left().expect("left subtree");
    assert_eq!(
        left.syntax().text().to_string().trim(),
        "x = 1 AND y IS NOT NULL"
    );

    let left_bin = left.as_binary().expect("left subtree is binary");
    assert_eq!(
        left_bin.operator().as_deref(),
        Some("AND"),
        "as_binary must wrap the AND subtree itself, not its first-child comparison"
    );
    assert_eq!(
        left_bin
            .right()
            .expect("right operand")
            .syntax()
            .text()
            .to_string()
            .trim(),
        "y IS NOT NULL",
        "the right operand of the inner AND must be reachable through as_binary"
    );
}

/// Same self-vs-first-child ambiguity as the `as_binary` test above, for
/// the other node kinds observed to nest a same-kind direct child:
/// `AT_TIME_ZONE_EXPR` (chained `AT TIME ZONE`) and `SUBQUERY`
/// (`((SELECT 1))`). A child-first cast returns the inner node, silently
/// dropping the outer node's own structure from a recursive walk.
#[test]
fn self_kind_accessors_prefer_self_over_same_kind_child() {
    let src = "SELECT a AT TIME ZONE 'utc' AT TIME ZONE 'est' FROM t";
    let parsed = parse(src);
    assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
    let outer = parsed
        .syntax()
        .descendants()
        .find(|n| n.kind() == SyntaxKind::AT_TIME_ZONE_EXPR)
        .expect("outer AT_TIME_ZONE_EXPR");
    let tz = Expr::cast(outer.clone())
        .expect("expr")
        .as_at_time_zone()
        .expect("at-time-zone")
        .timezone_expr()
        .expect("timezone expr");
    assert_eq!(
        tz.syntax().text().to_string().trim(),
        "'est'",
        "as_at_time_zone on the outer chained node must wrap that node, not the inner one"
    );
}

/// `is_struct_spread_call` must recognize a `smelt.<path>(args).*`
/// select item, must NOT mistake a bare `*` or an ordinary select item
/// for one, and must NOT mistake a plain (non-`.*`) `smelt.<path>(args)`
/// call for one either.
#[test]
fn is_struct_spread_call_detects_only_the_smelt_path_call_star_shape() {
    let src = "SELECT id, smelt.functions.parse_event(payload).*, smelt.functions.other(x), y \
             FROM t";
    let parsed = parse(src);
    assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
    let select_list = parsed
        .syntax()
        .descendants()
        .find_map(SelectList::cast)
        .expect("must find SELECT_LIST");
    let items: Vec<SelectItem> = select_list.items().collect();
    assert_eq!(items.len(), 4, "items: {items:?}");

    assert!(
        !items[0].is_struct_spread_call(),
        "a plain column reference must not be treated as a struct spread"
    );
    assert!(
        items[1].is_struct_spread_call(),
        "smelt.<path>(args).* must be detected as a struct spread"
    );
    assert!(
        !items[2].is_struct_spread_call(),
        "a plain smelt.<path>(args) call (no trailing .*) must not be \
             treated as a struct spread"
    );
    assert!(
        !items[3].is_struct_spread_call(),
        "a plain column reference must not be treated as a struct spread"
    );

    let bare_star = parse("SELECT * FROM t");
    assert!(
        bare_star.errors.is_empty(),
        "errors: {:?}",
        bare_star.errors
    );
    let star_item = bare_star
        .syntax()
        .descendants()
        .find_map(SelectItem::cast)
        .expect("must find a SELECT_ITEM");
    assert!(
        star_item.is_wildcard(),
        "sanity check: bare `*` must still be a wildcard"
    );
    assert!(
        !star_item.is_struct_spread_call(),
        "a bare `*` is a wildcard, not a struct spread"
    );
}
