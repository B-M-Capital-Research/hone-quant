#!/usr/bin/env bash
# Copies honeclaw's industry ontology and its live edit log into hone-quant's state directory,
# for hone-quant installed on honeclaw's own host (hone-claw.com/quant). Run as root by
# hone-quant-ontology.timer: honeclaw's files stay private to honeclaw, and hone-quant reads
# only these copies (HONE_QUANT_ONTOLOGY / HONE_QUANT_ONTOLOGY_EDITS).
#
#   scripts/copy-ontology.sh [target directory]
#
# A missing edit log (honeclaw has none until an administrator edits the map) is not an error.
# Copies are replaced atomically, so hone-quant never reads a half-written file.
set -euo pipefail

target="${1:-/var/lib/hone-quant/ontology}"
ontology="${HONECLAW_ONTOLOGY:-/srv/honeclaw/skills/industry-map/references/industry-map.json}"
edits="${HONECLAW_ONTOLOGY_EDITS:-/srv/honeclaw/data/industry_map/edits.json}"
owner="${HONE_QUANT_USER:-hone-quant}"

[[ -f "$ontology" ]] || { echo "error: $ontology is missing" >&2; exit 1; }
install -d -o "$owner" -g "$owner" -m 0750 "$target"

copy() {
  local source="$1" name
  name="$(basename "$source")"
  install -o "$owner" -g "$owner" -m 0640 "$source" "$target/.$name.tmp"
  mv -f "$target/.$name.tmp" "$target/$name"
}

copy "$ontology"
if [[ -f "$edits" ]]; then
  copy "$edits"
fi
