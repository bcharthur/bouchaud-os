#!/usr/bin/env python3
"""BOUCHAUD_RAPPORT_GEL_V1 (#283) -- rapport automatique d'un noyau fige.

    rapport_gel.py JOURNAL_SERIE [NOYAU_ELF]

Lit la DERNIERE ligne `[DIAG-EN-SOUFFRANCE]` du journal serie (la seule qui
sort quand tous les coeurs sont figes), la decoupe champ par champ et, si le
noyau ELF du MEME SHA est fourni, symbolise par `addr2line` :

  * le RIP que chaque coeur a donne a la NMI (BOUCHAUD_RELEVE_NMI_V1) et les
    adresses de code trouvees sur sa pile ;
  * le RIP du dernier IPI recu (perime si l'IPI est ancien : c'est dit).

Il repond aux questions d'un gel, une ligne `RAPPORT_GEL` chacune :
  verdict      noyau_fige | pas_de_ligne_noyau | indetermine
  coeur        pour chaque coeur : tache courante, attente de verrou, NMI
  verrou       verrou attendu, coeur detenteur, site d'attente, site de prise
  fautif       le detenteur du verrou attendu depuis le plus longtemps
  progression  l'IRQ du minuteur progresse-t-elle (lignes successives) ?

Sans ligne de souffrance, le rapport ne conclut RIEN sur le noyau : il le dit
(`indetermine`). L'absence de ligne n'est PAS une preuve d'ordonnancement :
un coeur 0 fige interruptions masquees n'en emet aucune (gel total du 17/11,
tempete fork+exec) -- sauf si les registres vus par QEMU (`[QEMU-CPU]`, releve_cpus_qemu.py, lus
aussi dans `JOURNAL.cpus`) sont la : `qemu_verdict=boucle_noyau` (meme RIP
noyau a 1 s d'intervalle hors HLT), `vm_au_repos` (tous en HLT) ou
`vm_active`.
Le code de retour est toujours 0 : c'est un releve, pas un verdict de test.
"""
from __future__ import annotations

import re
import shutil
import subprocess
import sys
from dataclasses import dataclass, field

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LIGNE = "[DIAG-EN-SOUFFRANCE]"


@dataclass
class Attente:
    verrou: int
    age: int
    proprio: int
    site: str
    site_detenteur: str


@dataclass
class Nmi:
    rip: int
    mode: str
    si: int
    pile: list = field(default_factory=list)


@dataclass
class Releve:
    t: int
    demande_ms: int
    cur: list
    sites: list
    spin: list
    ipi: list
    nmi: list


def _hex(x: str) -> int:
    return int(x, 16)


def decoupe(ligne: str) -> Releve | None:
    ligne = ANSI.sub("", ligne)
    if LIGNE not in ligne:
        return None
    corps = ligne[ligne.index(LIGNE):]
    def champ(nom):
        m = re.search(nom + r"=\[([^\]]*)\]", corps)
        return m.group(1) if m else None
    t = re.search(r" t=(\d+)", corps)
    d = re.search(r"demande_ms=(\d+)", corps)
    cur = [int(x) for x in (champ("cur") or "").split(",") if x.strip().lstrip("-").isdigit()]
    sites = [int(x) for x in (champ("site") or "").split() if x.isdigit()]
    spin = []
    for e in (champ("spin") or "").split():
        m = re.fullmatch(r"(0x[0-9a-f]+)/(\d+)(?:/p(-?\d+)@([^<\s]+?)(?::(\d+))?(?:<([^:\s]+):(\d+))?)?", e)
        if not m:
            spin.append(None)
            continue
        verrou = _hex(m.group(1))
        if verrou == 0:
            spin.append(None)
            continue
        spin.append(Attente(
            verrou, int(m.group(2)), int(m.group(3) or -1),
            f"{m.group(4)}:{m.group(5)}" if m.group(4) else "?",
            f"{m.group(6)}:{m.group(7)}" if m.group(6) else "?",
        ))
    ipi = []
    for e in (champ("ipi") or "").split():
        m = re.fullmatch(r"(\d+)/(0x[0-9a-f]+)/u(\d)", e)
        ipi.append((int(m.group(1)), _hex(m.group(2)), int(m.group(3))) if m else None)
    nmi = [None] * 4
    for e in (champ("nmi") or "").split():
        m = re.fullmatch(r"c(\d+):(?:(0x[0-9a-f]+)/([ku])/if([01]):?([0-9a-fx,]*))?(-)?", e)
        if not m or m.group(6) or not m.group(2):
            continue
        cpu = int(m.group(1))
        if cpu < 4:
            pile = [_hex(x) for x in m.group(5).split(",") if x]
            nmi[cpu] = Nmi(_hex(m.group(2)), m.group(3), int(m.group(4)), pile)
    return Releve(int(t.group(1)) if t else 0, int(d.group(1)) if d else 0,
                  cur, sites, spin, ipi, nmi)


