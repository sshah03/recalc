#!/bin/sh
# Every place the version is written agrees with $1.
want=$1
py=$(sed -n 's/^version = "\(.*\)"/\1/p' pyproject.toml)
pkg=$(sed -n 's/^__version__ = "\(.*\)"/\1/p' src/acme/__init__.py)
echo "pyproject.toml $py, src/acme/__init__.py $pkg"
[ "$py" = "$want" ] && [ "$pkg" = "$want" ]
