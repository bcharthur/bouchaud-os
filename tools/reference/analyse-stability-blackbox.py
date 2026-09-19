#!/usr/bin/env python3
from pathlib import Path
import argparse
import re
import sys

ap = argparse.ArgumentParser(description="Verifie les criteres physiques HID/M11 d'une blackbox extraite")
ap.add_argument("session", help="dossier de session extrait contenant serial.log/fatal.log/terminal.log/samples.log")
a = ap.parse_args()
root = Path(a.session)

def read(name):
    p = root / name
    return p.read_text(encoding="utf-8", errors="replace") if p.exists() else ""

serial = read("serial.log")
fatal = read("fatal.log")
terminal = read("terminal.log")
samples = read("samples.log")

ready = list(re.finditer(r"M11_TAB_STAGE 80 READY attempt=(\d+) page=(\d+)", serial))
host = {}
for m in re.finditer(r"M11_HOST_TAB_STAGE 40 VIEW_REGISTERED page=(\d+)", serial):
    host.setdefault(m.group(1), m.start())
create = {}
for m in re.finditer(r"M11_TAB_STAGE 40 CREATE_PAGE_BEGIN attempt=(\d+) page=(\d+)", serial):
    create.setdefault(m.group(2), m.start())

order_errors=[]
for page,pos in create.items():
    hp=host.get(page)
    if hp is None:
        order_errors.append(f"page {page}: VIEW_REGISTERED absent")
    elif hp >= pos:
        order_errors.append(f"page {page}: VIEW_REGISTERED arrive apres CREATE_PAGE")

print(f"fatal_records_text={1 if fatal.strip() else 0}")
print(f"tabs_stage80={len(ready)}")
print(f"host_registered={len(host)} create_page={len(create)} order_errors={len(order_errors)}")
for e in order_errors:
    print("ORDER_ERROR", e)

markers = [
    "BOUCHAUD_HID_SENTINEL_EP0",
    "BOUCHAUD_HID_SENTINEL_ACTIVITY",
    "BOUCHAUD_HID_STALL_CONFIRMED",
    "BOUCHAUD_HID_FAILOVER_EP0",
    "BOUCHAUD_HID_STOP_OK",
    "BOUCHAUD_HID_DEQUEUE_RESET",
    "BOUCHAUD_HID_REARMED",
    "BOUCHAUD_HID_INTERRUPT_BACK",
]
for marker in markers:
    print(f"{marker}={serial.count(marker)}")

cmds = sum(1 for l in terminal.splitlines() if l.startswith("TERM CMD "))
outs = sum(1 for l in terminal.splitlines() if l.startswith("TERM OUT "))
results = sum(1 for l in terminal.splitlines() if l.startswith("TERM RESULT "))
print(f"terminal_commands={cmds} terminal_outputs={outs} terminal_results={results}")
print(f"hid_endpoint_samples={sum(1 for l in samples.splitlines() if l.startswith('hid_ep '))}")

for url in ("example.com", "wikipedia.org", "google.com"):
    print(f"seen_{url}={1 if url in serial else 0}")

ok = not fatal.strip() and len(ready) >= 10 and not order_errors
if ok:
    print("ACCEPTANCE_M11_10_TABS_OK")
    raise SystemExit(0)
print("ACCEPTANCE_INCOMPLETE_OR_FAILED")
raise SystemExit(2)
