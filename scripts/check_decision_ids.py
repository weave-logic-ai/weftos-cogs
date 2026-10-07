#!/usr/bin/env python3
"""One non-redirect decision per series and number, and the H1 matches the filename."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIRS = (ROOT / "docs" / "adrs", ROOT / "docs" / "decisions")
NAME = re.compile(r"^(ADR|COG)-(\d+)(?:-.+)?\.md$")
H1 = re.compile(r"^# (ADR|COG)-(\d+)\b")


def heading(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("#"):
            return line.strip()
    return None


def main() -> int:
    owners: dict[tuple[str, str], Path] = {}
    errors: list[str] = []
    seen = 0
    for directory in DIRS:
        if not directory.is_dir():
            errors.append(f"missing {directory.relative_to(ROOT)}")
            continue
        for path in sorted(directory.glob("*.md")):
            match = NAME.match(path.name)
            if not match:
                continue
            seen += 1
            series, number = match.group(1), match.group(2)
            title = heading(path)
            if title is None:
                errors.append(f"{path.relative_to(ROOT)}: no heading")
                continue
            if title == "# Redirect" or title.startswith("# Redirect "):
                continue
            got = H1.match(title)
            if not got or got.group(1) != series or got.group(2) != number:
                errors.append(
                    f"{path.relative_to(ROOT)}: heading {title!r} does not match {series}-{number}"
                )
                continue
            key = (series, number)
            previous = owners.get(key)
            if previous is not None:
                errors.append(
                    f"{series}-{number}: {previous.relative_to(ROOT)} and {path.relative_to(ROOT)}"
                )
                continue
            owners[key] = path
    if seen == 0:
        errors.append("no decision files")
    if errors:
        print("decision ids failed:")
        for item in errors:
            print(f"  {item}")
        return 1
    print(f"decision ids ok ({len(owners)} non-redirect, {seen} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
