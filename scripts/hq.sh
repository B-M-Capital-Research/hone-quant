#!/usr/bin/env bash
# Runs a hone-quant command on the host with the service's environment and user, e.g.
#
#   sudo /opt/hone-quant/current/scripts/hq.sh user add alice --role viewer
#   sudo /opt/hone-quant/current/scripts/hq.sh fmp-check
#   sudo /opt/hone-quant/current/scripts/hq.sh sync --full
#
# Interactive when run from a terminal (password prompts work); otherwise stdin is piped through.
set -euo pipefail
[[ $EUID -eq 0 ]] || { echo "error: run as root (sudo)" >&2; exit 1; }
mode=--pipe
[[ -t 0 && -t 1 ]] && mode=--pty
exec systemd-run --quiet --wait --collect "$mode" --uid=hone-quant --gid=hone-quant \
  --property=EnvironmentFile=/etc/hone-quant/runtime.env \
  --setenv=HONE_QUANT_STATE_DIR=/var/lib/hone-quant \
  --setenv=HONE_QUANT_LOG_FORMAT=text \
  --working-directory=/var/lib/hone-quant \
  /opt/hone-quant/current/hone-quant "$@"
