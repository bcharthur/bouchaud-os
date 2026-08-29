#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path

SITES = {
    600: "try_enter: début",
    601: "try_enter: lecture OWNER",
    602: "try_enter: avant helper nouvel entrant",
    610: "helper: contrôle RESUME avant CAS",
    611: "helper: contrôle HANDOFF avant CAS",
    612: "helper: juste avant CAS OWNER",
    613: "helper: CAS réussi, OWNER acquis",
    614: "helper: contrôle RESUME après CAS",
    615: "helper: contrôle HANDOFF après CAS",
    616: "helper: avant claim handoff",
    617: "helper: claim handoff terminé",
    618: "helper: refus/rollback",
    620: "try_enter: helper revenu",
    621: "try_enter: profondeur lue",
    622: "try_enter: DEPTH=1 publié",
    623: "try_enter: avant probe_note_acquire",
    624: "try_enter: probe_note_acquire terminé",
    625: "try_enter: avant flight recorder",
    626: "try_enter: flight recorder terminé",
    627: "try_enter: retour KernelGuard",
    630: "try_enter: chemin réentrant",
    631: "try_enter: réentrance DEPTH++ terminée",
    639: "try_enter: retour réentrant",
    640: "try_enter: OWNER occupé",
    641: "try_enter: helper refusé",
    650: "try_enter_depuis_zero: lecture OWNER",
    651: "try_enter_depuis_zero: OWNER occupé",
    652: "try_enter_depuis_zero: avant helper",
    653: "try_enter_depuis_zero: helper refusé",
    660: "try_enter_depuis_zero: helper réussi",
    661: "try_enter_depuis_zero: DEPTH=1",
    662: "try_enter_depuis_zero: probe terminé",
    663: "try_enter_depuis_zero: retour",
}

RE_SITE = re.compile(r"site=\[([0-9]+):")
RE_HOLD = re.compile(r"\[SMP-PROV\].*held=([0-9]+)ms")
RE_BKL = re.compile(
    r"\[BKL-COMPTES\].*parks=([0-9]+).*wake_ipis=([0-9]+).*reveils_sans_acq=([0-9]+)"
)
RE_RESUME = re.compile(r"reprise_max_ns=([0-9]+)")
RE_HANDOFF = re.compile(
    r"\[BKL-HANDOFF\].*prepared=([0-9]+).*claims=([0-9]+).*deferrals=([0-9]+).*expired=([0-9]+)"
)
RE_SYSCALL = re.compile(r"\[BKL-SYSCALL\](.*)")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-v11c.py <log>")
        return 2

    lines = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace").splitlines()
    stalls = [line for line in lines if "[SMP-STALL]" in line]
    max_hold = 0
    last_site = None
    last_bkl = None
    last_resume = 0
    last_handoff = None
    last_hot = None

    for line in lines:
        if "[SMP-PROV]" in line:
            m = RE_HOLD.search(line)
            if m:
                max_hold = max(max_hold, int(m.group(1)))
        if "[SMP-STALL]" in line:
            m = RE_SITE.search(line)
            if m:
                site = int(m.group(1))
                if 600 <= site <= 699:
                    last_site = site
        if m := RE_BKL.search(line):
            last_bkl = tuple(int(v) for v in m.groups())
        if "[BKL-COMPTES]" in line:
            if m := RE_RESUME.search(line):
                last_resume = max(last_resume, int(m.group(1)))
        if m := RE_HANDOFF.search(line):
            last_handoff = tuple(int(v) for v in m.groups())
        if m := RE_SYSCALL.search(line):
            last_hot = m.group(1).strip()

    print("=== Bouchaud OS Final V11C ===")
    print(f"SMP-STALL                 : {len(stalls)}")
    print(f"max held observé          : {max_hold} ms")
    print(f"resume max                : {last_resume / 1e6:.3f} ms")

    if last_bkl:
        parks, ipis, useless = last_bkl
        print(f"parks / wake_ipis / inutiles: {parks} / {ipis} / {useless}")

    if last_handoff:
        prepared, claims, deferrals, expired = last_handoff
        print(f"handoff prep/claim/def/exp : {prepared}/{claims}/{deferrals}/{expired}")

    if last_hot:
        print("dernier classement BKL :")
        print("  " + last_hot)

    if last_site is not None:
        print(f"dernier site acquisition : {last_site} — {SITES.get(last_site, 'inconnu')}")
        if last_site == 612:
            print("verdict acquisition: bloqué autour du CAS OWNER")
        elif 613 <= last_site <= 617:
            print("verdict acquisition: OWNER acquis, regarder post-CAS/handoff")
        elif last_site in (623, 624):
            print("verdict acquisition: regarder probe/provenance")
        elif last_site in (625, 626):
            print("verdict acquisition: regarder flight recorder")
    else:
        print("aucun site 600..699 dans un SMP-STALL")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
