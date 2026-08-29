#!/usr/bin/env python3
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]

FACADES = {
    ROOT / "src/kernel/process/thread.rs": "thread",
    ROOT / "src/kernel/sync/bkl/acquisition.rs": "acquisition",
    ROOT / "src/fs/persistance.rs": "persistance",
    ROOT / "src/arch/x86_64/idt.rs": "idt",
}

def main() -> int:
    errors = []
    includes = 0

    for facade, name in FACADES.items():
        if not facade.exists():
            errors.append(f"{name}: façade absente: {facade}")
            continue
        text = facade.read_text(encoding="utf-8")
        for rel in re.findall(r'include!\("([^"]+)"\);', text):
            includes += 1
            target = facade.parent / rel
            if not target.exists():
                errors.append(f"{name}: fragment absent: {target}")
                continue
            frag = target.read_text(encoding="utf-8")
            if re.search(r"(?m)^\s*//!", frag):
                errors.append(f"{name}: inner rustdoc //! interdit dans {target}")

    if errors:
        print("FRAGMENTATION V11C: ECHEC")
        for error in errors:
            print(" -", error)
        return 1

    print(f"FRAGMENTATION V11C: OK — {includes} include! vérifiés")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
