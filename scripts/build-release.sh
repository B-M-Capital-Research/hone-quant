#!/usr/bin/env bash
# Builds a self-contained release tarball. The web UI is compiled first and embedded in the
# binary, so a release is one executable plus its helper scripts and deployment templates.
#
#   scripts/build-release.sh   →  dist/hone-quant-<version>-<rev12>-linux-<arch>.tar.gz (+ .sha256)
#
# Build on the oldest glibc you deploy to (e.g. Ubuntu 22.04 for Debian 12 / Ubuntu 22.04+ hosts),
# or use the release workflow (.github/workflows/release.yml), which does exactly that.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

revision="$(git rev-parse HEAD)"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  if [[ "${ALLOW_DIRTY:-0}" != "1" ]]; then
    echo "error: the working tree has uncommitted changes; commit them or set ALLOW_DIRTY=1" >&2
    exit 1
  fi
  revision="${revision}-dirty"
fi
version="$(sed -n '/^version = "/{s/^version = "\(.*\)"/\1/p;q;}' Cargo.toml)"
name="hone-quant-${version}-${revision:0:12}-linux-$(uname -m)"
[[ "$revision" == *-dirty ]] && name="${name}-dirty"

echo "==> web UI"
(cd web && bun install --frozen-lockfile && bun run typecheck && bun run build)

echo "==> server ($revision)"
HONE_QUANT_REVISION="$revision" cargo build --release --locked -p quant-server

echo "==> package"
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
dest="$stage/$name"
mkdir -p "$dest/scripts" "$dest/deploy"
install -m 0755 target/release/hone-quant "$dest/hone-quant"
install -m 0755 scripts/backup.sh scripts/deploy.sh scripts/install-host.sh scripts/hq.sh "$dest/scripts/"
cp -R deploy/systemd deploy/postgres deploy/caddy deploy/runtime.env.example "$dest/deploy/"
cat > "$dest/RELEASE" <<META
name=$name
version=$version
revision=$revision
built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
META
(cd "$dest" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)

mkdir -p dist
tar -C "$stage" -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
"$dest/hone-quant" --version
echo "dist/$name.tar.gz"
