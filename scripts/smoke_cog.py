#!/usr/bin/env python3
"""Native binary smoke test for gate step 8.

    python3 scripts/smoke_cog.py <binary> --cog <id>

Runs console commands from cog.toml that need no hardware and no extra arguments:
commands made only of --help, --once, and --simulate. Each must exit 0 within
max_runtime_secs and print something. A command that names a device or takes an
argument is skipped. A bare --once must report a missing source (`no_source`) or
an empty host (`"readings":0`). A blob that only claims a detection does not pass.
Every cog must run at least one of these commands.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path

SAFE = {"--help", "--once", "--simulate"}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("--cog", required=True)
    args = ap.parse_args()
    root = Path(__file__).resolve().parent.parent
    cfg = tomllib.loads((root / "src" / "cogs" / args.cog / "cog.toml").read_text())
    console = cfg.get("console", {})
    timeout = int(console.get("max_runtime_secs", 15))
    ran = 0
    for cmd in console.get("allowed_commands", []):
        parts = cmd.split()
        if not parts or not set(parts) <= SAFE:
            print(f"smoke {args.cog}: skip {cmd!r} (needs hardware or an argument)")
            continue
        try:
            proc = subprocess.run(
                [args.binary, *parts],
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            print(f"smoke {args.cog}: FAIL {cmd!r} exceeded {timeout}s")
            return 1
        text = proc.stdout + proc.stderr
        if proc.returncode != 0 or not text.strip():
            tail = proc.stderr[-400:]
            print(f"smoke {args.cog}: FAIL {cmd!r} exit {proc.returncode}\n{tail}")
            return 1
        if parts == ["--once"] and "no_source" not in text and '"readings":0' not in text:
            print(f"smoke {args.cog}: FAIL {cmd!r} did not report a missing source")
            return 1
        print(f"smoke {args.cog}: ok {cmd!r}")
        ran += 1
    if ran == 0:
        print(f"smoke {args.cog}: FAIL no runnable console command (add --help to allowed_commands)")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
