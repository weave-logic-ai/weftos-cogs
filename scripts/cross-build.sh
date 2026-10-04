#!/usr/bin/env bash
# Container cross-compile of cogs for armv7-unknown-linux-gnueabihf (pi-zero-2w) and
# aarch64-unknown-linux-gnu (v0-appliance), then strip and report sizes.
#
#   scripts/cross-build.sh              # every cog under src/cogs/
#   scripts/cross-build.sh <cog-id>...  # named cogs
#
# Fails if any stripped binary is > 5 MB, warns if > 1 MB. Stripped binaries land in
# .cargo-target/dist/<cog-id>/cog-<cog-id>-{arm,arm64} (upstream's artifact names).
# Set CROSS_PLATFORM=linux/amd64 to run the image under emulation with upstream's exact
# apt cross packages; the default is the host's native Docker platform.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="weavelogic-cogs-cross:1.97.1"
PLATFORM="${CROSS_PLATFORM:-}"
plat_args=()
if [ -n "$PLATFORM" ]; then
  plat_args=(--platform "$PLATFORM")
  IMAGE="$IMAGE-${PLATFORM//\//-}"
fi

cogs=("$@")
if [ ${#cogs[@]} -eq 0 ]; then
  for d in "$ROOT"/src/cogs/*/; do [ -f "$d/cog.toml" ] && cogs+=("$(basename "$d")"); done
fi
if [ ${#cogs[@]} -eq 0 ]; then echo "cross-build: no cogs"; exit 0; fi

command -v docker >/dev/null || { echo "cross-build: docker not found" >&2; exit 1; }
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "cross-build: building image $IMAGE"
  docker build ${plat_args[@]+"${plat_args[@]}"} -t "$IMAGE" "$ROOT/scripts/cross"
fi

# Build inside the container. Registry cache lives in a named volume; the target dir is
# separate from the host's so macOS and Linux artifacts never mix.
docker run --rm ${plat_args[@]+"${plat_args[@]}"} \
  -v "$ROOT":/work -w /work \
  -v weavelogic-cogs-cargo-registry:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/work/.cargo-target/cross \
  -e CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  "$IMAGE" bash -euo pipefail -c '
    for cog in "$@"; do
      cd "/work/src/cogs/$cog"
      echo "== $cog armv7-unknown-linux-gnueabihf"
      env -u RUSTFLAGS cargo build --locked --release --target armv7-unknown-linux-gnueabihf
      echo "== $cog aarch64-unknown-linux-gnu"
      RUSTFLAGS="-C target-cpu=cortex-a76" cargo build --locked --release --target aarch64-unknown-linux-gnu
      out="/work/.cargo-target/dist/$cog"; mkdir -p "$out"
      cp "/work/.cargo-target/cross/armv7-unknown-linux-gnueabihf/release/cog-$cog" "$out/cog-$cog-arm"
      cp "/work/.cargo-target/cross/aarch64-unknown-linux-gnu/release/cog-$cog" "$out/cog-$cog-arm64"
      arm-linux-gnueabihf-strip "$out/cog-$cog-arm"
      aarch64-linux-gnu-strip "$out/cog-$cog-arm64"
    done
  ' _ "${cogs[@]}"

fail=0
printf '%-28s %-12s %10s\n' "binary" "arch" "bytes"
for cog in "${cogs[@]}"; do
  for arch in arm arm64; do
    f="$ROOT/.cargo-target/dist/$cog/cog-$cog-$arch"
    [ -f "$f" ] || { echo "cross-build: missing $f" >&2; fail=1; continue; }
    size=$(wc -c < "$f" | tr -d ' ')
    printf '%-28s %-12s %10s\n' "cog-$cog" "$arch" "$size"
    if [ "$size" -gt 5242880 ]; then
      echo "cross-build: FAIL cog-$cog-$arch is $size bytes (> 5 MB)" >&2; fail=1
    elif [ "$size" -gt 1048576 ]; then
      echo "cross-build: WARN cog-$cog-$arch is $size bytes (> 1 MB)" >&2
    fi
  done
done
exit "$fail"
