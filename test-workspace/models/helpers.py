"""Utility functions for Python models. No @model decorator = not a model."""


def format_union(model_names):
    """Helper to build UNION ALL queries from a list of model names."""
    refs = [f"SELECT * FROM smelt.ref('{name}')" for name in model_names]
    return " UNION ALL ".join(refs)
