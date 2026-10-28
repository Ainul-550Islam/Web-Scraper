#!/bin/sh
# Prometheus does not interpolate arbitrary YAML credential values.
# Write the environment-supplied token as a private runtime-only file.
set -eu
: "${HARVEST_ADMIN_TOKEN:?set HARVEST_ADMIN_TOKEN before starting the stack}"
umask 077
printf '%s' "$HARVEST_ADMIN_TOKEN" > /tmp/harvest-admin-token
unset HARVEST_ADMIN_TOKEN
exec /bin/prometheus "$@"
