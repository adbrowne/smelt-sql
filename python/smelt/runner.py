"""
Subprocess entry point for executing smelt Python models.

Invoked by the Rust CLI as:
    python -m smelt.runner /path/to/model.py '{"models": [...]}'

Outputs JSON to stdout:
    [{"name": "model_name", "sql": "SELECT ...", "queries": [...]}]
"""
import importlib.util
import json
import sys


def main():
    if len(sys.argv) != 3:
        print(
            "Usage: python -m smelt.runner <model_file> <project_json>",
            file=sys.stderr,
        )
        sys.exit(1)

    file_path = sys.argv[1]
    project_json = sys.argv[2]

    project_data = json.loads(project_json)

    from smelt.core import ProjectContext, _registered_models

    # Clear any previously registered models
    _registered_models.clear()

    project = ProjectContext(project_data.get("models", []))

    # Load the model file
    spec = importlib.util.spec_from_file_location("model", file_path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    results = []
    for name, func in _registered_models.items():
        # Reset query log so each model only records its own queries
        project._queries = []
        sql = func(project)
        if not isinstance(sql, str):
            print(
                f"Error: Model '{name}' must return a string, got {type(sql).__name__}",
                file=sys.stderr,
            )
            sys.exit(1)
        results.append({
            "name": name,
            "sql": sql,
            "queries": list(project._queries),
        })

    json.dump(results, sys.stdout)


if __name__ == "__main__":
    main()
