#!/bin/sh
# Every up migration has a down migration.
missing=0
for up in migrations/*.up.sql; do
  down="${up%.up.sql}.down.sql"
  if [ ! -f "$down" ]; then echo "no down migration for $(basename "$up")"; missing=$((missing+1)); fi
done
[ $missing -eq 0 ] && echo "every migration can be rolled back"
[ $missing -eq 0 ]
