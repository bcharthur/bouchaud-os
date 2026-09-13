#!/usr/bin/env python3
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]

checks = {
    "src/kernel/process/thread/lifecycle.rs": [
        "BOUCHAUD_V16_2_ZOMBIE_RETIRE_NONRETURNING",
        "fn retire_exec_zombie_current() -> !",
        "retire_exec_zombie_current();",
    ],
    "src/drivers/serial/uart16550.rs": [
        "BOUCHAUD_V16_2_SERIAL_FORMAT_BUFFER",
        "const FORMAT_BUFFER_SIZE: usize = 2048;",
        "outb(COM1 + 0, 0x01)",
        "pub fn ecris_octets_sans_prefixe",
        "struct TamponFormat",
    ],
    "src/kernel/debug/journal.rs": [
        "BOUCHAUD_V16_2_PREFIX_BUFFER",
        "struct Prefixe",
        "ecris_octets_sans_prefixe(out.octets())",
    ],
    "src/gui/politique.rs": [
        "BOUCHAUD_V16_2_TELEMETRY_CADENCE",
        "pub const PERIODE_RELEVE_MS: u64 = 30_000;",
    ],
}

# LA CADENCE DE TRAME EST UN PLAFOND, PAS UNE VALEUR GRAVEE.
#
# Ce controle exigeait le litteral `PERIODE_TRAME_MS: u64 = 16;`. Il ne
# protegeait rien du contrat V16.2 -- qui porte sur la cadence des RELEVES --
# et interdisait au passage d'aller au-dela de soixante-deux images par
# seconde. Ce qui compte est que la constante existe et ne DESCENDE pas en
# dessous de soixante hertz : un bureau plafonne plus bas se sent, et c'est
# exactement ce que l'utilisateur decrit.
PLAFOND_TRAME_MS = 16

errors = []

politique = (ROOT / "src/gui/politique.rs").read_text(encoding="utf-8")
m = re.search(r"pub const PERIODE_TRAME_MS: u64 = ([0-9_]+);", politique)
if m is None:
    errors.append("politique.rs: PERIODE_TRAME_MS n'est plus lisible")
elif int(m.group(1).replace("_", "")) > PLAFOND_TRAME_MS:
    errors.append(
        "politique.rs: le compositeur est plafonne sous soixante images par "
        "seconde (PERIODE_TRAME_MS=%s ms)" % m.group(1)
    )

for rel, tokens in checks.items():
    p = ROOT / rel
    if not p.exists():
        errors.append(f"absent: {rel}")
        continue
    text = p.read_text(encoding="utf-8")
    for token in tokens:
        if token not in text:
            errors.append(f"{rel}: token absent: {token}")

life = (ROOT / "src/kernel/process/thread/lifecycle.rs").read_text(encoding="utf-8")
if 'panic!("zombie task resumed after exec quiescence")' in life:
    errors.append("lifecycle: ancien panic exec-zombie encore present")

serial = (ROOT / "src/drivers/serial/uart16550.rs").read_text(encoding="utf-8")
if "outb(COM1 + 0, 0x03)" in serial:
    errors.append("serial: ancien diviseur 38400 encore present")

if errors:
    print("V16.2 contracts: ECHEC")
    for e in errors:
        print(" -", e)
    sys.exit(1)

print("V16.2 kernel fluidity + zombie retirement contracts: OK")
