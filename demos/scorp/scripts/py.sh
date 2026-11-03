#!/bin/sh
# Run Python from .venv once Setup has made it, else the system's python3.
if [ -x .venv/bin/python ]; then exec .venv/bin/python "$@"; fi
exec python3 "$@"
