"""Tests for the Python incremental rule."""

from smelt_sdk import (
    CountDistinct,
    GroupByKey,
    IncrementalConfig,
    ModelInfo,
    OtherAggregate,
    SelectAnalysis,
    SetIncremental,
)
from smelt_rules_builtin.incremental_python import IncrementalPythonRule, _extract_time_column


rule = IncrementalPythonRule()


def _analysis(items, group_by_exprs):
    return SelectAnalysis(
        items=items,
        from_text="FROM events",
        where_text=None,
        group_by_exprs=group_by_exprs,
        has_cube_split_annotation=False,
    )


def test_detect_incremental():
    model = ModelInfo(
        name="daily",
        sql="",
        incremental_config=IncrementalConfig(partition_column="event_date", event_time_column="event_time", granularity="day"),
    )
    analysis = _analysis(
        items=[
            GroupByKey(text="date_trunc('day', event_time)", alias="event_date"),
            GroupByKey(text="user_id", alias="user_id"),
            OtherAggregate(text="COUNT(*)", alias="cnt"),
        ],
        group_by_exprs=["date_trunc('day', event_time)", "user_id"],
    )
    opp = rule.detect(model, analysis)
    assert opp is not None
    assert opp.rule_name == "incremental_python"
    assert opp.data["event_time_column"] == "event_time"
    assert opp.data["partition_column"] == "event_date"


def test_detect_no_config():
    model = ModelInfo(name="test", sql="")
    analysis = _analysis(
        items=[GroupByKey(text="a", alias="a")],
        group_by_exprs=["a"],
    )
    opp = rule.detect(model, analysis)
    assert opp is None


def test_detect_missing_partition_column():
    model = ModelInfo(
        name="test",
        sql="",
        incremental_config=IncrementalConfig(partition_column="nonexistent", event_time_column="event_time", granularity="day"),
    )
    analysis = _analysis(
        items=[GroupByKey(text="a", alias="a")],
        group_by_exprs=["a"],
    )
    opp = rule.detect(model, analysis)
    assert opp is None


def test_detect_partition_not_in_group_by():
    model = ModelInfo(
        name="test",
        sql="",
        incremental_config=IncrementalConfig(partition_column="event_date", event_time_column="event_time", granularity="day"),
    )
    analysis = _analysis(
        items=[
            GroupByKey(text="date_trunc('day', event_time)", alias="event_date"),
        ],
        group_by_exprs=[],  # not in GROUP BY
    )
    opp = rule.detect(model, analysis)
    assert opp is None


def test_rewrite_produces_set_incremental():
    model = ModelInfo(
        name="daily",
        sql="",
        incremental_config=IncrementalConfig(partition_column="event_date", event_time_column="event_time", granularity="day"),
    )
    analysis = _analysis(
        items=[
            GroupByKey(text="date_trunc('day', event_time)", alias="event_date"),
            GroupByKey(text="user_id", alias="user_id"),
            OtherAggregate(text="COUNT(*)", alias="cnt"),
        ],
        group_by_exprs=["date_trunc('day', event_time)", "user_id"],
    )
    result = rule.rewrite(model, analysis)
    assert isinstance(result, SetIncremental)
    assert result.model == "daily"
    assert result.event_time_column == "event_time"
    assert result.partition_column == "event_date"


def test_extract_time_column_date_trunc():
    assert _extract_time_column("date_trunc('day', event_time)") == "event_time"


def test_extract_time_column_date_func():
    assert _extract_time_column("DATE(event_time)") == "event_time"


def test_extract_time_column_simple():
    assert _extract_time_column("event_date") == "event_date"
