#!/usr/bin/env python3
"""Catalog contract linter for a cognitum-one/cogs-shaped tree.

Checks every src/cogs/<id>/ against the contract upstream CI enforces
(manifest-validate, adr-required, asset-sha256-validate) plus the drift the
catalog's own issues describe (#62, #64, #65) and ADR-001's release profile.

    python3 scripts/cog_lint.py --root ../vendor/cogs
    python3 scripts/cog_lint.py --root ../vendor/cogs --json > report.json
    python3 scripts/cog_lint.py --root . --strict-portable --fail-on warning
    python3 scripts/cog_lint.py --root ../vendor/cogs --baseline report.json

Stdlib only (Python 3.11+: tomllib). Exit 0 when no finding at or above
--fail-on (default: error) survives --skip/--baseline, 1 otherwise, 2 on usage.
L013 (release-registry readiness) is informational: it counts what cogs
declare and never fills in a default.
"""
from __future__ import annotations

import argparse
import functools
import json
import re
import subprocess
import sys
import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path

LEVELS = {"info": 0, "warning": 1, "error": 2}
ID_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
FLAG_RE = re.compile(r"^--?[A-Za-z][A-Za-z0-9-]*$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
KNOWN_HARDWARE = {"pi-zero-2w", "v0-appliance"}
KNOWN_CATEGORIES = {
    "ai", "health", "security", "research", "swarm", "building", "retail",
    "industrial", "developer", "signal", "presence", "network",
    "wearable",  # ours, not upstream: body-worn devices with their own sensors (COG-008, Categories)
}
BUILTIN_FLAGS = {"--once", "--interval", "--help"}
ADR001_PROFILE = {  # key -> accepted values
    "opt-level": {"s"},
    "lto": {True, "fat"},
    "codegen-units": {1},
    "panic": {"abort"},
    "strip": {True, "symbols"},
}
DISCOURAGED_DEPS = {"tokio", "reqwest", "hyper", "clap"}
SHARED_CRATE_PATH = "../../../crates/cog-sensor-sources"
# cogs#64: the eight release-registry fields no cog.toml declares.
REGISTRY_FIELDS = [
    "packaging", "deploymentDriver", "tenancyMode", "statePolicy",
    "stateSchemaVersion", "rollbackCompatibility", "networkPolicy.egressPolicy",
    "residency.allowedRegions+dataResidency",
]


@dataclass
class Finding:
    check: str
    level: str
    cog: str
    file: str
    line: int
    message: str

    def key(self) -> tuple[str, str, str]:
        return (self.check, self.cog, self.file)


def toml_line(text: str, table: str | None, key: str | None = None) -> int:
    """1-based line of `key` inside `[table]` (or of the header); 1 when not found."""
    current = None
    header_line = 0
    for n, raw in enumerate(text.splitlines(), 1):
        s = raw.strip()
        if s.startswith("["):
            current = s.split("#", 1)[0].strip()
            if table is not None and current == table and not header_line:
                header_line = n
                if key is None:
                    return n
            continue
        if key and (table is None or current == table) and re.match(rf"^{re.escape(key)}\s*=", s):
            return n
    return header_line or 1


def load_toml(path: Path) -> tuple[dict | None, str, str | None]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as e:
        return None, "", str(e)
    try:
        return tomllib.loads(text), text, None
    except tomllib.TOMLDecodeError as e:
        return None, text, str(e)


def git(root: Path, *args: str) -> subprocess.CompletedProcess | None:
    try:
        return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=False)
    except OSError:
        return None


@functools.lru_cache(maxsize=None)
def is_git_root(root: Path) -> bool:
    """True only when `root` is itself the top of a git work tree (not a subdirectory
    of one, like a test fixture inside this repo)."""
    r = git(root, "rev-parse", "--show-toplevel")
    return bool(r and r.returncode == 0 and Path(r.stdout.strip()).resolve() == root.resolve())


def git_tracked(root: Path, rel: str) -> bool | None:
    """True/False if `root` is a git work tree root, None otherwise."""
    if not is_git_root(root):
        return None
    r = git(root, "ls-files", "--error-unmatch", rel)
    return bool(r and r.returncode == 0)


