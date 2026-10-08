#!/usr/bin/env bash
# Install a cross-built cog onto a Seed the way the Cognitum agent stores its own apps:
# /var/lib/cognitum/apps/<id>/{cog-<id>-arm, manifest.json}. The agent lists it immediately.
#
#   scripts/cross-build.sh <cog-id>
#   scripts/seed-sideload.sh <cog-id> [ssh-target]      # default genesis@169.254.42.1 (USB)
#
# Needs SSH key auth to the Seed (sudo is NOPASSWD for genesis). The manifest is
# scripts/cog_manifest.py, from src/cogs/<id>/cog.toml and the arm binary.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
id="${1:?usage: seed-sideload.sh <cog-id> [ssh-target]}"
target="${2:-${SEED_SSH:-genesis@169.254.42.1}}"
case "$id" in *[!a-z0-9-]*|'') echo "bad cog id: $id" >&2; exit 2 ;; esac
dist="$ROOT/.cargo-target/dist/$id"
bin="$dist/cog-$id-arm"
[ -f "$bin" ] || { echo "no $bin; run scripts/cross-build.sh $id first" >&2; exit 1; }

python3 "$ROOT/scripts/cog_manifest.py" "$ROOT/src/cogs/$id/cog.toml" "$bin" "$dist/manifest.json"

scp -q "$bin" "$dist/manifest.json" "$target:/tmp/"
ssh -o BatchMode=yes "$target" "set -e; D=/var/lib/cognitum/apps/$id
  sudo install -d -m 755 \$D
  sudo install -m 755 /tmp/cog-$id-arm \$D/
  sudo install -m 644 /tmp/manifest.json \$D/
  rm -f /tmp/cog-$id-arm /tmp/manifest.json
  sudo sha256sum \$D/cog-$id-arm"
echo "installed $id on $target"
