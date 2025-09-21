#!/usr/bin/env python3
"""Generate usage data for event variants across the repository."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List

REPO_ROOT = Path(__file__).resolve().parents[1]
INVENTORY_PATH = Path(__file__).with_name("EVENTS_INVENTORY.json")
OUTPUT_PATH = Path(__file__).with_name("event_variant_usage.json")


@dataclass
class Occurrence:
    path: Path
    line: int
    text: str

    def to_dict(self) -> Dict[str, str]:
        return {
            "path": str(self.path.relative_to(REPO_ROOT)),
            "line": self.line,
            "line_text": self.text.strip(),
        }


def load_inventory() -> List[Dict]:
    data = json.loads(INVENTORY_PATH.read_text(encoding="utf-8"))
    return data["items"]


def ripgrep(pattern: str) -> List[tuple[Path, int, str]]:
    cmd = [
        "rg",
        "--no-heading",
        "--line-number",
        pattern,
        str(REPO_ROOT),
    ]
    proc = subprocess.run(cmd, check=False, capture_output=True, text=True)
    results: List[tuple[Path, int, str]] = []
    if proc.returncode not in {0, 1}:  # 1 => no matches
        raise RuntimeError(proc.stderr)
    if proc.stdout.strip() == "":
        return []
    for line in proc.stdout.splitlines():
        path_str, line_no, text = line.split(":", 2)
        path = Path(path_str)
        try:
            rel = path.relative_to(REPO_ROOT)
        except ValueError:
            # Outside repo root – skip
            continue
        if rel.parts and rel.parts[0] in {"refactor", "target"}:
            continue
        results.append((path, int(line_no), text))
    return results


def main() -> None:
    inventory = load_inventory()
    usage_report: List[Dict] = []
    for item in inventory:
        module = item["module"]
        name = item["name"]
        module_path = REPO_ROOT / item["path"]
        module_path = module_path.resolve()

        for variant in item["variants"]:
            variant_name = variant["name"]
            pattern = f"{name}::{variant_name}"
            hits = []
            for path, line, text in ripgrep(pattern):
                if path.resolve() == module_path:
                    continue  # Skip definition site
                hits.append(Occurrence(path, line, text))

            if hits:
                usage_report.append(
                    {
                        "domain": module,
                        "event_type": name,
                        "variant": variant_name,
                        "occurrences": [hit.to_dict() for hit in hits],
                    }
                )
            else:
                usage_report.append(
                    {
                        "domain": module,
                        "event_type": name,
                        "variant": variant_name,
                        "occurrences": [],
                    }
                )

    OUTPUT_PATH.write_text(json.dumps(usage_report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
