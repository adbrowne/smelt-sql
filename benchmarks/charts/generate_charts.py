#!/usr/bin/env python3
"""Generate benchmark charts from JSON result files.

Usage:
    python generate_charts.py <results_dir> [--output <output_dir>]
"""

import argparse
import json
import os
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
import seaborn as sns


def load_results(results_dir: str) -> pd.DataFrame:
    """Load all JSON result files into a DataFrame."""
    records = []
    for filename in sorted(Path(results_dir).glob("*.json")):
        with open(filename) as f:
            data = json.load(f)
        # Flatten nested structures
        record = {
            "git_commit": data.get("git_commit", "")[:7],
            "git_branch": data.get("git_branch", ""),
            "timestamp": data.get("timestamp", ""),
            "model_count": data.get("model_count", 0),
            # Build metrics
            "build_total_ms": data.get("build", {}).get("total_ms", 0),
            "build_discovery_ms": data.get("build", {}).get("discovery_ms", 0),
            "build_graph_build_ms": data.get("build", {}).get("graph_build_ms", 0),
            "build_topo_sort_ms": data.get("build", {}).get("topo_sort_ms", 0),
            "build_validation_ms": data.get("build", {}).get("validation_ms", 0),
            # Salsa metrics
            "salsa_initial_load_ms": data.get("salsa", {}).get("initial_load_ms", 0),
            "salsa_leaf_edit_ms": data.get("salsa", {}).get("leaf_edit_diagnostics_ms", 0),
            "salsa_mid_edit_ms": data.get("salsa", {}).get("mid_edit_diagnostics_ms", 0),
            "salsa_root_edit_ms": data.get("salsa", {}).get("root_edit_diagnostics_ms", 0),
            "salsa_add_file_ms": data.get("salsa", {}).get("add_file_all_models_ms", 0),
            "salsa_full_diagnostics_ms": data.get("salsa", {}).get("full_diagnostics_ms", 0),
            # Parser metrics
            "parser_simple_us": data.get("parser", {}).get("single_simple_us", 0),
            "parser_complex_us": data.get("parser", {}).get("single_complex_us", 0),
            "parser_batch_ms": data.get("parser", {}).get("batch_all_ms", 0),
            "parser_bytes_per_sec": data.get("parser", {}).get("bytes_per_second", 0),
        }
        records.append(record)

    if not records:
        print("No benchmark results found.", file=sys.stderr)
        sys.exit(1)

    df = pd.DataFrame(records)
    df["timestamp"] = pd.to_datetime(df["timestamp"])
    df = df.sort_values("timestamp").reset_index(drop=True)
    return df


def chart_build_time(df: pd.DataFrame, output_dir: str):
    """Chart 1: Build time over commits with sub-phase breakdown."""
    fig, ax = plt.subplots(figsize=(12, 6))

    x = range(len(df))
    labels = df["git_commit"]

    ax.plot(x, df["build_total_ms"], "o-", label="Total", linewidth=2, color="black")
    ax.plot(x, df["build_discovery_ms"], "s--", label="Discovery", alpha=0.7)
    ax.plot(x, df["build_graph_build_ms"], "^--", label="Graph Build", alpha=0.7)
    ax.plot(x, df["build_topo_sort_ms"], "D--", label="Topo Sort", alpha=0.7)
    ax.plot(x, df["build_validation_ms"], "v--", label="Validation", alpha=0.7)

    ax.set_xlabel("Commit")
    ax.set_ylabel("Time (ms)")
    ax.set_title("Build Pipeline Performance Over Commits")
    ax.set_xticks(list(x))
    ax.set_xticklabels(labels, rotation=45, ha="right")
    ax.legend()
    ax.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, "build_time.png"), dpi=150)
    plt.close()


def chart_phase_breakdown(df: pd.DataFrame, output_dir: str):
    """Chart 2: Stacked bar showing build phase percentages."""
    fig, ax = plt.subplots(figsize=(12, 6))

    phases = ["build_discovery_ms", "build_graph_build_ms", "build_topo_sort_ms", "build_validation_ms"]
    phase_labels = ["Discovery", "Graph Build", "Topo Sort", "Validation"]
    colors = sns.color_palette("Set2", len(phases))

    x = range(len(df))
    bottom = [0.0] * len(df)

    for phase, label, color in zip(phases, phase_labels, colors):
        values = df[phase].tolist()
        ax.bar(x, values, bottom=bottom, label=label, color=color)
        bottom = [b + v for b, v in zip(bottom, values)]

    ax.set_xlabel("Commit")
    ax.set_ylabel("Time (ms)")
    ax.set_title("Build Phase Breakdown")
    ax.set_xticks(list(x))
    ax.set_xticklabels(df["git_commit"], rotation=45, ha="right")
    ax.legend()

    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, "phase_breakdown.png"), dpi=150)
    plt.close()


