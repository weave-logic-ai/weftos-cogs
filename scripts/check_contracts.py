#!/usr/bin/env python3
"""Validate WeaveLogic cog-output contracts. Stdlib only, no jsonschema dependency
(the same rule upstream follows in scripts/cog_release_schema_check.py).

    python3 scripts/check_contracts.py                       # full check (gate step 7)
    python3 scripts/check_contracts.py --validate <schema> [FILE|-]
        validate JSON lines (one object per non-empty line) against a schema

Full check:
  * every schemas/weavelogic/*.schema.json parses and declares the 2020-12 dialect;
  * every tests/fixtures/contracts/<cog>/valid/*.json passes <cog>.v0.schema.json;
  * every reject/<reason>[--n].json fails, and fails ONLY for <reason>. Reasons come from
    `x-reject-reason` annotations in the schema; an error under no annotation reports as
    `schema:<keyword>`, so a fixture that fails for a structural accident is caught;
  * each cog has >= 3 valid fixtures and the required reject reasons are all covered;
  * every expected-output line under src/cogs/<cog>/tests/fixtures/*.expected.jsonl
    validates against that cog's schema (the contract and the code meet here).

Supported keyword subset (anything else fails closed): $ref (local and sibling-file),
$defs, type, enum, const, required, properties, additionalProperties,
unevaluatedProperties, propertyNames, items, minItems, maxItems, minimum, maximum,
exclusiveMinimum, exclusiveMaximum, pattern, minLength, allOf, anyOf, oneOf, not,
if/then/else. Annotations from failed subschemas are kept for unevaluatedProperties,
so a forbidden property reports its named reason once rather than twice.
"""
from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCHEMA_DIR = ROOT / "schemas" / "weavelogic"
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "contracts"
DIALECT = "https://json-schema.org/draft/2020-12/schema"
MIN_VALID = 3
REQUIRED_REJECTS = {
    "status-live", "aabb-present", "pose-present", "promoted-flag",
    "quality-without-verified-source", "water-sound-speed",
    "missing-atex-disclaimer", "ms-timestamp-in-timestamp",
}
ANNOTATIONS = {"$schema", "$id", "$comment", "$defs", "title", "description", "examples", "default", "x-reject-reason"}
TYPES = {
    "null": lambda v: v is None,
    "boolean": lambda v: isinstance(v, bool),
    "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
    "string": lambda v: isinstance(v, str),
    "array": lambda v: isinstance(v, list),
    "object": lambda v: isinstance(v, dict),
}


@dataclass(frozen=True)
class Error:
    path: str
    keyword: str
    reason: str
    detail: str

    def __str__(self) -> str:
        return f"{self.path or '/'}: [{self.reason}] {self.keyword}: {self.detail}"


class SchemaError(Exception):
    pass


