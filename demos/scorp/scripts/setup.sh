#!/bin/sh
# Install pypdf, which reads and fills the IRS's PDF, into .venv here. Run once.
set -e
if [ ! -x .venv/bin/python ]; then python3 -m venv .venv; fi
.venv/bin/python -m pip install -q --disable-pip-version-check pypdf
touch .venv/ready
echo "pypdf installed in .venv"
