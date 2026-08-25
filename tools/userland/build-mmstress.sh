#!/bin/sh
set -eu
CC="${CC:-x86_64-linux-musl-gcc}"
BASE=0x400000000000
LIMIT=0x600000000000
LOAD_BASE=0x400000400000

# Match build.sh's normal musl path: Bouchaud relocates ET_DYN images to
# user_load_base(), avoiding the range-limited relocations in musl's startup
# objects that make a fixed high-address ET_EXEC fail to link.
"$CC" -O2 -static-pie -fPIE -pthread -Wl,-z,noexecstack \
    tools/userland/mmstress.c -o mmstress

REPORT="${ELF_REPORT:-mmstress.readelf.txt}"
{
    file mmstress
    printf '\n'
    readelf -h mmstress
    printf '\n'
    readelf -W -l mmstress
    printf '\n'
    readelf -d mmstress
} > "$REPORT"

python3 - "$REPORT" "$BASE" "$LIMIT" "$LOAD_BASE" <<'PY'
import re
import sys
from pathlib import Path

report = Path(sys.argv[1]).read_text(encoding="utf-8")
base, limit, load_base = (int(value, 0) for value in sys.argv[2:5])
if not re.search(r"Machine:\s+Advanced Micro Devices X86-64", report):
    raise SystemExit("mmstress: ELF is not x86-64")
kind = re.search(r"^\s*Type:\s+(DYN|EXEC)\b", report, re.MULTILINE)
if not kind:
    raise SystemExit("mmstress: ELF type must be ET_DYN or ET_EXEC")
if re.search(r"^\s*INTERP\s", report, re.MULTILINE):
    raise SystemExit("mmstress: PT_INTERP is forbidden")
if re.search(r"\(NEEDED\)", report):
    raise SystemExit("mmstress: host shared-library dependency is forbidden")
loads = []
for line in report.splitlines():
    fields = line.split()
    if fields and fields[0] == "LOAD":
        virt, memsz = int(fields[2], 16), int(fields[5], 16)
        if virt > (1 << 64) - 1 - memsz:
            raise SystemExit("mmstress: PT_LOAD virtual range overflows u64")
        loads.append((virt, virt + memsz))
if not loads:
    raise SystemExit("mmstress: no PT_LOAD")
effective = []
for raw_start, raw_end in loads:
    relocation = load_base if kind.group(1) == "DYN" else 0
    if raw_start > (1 << 64) - 1 - relocation or raw_end > (1 << 64) - 1 - relocation:
        raise SystemExit("mmstress: relocated PT_LOAD range overflows u64")
    start, end = relocation + raw_start, relocation + raw_end
    if start < base or end > limit or end < start:
        raise SystemExit(
            f"mmstress: effective PT_LOAD {start:#x}..{end:#x} outside "
            f"Bouchaud window {base:#x}..{limit:#x}"
        )
    effective.append((start, end))
print(f"mmstress type: ET_{kind.group(1)}")
print("mmstress raw PT_LOAD:", ", ".join(f"{a:#x}..{b:#x}" for a, b in loads))
print("mmstress effective PT_LOAD:", ", ".join(f"{a:#x}..{b:#x}" for a, b in effective))
PY

echo "built: mmstress (ELF report: $REPORT)"
