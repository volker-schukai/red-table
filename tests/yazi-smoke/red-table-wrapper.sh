#!/usr/bin/env bash
set -euo pipefail

: "${RED_TABLE_SMOKE_BINARY:?}"
: "${RED_TABLE_SMOKE_MARKER:?}"
: "${RED_TABLE_SMOKE_STATUS:?}"

touch "$RED_TABLE_SMOKE_MARKER"
set +e
"$RED_TABLE_SMOKE_BINARY" --no-config --graphics-protocol halfblocks "$@"
smoke_status=$?
set -e
printf '%s\n' "$smoke_status" >"$RED_TABLE_SMOKE_STATUS"
exit "$smoke_status"