@dataclass
class CpuQemu:
    ech: int
    cpu: int
    rip: int
    cpl: int
    si: int
    hlt: int
    pile: list = field(default_factory=list)


def cpus_qemu(texte: str) -> list:
    """Lignes [QEMU-CPU] de releve_cpus_qemu.py (BOUCHAUD_RELEVE_CPUS_QEMU_V1)."""
    out = []
    for m in re.finditer(r"\[QEMU-CPU\] ech=(\d+) cpu=(\d+) rip=(0x[0-9a-f]+) cpl=(-?\d+) "
                         r"if=(-?\d+) hlt=(-?\d+) rsp=0x[0-9a-f]+ pile=\[([0-9a-fx,]*)\]", texte):
        out.append(CpuQemu(int(m.group(1)), int(m.group(2)), _hex(m.group(3)), int(m.group(4)),
                           int(m.group(5)), int(m.group(6)),
                           [_hex(x) for x in m.group(7).split(",") if x]))
    return out


def lecture_qemu(cpus: list, s) -> list:
    """Lignes de rapport tirees des registres vus par QEMU."""
    if not cpus:
        return []
    out = []
    par_cpu = {}
    for c in cpus:
        par_cpu.setdefault(c.cpu, {})[c.ech] = c
    boucles = []
    for cpu in sorted(par_cpu):
        e = par_cpu[cpu]
        a, b = e.get(1), e.get(2)
        dernier = b or a
        immobile = a is not None and b is not None and a.rip == b.rip and a.hlt == 0 and b.hlt == 0
        if immobile and dernier.cpl == 0:
            boucles.append(dernier)
        pile = " < ".join(s(x) for x in dernier.pile if s(x) != "?") or "-"
        out.append(f"RAPPORT_GEL qemu coeur={cpu} rip={dernier.rip:#x} [{s(dernier.rip)}] "
                   f"{'noyau' if dernier.cpl == 0 else 'utilisateur' if dernier.cpl == 3 else '?'} "
                   f"if={dernier.si} hlt={dernier.hlt} immobile={'oui' if immobile else 'non'} pile: {pile}")
    if boucles:
        for c in boucles:
            out.append(f"RAPPORT_GEL qemu_verdict=boucle_noyau coeur={c.cpu} rip={c.rip:#x} [{s(c.rip)}] if={c.si} "
                       f"(meme RIP noyau a 1 s d'intervalle, hors HLT)")
    elif all((e.get(2) or e.get(1)).hlt == 1 for e in par_cpu.values()):
        out.append("RAPPORT_GEL qemu_verdict=vm_au_repos tous les vCPU en HLT : aucune boucle, la machine "
                   "attend un reveil qui ne vient pas (cote navigateur/IPC ou reveil perdu)")
    else:
        out.append("RAPPORT_GEL qemu_verdict=vm_active des vCPU progressent : pas de boucle noyau immobile")
    return out


def releves(texte: str) -> list:
    return [r for r in (decoupe(l) for l in texte.splitlines()) if r]


def noms_des_taches(texte: str) -> dict:
    """idx -> (tid, pid, nom) : la DERNIERE affectation de chaque emplacement."""
    noms = {}
    for m in re.finditer(r"\[SMP-TASK\] idx=(\d+) tid=(\d+) pid=(\d+) .*? name=(\S+)", ANSI.sub("", texte)):
        noms[int(m.group(1))] = (int(m.group(2)), int(m.group(3)), m.group(4))
    return noms


def symbolise(elf: str | None, adresses: list) -> dict:
    """adresse -> 'fonction (fichier:ligne)' ; vide sans ELF ni addr2line."""
    adresses = sorted({a for a in adresses if a})
    if not elf or not adresses:
        return {}
    outil = shutil.which("addr2line") or shutil.which("llvm-addr2line")
    if not outil:
        return {}
    try:
        sortie = subprocess.run(
            [outil, "-f", "-C", "-e", elf] + [hex(a) for a in adresses],
            capture_output=True, text=True, timeout=120, check=False,
        ).stdout.splitlines()
    except (OSError, subprocess.TimeoutExpired):
        return {}
    noms = {}
    for i, a in enumerate(adresses):
        if 2 * i + 1 >= len(sortie):
            break
        fonction, ou = sortie[2 * i], sortie[2 * i + 1].rsplit("/", 1)[-1]
        if fonction != "??":
            noms[a] = f"{fonction} ({ou})"
    return noms


