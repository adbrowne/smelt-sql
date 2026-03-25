"""Python port of the Rust cube_split planner rule."""

from __future__ import annotations

from smelt_sdk import (
    CountDistinct,
    CreateTemp,
    DropTemp,
    FinalQuery,
    GroupByKey,
    ModelInfo,
    Opportunity,
    PlannerRule,
    OtherAggregate,
    ReplaceWithPlan,
    SelectAnalysis,
    SetIncremental,
)


class CubeSplitPythonRule(PlannerRule):
    def name(self) -> str:
        return "cube_split_python"

    def detect(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> Opportunity | None:
        if analysis is None:
            return None
        if not analysis.has_cube_split_annotation:
            return None

        count_distincts = [i for i in analysis.items if isinstance(i, CountDistinct)]
        if len(count_distincts) < 2:
            return None

        group_by_keys = [
            i.alias for i in analysis.items if isinstance(i, GroupByKey)
        ]

        return Opportunity(
            rule_name=self.name(),
            model=model.name,
            description=(
                f"Split {len(count_distincts)} COUNT(DISTINCT) expressions "
                f"into parallel sub-queries joined on {len(group_by_keys)} GROUP BY key(s)"
            ),
            data={
                "count_distinct_count": len(count_distincts),
                "group_by_keys": group_by_keys,
            },
        )

    def rewrite(
        self,
        model: ModelInfo,
        analysis: SelectAnalysis | None,
    ) -> ReplaceWithPlan | SetIncremental | None:
        if analysis is None:
            return None

        group_keys: list[tuple[str, str]] = []
        count_distincts: list[tuple[str, str]] = []
        other_aggs: list[tuple[str, str]] = []

        for item in analysis.items:
            if isinstance(item, GroupByKey):
                group_keys.append((item.text, item.alias))
            elif isinstance(item, CountDistinct):
                count_distincts.append((item.argument, item.alias))
            elif isinstance(item, OtherAggregate):
                other_aggs.append((item.text, item.alias))

        if len(count_distincts) < 2:
            return None

        from_clause = analysis.from_text
        where_clause = f"WHERE {analysis.where_text}" if analysis.where_text else ""

        if group_keys:
            gb_exprs = ", ".join(expr for expr, _ in group_keys)
            group_by_clause = f"GROUP BY {gb_exprs}"
        else:
            group_by_clause = ""

        steps = []

        # Query 0: group keys + first COUNT DISTINCT + all other aggregates
        select_items = [f"{expr} as {alias}" for expr, alias in group_keys]
        arg0, alias0 = count_distincts[0]
        select_items.append(f"COUNT(DISTINCT {arg0}) as {alias0}")
        select_items.extend(f"{expr} as {alias}" for expr, alias in other_aggs)

        sql = f"SELECT {', '.join(select_items)} {from_clause} {where_clause} {group_by_clause}".strip()
        steps.append(CreateTemp(name=f"__cube_{model.name}_0", sql=sql))

        # Queries 1..N: group keys + one COUNT DISTINCT each
        for i, (arg, alias) in enumerate(count_distincts[1:], start=1):
            select_items = [f"{expr} as {ka}" for expr, ka in group_keys]
            select_items.append(f"COUNT(DISTINCT {arg}) as {alias}")
            sql = f"SELECT {', '.join(select_items)} {from_clause} {where_clause} {group_by_clause}".strip()
            steps.append(CreateTemp(name=f"__cube_{model.name}_{i}", sql=sql))

        # Final join query
        t0 = f"__cube_{model.name}_0"
        final_select = [f"t0.{alias}" for _, alias in group_keys]
        final_select.append(f"t0.{count_distincts[0][1]}")
        for i, (_, alias) in enumerate(count_distincts[1:], start=1):
            final_select.append(f"t{i}.{alias}")
        final_select.extend(f"t0.{alias}" for _, alias in other_aggs)

        join_clauses = ""
        for i in range(1, len(count_distincts)):
            tn = f"__cube_{model.name}_{i}"
            if group_keys:
                conditions = " AND ".join(
                    f"t0.{alias} IS NOT DISTINCT FROM t{i}.{alias}"
                    for _, alias in group_keys
                )
                join_clauses += f" JOIN {tn} t{i} ON {conditions}"
            else:
                join_clauses += f" CROSS JOIN {tn} t{i}"

        final_sql = f"SELECT {', '.join(final_select)} FROM {t0} t0{join_clauses}".strip()
        steps.append(FinalQuery(sql=final_sql))

        # Cleanup temp tables
        for i in range(len(count_distincts)):
            steps.append(DropTemp(name=f"__cube_{model.name}_{i}"))

        return ReplaceWithPlan(model=model.name, steps=steps)
