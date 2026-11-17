#!/usr/bin/env python3
"""BOUCHAUD_RELEVE_CPUS_QEMU_V1 (#283) -- ou s'execute chaque vCPU, vu de QEMU.

    releve_cpus_qemu.py MONITEUR_UNIX [SORTIE]

Le noyau ne peut plus rien dire quand le coeur 0 est fige interruptions
masquees : l'IRQ du minuteur n'arrive qu'a lui, et la ligne
[DIAG-EN-SOUFFRANCE] ne sort jamais (forme « silence total » de #283, job
132489983914). QEMU, lui, voit toujours l'etat de chaque vCPU. Par le
moniteur : `cpu N`, `info registers` (RIP, RSP, RFLAGS, CPL, HLT), puis
`x /24gx RSP` en mode noyau (les adresses de retour sur la pile). DEUX
echantillons a une seconde d'intervalle : un RIP noyau inchange, hors HLT,
est une boucle ; tous les vCPU en HLT, c'est une machine qui attend un reveil.

Une ligne par vCPU et par echantillon, sur la sortie et dans SORTIE :

    [QEMU-CPU] ech=1 cpu=0 rip=0x94a472 cpl=0 if=1 hlt=0 rsp=0x... pile=[0x...,...]

Outil de DIAGNOSTIC : rend toujours 0 ; `QEMU_CPUS_ABSENT raison=...` si le
moniteur ne repond pas. La symbolisation est faite par rapport_gel.py.
"""
import re
import socket
import sys
import time

IMAGE_MIN, IMAGE_MAX = 0x20_0000, 0x400_0000


class Moniteur:
    def __init__(self, chemin):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(5)
        self.s.connect(chemin)
        self._lit()

    def _lit(self, delai=0.6):
        fin = time.monotonic() + delai
        morceaux = []
        while time.monotonic() < fin:
            try:
                self.s.settimeout(max(0.05, fin - time.monotonic()))
                d = self.s.recv(65536)
                if not d:
                    break
                morceaux.append(d)
                if d.rstrip().endswith(b"(qemu)"):
                    break
            except socket.timeout:
                break
        return b"".join(morceaux).decode(errors="replace")

    def cmd(self, c, delai=1.5):
        self.s.sendall((c + "\n").encode())
        return self._lit(delai)


def registre(texte, nom):
    m = re.search(r"\b" + nom + r"=([0-9a-f]+)", texte)
    return int(m.group(1), 16) if m else None


def releve(mon, echantillon):
    lignes = []
    for cpu in range(64):
        r = mon.cmd(f"cpu {cpu}", 0.8)
        if "invalid" in r.lower() or "error" in r.lower():
            break
        regs = mon.cmd("info registers", 1.5)
        rip, rsp, rfl = registre(regs, "RIP"), registre(regs, "RSP"), registre(regs, "RFL")
        cpl = re.search(r"CPL=(\d)", regs)
        hlt = re.search(r"HLT=(\d)", regs)
        if rip is None:
            break
        cpl = int(cpl.group(1)) if cpl else -1
        pile = []
        if cpl == 0 and rsp:
            x = mon.cmd(f"x /24gx 0x{rsp:x}", 1.5)
            for mot in re.findall(r"0x([0-9a-f]{16})", x.split(":", 1)[-1] if ":" in x else x):
                v = int(mot, 16)
                if IMAGE_MIN <= v < IMAGE_MAX and v != rip:
                    pile.append(v)
        lignes.append(
            f"[QEMU-CPU] ech={echantillon} cpu={cpu} rip={rip:#x} cpl={cpl} "
            f"if={(rfl >> 9) & 1 if rfl is not None else -1} hlt={hlt.group(1) if hlt else -1} "
            f"rsp={rsp or 0:#x} pile=[{','.join(hex(v) for v in pile[:8])}]"
        )
    return lignes


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 0
    try:
        mon = Moniteur(argv[1])
        lignes = releve(mon, 1)
        time.sleep(1.0)
        lignes += releve(mon, 2)
    except OSError as exc:
        print(f"QEMU_CPUS_ABSENT raison=moniteur_injoignable ({exc})")
        return 0
    if not lignes:
        print("QEMU_CPUS_ABSENT raison=aucun_registre")
        return 0
    for l in lignes:
        print(l)
    if len(argv) > 2:
        try:
            with open(argv[2], "a") as f:
                f.write("\n".join(lignes) + "\n")
        except OSError:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