def rapport(texte: str, elf: str | None = None) -> list:
    tous = releves(texte)
    qemu = cpus_qemu(texte)
    if not tous:
        sym_q = symbolise(elf, [c.rip for c in qemu] + [x for c in qemu for x in c.pile])
        def sq(a):
            return sym_q.get(a, "?") if a else "-"
        if qemu:
            return (["RAPPORT_GEL verdict=pas_de_ligne_noyau raison=aucune_ligne_DIAG-EN-SOUFFRANCE "
                     "(noyau qui ordonnance encore, OU coeur 0 fige IF=0 : lire les registres QEMU)"]
                    + lecture_qemu(qemu, sq))
        return ["RAPPORT_GEL verdict=indetermine raison=aucune_ligne_DIAG-EN-SOUFFRANCE_ni_registres_QEMU "
                "(preuves insuffisantes : un coeur 0 fige IF=0 n'emet aucune ligne ; "
                "relever les vCPU par le moniteur QEMU)"]
    r = tous[-1]
    noms = noms_des_taches(texte)
    a_symboliser = []
    for n in r.nmi:
        if n:
            a_symboliser += [n.rip] + n.pile
    for i in r.ipi:
        if i:
            a_symboliser.append(i[1])
    a_symboliser += [c.rip for c in qemu] + [x for c in qemu for x in c.pile]
    sym = symbolise(elf, a_symboliser)
    def s(a):
        return sym.get(a, "?") if a else "-"
    out = [f"RAPPORT_GEL verdict=noyau_fige t_ms={r.t} demande_non_servie_ms={r.demande_ms} "
           f"releves={len(tous)} elf={'oui' if elf and sym else 'non'}"]
    if len(tous) >= 2:
        out.append(f"RAPPORT_GEL progression irq_minuteur=vivante (t {tous[-2].t} -> {r.t}) ; "
                   f"le coeur 0 sert encore ses interruptions")
    for cpu in range(max(len(r.cur), 4)):
        idx = r.cur[cpu] if cpu < len(r.cur) else -1
        tache = "repos" if idx < 0 else "idx=%d %s" % (idx, "tid=%d pid=%d %s" % noms[idx] if idx in noms else "?")
        site = r.sites[cpu] if cpu < len(r.sites) else 0
        att = r.spin[cpu] if cpu < len(r.spin) else None
        n = r.nmi[cpu] if cpu < len(r.nmi) else None
        ip = r.ipi[cpu] if cpu < len(r.ipi) else None
        attente = (f"attend verrou={att.verrou:#x} depuis={att.age}tics detenteur=coeur{att.proprio} "
                   f"site_attente={att.site} site_prise={att.site_detenteur}") if att else "aucune_attente_de_verrou"
        if n:
            pile = " < ".join(s(a) for a in n.pile if sym.get(a)) or "-"
            nmi = f"nmi_rip={n.rip:#x} [{s(n.rip)}] mode={'noyau' if n.mode == 'k' else 'utilisateur'} if={n.si} pile: {pile}"
        else:
            nmi = "nmi=sans_reponse"
        ipi_txt = f"dernier_ipi_age={ip[0]}tics rip={ip[1]:#x} [{s(ip[1])}]" if ip else "dernier_ipi=?"
        out.append(f"RAPPORT_GEL coeur={cpu} tache={tache} site_noyau={site} {attente} {nmi} {ipi_txt}")
    attentes = [(a.age, c, a) for c, a in enumerate(r.spin) if a]
    if attentes:
        age, c, a = max(attentes, key=lambda x: x[0])
        n = r.nmi[a.proprio] if 0 <= a.proprio < len(r.nmi) else None
        ou = f"{n.rip:#x} [{s(n.rip)}]" if n else "inconnu (pas de reponse NMI)"
        out.append(f"RAPPORT_GEL fautif coeur{a.proprio} tient verrou={a.verrou:#x} pris_en={a.site_detenteur} "
                   f"attendu_par=coeur{c} depuis={age}tics ; le detenteur s'execute en {ou}")
        # A spin can cover multiple instructions. Correlate the lock owner
        # with both QEMU samples by symbol, independently of RIP immobility.
        if n and n.mode == "k" and n.rip in sym:
            samples = {sample.ech: sample for sample in qemu if sample.cpu == a.proprio}
            function = s(n.rip).split(" (", 1)[0]
            if all(
                (sample := samples.get(index)) is not None
                and sample.cpl == 0 and sample.hlt == 0
                and s(sample.rip).split(" (", 1)[0] == function
                for index in (1, 2)
            ):
                out.append(f"RAPPORT_GEL qemu_detenteur_confirme coeur={a.proprio} [{function}]")

    else:
        out.append("RAPPORT_GEL fautif=inconnu aucune attente de verrou publiee "
                   "(boucle sans verrou, ou attente non instrumentee : lire les RIP NMI)")
    out += lecture_qemu(qemu, s)
    return out


def main(argv: list) -> int:
    if len(argv) < 2:
        print(__doc__)
        return 2
    try:
        texte = open(argv[1], errors="replace").read()
    except OSError as e:
        print(f"RAPPORT_GEL verdict=journal_illisible {e}")
        return 0
    # Registres vus par QEMU (releve_cpus_qemu.py), deposes a cote du journal.
    try:
        texte += "\n" + open(argv[1] + ".cpus", errors="replace").read()
    except OSError:
        pass
    for ligne in rapport(texte, argv[2] if len(argv) > 2 else None):
        print(ligne)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
