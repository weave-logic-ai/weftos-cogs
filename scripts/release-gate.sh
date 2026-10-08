#!/usr/bin/env bash
# Controlled-host release check for COG-008. Does not generate a key, tag, or deploy.
#
#   scripts/release-gate.sh --key /secure/weavelogic-cog-release.pem
#
# The key file must already exist outside this repository. sign refuses a key
# whose public half is not the compiled WEAVELOGIC_PUBKEY_HEX.
#
#   scripts/cross-build.sh
#   weft-cog-repo sign --from .cargo-target/dist --out release/repo --key <pem>
#   weft-cog-repo verify release/repo
#   weft-cog-repo check-matrix --from .cargo-target/dist --cogs src/cogs release/repo
#
# sign still returns success when at least one cog is signable. check-matrix
# fails unless every cog under src/cogs has arm and arm64 in registry.json,
# and x86_64 when that binary is in the dist directory.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
key=""
from="$ROOT/.cargo-target/dist"
out="$ROOT/release/repo"
while [ $# -gt 0 ]; do
  case "$1" in
    --key) key="${2:?release-gate: --key needs a path}"; shift 2 ;;
    --from) from="${2:?release-gate: --from needs a directory}"; shift 2 ;;
    --out) out="${2:?release-gate: --out needs a directory}"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "release-gate: unknown argument $1" >&2; exit 2 ;;
  esac
done
if [ -z "$key" ]; then
  echo "release-gate: pass --key <pem>. This script does not generate a key." >&2
  exit 2
fi
if [ ! -f "$key" ]; then
  echo "release-gate: key file not found" >&2
  exit 1
fi
key_dir="$(cd "$(dirname "$key")" && pwd)"
key_abs="$key_dir/$(basename "$key")"
case "$key_abs" in
  "$ROOT"|"$ROOT"/*)
    echo "release-gate: the key must stay outside the repository" >&2
    exit 2
    ;;
esac

export CARGO_TARGET_DIR="$ROOT/.cargo-target/host"
bin="$CARGO_TARGET_DIR/debug/weft-cog-repo"
if [ ! -x "$bin" ]; then
  cargo build --locked -p cog-repo --bin weft-cog-repo
fi

scripts/cross-build.sh
"$bin" sign --from "$from" --out "$out" --key "$key"
"$bin" verify "$out"
"$bin" check-matrix --from "$from" --cogs "$ROOT/src/cogs" "$out"
echo "release-gate: matrix matches registry.json"
