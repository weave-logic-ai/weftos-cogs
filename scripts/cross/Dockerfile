# Cross-build image for WeaveLogic cogs. Mirrors upstream's native-cross-gcc route
# (build-all-cogs.yml: apt gcc-arm-linux-gnueabihf / gcc-aarch64-linux-gnu, no cross-rs,
# no zig) inside Debian because the dev Mac has no cross-gcc.
#
# On an arm64 host (Apple Silicon Docker) Debian ships no gcc-aarch64-linux-gnu package:
# the native gcc already answers to aarch64-linux-gnu-gcc, so that target links with it.
# On an amd64 host both cross packages are installed, which is exactly upstream's setup.
FROM rust:1.97.1-bookworm
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    pkgs="gcc-arm-linux-gnueabihf libc6-dev-armhf-cross binutils-arm-linux-gnueabihf"; \
    if [ "$arch" != "arm64" ]; then pkgs="$pkgs gcc-aarch64-linux-gnu libc6-dev-arm64-cross binutils-aarch64-linux-gnu"; fi; \
    apt-get update; \
    apt-get install -y --no-install-recommends $pkgs; \
    rm -rf /var/lib/apt/lists/*; \
    command -v arm-linux-gnueabihf-gcc; command -v aarch64-linux-gnu-gcc; command -v aarch64-linux-gnu-strip
RUN rustup component add clippy rustfmt \
 && rustup target add armv7-unknown-linux-gnueabihf aarch64-unknown-linux-gnu
