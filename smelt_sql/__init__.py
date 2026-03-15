"""smelt-sql: Modern data transformation framework.

Provides helper functions to locate bundled smelt binaries
installed via `pip install smelt-sql`.
"""

import os
import sys
import sysconfig


def _find_binary(name: str) -> str:
    """Locate a bundled binary by name.

    Search order:
    1. sysconfig scripts directory (standard pip install location)
    2. Directory containing the Python executable (virtualenv fallback)
    """
    scripts_dir = sysconfig.get_path("scripts")
    if scripts_dir:
        path = os.path.join(scripts_dir, name)
        if os.path.isfile(path):
            return path

    bin_dir = os.path.dirname(sys.executable)
    path = os.path.join(bin_dir, name)
    if os.path.isfile(path):
        return path

    raise FileNotFoundError(
        f"Could not find '{name}' binary. "
        f"Searched: {scripts_dir}, {bin_dir}. "
        f"Is smelt-sql installed correctly?"
    )


def lsp_binary_path() -> str:
    """Return the absolute path to the bundled smelt-lsp binary."""
    return _find_binary("smelt-lsp")


def cli_binary_path() -> str:
    """Return the absolute path to the bundled smelt CLI binary."""
    return _find_binary("smelt")
