#!/usr/bin/env python3
"""Write one cog's agent manifest.json.

This is the document ``scripts/seed-sideload.sh`` installs under
``/var/lib/cognitum/apps/<id>/``. ``scripts/cross-build.sh`` calls this same
writer. The ``sha256`` and ``binary_size`` are of the binary argument, which
sideload passes as ``cog-<id>-arm`` because the agent runs that file.

Python 3.11 or newer (``tomllib``).
"""
import hashlib
import json
import sys
import tomllib


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit("usage: cog_manifest.py <cog.toml> <binary> <manifest.json>")
    toml_path, bin_path, out = sys.argv[1:]
    with open(toml_path, "rb") as handle:
        doc = tomllib.load(handle)
    with open(bin_path, "rb") as handle:
        binary = handle.read()
    cog = doc["cog"]
    config = [{"key": key, **value} for key, value in doc.get("config", {}).items()]
    manifest = {
        "binary_size": len(binary),
        "category": cog["category"],
        "config": config,
        "description": cog["description"],
        "difficulty": "medium",
        "id": cog["id"],
        "name": cog["name"],
        "sha256": hashlib.sha256(binary).hexdigest(),
        "size_kb": round(len(binary) / 1024),
        "version": cog["version"],
    }
    with open(out, "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, separators=(",", ":"))
    digest = manifest["sha256"][:16]
    print(
        f"manifest: {manifest['id']} {manifest['version']} "
        f"sha256={digest}... {manifest['binary_size']} bytes"
    )


if __name__ == "__main__":
    main()
