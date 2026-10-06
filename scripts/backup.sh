#!/usr/bin/env bash
# Dumps the hone-quant schema (custom format) and keeps the newest 30 dumps.
#
#   scripts/backup.sh /var/backups/hone-quant
#
# Reads the same connection settings as the server (HONE_QUANT_DATABASE_URL, HONE_QUANT_PG_*,
# DATABASE_URL, HONE_POSTGRES_*). Restore with:
#   pg_restore --clean --if-exists --no-owner -d "$HONE_QUANT_DATABASE_URL" <dump>
set -euo pipefail

dir="${1:-/var/backups/hone-quant}"
keep="${HONE_QUANT_BACKUP_KEEP:-30}"
schema="${HONE_QUANT_DB_SCHEMA:-hone_quant}"

url="${HONE_QUANT_DATABASE_URL:-}"
if [[ -z "$url" && -n "${HONE_QUANT_PG_HOST:-}" ]]; then
  export PGHOST="$HONE_QUANT_PG_HOST" PGPORT="${HONE_QUANT_PG_PORT:-5432}" PGUSER="$HONE_QUANT_PG_USER" \
    PGPASSWORD="${HONE_QUANT_PG_PASSWORD:-}" PGDATABASE="$HONE_QUANT_PG_DATABASE"
elif [[ -z "$url" && -n "${DATABASE_URL:-}" ]]; then
  url="$DATABASE_URL"
elif [[ -z "$url" && -n "${HONE_POSTGRES_HOST:-}" ]]; then
  export PGHOST="$HONE_POSTGRES_HOST" PGPORT="${HONE_POSTGRES_PORT:-5432}" PGUSER="$HONE_POSTGRES_USER" \
    PGPASSWORD="${HONE_POSTGRES_PASSWORD:-}" PGDATABASE="$HONE_POSTGRES_DATABASE"
fi

mkdir -p "$dir"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
target="$dir/${schema}-${stamp}.dump"
tmp="$target.partial"
if [[ -n "$url" ]]; then
  pg_dump --format=custom --schema="$schema" --no-owner --file="$tmp" "$url"
else
  pg_dump --format=custom --schema="$schema" --no-owner --file="$tmp"
fi
mv "$tmp" "$target"
echo "wrote $target ($(du -h "$target" | cut -f1))"

# Retention: newest $keep dumps.
mapfile -t old < <(ls -1t "$dir"/"${schema}"-*.dump 2>/dev/null | tail -n +"$((keep + 1))")
for file in "${old[@]}"; do
  rm -f -- "$file"
done