class Validator:
    def __init__(self, schema_path: Path):
        self.docs: dict[Path, dict] = {}
        self.base = schema_path.resolve()
        self.root = self._load(self.base)

    def _load(self, path: Path) -> dict:
        if path not in self.docs:
            self.docs[path] = json.loads(path.read_text(encoding="utf-8"))
        return self.docs[path]

    def _resolve(self, ref: str, doc: Path) -> tuple[object, Path]:
        file_part, _, pointer = ref.partition("#")
        target = (doc.parent / file_part).resolve() if file_part else doc
        node: object = self._load(target)
        for token in [t for t in pointer.split("/") if t]:
            token = token.replace("~1", "/").replace("~0", "~")
            if not isinstance(node, dict) or token not in node:
                raise SchemaError(f"unresolvable $ref {ref!r} from {doc.name}")
            node = node[token]
        return node, target

    def validate(self, instance) -> list[Error]:
        errors, _ = self._v(self.root, instance, "", None, self.base)
        return errors

    def _v(self, schema, inst, path: str, reason: str | None, doc: Path) -> tuple[list[Error], set]:
        if schema is True:
            return [], set()
        if schema is False:
            return [Error(path, "false", reason or "schema:false", "no value is allowed here")], set()
        if not isinstance(schema, dict):
            raise SchemaError(f"schema at {path or '/'} is not an object or boolean")
        reason = schema.get("x-reject-reason", reason)
        errs: list[Error] = []
        seen: set = set()

        def err(kw: str, detail: str) -> None:
            errs.append(Error(path, kw, reason or f"schema:{kw}", detail))

        def sub(s, i, p, d=doc):
            e, ev = self._v(s, i, p, reason, d)
            return e, ev

        for kw in schema:
            if kw not in ANNOTATIONS and kw not in HANDLED:
                raise SchemaError(f"unsupported keyword {kw!r} in {doc.name}")

        if "$ref" in schema:
            target, tdoc = self._resolve(schema["$ref"], doc)
            e, ev = sub(target, inst, path, tdoc)
            errs += e
            seen |= ev
        if "type" in schema:
            types = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
            if not any(TYPES[t](inst) for t in types):
                err("type", f"expected {'/'.join(types)}, got {json.dumps(inst)[:60]}")
        if "enum" in schema and not any(_eq(inst, v) for v in schema["enum"]):
            err("enum", f"{json.dumps(inst)[:60]} not in {schema['enum']}")
        if "const" in schema and not _eq(inst, schema["const"]):
            err("const", f"{json.dumps(inst)[:60]} != {json.dumps(schema['const'])}")
        if TYPES["number"](inst):
            for kw, bad in (("minimum", lambda v, b: v < b), ("maximum", lambda v, b: v > b),
                            ("exclusiveMinimum", lambda v, b: v <= b), ("exclusiveMaximum", lambda v, b: v >= b)):
                if kw in schema and bad(inst, schema[kw]):
                    err(kw, f"{inst} violates {kw} {schema[kw]}")
        if isinstance(inst, str):
            if "pattern" in schema and not re.search(schema["pattern"], inst):
                err("pattern", f"{inst!r} does not match {schema['pattern']}")
            if "minLength" in schema and len(inst) < schema["minLength"]:
                err("minLength", f"{inst!r} shorter than {schema['minLength']}")
        if isinstance(inst, list):
            if "minItems" in schema and len(inst) < schema["minItems"]:
                err("minItems", f"{len(inst)} < {schema['minItems']}")
            if "maxItems" in schema and len(inst) > schema["maxItems"]:
                err("maxItems", f"{len(inst)} > {schema['maxItems']}")
            if "items" in schema:
                for i, item in enumerate(inst):
                    errs += sub(schema["items"], item, f"{path}/{i}")[0]
        if isinstance(inst, dict):
            for name in schema.get("required", []):
                if name not in inst:
                    err("required", f"missing {name!r}")
            props = schema.get("properties", {})
            for name, value in inst.items():
                if name in props:
                    seen.add(name)
                    errs += sub(props[name], value, f"{path}/{name}")[0]
                elif "additionalProperties" in schema:
                    seen.add(name)
                    errs += sub(schema["additionalProperties"], value, f"{path}/{name}")[0]
            if "propertyNames" in schema:
                for name in inst:
                    errs += sub(schema["propertyNames"], name, f"{path}/{name}")[0]
        for s in schema.get("allOf", []):
            e, ev = sub(s, inst, path)
            errs += e
            seen |= ev
        for kw in ("anyOf", "oneOf"):
            if kw in schema:
                results = [sub(s, inst, path) for s in schema[kw]]
                passing = [ev for e, ev in results if not e]
                for ev in passing:
                    seen |= ev
                if kw == "anyOf" and not passing:
                    err(kw, "matches none of the alternatives")
                if kw == "oneOf" and len(passing) != 1:
                    err(kw, f"matches {len(passing)} alternatives, expected exactly 1")
        if "not" in schema and not sub(schema["not"], inst, path)[0]:
            err("not", "matches a schema it must not match")
        if "if" in schema:
            cond_errs, ev = sub(schema["if"], inst, path)
            branch = "then" if not cond_errs else "else"
            if not cond_errs:
                seen |= ev
            if branch in schema:
                e, ev = sub(schema[branch], inst, path)
                errs += e
                seen |= ev
        if "unevaluatedProperties" in schema and isinstance(inst, dict):
            for name, value in inst.items():
                if name not in seen:
                    errs += sub(schema["unevaluatedProperties"], value, f"{path}/{name}")[0]
                    seen.add(name)
        return errs, seen