def dig(obj: dict, dotted: str):
    for part in dotted.split("."):
        if not isinstance(obj, dict) or part not in obj:
            return None
        obj = obj[part]
    return obj


def registry_fields_declared(cog_dir: Path, cog_toml: dict) -> set[str]:
    """Which of the eight cogs#64 fields this cog declares, from release-policy.json
    or a cog.toml [contract] table. Presence only; values are never invented."""
    sources = []
    policy = cog_dir / "release-policy.json"
    if policy.is_file():
        try:
            sources.append(json.loads(policy.read_text(encoding="utf-8")))
        except (OSError, json.JSONDecodeError):
            pass
    if isinstance(cog_toml.get("contract"), dict):
        sources.append(cog_toml["contract"])
    found = set()
    for src in sources:
        for f in REGISTRY_FIELDS:
            if f.startswith("residency."):
                if dig(src, "residency.allowedRegions") is not None and dig(src, "residency.dataResidency") is not None:
                    found.add(f)
            elif dig(src, f) is not None:
                found.add(f)
    return found


def declares_tenancy(value) -> bool:
    if isinstance(value, dict):
        return any("tenancy" in str(k).lower() or declares_tenancy(v) for k, v in value.items())
    if isinstance(value, list):
        return any(declares_tenancy(v) for v in value)
    return False


def dep_tables(cargo: dict):
    """(table-name, deps-dict) for every dependency table, target-specific included."""
    for name in ("dependencies", "dev-dependencies", "build-dependencies"):
        if isinstance(cargo.get(name), dict):
            yield name, cargo[name]
    for tgt, body in (cargo.get("target") or {}).items():
        if isinstance(body, dict):
            for name in ("dependencies", "dev-dependencies", "build-dependencies"):
                if isinstance(body.get(name), dict):
                    yield f"target.{tgt}.{name}", body[name]


