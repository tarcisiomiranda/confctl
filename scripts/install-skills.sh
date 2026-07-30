#!/bin/sh
# Thin wrapper: detect AI agents on this machine and install confctl skills.
# Prefer the Python installer (scripts/install_skills.py).
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
exec python3 "${ROOT}/scripts/install_skills.py" "$@"
