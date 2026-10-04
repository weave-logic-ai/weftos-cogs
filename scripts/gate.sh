#!/usr/bin/env bash
# The one gate. Run before every commit and before any export.
#
#   scripts/gate.sh             # full gate
#   scripts/gate.sh --fast      # skip the container cross-build (step 9)
#   scripts/gate.sh <cog-id>    # gate a single cog (flags combine: --fast <cog-id>)
#
# Fails fast; exits non-zero on the first failing step. Agents run it detached:
#   nohup scripts/gate.sh > .logs/gate.log 2>&1 &
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export CARGO_TARGET_DIR="$ROOT/.cargo-target/host"

FAST=0
only=()
for a in "$@"; do
  case "$a" in
    --fast) FAST=1 ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    -*) echo "gate: unknown flag $a" >&2; exit 2 ;;
    *) only+=("$a") ;;
  esac
done

cogs=()
if [ ${#only[@]} -gt 0 ]; then
  for c in "${only[@]}"; do
    [ -f "src/cogs/$c/cog.toml" ] || { echo "gate: no cog '$c' under src/cogs/" >&2; exit 2; }
    cogs+=("$c")
  done
else
  for d in src/cogs/*/; do [ -f "$d/cog.toml" ] && cogs+=("$(basename "$d")"); done
fi

step() { echo; echo "=== gate step $1: $2"; }
fail() { echo; echo "GATE FAILED at step $1 ($2), exit $3"; exit "$3"; }
run() { # run <step-no> <label> <cmd...>
  local n="$1" label="$2"; shift 2
  "$@"; local rc=$?
  [ $rc -eq 0 ] || fail "$n" "$label" "$rc"
}
in_cog() { (cd "src/cogs/$1" && shift && "$@"); }

step 1 "vendored crate matches UPSTREAM.lock"
run 1 sync-verify scripts/sync-upstream.sh --verify

step 2 "cargo fmt --check"
for c in ${cogs[@]+"${cogs[@]}"}; do echo "-- $c"; run 2 "fmt $c" in_cog "$c" cargo fmt --check; done
# The vendored crate is byte-for-byte upstream and upstream does not rustfmt it, so its fmt
# state is reported, not gated. Its integrity is gated by step 1.
if (cd crates/cog-sensor-sources && cargo fmt --check >/dev/null 2>&1); then
  echo "-- crates/cog-sensor-sources: fmt clean"
else
  echo "-- crates/cog-sensor-sources: NOT rustfmt-clean upstream (informational; vendored, not edited here)"
fi

step 3 "cargo clippy --release --all-targets -- -D warnings"
for c in ${cogs[@]+"${cogs[@]}"}; do echo "-- $c"; run 3 "clippy $c" in_cog "$c" cargo clippy --locked --release --all-targets -- -D warnings; done

step 4 "cargo test --release --all-targets"
for c in ${cogs[@]+"${cogs[@]}"}; do echo "-- $c"; run 4 "test $c" in_cog "$c" cargo test --locked --release --all-targets; done

step 5 "python unittest"
run 5 unittest python3 -m unittest discover -s tests -p 'test_*.py'

step 6 "cog_lint.py --strict-portable (this repo, zero findings)"
run 6 cog-lint python3 scripts/cog_lint.py --root . --strict-portable --fail-on warning

step 7 "check_contracts.py"
run 7 contracts python3 scripts/check_contracts.py

step 8 "native binary smoke test"
for c in ${cogs[@]+"${cogs[@]}"}; do
  echo "-- $c"
  run 8 "build $c" in_cog "$c" cargo build --locked --release
  run 8 "smoke $c" python3 scripts/smoke_cog.py "$CARGO_TARGET_DIR/release/cog-$c" --cog "$c"
done

step 9 "container cross-build (armhf + aarch64)"
if [ "$FAST" -eq 1 ]; then
  echo "SKIPPED (--fast)"
elif [ ${#cogs[@]} -eq 0 ]; then
  echo "no cogs"
else
  run 9 cross-build scripts/cross-build.sh ${cogs[@]+"${cogs[@]}"}
fi

step 10 "every cog has docs/adrs/ADR-*<id>*.md"
for c in ${cogs[@]+"${cogs[@]}"}; do
  if ! ls docs/adrs/ADR-*"$c"*.md >/dev/null 2>&1; then
    echo "missing ADR for $c"; fail 10 "adr $c" 1
  fi
  echo "-- $c: $(ls docs/adrs/ADR-*"$c"*.md | head -1)"
done

step 11 "sensor guides validate (WeftOS ADR-104 guide-check)"
GUIDE_CHECK="${GUIDE_CHECK:-$(command -v guide-check || true)}"
guides=()
for c in ${cogs[@]+"${cogs[@]}"}; do [ -f "src/cogs/$c/guide/guide.toml" ] && guides+=("src/cogs/$c/guide"); done
if [ ${#guides[@]} -eq 0 ]; then
  echo "no cog guides"
elif [ -z "$GUIDE_CHECK" ]; then
  echo "SKIPPED: guide-check not found (cargo install --path <weftos>/crates/weftos-sensor-guide, or set GUIDE_CHECK)"
else
  run 11 guide-check "$GUIDE_CHECK" "${guides[@]}"
fi

echo
if [ "$FAST" -eq 1 ]; then echo "GATE PASSED (fast: cross-build skipped)"; else echo "GATE PASSED"; fi
