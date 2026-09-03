#!/usr/bin/env python3
"""Generate the web-analytics incremental-pipeline tutorial series (multi-page).

Renders `docs-site/docs/examples/web-analytics/<page>.md` from the matching
template in `examples/web_analytics/tutorial_pages/<page>.md`. Every template
is copied through verbatim except for two directive forms, which are
replaced by the directive line (kept, as provenance) followed by a fenced
block whose content is derived from real behaviour rather than hand-written:

  - `<!-- smelt-include: <path> -->` — embeds a model/function source file
    (relative to `examples/web_analytics/`), stripped of full-line SQL
    comments.
  - `<!-- smelt-generate: [@opts...] <argv...> -->` — runs the real `smelt`
    CLI against a fresh temp copy of a workspace and embeds its (rendered)
    output, so the embedded SQL/transcripts can never drift from what the
    compiler and CLI actually produce.

See `docs/plans/` / the generator spec for the full directive grammar. The
companion drift gate, `crates/smelt-cli/tests/tutorial_freshness.rs`,
re-derives every `smelt-generate` and `smelt-include` block independently
(spawning the compiled `smelt` binary itself, not calling into this script)
and byte-compares against the committed pages.

Usage:
    python3 generate_tutorial.py            # render and write all pages
    python3 generate_tutorial.py --check    # exit non-zero if any committed
                                             # page would change
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
TEMPLATES_DIR = SCRIPT_DIR / "tutorial_pages"
OUTPUT_DIR = REPO_ROOT / "docs-site" / "docs" / "examples" / "web-analytics"

# Names excluded when materializing a temp copy of a workspace directory for
# a `smelt-generate` block to run against. `*.duckdb` files are excluded by
# suffix, handled separately below.
EXCLUDE_NAMES = {
    "target",
    ".smelt",
    "data",
    "output",
    "generate_tutorial.py",
    "tutorial_pages",
    "tutorial_stages",
    "deployed_schema_fixture",
    "__pycache__",
}

# Comment-line prefixes that survive `strip_sql_comments` — structural
# markers the tutorial prose refers to, not restated source comments.
STRUCTURAL_PREFIXES = ("-- trigger:", "-- chunk", "-- (no statements", "-- Would run:")

INCLUDE_RE = re.compile(r"^(?P<indent>[ \t]*)<!-- smelt-include: (?P<path>.+?) -->\s*$")
GENERATE_RE = re.compile(r"^(?P<indent>[ \t]*)<!-- smelt-generate: (?P<rest>.+?) -->\s*$")


# ---------------------------------------------------------------------------
# smelt invocation
# ---------------------------------------------------------------------------


def _smelt_env() -> dict:
    env = os.environ.copy()
    env.setdefault("DUCKDB_LIB_DIR", os.path.expanduser("~/.local/lib/duckdb"))
    duckdb_lib = env["DUCKDB_LIB_DIR"]
    ld_path = env.get("LD_LIBRARY_PATH", "")
    if duckdb_lib not in ld_path.split(":"):
        env["LD_LIBRARY_PATH"] = duckdb_lib + (":" + ld_path if ld_path else "")
    return env


def _copy_workspace(src: Path, dest: Path) -> None:
    def ignore(_dir: str, names: list[str]) -> set[str]:
        return {n for n in names if n in EXCLUDE_NAMES or n.endswith(".duckdb")}

    shutil.copytree(src, dest, ignore=ignore)


class WorkspaceCache:
    """Materializes (and reuses) one temp copy per (cwd, fixture-schemas)
    pair within a single generation run."""

    def __init__(self, tmp_root: Path):
        self.tmp_root = tmp_root
        self._cache: dict[tuple[str, bool], Path] = {}
        self._counter = 0

    def get(self, cwd_rel: str, fixture_schemas: bool) -> Path:
        key = (cwd_rel, fixture_schemas)
        cached = self._cache.get(key)
        if cached is not None:
            return cached
        src = (SCRIPT_DIR / cwd_rel) if cwd_rel else SCRIPT_DIR
        self._counter += 1
        dest = self.tmp_root / f"ws_{self._counter}"
        _copy_workspace(src, dest)
        if fixture_schemas:
            # `.smelt/targets/<target>/schemas/` is the current on-disk
            # layout (`smelt-state::file_store::FileStore::schemas_dir`) —
            # not the flat `.smelt/schemas/` legacy pre-partitioning path,
            # which only migrates into the partitioned layout inside
            # `FileStore::lock()` (never called by `smelt diff`). Writing
            # straight into the partitioned path here is what `smelt diff`
            # (an unlocked reader) actually looks under; all tutorial
            # projects use the `dev` target.
            schemas_dir = dest / ".smelt" / "targets" / "dev" / "schemas"
            schemas_dir.mkdir(parents=True, exist_ok=True)
            fixture_src = src / "deployed_schema_fixture"
            if fixture_src.is_dir():
                for f in sorted(fixture_src.glob("*.json")):
                    shutil.copy(f, schemas_dir / f.name)
        self._cache[key] = dest
        return dest


def run_smelt(
    argv: list[str],
    *,
    cwd_rel: str,
    expect_exit: int,
    fixture_schemas: bool,
    is_rebuild: bool,
    workspaces: WorkspaceCache,
) -> str:
    """Run the smelt CLI (via `cargo run`, so it always matches the current
    source tree) against a fresh temp copy of the resolved workspace and
    return its captured output per the exit-code rule."""
    workspace = workspaces.get(cwd_rel, fixture_schemas)

    args = list(argv)
    if is_rebuild:
        target_dir = workspace / "target"
        target_dir.mkdir(parents=True, exist_ok=True)
        args = [*args, "--database", str(target_dir / "db.duckdb")]

    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "smelt-cli",
        "--bin",
        "smelt",
        "--manifest-path",
        str(REPO_ROOT / "Cargo.toml"),
        "--",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=workspace,
        env=_smelt_env(),
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != expect_exit:
        raise RuntimeError(
            f"smelt {' '.join(args)} (cwd={workspace}) exited "
            f"{result.returncode}, expected {expect_exit}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    if expect_exit != 0:
        return result.stdout + result.stderr
    return result.stdout


# ---------------------------------------------------------------------------
# Shared render-mode transforms (mirrored exactly by
# crates/smelt-cli/tests/tutorial_freshness.rs — keep in lockstep).
# ---------------------------------------------------------------------------


def canonicalize(text: str) -> str:
    """Drop comment-placeholder lines that are only `--` once trimmed (a
    cosmetic emitter artifact)."""
    lines = [line for line in text.split("\n") if line.strip() != "--"]
    return "\n".join(lines).strip("\n")


def _collapse_and_trim(lines: list[str]) -> str:
    """Collapse runs of >1 consecutive blank lines to one, then trim leading
    and trailing blank lines."""
    collapsed: list[str] = []
    blank_run = 0
    for line in lines:
        if line.strip() == "":
            blank_run += 1
            if blank_run <= 1:
                collapsed.append(line)
        else:
            blank_run = 0
            collapsed.append(line)
    while collapsed and collapsed[0].strip() == "":
        collapsed.pop(0)
    while collapsed and collapsed[-1].strip() == "":
        collapsed.pop()
    return "\n".join(collapsed)


def strip_sql_comments(text: str) -> str:
    """Drop every line whose lstrip() starts with `--` unless it starts with
    a structural prefix, then collapse/trim blank runs."""
    kept = []
    for line in text.split("\n"):
        stripped = line.lstrip()
        if stripped.startswith("--") and not any(
            stripped.startswith(p) for p in STRUCTURAL_PREFIXES
        ):
            continue
        kept.append(line)
    return _collapse_and_trim(kept)


def render_cells_sql(cells: list[dict]) -> str:
    """Build a compact SQL block from an `explain --show-sql --json` cells
    array: one section per cell (labelled by its trigger), statements
    sharing a `transactional_group` wrapped in `BEGIN`/`COMMIT`."""
    blocks = []
    for cell in cells:
        trigger = cell["trigger"]
        statements = cell.get("statements") or []
        if not statements:
            reason = cell.get("no_statements_reason")
            if reason:
                blocks.append(f"-- trigger: {trigger}\n-- (no statements: {reason})")
            continue

        lines = [f"-- trigger: {trigger}"]
        groups: list[tuple[int, list[str]]] = []
        for stmt in statements:
            gid = stmt["transactional_group"]
            if groups and groups[-1][0] == gid:
                groups[-1][1].append(stmt["sql"])
            else:
                groups.append((gid, [stmt["sql"]]))

        for _gid, sqls in groups:
            if len(sqls) > 1:
                lines.append("BEGIN")
                for sql in sqls:
                    for sql_line in sql.split("\n"):
                        lines.append("  " + sql_line if sql_line else "")
                lines.append("COMMIT")
            else:
                lines.append(sqls[0])
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks)


def _is_skeleton_structural(line: str) -> bool:
    stripped = line.lstrip()
    if stripped == "":
        return True
    if any(stripped.startswith(p) for p in STRUCTURAL_PREFIXES):
        return True
    if line.strip() in ("BEGIN", "COMMIT"):
        return True
    if stripped.startswith("DELETE FROM "):
        return True
    if stripped.startswith("INSERT INTO "):
        return True
    if stripped.startswith(") AS _smelt_output_clamp") or stripped.startswith(
        ") _smelt_typed"
    ):
        return True
    return False


def skeleton(text: str) -> str:
    """Reduce each SQL statement in a rendered block to its frame, replacing
    dropped run(s) of lines with a single placeholder comment."""
    lines = text.split("\n")
    out: list[str] = []
    i, n = 0, len(lines)
    while i < n:
        if _is_skeleton_structural(lines[i]):
            out.append(lines[i])
            i += 1
        else:
            indent = lines[i][: len(lines[i]) - len(lines[i].lstrip())]
            out.append(f"{indent}-- … model SELECT body (see the full SQL below) …")
            i += 1
            while i < n and not _is_skeleton_structural(lines[i]):
                i += 1
    return _collapse_and_trim(out)


def render_text(raw: str) -> str:
    lines = [line.rstrip() for line in raw.split("\n")]
    while lines and lines[0].strip() == "":
        lines.pop(0)
    while lines and lines[-1].strip() == "":
        lines.pop()
    return "\n".join(lines)


def render_dirty_set(raw: str) -> str:
    lines = raw.split("\n")
    for idx, line in enumerate(lines):
        if line.lstrip().startswith("-- Would run:"):
            lines = lines[:idx]
            break
    return render_text("\n".join(lines))


def render_sql(raw: str) -> str:
    return strip_sql_comments(canonicalize(raw))


def render_cells(raw_stdout: str, argv: list[str], marker: str) -> str:
    if "--json" not in argv:
        raise ValueError(f"cells render mode requires --json in argv: {marker}")
    data = json.loads(raw_stdout)
    sql = render_cells_sql(data["cells"])
    return strip_sql_comments(canonicalize(sql))


# ---------------------------------------------------------------------------
# smelt-include transform
# ---------------------------------------------------------------------------


def render_include(path_rel: str) -> str:
    file_path = SCRIPT_DIR / path_rel
    text = file_path.read_text()
    lines = text.split("\n")

    frontmatter: list[str] = []
    body_start = 0
    if lines and lines[0].strip() == "---":
        for idx in range(1, len(lines)):
            if lines[idx].strip() == "---":
                frontmatter = lines[: idx + 1]
                body_start = idx + 1
                break

    body_lines = lines[body_start:]
    kept = [line for line in body_lines if not line.lstrip().startswith("--")]
    body = _collapse_and_trim(kept)

    if frontmatter:
        if body:
            return "\n".join(frontmatter) + "\n" + body
        return "\n".join(frontmatter)
    return body


# ---------------------------------------------------------------------------
# smelt-generate directive parsing + dispatch
# ---------------------------------------------------------------------------

_DEFAULT_RENDER_BY_SUBCOMMAND = {
    "explain": "cells",
    "rebuild": "sql",
    "run": "text",
    "diff": "text",
}


def parse_generate_directive(rest: str) -> tuple[dict, list[str]]:
    tokens = rest.split()
    opts: dict[str, object] = {}
    idx = 0
    while idx < len(tokens) and tokens[idx].startswith("@"):
        token = tokens[idx][1:]
        if "=" in token:
            key, value = token.split("=", 1)
            opts[key] = value
        else:
            opts[token] = True
        idx += 1
    argv = tokens[idx:]
    if not argv:
        raise ValueError(f"smelt-generate directive has no argv: {rest}")
    return opts, argv


def render_generate(rest: str, workspaces: WorkspaceCache) -> tuple[str, str]:
    """Returns (fenced block content, fence language)."""
    opts, argv = parse_generate_directive(rest)
    marker = f"<!-- smelt-generate: {rest} -->"

    cwd_rel = str(opts.get("cwd", ""))
    expect_exit = int(opts.get("expect-exit", 0))
    fixture_schemas = bool(opts.get("fixture-schemas", False))
    subcommand = argv[0]
    is_rebuild = subcommand == "rebuild"

    default_render = _DEFAULT_RENDER_BY_SUBCOMMAND.get(subcommand)
    mode = str(opts.get("render", default_render or ""))
    if not mode:
        raise ValueError(
            f"cannot infer a default render mode for argv[0]={subcommand!r}; "
            f"specify @render=... in: {marker}"
        )

    raw = run_smelt(
        argv,
        cwd_rel=cwd_rel,
        expect_exit=expect_exit,
        fixture_schemas=fixture_schemas,
        is_rebuild=is_rebuild,
        workspaces=workspaces,
    )

    if mode == "cells":
        return render_cells(raw, argv, marker), "sql"
    if mode == "sql":
        return render_sql(raw), "sql"
    if mode == "skeleton":
        if is_rebuild:
            base = render_sql(raw)
        else:
            base = render_cells(raw, argv, marker)
        return skeleton(base), "sql"
    if mode == "text":
        return render_text(raw), "text"
    if mode == "dirty-set":
        return render_dirty_set(raw), "text"
    raise ValueError(f"unknown render mode {mode!r} in: {marker}")


# ---------------------------------------------------------------------------
# Template processing
# ---------------------------------------------------------------------------


def _build_replacement(indent: str, directive_line: str, fence_lang: str, content: str) -> list[str]:
    out = [directive_line, f"{indent}```{fence_lang}"]
    for line in content.split("\n"):
        out.append(f"{indent}{line}" if line != "" else indent)
    out.append(f"{indent}```")
    out.append("")  # exactly one blank line after the fence
    return out


def process_template(text: str, workspaces: WorkspaceCache) -> str:
    raw_lines = text.splitlines()
    out: list[str] = []
    i, n = 0, len(raw_lines)
    while i < n:
        line = raw_lines[i]
        m_inc = INCLUDE_RE.match(line)
        m_gen = GENERATE_RE.match(line)
        if m_inc:
            indent = m_inc.group("indent")
            content = render_include(m_inc.group("path").strip())
            out.extend(_build_replacement(indent, line, "sql", content))
            i += 1
            if i < n and raw_lines[i].strip() == "":
                i += 1
            continue
        if m_gen:
            indent = m_gen.group("indent")
            content, fence_lang = render_generate(m_gen.group("rest").strip(), workspaces)
            out.extend(_build_replacement(indent, line, fence_lang, content))
            i += 1
            if i < n and raw_lines[i].strip() == "":
                i += 1
            continue
        out.append(line)
        i += 1
    return "\n".join(out) + "\n"


def render_page(template_path: Path, workspaces: WorkspaceCache) -> str:
    text = template_path.read_text()
    body = process_template(text, workspaces)
    header = (
        f"<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/"
        f"{template_path.name} and run python3 examples/web_analytics/generate_tutorial.py -->\n\n"
    )
    return header + body


def generate_all() -> dict[str, str]:
    pages: dict[str, str] = {}
    with tempfile.TemporaryDirectory(prefix="smelt-tutorial-gen-") as tmp:
        workspaces = WorkspaceCache(Path(tmp))
        for template_path in sorted(TEMPLATES_DIR.glob("*.md")):
            pages[template_path.stem] = render_page(template_path, workspaces)
    return pages


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if any committed page differs from a fresh render",
    )
    args = parser.parse_args()

    pages = generate_all()

    if args.check:
        stale: list[Path] = []
        for name, content in pages.items():
            out_path = OUTPUT_DIR / f"{name}.md"
            existing = out_path.read_text() if out_path.exists() else None
            if existing != content:
                stale.append(out_path)
        if stale:
            for p in stale:
                print(
                    f"{p} is stale relative to a fresh render; "
                    "run `python3 generate_tutorial.py` to regenerate it.",
                    file=sys.stderr,
                )
            return 1
        print("All web-analytics tutorial pages are fresh.")
        return 0

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, content in pages.items():
        out_path = OUTPUT_DIR / f"{name}.md"
        out_path.write_text(content)
        print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