def rust_sources(cog_dir: Path) -> str:
    parts = []
    for p in sorted((cog_dir / "src").rglob("*.rs")):
        try:
            parts.append(p.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            pass
    return "\n".join(parts)


def lint_cog(root: Path, cog_dir: Path, strict_portable: bool, stats: dict) -> list[Finding]:
    cog = cog_dir.name
    def rel(p: Path) -> str:
        return str(p.relative_to(root))

    out: list[Finding] = []

    def add(check, level, path, line, msg):
        out.append(Finding(check, level, cog, rel(path), line, msg))

    cog_path, cargo_path, main_rs = cog_dir / "cog.toml", cog_dir / "Cargo.toml", cog_dir / "src" / "main.rs"

    cogm, cog_text, cog_err = (None, "", "missing") if not cog_path.is_file() else load_toml(cog_path)
    cargo, cargo_text, cargo_err = (None, "", "missing") if not cargo_path.is_file() else load_toml(cargo_path)
    if cogm is None:
        add("L002", "error", cog_path, 1, f"cog.toml {'is missing' if cog_err == 'missing' else 'does not parse: ' + cog_err}")
    if cargo is None:
        add("L003", "error", cargo_path, 1, f"Cargo.toml {'is missing' if cargo_err == 'missing' else 'does not parse: ' + cargo_err}")
    if not main_rs.is_file():
        add("L003", "error", main_rs, 1, "src/main.rs is missing")
    meta = (cogm or {}).get("cog", {}) if cogm is not None else {}

    # L001 version agreement (#65)
    if cogm is not None and cargo is not None:
        cv, pv = meta.get("version"), (cargo.get("package") or {}).get("version")
        if cv is None or pv is None or cv != pv:
            add("L001", "error", cog_path, toml_line(cog_text, "[cog]", "version"),
                f"cog.toml [cog].version {cv!r} != Cargo.toml [package].version {pv!r}")

    if cogm is not None:
        # L002 id == directory name, and the publish-cog.yml id pattern
        cid = meta.get("id")
        if cid != cog:
            add("L002", "error", cog_path, toml_line(cog_text, "[cog]", "id"), f"[cog].id {cid!r} != directory name {cog!r}")
        elif not ID_RE.match(cog):
            add("L002", "error", cog_path, toml_line(cog_text, "[cog]", "id"), f"id {cog!r} is not lowercase-hyphenated")

        # L004 hardware_requirement
        hw = meta.get("hardware_requirement")
        hw_line = toml_line(cog_text, "[cog]", "hardware_requirement")
        if isinstance(hw, str):
            hw = [hw]
        if not isinstance(hw, list) or not hw:
            add("L004", "error", cog_path, hw_line, "hardware_requirement missing or empty")
        else:
            bad = [d for d in hw if d not in KNOWN_HARDWARE]
            if bad:
                add("L004", "error", cog_path, hw_line, f"unknown device(s) {bad}; allowed {sorted(KNOWN_HARDWARE)}")

        # L005 category
        cat = meta.get("category")
        if cat not in KNOWN_CATEGORIES:
            add("L005", "warning", cog_path, toml_line(cog_text, "[cog]", "category"), f"category {cat!r} not in the known set")

        # L006 [console]
        console = cogm.get("console")
        cmds: list = []
        if not isinstance(console, dict):
            add("L006", "error", cog_path, 1, "[console] table missing (cog-runner fails closed)")
        else:
            cmds = console.get("allowed_commands") or []
            if not isinstance(cmds, list) or not cmds or not all(isinstance(c, str) for c in cmds):
                add("L006", "error", cog_path, toml_line(cog_text, "[console]", "allowed_commands"),
                    "[console].allowed_commands missing, empty or not a list of strings")
                cmds = [c for c in cmds if isinstance(c, str)] if isinstance(cmds, list) else []
            for lim in ("max_runtime_secs", "output_limit_bytes"):
                v = console.get(lim)
                if not isinstance(v, int) or isinstance(v, bool) or v <= 0:
                    add("L006", "error", cog_path, toml_line(cog_text, "[console]", lim), f"[console].{lim} must be a positive integer, got {v!r}")

        # L007 every console flag is a declared config cli_arg or a built-in
        config = cogm.get("config") if isinstance(cogm.get("config"), dict) else {}
        cli_args = {}
        for name, body in config.items():
            if isinstance(body, dict) and isinstance(body.get("cli_arg"), str):
                cli_args[body["cli_arg"]] = name
        # A clap-derived CLI implements its flags (and --help) without the literal in source.
        uses_clap = cargo is not None and any("clap" in deps for _, deps in dep_tables(cargo))
        src = rust_sources(cog_dir)
        flags = sorted({t for c in cmds for t in c.split() if FLAG_RE.match(t)})
        cmds_line = toml_line(cog_text, "[console]", "allowed_commands")
        for f in flags:
            if f in cli_args or f in BUILTIN_FLAGS:
                continue
            if f'"{f}"' in src:
                add("L007", "warning", cog_path, cmds_line,
                    f"allowed_commands uses {f}; it is parsed in src but no [config.*].cli_arg declares it")
            elif uses_clap:
                add("L007", "warning", cog_path, cmds_line,
                    f"allowed_commands uses {f}; no [config.*].cli_arg declares it (clap-derived CLI, not statically verified)")
            else:
                add("L007", "error", cog_path, cmds_line,
                    f"allowed_commands uses {f} but no [config.*].cli_arg declares it and src never matches it")

        # L008 every cli_arg appears in the Rust source (static heuristic)
        for arg, name in sorted(cli_args.items()):
            bare = arg.lstrip("-")
            clap_hit = uses_clap and (f'long = "{bare}"' in src
                                      or re.search(rf"\b{re.escape(bare.replace('-', '_'))}\s*:", src))
            if arg not in src and not clap_hit:
                add("L008", "warning", cog_path, toml_line(cog_text, f"[config.{name}]", "cli_arg"),
                    f"[config.{name}].cli_arg {arg} does not appear in src/**/*.rs")

        # L009 --help advertised => implemented (#62)
        if "--help" in flags and main_rs.is_file():
            main_text = main_rs.read_text(encoding="utf-8", errors="replace")
            if not uses_clap and not re.search(r"\bhandle_help\b|\bhelp_text\b|\"--help\"", main_text):
                add("L009", "error", main_rs, 1, "--help is in allowed_commands but main.rs never calls handle_help/help_text or matches \"--help\"")

        # L011 [[assets]] sha256
        assets = cogm.get("assets") or []
        marker = (cog_dir / ".allow-unpublished-assets").exists()
        for i, a in enumerate(assets if isinstance(assets, list) else []):
            sha = a.get("sha256") if isinstance(a, dict) else None
            if not (isinstance(sha, str) and SHA256_RE.match(sha)) and not marker:
                add("L011", "error", cog_path, toml_line(cog_text, "[[assets]]", "sha256"),
                    f"[[assets]][{i}].sha256 {sha!r} is not a 64-hex digest and no .allow-unpublished-assets marker")

        # L012 ADR present
        if not list((root / "docs" / "adrs").glob(f"ADR-*{cog}*.md")):
            add("L012", "warning", cog_path, 1, f"no docs/adrs/ADR-*{cog}*.md")

        # L013 release-registry readiness (#64): counted, never defaulted
        declared = registry_fields_declared(cog_dir, cogm)
        for f in REGISTRY_FIELDS:
            stats["registry_field_counts"][f] += f in declared
        if len(declared) == len(REGISTRY_FIELDS):
            stats["registry_complete"].append(cog)
        else:
            missing = [f for f in REGISTRY_FIELDS if f not in declared]
            add("L013", "info", cog_path, 1, f"{len(missing)}/8 cogs#64 registry fields not declared: {', '.join(missing)}")
        if declares_tenancy(cogm):
            stats["cog_toml_declares_tenancy"].append(cog)

    if cargo is not None:
        pkg = cargo.get("package") or {}
        # L003 [[bin]].name == cog-<id>
        bins = cargo.get("bin") or []
        names = [b.get("name") for b in bins if isinstance(b, dict)]
        if f"cog-{cog}" not in names[:1]:
            add("L003", "error", cargo_path, toml_line(cargo_text, "[[bin]]", "name"),
                f"first [[bin]].name {names[0] if names else None!r} != 'cog-{cog}'")

        # L010 ADR-001 release profile
        prof = (cargo.get("profile") or {}).get("release") or {}
        bad = [f"{k}={prof.get(k)!r}" for k, ok in ADR001_PROFILE.items() if prof.get(k) not in ok]
        if bad:
            add("L010", "warning", cargo_path, toml_line(cargo_text, "[profile.release]"),
                f"[profile.release] differs from ADR-001: {', '.join(bad)}")

        # L014 Cargo.lock committed
        lock_rel = rel(cog_dir / "Cargo.lock")
        tracked = git_tracked(root, lock_rel)
        if tracked is False or (tracked is None and not (cog_dir / "Cargo.lock").is_file()):
            add("L014", "error", cog_dir / "Cargo.lock", 1, "Cargo.lock is not committed in the cog directory")

        # L015 dependency hygiene, P001 portability
        for table, deps in dep_tables(cargo):
            for dep, spec in deps.items():
                real = spec.get("package", dep) if isinstance(spec, dict) else dep
                runtime_dep = not table.endswith(("dev-dependencies", "build-dependencies"))
                if runtime_dep and real in DISCOURAGED_DEPS:
                    add("L015", "warning", cargo_path, toml_line(cargo_text, f"[{table}]", dep), f"[{table}] {real} is discouraged by ADR-001")
                if strict_portable and isinstance(spec, dict):
                    if spec.get("workspace") is True:
                        add("P001", "error", cargo_path, toml_line(cargo_text, f"[{table}]", dep), f"{dep} uses workspace inheritance")
                    elif "path" in spec and spec["path"] != SHARED_CRATE_PATH:
                        add("P001", "error", cargo_path, toml_line(cargo_text, f"[{table}]", dep),
                            f"{dep} path {spec['path']!r} is outside {SHARED_CRATE_PATH}")
        if strict_portable and any(v == {"workspace": True} or (isinstance(v, dict) and v.get("workspace") is True) for v in pkg.values()):
            add("P001", "error", cargo_path, toml_line(cargo_text, "[package]"), "[package] uses workspace inheritance")
    return out


def upstream_commit(root: Path) -> str | None:
    if not is_git_root(root):
        return None
    r = git(root, "rev-parse", "HEAD")
    return (r.stdout.strip() or None) if r and r.returncode == 0 else None


def run(root: Path, strict_portable: bool = False, skip: set[str] | None = None,
        baseline: set[tuple] | None = None) -> dict:
    skip = skip or set()
    stats = {"registry_field_counts": {f: 0 for f in REGISTRY_FIELDS}, "registry_complete": [], "cog_toml_declares_tenancy": []}
    cogs_dir = root / "src" / "cogs"
    dirs = sorted(p for p in cogs_dir.iterdir() if p.is_dir()) if cogs_dir.is_dir() else []
    findings: list[Finding] = []
    for d in dirs:
        findings.extend(lint_cog(root, d, strict_portable, stats))
    findings = [f for f in findings if f.check not in skip]
    suppressed = [f for f in findings if baseline and f.key() in baseline]
    findings = [f for f in findings if not (baseline and f.key() in baseline)]
    by_check: dict[str, int] = {}
    cogs_by_check: dict[str, set] = {}
    for f in findings:
        by_check[f.check] = by_check.get(f.check, 0) + 1
        cogs_by_check.setdefault(f.check, set()).add(f.cog)
    by_level = {lvl: sum(f.level == lvl for f in findings) for lvl in LEVELS}
    return {
        "tool": "cog_lint.py",
        "root": str(root),
        "commit": upstream_commit(root),
        "strict_portable": strict_portable,
        "findings": [asdict(f) for f in findings],
        "summary": {
            "cogs": len(dirs),
            "findings": len(findings),
            "suppressed_by_baseline": len(suppressed),
            "by_level": by_level,
            "by_check": dict(sorted(by_check.items())),
            "cogs_by_check": {k: len(v) for k, v in sorted(cogs_by_check.items())},
            "registry_field_counts": stats["registry_field_counts"],
            "registry_complete": stats["registry_complete"],
            "cog_toml_declares_tenancy": len(stats["cog_toml_declares_tenancy"]),
        },
    }


def load_baseline(path: Path) -> set[tuple]:
    data = json.loads(path.read_text(encoding="utf-8"))
    items = data["findings"] if isinstance(data, dict) else data
    return {(f["check"], f["cog"], f["file"]) for f in items}


def print_table(report: dict) -> None:
    for f in report["findings"]:
        print(f"{f['level']:<7} {f['check']:<5} {f['file']}:{f['line']}  {f['message']}")
    s = report["summary"]
    print(f"\n{s['cogs']} cogs, {s['findings']} findings "
          f"({s['by_level']['error']} error, {s['by_level']['warning']} warning, {s['by_level']['info']} info)"
          + (f", {s['suppressed_by_baseline']} suppressed by baseline" if s["suppressed_by_baseline"] else ""))
    for check, n in s["by_check"].items():
        print(f"  {check}: {n} finding(s) across {s['cogs_by_check'][check]} cog(s)")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--root", default=".", help="repo root containing src/cogs/")
    ap.add_argument("--json", action="store_true", help="emit the JSON report on stdout")
    ap.add_argument("--baseline", type=Path, help="JSON report whose findings are suppressed (keyed on check, cog, file)")
    ap.add_argument("--skip", default="", help="comma-separated check IDs to disable")
    ap.add_argument("--strict-portable", action="store_true", help="enable P001 (no path deps outside the shared crate)")
    ap.add_argument("--fail-on", choices=["error", "warning"], default="error")
    args = ap.parse_args(argv)
    root = Path(args.root).resolve()
    if not (root / "src").is_dir():
        print(f"cog_lint: {root} has no src/ directory", file=sys.stderr)
        return 2
    baseline = load_baseline(args.baseline) if args.baseline else None
    skip = {s.strip() for s in args.skip.split(",") if s.strip()}
    report = run(root, args.strict_portable, skip, baseline)
    report["root"] = args.root  # as given, so committed reports carry no local absolute path
    if args.json:
        json.dump(report, sys.stdout, indent=2)
        sys.stdout.write("\n")
    else:
        print_table(report)
    threshold = LEVELS[args.fail_on]
    return 1 if any(LEVELS[f["level"]] >= threshold for f in report["findings"]) else 0


if __name__ == "__main__":
    sys.exit(main())
