#!/usr/bin/env bash
set -euo pipefail

VENV_DIR="${1:-.venv}"

if ! command -v uv &>/dev/null; then
    echo "Error: uv not found on PATH. Install uv from https://docs.astral.sh/uv/." >&2
    exit 1
fi

echo "Creating or updating virtual environment in $VENV_DIR …"
uv venv "$VENV_DIR"

echo "Installing dependencies with uv sync …"
uv sync --all-groups --python "$VENV_DIR/bin/python"

echo ""
echo "Setup complete. Activate the environment with:"
echo "  source $VENV_DIR/bin/activate"
