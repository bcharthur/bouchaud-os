#!/usr/bin/env python3
from pathlib import Path
import re
import sys

errors = []
root = Path("src/kernel/native")
required = [
    root / "abi/mod.rs",
    root / "abi/dispatch.rs",
    root / "abi/numbers.rs",
    root / "abi/types.rs",
    root / "handle/table.rs",
    root / "handle/registry.rs",
    root / "ipc/channel.rs",
    root / "ipc/message.rs",
    root / "event.rs",
    root / "waitset.rs",
    root / "shm.rs",
]
for path in required:
    if not path.is_file():
        errors.append(f"module absent: {path}")

native_text = "\n".join(
    p.read_text(encoding="utf-8", errors="replace")
    for p in root.rglob("*.rs")
)
for forbidden in ("crate::kernel::smp_lock", "smp_lock::enter", "Domaine::Syscall"):
    if forbidden in native_text:
        errors.append(f"dependance BKL interdite dans native/: {forbidden}")

# The native core must not be implemented on top of the Linux compatibility ABI.
if "crate::kernel::abi::" in native_text:
    errors.append("native/: depend de kernel::abi (Linux compat)")

usermode = Path("src/arch/x86_64/usermode.rs").read_text(encoding="utf-8")
if "BOUCHAUD_NATIVE_ABI_V1" not in usermode:
    errors.append("usermode: routage ABI natif absent")
if "native::abi::is_native_syscall" not in usermode:
    errors.append("usermode: detection namespace natif absente")
if "let sans_verrou = native" not in usermode:
    errors.append("usermode: les appels natifs ne sont pas explicitement hors BKL")

legacy = Path("src/kernel/object/handle.rs").read_text(encoding="utf-8")
legacy_code = '\n'.join(line.split('//', 1)[0] for line in legacy.splitlines())
if re.search(r"\bstatic\s+mut\b", legacy_code):
    errors.append("object/handle.rs: static mut legacy encore present")

numbers = Path("src/kernel/native/abi/types.rs").read_text(encoding="utf-8")
if "0x424f_0000" not in numbers:
    errors.append("namespace ABI natif inattendu")

if errors:
    print("\n".join("ECHEC: " + error for error in errors), file=sys.stderr)
    raise SystemExit(1)

print("ABI_NATIVE_OK")