HANDLED = {
    "$ref", "type", "enum", "const", "required", "properties", "additionalProperties",
    "unevaluatedProperties", "propertyNames", "items", "minItems", "maxItems", "minimum",
    "maximum", "exclusiveMinimum", "exclusiveMaximum", "pattern", "minLength", "allOf",
    "anyOf", "oneOf", "not", "if", "then", "else",
}


def _eq(a, b) -> bool:
    """JSON equality: 1 == 1.0 but True != 1."""
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    return a == b


def reasons(errors: list[Error]) -> set[str]:
    return {e.reason for e in errors}


def expected_reason(fixture: Path) -> str:
    return fixture.stem.split("--", 1)[0]


def check_all() -> list[str]:
    problems: list[str] = []
    schemas = sorted(SCHEMA_DIR.glob("*.schema.json"))
    for s in schemas:
        try:
            doc = json.loads(s.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            problems.append(f"{s.relative_to(ROOT)}: does not parse: {e}")
            continue
        if doc.get("$schema") != DIALECT:
            problems.append(f"{s.relative_to(ROOT)}: $schema must be {DIALECT}")
    covered: set[str] = set()
    cog_dirs = sorted(d for d in FIXTURE_DIR.iterdir() if d.is_dir()) if FIXTURE_DIR.is_dir() else []
    for cog_dir in cog_dirs:
        schema = SCHEMA_DIR / f"{cog_dir.name}.v0.schema.json"
        if not schema.is_file():
            problems.append(f"{cog_dir.relative_to(ROOT)}: no schema {schema.relative_to(ROOT)}")
            continue
        v = Validator(schema)
        valid = sorted((cog_dir / "valid").glob("*.json"))
        if len(valid) < MIN_VALID:
            problems.append(f"{cog_dir.name}: {len(valid)} valid fixtures, need >= {MIN_VALID}")
        for f in valid:
            errs = v.validate(json.loads(f.read_text(encoding="utf-8")))
            if errs:
                problems.append(f"{f.relative_to(ROOT)}: valid fixture rejected: " + "; ".join(map(str, errs)))
        for f in sorted((cog_dir / "reject").glob("*.json")):
            want = expected_reason(f)
            covered.add(want)
            got = reasons(v.validate(json.loads(f.read_text(encoding="utf-8"))))
            if got != {want}:
                problems.append(f"{f.relative_to(ROOT)}: expected to fail for [{want}] only, got {sorted(got) or 'no failure'}")
    if cog_dirs:
        missing = REQUIRED_REJECTS - covered
        if missing:
            problems.append(f"required reject reasons with no fixture: {sorted(missing)}")
    for expected in sorted((ROOT / "src" / "cogs").glob("*/tests/fixtures/*.expected.jsonl")):
        cog = expected.parents[2].name
        schema = SCHEMA_DIR / f"{cog}.v0.schema.json"
        if not schema.is_file():
            problems.append(f"{expected.relative_to(ROOT)}: no schema for cog {cog}")
            continue
        problems += validate_lines(Validator(schema), expected.read_text(encoding="utf-8"), str(expected.relative_to(ROOT)))
    return problems


def validate_lines(v: Validator, text: str, label: str) -> list[str]:
    out = []
    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines:
        out.append(f"{label}: no JSON lines")
    for n, line in enumerate(lines, 1):
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as e:
            out.append(f"{label}:{n}: not JSON: {e}")
            continue
        errs = v.validate(obj)
        if errs:
            out.append(f"{label}:{n}: " + "; ".join(map(str, errs)))
    return out


def main(argv: list[str]) -> int:
    try:
        if argv[:1] == ["--validate"]:
            if len(argv) not in (2, 3):
                print(__doc__, file=sys.stderr)
                return 2
            src = argv[2] if len(argv) == 3 else "-"
            text = sys.stdin.read() if src == "-" else Path(src).read_text(encoding="utf-8")
            problems = validate_lines(Validator(Path(argv[1])), text, src)
        elif not argv:
            problems = check_all()
        else:
            print(__doc__, file=sys.stderr)
            return 2
    except SchemaError as e:
        print(f"check_contracts: schema error: {e}", file=sys.stderr)
        return 2
    for p in problems:
        print(p)
    if problems:
        print(f"check_contracts: {len(problems)} problem(s)")
        return 1
    print("check_contracts: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
