"""Dataclass mirrors of Rust optimizer types."""

from __future__ import annotations

from dataclasses import dataclass, field


# --- Model types ---


@dataclass
class IncrementalConfig:
    partition_column: str
    event_time_column: str
    granularity: str  # "hour", "day", "week:<day>" (e.g. "week:monday"), or "month"


@dataclass
class ModelInfo:
    name: str
    sql: str
    refs: list[str] = field(default_factory=list)
    incremental_config: IncrementalConfig | None = None


# --- SelectAnalysis types ---


@dataclass
class CountDistinct:
    argument: str
    alias: str


@dataclass
class OtherAggregate:
    text: str
    alias: str


@dataclass
class GroupByKey:
    text: str
    alias: str


SelectItemKind = CountDistinct | OtherAggregate | GroupByKey


@dataclass
class SelectAnalysis:
    items: list[SelectItemKind]
    from_text: str
    where_text: str | None
    group_by_exprs: list[str]
    has_cube_split_annotation: bool


# --- Execution step types ---


@dataclass
class CreateTemp:
    name: str
    sql: str


@dataclass
class AppendToTemp:
    name: str
    sql: str


@dataclass
class FinalQuery:
    sql: str


@dataclass
class DropTemp:
    name: str


ExecutionStep = CreateTemp | AppendToTemp | FinalQuery | DropTemp


# --- Transformation types ---


@dataclass
class ReplaceWithPlan:
    model: str
    steps: list[ExecutionStep]


@dataclass
class SetIncremental:
    model: str
    event_time_column: str
    partition_column: str
    granularity: str  # "hour", "day", "week:<day>" (e.g. "week:monday"), or "month"


Transformation = ReplaceWithPlan | SetIncremental


# --- Opportunity ---


@dataclass
class Opportunity:
    rule_name: str
    model: str
    description: str
    data: dict = field(default_factory=dict)
