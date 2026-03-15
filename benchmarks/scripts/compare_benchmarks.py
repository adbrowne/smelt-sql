#!/usr/bin/env python3
"""Compare two smelt benchmark result JSON files and output a markdown table."""

import json
import sys


def extract_metrics(data):
    """Extract all metrics as a flat list of (name, unit, value, direction).

    direction: 'lower' means lower is better, 'higher' means higher is better.
    """
    b = data["build"]
    s = data["salsa"]
    p = data["parser"]

    metrics = [
        ("Build / Total", "ms", b["total_ms"], "lower"),
        ("Build / Discovery", "ms", b["discovery_ms"], "lower"),
        ("Build / Graph Build", "ms", b["graph_build_ms"], "lower"),
        ("Build / Topo Sort", "ms", b["topo_sort_ms"], "lower"),
        ("Build / Validation", "ms", b["validation_ms"], "lower"),
        ("Salsa / Initial Load", "ms", s["initial_load_ms"], "lower"),
        ("Salsa / Leaf Edit Diagnostics", "ms", s["leaf_edit_diagnostics_ms"], "lower"),
        ("Salsa / Mid Edit Diagnostics", "ms", s["mid_edit_diagnostics_ms"], "lower"),
        ("Salsa / Root Edit Diagnostics", "ms", s["root_edit_diagnostics_ms"], "lower"),
        ("Salsa / Add File", "ms", s["add_file_all_models_ms"], "lower"),
        ("Salsa / Full Diagnostics", "ms", s["full_diagnostics_ms"], "lower"),
        ("Parser / Simple SQL", "μs", p["single_simple_us"], "lower"),
        ("Parser / Complex SQL", "μs", p["single_complex_us"], "lower"),
        (f"Parser / Batch ({p['batch_count']})", "ms", p["batch_all_ms"], "lower"),
        ("Parser / Throughput", "MB/s", p["bytes_per_second"] / 1_000_000, "higher"),
    ]
    return metrics


def fmt_value(value, unit):
    """Format a value with appropriate precision."""
    if unit == "μs" or value < 1:
        return f"{value:.2f}"
    elif value < 10:
        return f"{value:.1f}"
    else:
        return f"{value:.1f}"


def emoji(pct_change, direction):
    """Return an emoji indicator based on the percentage change and direction.

    For 'lower' metrics: positive change = regression (value went up).
    For 'higher' metrics: negative change = regression (value went down).
    """
    if direction == "lower":
        if pct_change > 5:
            return "🔴"
        elif pct_change < -5:
            return "🟢"
    else:  # higher is better
        if pct_change < -5:
            return "🔴"
        elif pct_change > 5:
            return "🟢"
    return "⚪"


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <base_result.json> <pr_result.json>", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1]) as f:
        base_data = json.load(f)
    with open(sys.argv[2]) as f:
        pr_data = json.load(f)

    base_metrics = extract_metrics(base_data)
    pr_metrics = extract_metrics(pr_data)

    lines = [
        "## Benchmark Comparison",
        "",
        f"**Base:** `{base_data['git_commit'][:7]}` ({base_data['git_branch']}) | "
        f"**PR:** `{pr_data['git_commit'][:7]}` ({pr_data['git_branch']})",
        "",
        "| Metric | Base | PR | Delta | % |  |",
        "|--------|-----:|---:|------:|--:|----|",
    ]

    for (name, unit, base_val, direction), (_, _, pr_val, _) in zip(base_metrics, pr_metrics):
        delta = pr_val - base_val
        if base_val != 0:
            pct = (delta / base_val) * 100
        else:
            pct = 0.0

        sign = "+" if delta > 0 else ""
        indicator = emoji(pct, direction)

        lines.append(
            f"| {name} | {fmt_value(base_val, unit)} {unit} "
            f"| {fmt_value(pr_val, unit)} {unit} "
            f"| {sign}{fmt_value(delta, unit)} {unit} "
            f"| {sign}{pct:.1f}% "
            f"| {indicator} |"
        )

    lines.append("")
    lines.append(
        "> 🟢 >5% improvement · 🔴 >5% regression · ⚪ within 5%"
    )

    print("\n".join(lines))


if __name__ == "__main__":
    main()
