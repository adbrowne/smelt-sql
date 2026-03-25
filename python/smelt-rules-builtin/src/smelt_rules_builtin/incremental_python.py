"""Python port of the Rust incremental planner rule."""

from __future__ import annotations

import re

from smelt_sdk import (
    GroupByKey,
    ModelInfo,
    Opportunity,
    PlannerRule,
    ReplaceWithPlan,
    SelectAnalysis,
    SetIncremental,
)


def _extract_time_column(expr: str) -> str | None:
    """Extract the source time column from a partition expression.

    Handles:
    - date_trunc('interval', column) -> column
    - DATE(column) -> column
    - simple_column -> simple_column
    """
    trimmed = expr.strip()

    # date_trunc('interval', column)
    m = re.match(r"(?i)date_trunc\(\s*'[^']*'\s*,\s*(.+)\s*\)$", trimmed)
    if m:
        return m.group(1).strip()

    # DATE(column)
    m = re.match(r"(?i)date\(\s*(.+)\s*\)$", trimmed)
    if m:
        return m.group(1).strip()

    # Simple column reference (no parens)
    if "(" not in trimmed:
        return trimmed

    return None


class IncrementalPythonRule(PlannerRule):
    def name(self) -> str:
        return "incremental_python"

    def detect(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> Opportunity | None:
        if model.incremental_config is None:
            return None
        if analysis is None:
            return None

        partition_col = model.incremental_config.partition_column

        # Find partition column alias in SELECT list
        partition_expr: str | None = None
        for item in analysis.items:
            alias = getattr(item, "alias", None)
            if alias == partition_col:
                if isinstance(item, GroupByKey):
                    partition_expr = item.text
                else:
                    partition_expr = getattr(item, "text", getattr(item, "argument", None))
                break

        if partition_expr is None:
            return None

        # Validate it appears in GROUP BY
        if partition_expr not in analysis.group_by_exprs:
            return None

        event_time_column = _extract_time_column(partition_expr) or partition_expr

        return Opportunity(
            rule_name=self.name(),
            model=model.name,
            description=(
                f"Incremental materialization on partition column "
                f"'{partition_col}' (source: '{event_time_column}')"
            ),
            data={
                "event_time_column": event_time_column,
                "partition_column": partition_col,
            },
        )

    def rewrite(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> ReplaceWithPlan | SetIncremental | None:
        opp = self.detect(model, analysis)
        if opp is None:
            return None

        return SetIncremental(
            model=model.name,
            event_time_column=opp.data["event_time_column"],
            partition_column=opp.data["partition_column"],
        )
