#!/usr/bin/env bash
# Complete repository gate. Fails on the first failing step.
#
#   scripts/repo-gate.sh           # workspace, explorer, wasm, catalog, cog gate --fast
#   scripts/repo-gate.sh --cross   # same, and the ARM cross-build inside scripts/gate.sh
#
# Wasm uses the rust:1.95-bookworm container and bash -c. Host cargo uses
# .cargo-target/host. The container uses .cargo-target/wasm. Those directories
# are not the same, so a later manual wasm build can run beside a host build.
# This script runs them one after another and stops at the first failure.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="$ROOT/.cargo-target/host"

CROSS=0
for a in "$@"; do
  case "$a" in
    --cross) CROSS=1 ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "repo-gate: unknown flag $a" >&2; exit 2 ;;
  esac
done

step() { echo; echo "=== repo-gate: $1"; }
fail() { echo; echo "REPO GATE FAILED: $1"; exit 1; }

step "decision ids"
python3 scripts/check_decision_ids.py || fail "decision ids"

step "guide bundles"
if grep -q "weftos-cog-market" scripts/bundle-cog-guides.py; then
  fail "guide bundler still names weftos-cog-market"
fi
fresh="$(mktemp -d)"
# No --with-images. The header photo has no recorded source or licence.
python3 scripts/bundle-cog-guides.py src/cogs "$fresh" || fail "bundle guides"
diff -rq "$fresh" crates/cog-market/catalog/guides || fail "guide bundles drifted"
for f in crates/cog-market/catalog/guides/*.json; do
  id="$(basename "$f" .json)"
  grep -F -q "catalog/guides/${id}.json" apps/sensor-explorer/src/guides.ts || fail "explorer guide missing ${id}"
  grep -F -q "catalog/guides/${id}.json" crates/cog-market/src/guides.rs || fail "GUIDES missing ${id}"
done
rm -rf "$fresh"

step "cargo metadata has no clawft crate"
meta="$(mktemp)"
cargo metadata --locked --format-version 1 >"$meta" || fail "cargo metadata"
python3 - "$meta" << 'PY' || fail "clawft package in the locked graph"
import json, sys
doc = json.load(open(sys.argv[1]))
bad = [p["name"] for p in doc["packages"] if p["name"].startswith("clawft")]
print(f"packages {len(doc['packages'])} clawft {len(bad)}")
sys.exit(1 if bad else 0)
PY
rm -f "$meta"

step "workspace clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings || fail "workspace clippy"

step "workspace tests"
cargo test --workspace --locked || fail "workspace tests"

step "native manager, host, repo, and guide-check"
cargo build --locked \
  -p cog-manager --bins \
  -p cog-host --bin weft-cog-host \
  -p cog-repo --bin weft-cog-repo \
  -p sensor-guide --bin guide-check \
  || fail "native bins"
for b in weave-manager weft-cog-manager weft-cog-host weft-cog-repo guide-check; do
  [ -x "$CARGO_TARGET_DIR/debug/$b" ] || fail "missing $b"
done

step "sensor explorer typecheck, lint, tests, worker dry-run, local D1"
(
  cd apps/sensor-explorer
  npm run typecheck || exit 1
  npm run lint || exit 1
  npm run test || exit 1
  node scripts/publish-catalog-artifact.mjs || exit 1
  cmp -s public/catalog.json ../../crates/cog-market/catalog/catalog.json || exit 1
  node -e 'const fs=require("fs"); const {createHash}=require("crypto"); const b=fs.readFileSync("public/catalog.json"); const r=JSON.parse(fs.readFileSync("public/catalog-release.json","utf8")); const d=createHash("sha256").update(b).digest("hex"); if (r.digest !== d || r.version !== r.generated || r.source_path !== "crates/cog-market/catalog/catalog.json") process.exit(1)' || exit 1
  npx --no-install wrangler deploy --dry-run --outdir .wrangler/gate-worker --minify || exit 1
  npx --no-install wrangler d1 migrations apply sensor-explorer --local --persist-to .wrangler/gate-d1 || exit 1
  second="$(npx --no-install wrangler d1 migrations apply sensor-explorer --local --persist-to .wrangler/gate-d1)" || exit 1
  printf '%s\n' "$second"
  printf '%s\n' "$second" | grep -q "No migrations to apply" || exit 1
) || fail "sensor explorer"

step "catalog seed"
seed="$(mktemp)"
node apps/sensor-explorer/scripts/gen-seed.mjs crates/cog-market/catalog/catalog.json "$seed" || fail "gen-seed"
[ -s "$seed" ] || fail "empty catalog seed"
grep -q "INSERT OR REPLACE INTO catalog_release" "$seed" || fail "catalog release row missing"
digest="$(node -e 'const {createHash}=require("crypto"); const fs=require("fs"); process.stdout.write(createHash("sha256").update(fs.readFileSync("crates/cog-market/catalog/catalog.json")).digest("hex"))')"
grep -q "$digest" "$seed" || fail "seed digest mismatch"
grep -q "$digest" apps/sensor-explorer/public/catalog-release.json || fail "artifact digest mismatch"
export CATALOG_SHA256="$digest"
# The workspace tests ran before this digest existed in the environment.
# Re-run the one comparison so the Rust embed and the node file hash are the same bytes.
cargo test --locked -p cog-market --lib bundled_sha256_is_the_raw_catalog_bytes || fail "catalog digest rust"
(
  cd apps/sensor-explorer
  npx --no-install wrangler d1 execute sensor-explorer --local --persist-to .wrangler/gate-d1 --file "$seed" || exit 1
  out="$(npx --no-install wrangler d1 execute sensor-explorer --local --persist-to .wrangler/gate-d1 --command "SELECT digest FROM catalog_release WHERE id=1")" || exit 1
  printf '%s\n' "$out"
  printf '%s\n' "$out" | grep -q "$CATALOG_SHA256" || exit 1
) || fail "catalog release"
rm -f "$seed"

step "private registry round trip"
round="$(mktemp -d)"
cleanup_round() { rm -rf "$round"; }
trap cleanup_round EXIT
key="$round/key.pem"
repo="$round/repo"
printf '\177ELF' > "$round/probe.bin"
dd if=/dev/zero bs=60 count=1 >> "$round/probe.bin" 2>/dev/null
"$CARGO_TARGET_DIR/debug/weft-cog-repo" init "$repo" --name gateprobe || fail "registry init"
# keygen prints the public key. Discard it. The pem stays in the temp dir until the trap.
"$CARGO_TARGET_DIR/debug/weft-cog-repo" keygen --out "$key" --repo "$repo" >/dev/null || fail "registry keygen"
"$CARGO_TARGET_DIR/debug/weft-cog-repo" add "$repo" --binary "$round/probe.bin" --id probe --arch x86_64 --name Probe --version 0.0.1 || fail "registry add"
"$CARGO_TARGET_DIR/debug/weft-cog-repo" sign "$repo" --key "$key" >/dev/null || fail "registry sign"
"$CARGO_TARGET_DIR/debug/weft-cog-repo" verify "$repo" >/dev/null || fail "registry verify"
[ -s "$repo/repo/registry.json" ] || fail "missing registry.json"
# Split so a source scan does not see a contiguous private-key banner.
priv_banner="BEGIN ""PRIVATE"
if grep -q "$priv_banner" "$repo/repo/registry.json"; then
  fail "registry.json contains a private key"
fi
cleanup_round
trap - EXIT

step "wasm libraries in rust:1.95-bookworm"
wasm_log="$(mktemp)"
docker run --rm \
  -v "$ROOT":/work -w /work \
  -v weftos-cogs-a1-cargo-registry:/usr/local/cargo/registry \
  -v weftos-cogs-a1-cargo-git:/usr/local/cargo/git \
  -e CARGO_TARGET_DIR=/work/.cargo-target/wasm \
  rust:1.95-bookworm \
  bash -c 'rustup target add wasm32-unknown-unknown && cargo build --locked --target wasm32-unknown-unknown -p cog-manager -p guide-view -p cog-ecg-scope -p cog-sound-scope -p cog-tof-scope --lib --profile release-wasm' \
  >"$wasm_log" 2>&1 || { tail -40 "$wasm_log"; rm -f "$wasm_log"; fail "wasm build"; }
if grep -E '`(cog-manager|guide-view|cog-ecg-scope|cog-sound-scope|cog-tof-scope|sensor-guide)` \(lib\) generated' "$wasm_log"; then
  rm -f "$wasm_log"
  fail "wasm lib generated warnings"
fi
rm -f "$wasm_log"
wasm_dir="$ROOT/.cargo-target/wasm/wasm32-unknown-unknown/release-wasm"
for a in cog_manager.wasm guide_view.wasm cog_ecg_scope.wasm cog_sound_scope.wasm cog_tof_scope.wasm; do
  [ -s "$wasm_dir/$a" ] || fail "missing $a"
  echo "-- $a"
done

step "cog gate"
export GUIDE_CHECK="$CARGO_TARGET_DIR/debug/guide-check"
if [ "$CROSS" -eq 1 ]; then
  scripts/gate.sh || fail "cog gate"
else
  scripts/gate.sh --fast || fail "cog gate"
fi

echo
echo "REPO GATE PASSED"
