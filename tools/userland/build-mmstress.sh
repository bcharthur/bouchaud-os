#!/bin/sh
set -eu
CC="${CC:-x86_64-linux-musl-gcc}"
BASE=0x400000000000
LIMIT=0x600000000000

# Bouchaud accepts ET_EXEC only inside its dedicated user PML4 window. Keep the
# pthread/futex musl ABI, but use the same fixed-address fallback as build.sh.
"$CC" -O2 -static -pthread -mcmodel=large -fno-pie -no-pie \
    -Wl,-Ttext-segment=$BASE -Wl,--no-relax -Wl,-z,noexecstack \
    tools/userland/mmstress.c -o mmstress

REPORT="${ELF_REPORT:-mmstress.readelf.txt}"
{
    readelf -h mmstress
    printf '\n'
    readelf -W -l mmstress
} > "$REPORT"

python3 - "$REPORT" "$BASE" "$LIMIT" <<'PY'
import re
import sys
from pathlib import Path

report = Path(sys.argv[1]).read_text(encoding="utf-8")
base, limit = int(sys.argv[2], 0), int(sys.argv[3], 0)
if not re.search(r"Machine:\s+Advanced Micro Devices X86-64", report):
    raise SystemExit("mmstress: ELF is not x86-64")
if re.search(r"^\s*INTERP\s", report, re.MULTILINE):
    raise SystemExit("mmstress: PT_INTERP is forbidden")
loads = []
for line in report.splitlines():
    fields = line.split()
    if fields and fields[0] == "LOAD":
        virt, memsz = int(fields[2], 16), int(fields[5], 16)
        loads.append((virt, virt + memsz))
if not loads:
    raise SystemExit("mmstress: no PT_LOAD")
for start, end in loads:
    if start < base or end > limit or end < start:
        raise SystemExit(
            f"mmstress: PT_LOAD {start:#x}..{end:#x} outside "
            f"Bouchaud window {base:#x}..{limit:#x}"
        )
print("mmstress PT_LOAD:", ", ".join(f"{a:#x}..{b:#x}" for a, b in loads))
PY

echo "built: mmstress (ELF report: $REPORT)"
