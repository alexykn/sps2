#!/usr/bin/env python3
"""Generate an inventory of event enums and variants in crates/events."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator, List


RE_ENUM = re.compile(r"pub\s+enum\s+(?P<name>[A-Za-z0-9_]+)")


@dataclass
class Variant:
    name: str
    start_line: int
    doc: List[str] = field(default_factory=list)
    definition: str = ""


@dataclass
class EnumItem:
    name: str
    module: str
    path: Path
    start_line: int
    doc: List[str]
    definition: str
    variants: List[Variant]


BASE = Path(__file__).resolve().parents[1] / "crates" / "events" / "src"


def main() -> None:
    items: List[EnumItem] = []
    for path in sorted(BASE.rglob("*.rs")):
        if "/tests/" in path.as_posix():
            continue
        items.extend(parse_file(path))

    output = {
        "base": str(BASE.relative_to(Path.cwd())),
        "items": [enum_to_dict(item) for item in items if item.name.endswith("Event")],
    }

    out_path = Path(__file__).with_name("EVENTS_INVENTORY.json")
    out_path.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")


def parse_file(path: Path) -> Iterator[EnumItem]:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    for idx, line in enumerate(lines):
        match = RE_ENUM.search(line)
        if not match:
            continue

        name = match.group("name")
        start_line = idx + 1
        enum_doc = collect_leading_doc(lines, idx)
        definition, end_idx = collect_block(lines, idx)
        variants = collect_variants(lines, idx, end_idx)
        module = compute_module(path)

        yield EnumItem(
            name=name,
            module=module,
            path=path.relative_to(Path.cwd()),
            start_line=start_line,
            doc=enum_doc,
            definition=definition,
            variants=variants,
        )


def collect_leading_doc(lines: List[str], start_index: int) -> List[str]:
    docs: List[str] = []
    i = start_index - 1
    while i >= 0:
        stripped = lines[i].strip()
        if stripped.startswith("///"):
            docs.append(stripped[3:].strip())
        elif stripped == "":
            i -= 1
            continue
        else:
            break
        i -= 1
    docs.reverse()
    return docs


def collect_block(lines: List[str], start_index: int) -> tuple[str, int]:
    buffer: List[str] = []
    brace_depth = 0
    started = False
    end_index = start_index
    for idx in range(start_index, len(lines)):
        line = lines[idx]
        buffer.append(line)
        if not started:
            if "{" in line:
                brace_depth += line.count("{") - line.count("}")
                started = True
        else:
            brace_depth += line.count("{") - line.count("}")
        if started and brace_depth <= 0:
            end_index = idx
            break
    definition = "\n".join(buffer).strip()
    return definition, end_index


def collect_variants(lines: List[str], start_index: int, end_index: int) -> List[Variant]:
    variants: List[Variant] = []
    brace_depth = 0
    inside_enum = False
    pending_doc: List[str] = []
    idx = start_index
    while idx <= end_index:
        line = lines[idx]
        stripped = line.strip()

        if not inside_enum:
            if "{" in line:
                inside_enum = True
                brace_depth += line.count("{") - line.count("}")
        else:
            if stripped.startswith("///"):
                pending_doc.append(stripped[3:].strip())
                brace_depth += line.count("{") - line.count("}")
                idx += 1
                continue
            if stripped.startswith("#["):
                pending_doc.append(stripped)
                brace_depth += line.count("{") - line.count("}")
                idx += 1
                continue
            if brace_depth == 1 and stripped and not stripped.startswith("//"):
                match = re.match(r"([A-Z][A-Za-z0-9_]*)", stripped)
                if match:
                    variant_name = match.group(1)
                    variant_def, consumed = collect_variant_definition(lines, idx)
                    variants.append(
                        Variant(
                            name=variant_name,
                            start_line=idx + 1,
                            doc=pending_doc,
                            definition=variant_def,
                        )
                    )
                    pending_doc = []
                    brace_depth += sum(line.count("{") - line.count("}") for line in lines[idx:idx+consumed])
                    idx += consumed
                    continue
            brace_depth += line.count("{") - line.count("}")
        idx += 1
    return variants


def collect_variant_definition(lines: List[str], start_index: int) -> tuple[str, int]:
    buffer: List[str] = []
    local_brace = 0
    local_paren = 0
    consumed = 0
    idx = start_index
    while idx < len(lines):
        line = lines[idx]
        buffer.append(line)
        stripped = line.strip()
        local_brace += line.count("{") - line.count("}")
        local_paren += line.count("(") - line.count(")")
        consumed += 1
        if local_brace <= 0 and local_paren <= 0 and stripped.endswith(","):
            break
        idx += 1
    definition = "\n".join(buffer).strip()
    return definition, consumed


def compute_module(path: Path) -> str:
    rel = path.relative_to(BASE)
    parts = list(rel.parts)
    if parts[-1] == "mod.rs":
        parts = parts[:-1]
    else:
        parts[-1] = parts[-1].removesuffix(".rs")
    return "::".join(part for part in parts if part)


def enum_to_dict(item: EnumItem) -> dict:
    return {
        "kind": "enum",
        "name": item.name,
        "module": item.module,
        "path": str(item.path),
        "start_line": item.start_line,
        "doc": item.doc,
        "definition": item.definition,
        "variants": [
            {
                "name": variant.name,
                "start_line": variant.start_line,
                "doc": variant.doc,
                "definition": variant.definition,
            }
            for variant in item.variants
        ],
    }


if __name__ == "__main__":
    main()
