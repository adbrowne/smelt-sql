"""Tests for the Python cube_split rule."""

from smelt_sdk import (
    CountDistinct,
    CreateTemp,
    DropTemp,
    FinalQuery,
    GroupByKey,
    ModelInfo,
    OtherAggregate,
    ReplaceWithPlan,
    SelectAnalysis,
)
from smelt_rules_builtin.cube_split_python import CubeSplitPythonRule


def _analysis(
    items=None,
    from_text="FROM events",
    where_text=None,
    group_by_exprs=None,
    has_cube_split_annotation=True,
):
    return SelectAnalysis(
        items=items or [],
        from_text=from_text,
        where_text=where_text,
        group_by_exprs=group_by_exprs or [],
        has_cube_split_annotation=has_cube_split_annotation,
    )


def _model(name="test"):
    return ModelInfo(name=name, sql="")


rule = CubeSplitPythonRule()


def test_detect_with_annotation():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="u"),
            CountDistinct(argument="session_id", alias="s"),
        ],
        group_by_exprs=["country"],
    )
    opp = rule.detect(_model(), analysis)
    assert opp is not None
    assert opp.rule_name == "cube_split_python"
    assert opp.data["count_distinct_count"] == 2


def test_detect_without_annotation():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="u"),
            CountDistinct(argument="session_id", alias="s"),
        ],
        has_cube_split_annotation=False,
    )
    opp = rule.detect(_model(), analysis)
    assert opp is None


def test_detect_insufficient_count_distinct():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="u"),
        ],
    )
    opp = rule.detect(_model(), analysis)
    assert opp is None


def test_rewrite_produces_correct_steps():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="unique_users"),
            CountDistinct(argument="session_id", alias="unique_sessions"),
        ],
        group_by_exprs=["country"],
    )
    result = rule.rewrite(_model("metrics"), analysis)
    assert isinstance(result, ReplaceWithPlan)
    # 2 CreateTemp + 1 FinalQuery + 2 DropTemp = 5
    assert len(result.steps) == 5
    assert isinstance(result.steps[0], CreateTemp)
    assert result.steps[0].name == "__cube_metrics_0"
    assert isinstance(result.steps[1], CreateTemp)
    assert result.steps[1].name == "__cube_metrics_1"
    assert isinstance(result.steps[2], FinalQuery)
    assert isinstance(result.steps[3], DropTemp)
    assert isinstance(result.steps[4], DropTemp)


def test_rewrite_final_query_joins():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="u"),
            CountDistinct(argument="session_id", alias="s"),
        ],
        group_by_exprs=["country"],
    )
    result = rule.rewrite(_model("m"), analysis)
    assert isinstance(result, ReplaceWithPlan)
    final = result.steps[2]
    assert isinstance(final, FinalQuery)
    assert "IS NOT DISTINCT FROM" in final.sql
    assert "t0.country" in final.sql
    assert "t0.u" in final.sql
    assert "t1.s" in final.sql


def test_rewrite_with_other_aggregates():
    analysis = _analysis(
        items=[
            GroupByKey(text="country", alias="country"),
            CountDistinct(argument="user_id", alias="u"),
            CountDistinct(argument="session_id", alias="s"),
            OtherAggregate(text="COUNT(*)", alias="total"),
            OtherAggregate(text="SUM(revenue)", alias="rev"),
        ],
        group_by_exprs=["country"],
    )
    result = rule.rewrite(_model("m"), analysis)
    assert isinstance(result, ReplaceWithPlan)

    # Query 0 should have other aggregates
    assert "COUNT(*)" in result.steps[0].sql
    assert "SUM(revenue)" in result.steps[0].sql

    # Query 1 should NOT have other aggregates
    assert "COUNT(*)" not in result.steps[1].sql
    assert "SUM(revenue)" not in result.steps[1].sql

    # Final should select other aggs from t0
    final = result.steps[2]
    assert isinstance(final, FinalQuery)
    assert "t0.total" in final.sql
    assert "t0.rev" in final.sql


def test_rewrite_no_group_keys_uses_cross_join():
    analysis = _analysis(
        items=[
            CountDistinct(argument="user_id", alias="u"),
            CountDistinct(argument="session_id", alias="s"),
        ],
        group_by_exprs=[],
    )
    result = rule.rewrite(_model("m"), analysis)
    assert isinstance(result, ReplaceWithPlan)
    final = result.steps[2]
    assert isinstance(final, FinalQuery)
    assert "CROSS JOIN" in final.sql