def chart_salsa_latency(df: pd.DataFrame, output_dir: str):
    """Chart 3: Grouped bars for Salsa edit diagnostics time."""
    fig, ax = plt.subplots(figsize=(12, 6))

    metrics = {
        "Leaf Edit": df["salsa_leaf_edit_ms"],
        "Mid Edit": df["salsa_mid_edit_ms"],
        "Root Edit": df["salsa_root_edit_ms"],
        "Add File": df["salsa_add_file_ms"],
    }

    x = range(len(df))
    width = 0.2
    colors = sns.color_palette("Set1", len(metrics))

    for i, (label, values) in enumerate(metrics.items()):
        offsets = [xi + i * width for xi in x]
        ax.bar(offsets, values, width=width, label=label, color=colors[i])

    ax.set_xlabel("Commit")
    ax.set_ylabel("Time (ms)")
    ax.set_title("Salsa Edit Latency by Layer")
    ax.set_xticks([xi + width * 1.5 for xi in x])
    ax.set_xticklabels(df["git_commit"], rotation=45, ha="right")
    ax.legend()
    ax.grid(True, alpha=0.3, axis="y")

    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, "salsa_latency.png"), dpi=150)
    plt.close()


def chart_parser_throughput(df: pd.DataFrame, output_dir: str):
    """Chart 4: Parser throughput (bytes/second) over time."""
    fig, ax = plt.subplots(figsize=(12, 6))

    x = range(len(df))
    mb_per_sec = df["parser_bytes_per_sec"] / 1_000_000

    ax.plot(x, mb_per_sec, "o-", linewidth=2, color="steelblue")
    ax.fill_between(x, mb_per_sec, alpha=0.2, color="steelblue")

    ax.set_xlabel("Commit")
    ax.set_ylabel("Throughput (MB/s)")
    ax.set_title("Parser Throughput Over Commits")
    ax.set_xticks(list(x))
    ax.set_xticklabels(df["git_commit"], rotation=45, ha="right")
    ax.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, "parser_throughput.png"), dpi=150)
    plt.close()


def chart_regression_detection(df: pd.DataFrame, output_dir: str):
    """Chart 5: Highlight commits with >10% regression in build time."""
    if len(df) < 2:
        return

    fig, ax = plt.subplots(figsize=(12, 6))

    x = list(range(len(df)))
    build_times = df["build_total_ms"].tolist()

    # Calculate percentage change
    pct_changes = [0.0]
    for i in range(1, len(build_times)):
        if build_times[i - 1] > 0:
            pct_changes.append(
                (build_times[i] - build_times[i - 1]) / build_times[i - 1] * 100
            )
        else:
            pct_changes.append(0.0)

    colors = ["red" if pct > 10 else "green" if pct < -10 else "gray" for pct in pct_changes]

    ax.bar(x, pct_changes, color=colors)
    ax.axhline(y=10, color="red", linestyle="--", alpha=0.5, label="+10% threshold")
    ax.axhline(y=-10, color="green", linestyle="--", alpha=0.5, label="-10% threshold")
    ax.axhline(y=0, color="black", linewidth=0.5)

    ax.set_xlabel("Commit")
    ax.set_ylabel("Build Time Change (%)")
    ax.set_title("Build Time Regression Detection")
    ax.set_xticks(x)
    ax.set_xticklabels(df["git_commit"], rotation=45, ha="right")
    ax.legend()
    ax.grid(True, alpha=0.3, axis="y")

    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, "regression_detection.png"), dpi=150)
    plt.close()


def main():
    parser = argparse.ArgumentParser(description="Generate benchmark charts")
    parser.add_argument("results_dir", help="Directory containing JSON result files")
    parser.add_argument(
        "--output", "-o", default="output", help="Output directory for charts"
    )
    args = parser.parse_args()

    os.makedirs(args.output, exist_ok=True)

    print(f"Loading results from {args.results_dir}...")
    df = load_results(args.results_dir)
    print(f"Loaded {len(df)} result(s)")

    print("Generating charts...")
    chart_build_time(df, args.output)
    chart_phase_breakdown(df, args.output)
    chart_salsa_latency(df, args.output)
    chart_parser_throughput(df, args.output)
    chart_regression_detection(df, args.output)

    print(f"Charts saved to {args.output}/")


if __name__ == "__main__":
    main()
