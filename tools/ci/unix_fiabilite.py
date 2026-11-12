#!/usr/bin/env python3
"""Unix & Filesystem Reliability -- banc QEMU des commandes et du systeme de fichiers.

    unix_fiabilite.py BOOTIMAGE SORTIE [--kvm] [--scenario NOM]

Pour chaque ETAPE d'un scenario, le script `autorun` joue :

    echo UNIX_DEBUT <id>
    <commande>
    echo UNIX_FIN <id>
    echo UNIX_TEMOIN <id>      # commande temoin : le shell repond-il encore ?

et le banc classe chaque etape :

  PASS                la commande a rendu la main, sa sortie est conforme, le
                      temoin a repondu dans son budget ;
  FAIL                elle a rendu la main mais sa sortie n'est pas conforme ;
  KERNEL_PANIC        une panique pendant l'etape ;
  DEADLOCK_SUSPECTED  la machine vit (QEMU tourne) mais plus RIEN ne sort sur
                      la console pendant le budget : le shell ne repond plus ;
  TIMEOUT             la console avance mais l'etape n'aboutit pas dans son
                      budget ;
  NOT_RUN             une etape precedente a empeche celle-ci de s'executer.

Un TIMEOUT n'est jamais un succes ; une machine vivante dont le shell ne
repond plus non plus. Sur DEADLOCK_SUSPECTED, le banc demande un releve des
fils (Alt+SysRq) avant d'arreter la machine : la preuve reste dans le journal.

Rend 0 et `UNIX_FIABILITE_OK ...` si toutes les etapes sont PASS, sinon 1 et
`UNIX_FIABILITE_ECHEC ...`. Rapport detaille : SORTIE/rapport.json.
"""
from __future__ import annotations

import io
import json
import re
import socket
import subprocess
import sys
import tarfile
import time
from dataclasses import dataclass, field
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
PREFIXE = re.compile(r"^(\[[^\]]*\]){3} ?")
FATAL = re.compile(r"KERNEL PANIC|panicked at|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|LOCKDEP inversion")
# Lignes du noyau intercalees dans la sortie d'une commande (le journal est
# coupe par `journal off`, pas les traces directes) : `[kernel] ...`,
# `[SCHED-RESUME] ...`, `BOUCHAUD_X a=1`, `NVME_IO ...`. Elles ne sont pas la
# sortie de la commande et sont ecartees avant le jugement.
NOYAU = re.compile(r"^(\[[A-Za-z][^\]]*\]|[A-Z][A-Z0-9]*_[A-Z0-9_]+(\s|=|:|$))")
STATUTS = ("PASS", "FAIL", "TIMEOUT", "KERNEL_PANIC", "DEADLOCK_SUSPECTED", "NOT_RUN")


@dataclass
class Etape:
    id: str
    commande: str
    attendus: list[str] = field(default_factory=list)   # chaque motif doit apparaitre
    interdits: list[str] = field(default_factory=list)  # aucun ne doit apparaitre
    toutes: str | None = None      # CHAQUE ligne de sortie doit correspondre
    lignes: int | None = None      # nombre exact de lignes de sortie
    budget_s: float = 60.0


# --- Le scenario `find` (BOUCHAUD_FIND_PARCOURS_V1) ---------------------------

PROFONDEUR_TAR = 100     # /arbre/p/p/.../p (ustar : chemin <= 255 octets)
PROFONDEUR_SHELL = 300   # /tmp/abysse/x/x/... cree par le shell (chemin > 600)
LARGEUR = 1500           # /arbre/large/f0000 .. f1499
NOM_TAR = "t" * 100     # le plus long nom de base qu ustar sache porter
NOM_LONG = "n" * 200     # au-dela : cree par le shell (touch)


def donnees_find() -> dict[str, bytes | None]:
    """Fichiers (contenu) et repertoires (None) poses dans l'archive."""
    d: dict[str, bytes | None] = {}
    chemin = "arbre"
    d[chemin] = None
    for _ in range(PROFONDEUR_TAR):
        chemin += "/p"
        d[chemin] = None
    d[chemin + "/fond.txt"] = b"fond\n"
    d["arbre/large"] = None
    for k in range(LARGEUR):
        d[f"arbre/large/f{k:04d}"] = b"x\n"
    d["arbre/noms"] = None
    for nom in ("nom avec espaces.txt", "accentue-éà.txt", "-n", NOM_TAR):
        d[f"arbre/noms/{nom}"] = b"n\n"
    return d


def scenario_find() -> tuple[list[str], list[Etape]]:
    prelude = ["journal off", "cd /", "mkdir /tmp/abysse", "cd /tmp/abysse"]
    prelude += ["mkdir x", "cd x"] * PROFONDEUR_SHELL
    prelude += ["cd /", f"touch /arbre/noms/{NOM_LONG}"]
    profond = "/arbre" + "/p" * PROFONDEUR_TAR
    abysse = "/tmp/abysse" + "/x" * PROFONDEUR_SHELL
    e = [
        Etape("find-sans-argument", "find", attendus=[r"^/etc/hosts$"]),
        Etape("find-racine", "find /", attendus=[r"^/etc/passwd$", r"^/arbre/large/f1499$", re.escape(profond + "/fond.txt")]),
        Etape("find-etc", "find /etc", attendus=[r"^/etc/hosts$", r"^/etc/passwd$", r"^/etc/group$"], toutes=r"^/etc/"),
        Etape("find-bin", "find /bin", toutes=r"^(/bin/.*|find: /bin: introuvable)$"),
        Etape("find-home", "find /home", toutes=r"^/home/"),
        Etape("find-inexistant", "find /chemin/inexistant", attendus=[r"^find: /chemin/inexistant: introuvable$"], lignes=1),
        Etape("find-profond", "find /arbre/p", attendus=[re.escape(profond + "/fond.txt") + "$"], lignes=PROFONDEUR_TAR),
        Etape("find-large", "find /arbre/large", toutes=r"^/arbre/large/f[0-9]{4}$", lignes=LARGEUR),
        Etape("find-noms", "find /arbre/noms",
              attendus=[r"^/arbre/noms/nom avec espaces\.txt$", r"^/arbre/noms/accentue-éà\.txt$",
                        r"^/arbre/noms/-n$", "^/arbre/noms/" + NOM_TAR + "$", "^/arbre/noms/" + NOM_LONG + "$"], lignes=5),
        Etape("find-filtre", "find / f149", toutes=r"^/arbre/large/f149[0-9]$", lignes=10),
        Etape("find-abysse", "find /tmp/abysse", attendus=[re.escape(abysse) + "$"], lignes=PROFONDEUR_SHELL),
        Etape("find-redirige", "find /etc > /tmp/find-etc.txt", lignes=0),
        Etape("relit-redirection", "cat /tmp/find-etc.txt", attendus=[r"^/etc/hosts$", r"^/etc/passwd$"], toutes=r"^/etc/"),
        Etape("cree-puis-trouve", "touch /tmp/zz-temoin.txt", lignes=0),
        Etape("trouve-cree", "find /tmp zz-temoin", attendus=[r"^/tmp/zz-temoin\.txt$"], lignes=1),
        Etape("supprime", "rm /tmp/zz-temoin.txt", lignes=0),
        Etape("ne-trouve-plus", "find /tmp zz-temoin", lignes=0),
    ]
    return prelude, e


# --- Le scenario `commandes` (audit des commandes du shell) -----------------

PROFONDEUR_TREE = 1500


def donnees_commandes() -> dict[str, bytes | None]:
    d = donnees_find()
    # Un script qui se relance lui-meme : l'imbrication doit etre bornee.
    d["r.sh"] = b"echo tour\nrun /r.sh\n"
    return d


def scenario_commandes() -> tuple[list[str], list[Etape]]:
    prelude = ["journal off", "cd /", "mkdir /tmp/gouffre", "cd /tmp/gouffre"]
    prelude += ["mkdir y", "cd y"] * PROFONDEUR_TREE
    prelude += ["cd /"]
    e = [
        # BOUCHAUD_RUN_SANS_VERROU_V1 : `run` tenait le verrou du RAMFS pendant
        # tout le script ; la premiere ligne qui y touchait le reprenait.
        Etape("ecrit-script", "echo ls /etc > /tmp/s.sh", lignes=0),
        Etape("run-script-vfs", "run /tmp/s.sh", attendus=[r"^hosts$", r"^passwd$"]),
        Etape("run-recursif-borne", "run /r.sh", budget_s=120,
              attendus=[r"^run: /r\.sh: imbrication trop profonde \(max 8\)$"],
              toutes=r"^(tour|run: /r\.sh: imbrication trop profonde \(max 8\))$"),
        Etape("run-inexistant", "run /pas/de/script", attendus=[r"^run: /pas/de/script: introuvable$"], lignes=1),
        Etape("run-dossier", "run /etc", attendus=[r"^run: /etc: pas un fichier$"], lignes=1),
        # BOUCHAUD_TREE_ITERATIF_V1 : une ligne par niveau, indentation bornee ;
        # avant, 1 print! par niveau et une sortie quadratique (TIMEOUT sous KVM).
        Etape("tree-profond", "tree /tmp/gouffre", budget_s=180, lignes=PROFONDEUR_TREE + 1,
              attendus=[r"^/tmp/gouffre$", r"^ {64}\[niveau 1499\] \|- y/$"]),
        Etape("tree-inexistant", "tree /chemin/inexistant", attendus=[r"^tree: /chemin/inexistant: introuvable$"], lignes=1),
        Etape("ls-large", "ls /arbre/large", lignes=LARGEUR, toutes=r"^f[0-9]{4}"),
        Etape("ls-inexistant", "ls /chemin/inexistant", attendus=[r"^ls: introuvable$"], lignes=1),
        Etape("cat-inexistant", "cat /chemin/inexistant", attendus=[r"^cat: introuvable$"], lignes=1),
        Etape("cat-dossier", "cat /etc", attendus=[r"^cat: dossier$"], lignes=1),
        Etape("cat-hosts", "cat /etc/hosts", attendus=[r"^127\.0\.0\.1\s+localhost bouchaud$"], lignes=1),
        Etape("df", "df", attendus=[r"^ramfs /\s+RAMFS\s+[0-9]+/4096"]),
        Etape("ls-l-etc", "ls -l /etc", attendus=[r"hosts$", r"passwd$"]),
        Etape("stat-hosts", "stat /etc/hosts", attendus=[r"hosts"]),
        Etape("wc-hosts", "wc /etc/hosts"),
        Etape("head-passwd", "head /etc/passwd", attendus=[r"^root:"]),
        Etape("tail-passwd", "tail /etc/passwd", attendus=[r"^guest:"]),
        Etape("grep-passwd", "grep root /etc/passwd", attendus=[r"^root:"], toutes=r"root"),
        Etape("mkdir-rmdir", "mkdir /tmp/vide && rmdir /tmp/vide", lignes=0),
        Etape("cp-mv-rm", "cp /etc/hosts /tmp/h1 && mv /tmp/h1 /tmp/h2 && cat /tmp/h2 && rm /tmp/h2",
              attendus=[r"^127\.0\.0\.1"], lignes=1),
        Etape("rm-inexistant", "rm /tmp/pas-la", lignes=1),
        Etape("ps", "ps", attendus=[r"PID|pid"]),
        Etape("free", "free"),
        Etape("pwd", "pwd", attendus=[r"^/$"], lignes=1),
    ]
    return prelude, e


# --- Le scenario `arborescence` (/etc, /bin, /usr/bin, type) -----------------

def donnees_arborescence() -> dict[str, bytes | None]:
    # Un programme REEL a la racine, comme le disque userland en installe
    # (`/doom`, `/hello`...) : `type` doit le trouver par le PATH par defaut.
    return {"programme-temoin": b"\x7fELF-temoin\n"}


def scenario_arborescence() -> tuple[list[str], list[Etape]]:
    etc = {"passwd", "hostname", "hosts", "resolv.conf", "group", "localtime", "ld-musl-x86_64.path", "ld.so.conf"}
    e = [
        Etape("ls-racine", "ls /", attendus=[rf"^{d}/$" for d in ("bin", "etc", "usr", "tmp", "home", "proc", "sys", "lib")]),
        Etape("ls-etc", "ls /etc", attendus=[rf"^{re.escape(f)}$" for f in sorted(etc)], lignes=len(etc)),
        Etape("cat-hosts", "cat /etc/hosts", attendus=[r"^127\.0\.0\.1\tlocalhost bouchaud$"], lignes=1),
        Etape("cat-hostname", "cat /etc/hostname", attendus=[r"^bouchaud$"], lignes=1),
        # Une seule source : plus de /bin/bsh d'un cote et /bin/sh de l'autre.
        Etape("cat-passwd", "cat /etc/passwd", lignes=2,
              attendus=[r"^root:x:0:0:root:/:/bin/sh$", r"^guest:x:1000:1000:guest:/home/guest:/bin/sh$"]),
        Etape("cat-group", "cat /etc/group", attendus=[r"^root:x:0:$", r"^guest:x:1000:$"], lignes=2),
        Etape("cat-resolv", "cat /etc/resolv.conf", attendus=[r"^(nameserver [0-9.]+|# Bouchaud: network not configured)$"], lignes=1),
        Etape("ls-bin", "ls /bin", lignes=0),
        Etape("ls-usr-bin", "ls /usr/bin", lignes=0),
        Etape("find-etc", "find /etc", lignes=len(etc), toutes=r"^/etc/"),
        Etape("find-bin", "find /bin", lignes=0),
        Etape("type-interne", "type ls find cat sh",
              attendus=[rf"^{c} : commande interne du shell \(noyau, sans fichier\)$" for c in ("ls", "find", "cat")]
              + [r"^sh : "]),
        Etape("type-programme", "type programme-temoin", attendus=[r"^programme-temoin : //?programme-temoin$"], lignes=1),
        Etape("type-introuvable", "type aucun-programme", attendus=[r"^aucun-programme : introuvable$"], lignes=1),
        # Revue #302 : des commandes internes absentes de la liste de completion.
        Etape("type-internes-hors-completion", "type smpstat gui-surface",
              attendus=[rf"^{c} : commande interne du shell \(noyau, sans fichier\)$" for c in ("smpstat", "gui-surface")],
              lignes=2),
        # Un fichier du meme nom dans PATH ne masque pas la commande interne.
        Etape("pose-bin-ls", "cp /programme-temoin /bin/ls && chmod 755 /bin/ls", lignes=0),
        Etape("type-ls-reste-interne", "type ls", attendus=[r"^ls : commande interne du shell"], lignes=1),
        # Revue #302 : un fichier NON executable plus tot dans PATH masque le
        # suivant, a l'execution comme pour `type`.
        Etape("pose-ombre", "cp /programme-temoin /bin/outil && chmod 644 /bin/outil && cp /programme-temoin /usr/bin/outil && chmod 755 /usr/bin/outil", lignes=0),
        Etape("type-ombre", "type outil", lignes=1,
              attendus=[r"^outil : /bin/outil \(non executable : le shell refuse, code 126\)$"]),
        Etape("execute-ombre", "outil", lignes=1, attendus=[r"^/bin/outil: permission refusee"]),
    ]
    return ["journal off", "cd /", "chmod 755 /programme-temoin"], e


# --- Le scenario `persistance` : plusieurs demarrages sur le MEME disque -------
#
# Disque de mkdisk.sh : l'archive (autorun) au debut, au moins
# (1025+1)*512 octets, puis la zone persistante de 128 Mio a la fin. Entre deux
# demarrages, SEULE l'archive est reecrite (un autre autorun) ; la zone --
# donc /persist -- n'est jamais touchee par le banc.

ARCHIVE_PLANCHER = (1025 + 1) * 512
ZONE_OCTETS = 262144 * 512
CONTENU = "persistant-0123456789-ok"


def disque_persistant(chemin: Path, autorun: str) -> None:
    chemin.write_bytes(b"\0" * (ARCHIVE_PLANCHER + ZONE_OCTETS))
    reecrit_autorun(chemin, autorun)


def reecrit_autorun(chemin: Path, autorun: str) -> None:
    tampon = io.BytesIO()
    with tarfile.open(fileobj=tampon, mode="w", format=tarfile.USTAR_FORMAT) as t:
        i = tarfile.TarInfo("./autorun")
        i.mode, i.uid, i.gid, i.uname, i.gname = 0o644, 0, 0, "root", "root"
        donnees = autorun.encode()
        i.size = len(donnees)
        t.addfile(i, io.BytesIO(donnees))
    octets = tampon.getvalue()
    if len(octets) > ARCHIVE_PLANCHER:
        raise ValueError("autorun trop gros pour la region d'archive")
    with chemin.open("r+b") as f:
        f.write(octets + b"\0" * (ARCHIVE_PLANCHER - len(octets)))


def demarrages_persistance() -> list[tuple[list[str], list[Etape]]]:
    d = "/persist/utilisateur"
    b1 = [
        Etape("b1-mkdir", f"mkdir {d}", lignes=0),
        Etape("b1-touch", f"touch {d}/note.txt", lignes=0),
        Etape("b1-write", f"write {d}/note.txt {CONTENU}", lignes=0),
        Etape("b1-relit", f"cat {d}/note.txt", attendus=[rf"^{CONTENU}$"], lignes=1),
        Etape("b1-sync", "sync", attendus=[r"^sync: [0-9]+ fichier\(s\) ecrit\(s\) sous /persist$"], lignes=1),
    ]
    b2 = [
        Etape("b2-relit", f"cat {d}/note.txt", attendus=[rf"^{CONTENU}$"], lignes=1),
        Etape("b2-find", "find /persist note", attendus=[rf"^{d}/note\.txt$"], lignes=1),
        Etape("b2-ls", f"ls {d}", attendus=[r"^note\.txt"], lignes=1),
        Etape("b2-mv", f"mv {d}/note.txt {d}/renomme.txt", lignes=0),
        Etape("b2-sync", "sync", attendus=[r"^sync: [0-9]+ fichier\(s\) ecrit\(s\) sous /persist$"], lignes=1),
    ]
    b3 = [
        Etape("b3-relit", f"cat {d}/renomme.txt", attendus=[rf"^{CONTENU}$"], lignes=1),
        Etape("b3-ancien-absent", f"cat {d}/note.txt", attendus=[r"^cat: introuvable$"], lignes=1),
        Etape("b3-find", "find /persist note", lignes=0),
        Etape("b3-find-nouveau", "find /persist renomme", attendus=[rf"^{d}/renomme\.txt$"], lignes=1),
        Etape("b3-ls", f"ls {d}", attendus=[r"^renomme\.txt"], lignes=1),
    ]
    return [(["journal off", "cd /"], b) for b in (b1, b2, b3)]


DEMARRAGES = {"persistance": demarrages_persistance}


# TEMOIN (--temoin-zone-effacee) : la zone persistante est remise a zero entre
# le premier et le second demarrage. Le banc DOIT alors echouer a la relecture :
# sans cela, il ne prouverait pas que le contenu relu vient du disque.
TEMOIN_ZONE_EFFACEE = False


def lance_demarrages(boot: Path, sortie: Path, nom: str, kvm: bool) -> tuple[list[dict], str]:
    """Chaque demarrage sur le MEME disque ; un demarrage qui ne s'eteint pas
    proprement arrete la serie (les suivants sont NOT_RUN)."""
    phases = DEMARRAGES[nom]()
    sortie.mkdir(parents=True, exist_ok=True)
    disque = sortie / "disque.img"
    res: list[dict] = []
    fin = "eteinte"
    for k, (prelude, etapes) in enumerate(phases, 1):
        if k == 1:
            disque_persistant(disque, script(prelude, etapes))
        else:
            reecrit_autorun(disque, script(prelude, etapes))
            if TEMOIN_ZONE_EFFACEE and k == 2:
                with disque.open("r+b") as f:
                    f.seek(ARCHIVE_PLANCHER)
                    f.write(b"\0" * ZONE_OCTETS)
        if fin != "eteinte" or any(r["statut"] != "PASS" for r in res):
            res += [{"id": e.id, "commande": e.commande, "statut": "NOT_RUN"} for e in etapes]
            continue
        d = sortie / f"demarrage-{k}"
        d.mkdir(exist_ok=True)
        r, fin = lance_disque(boot, d, nom, etapes, disque, kvm)
        res += r
    return res, fin


SCENARIOS = {"find": (scenario_find, donnees_find), "commandes": (scenario_commandes, donnees_commandes),
             "arborescence": (scenario_arborescence, donnees_arborescence)}


# --- Le classement (pur, teste par tools/ci/reliability/test_unix_fiabilite.py)

def nettoie(ligne: str) -> str:
    return PREFIXE.sub("", ANSI.sub("", ligne)).rstrip("\r\n")


def sortie_etape(lignes: list[str], ident: str) -> list[str] | None:
    """Les lignes produites entre UNIX_DEBUT et UNIX_FIN de l'etape (sans les
    echos `+ ...` du shell ni les lignes du noyau prefixees), ou None si la FIN
    n'a pas ete vue."""
    debut = fin = None
    for k, l in enumerate(lignes):
        if l == f"UNIX_DEBUT {ident}":
            debut = k
        elif l == f"UNIX_FIN {ident}" and debut is not None:
            fin = k
            break
    if debut is None or fin is None:
        return None
    return [l for l in lignes[debut + 1 : fin] if not l.startswith("+ ") and not NOYAU.match(l)]


def juge_contenu(e: Etape, sortie: list[str]) -> list[str]:
    raisons = []
    for m in e.attendus:
        if not any(re.search(m, l) for l in sortie):
            raisons.append(f"absent:{m[:60]}")
    for m in e.interdits:
        if any(re.search(m, l) for l in sortie):
            raisons.append(f"interdit:{m[:60]}")
    if e.toutes is not None:
        hors = [l for l in sortie if not re.search(e.toutes, l)]
        if hors:
            raisons.append(f"ligne-inattendue:{hors[0][:80]}")
    if e.lignes is not None and len(sortie) != e.lignes:
        raisons.append(f"lignes={len(sortie)}!={e.lignes}")
    return raisons


def classe(etapes: list[Etape], brut: list[str], fin_machine: str) -> list[dict]:
    """`fin_machine` : `eteinte` (poweroff atteint), `panique`, `muette`
    (vivante, console figee : arretee par le banc), `bavarde` (vivante, console
    active, budget epuise)."""
    lignes = [nettoie(l) for l in brut]
    panique = next((k for k, l in enumerate(lignes) if FATAL.search(l)), None)
    res = []
    bloque = False
    for e in etapes:
        r = {"id": e.id, "commande": e.commande}
        if bloque:
            r["statut"] = "NOT_RUN"
            res.append(r)
            continue
        debut = next((k for k, l in enumerate(lignes) if l == f"UNIX_DEBUT {e.id}"), None)
        temoin = next((k for k, l in enumerate(lignes) if l == f"UNIX_TEMOIN {e.id}"), None)
        if debut is None:
            r["statut"] = "NOT_RUN"
            bloque = True
            res.append(r)
            continue
        if panique is not None and panique > debut and (temoin is None or panique < temoin):
            r["statut"] = "KERNEL_PANIC"
            r["panique"] = lignes[panique][:200]
            bloque = True
        elif temoin is None:
            r["statut"] = {"muette": "DEADLOCK_SUSPECTED", "bavarde": "TIMEOUT"}.get(fin_machine, "TIMEOUT")
            bloque = True
        else:
            sortie = sortie_etape(lignes, e.id) or []
            raisons = juge_contenu(e, sortie)
            r["lignes"] = len(sortie)
            r["statut"] = "FAIL" if raisons else "PASS"
            if raisons:
                r["raisons"] = raisons
                r["extrait"] = sortie[:5]
        res.append(r)
    return res


# --- La machine -------------------------------------------------------------

def archive(chemin: Path, script: str, donnees: dict[str, bytes | None]) -> None:
    with tarfile.open(chemin, "w", format=tarfile.USTAR_FORMAT) as t:
        def ajoute(nom: str, contenu: bytes | None) -> None:
            i = tarfile.TarInfo("./" + nom)
            i.uid = i.gid = 0
            i.uname = i.gname = "root"
            if contenu is None:
                i.type = tarfile.DIRTYPE
                i.mode = 0o755
                t.addfile(i)
            else:
                i.mode = 0o644
                i.size = len(contenu)
                t.addfile(i, io.BytesIO(contenu))
        ajoute("autorun", script.encode())
        for nom, contenu in donnees.items():
            ajoute(nom, contenu)
    with chemin.open("ab") as f:
        f.write(b"\0" * (4 << 20))


def script(prelude: list[str], etapes: list[Etape]) -> str:
    l = list(prelude)
    for e in etapes:
        l += [f"echo UNIX_DEBUT {e.id}", e.commande, f"echo UNIX_FIN {e.id}", f"echo UNIX_TEMOIN {e.id}"]
    l += ["echo UNIX_SCENARIO_FIN", "poweroff"]
    return "\n".join(l) + "\n"


def verdict_attente(ecoule: float, silence: float, limite: float) -> str | None:
    """Pendant une etape pas encore terminee (temoin non vu) : `None` pour
    continuer d'attendre ; `muette` si la console n'a rien ecrit depuis plus
    que le budget (blocage suspecte) ; `bavarde` si l'etape depasse son budget
    alors que la console avance encore (TIMEOUT). Revue #300 : le budget est
    celui de l'ETAPE, compte depuis son debut, quelle que soit l'activite de
    la console -- une commande qui ecrit sans fin n'est jamais un succes."""
    if silence > limite:
        return "muette"
    if ecoule > limite:
        return "bavarde"
    return None


def sysrq(moniteur: Path) -> None:
    try:
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(3)
        s.connect(str(moniteur))
        s.sendall(b"sendkey alt-sysrq\n")
        time.sleep(0.5)
        s.close()
    except OSError:
        pass


def lance(boot: Path, sortie: Path, nom: str, kvm: bool) -> tuple[list[dict], str]:
    if nom in DEMARRAGES:
        return lance_demarrages(boot, sortie, nom, kvm)
    prelude, etapes = SCENARIOS[nom][0]()
    donnees = SCENARIOS[nom][1]()
    sortie.mkdir(parents=True, exist_ok=True)
    disque = sortie / "scenario.img"
    archive(disque, script(prelude, etapes), donnees)
    return lance_disque(boot, sortie, nom, etapes, disque, kvm)


def lance_disque(boot: Path, sortie: Path, nom: str, etapes: list[Etape], disque: Path,
                 kvm: bool) -> tuple[list[dict], str]:
    serie = sortie / "serie.log"
    serie.write_bytes(b"")
    moniteur = Path(f"/tmp/bo-unix-{nom}-{int(time.time())}.sock")
    accel = ["-enable-kvm", "-cpu", "host"] if kvm else ["-accel", "tcg"]
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", *accel, "-drive", f"format=raw,file={boot}", "-drive", f"format=raw,file={disque}",
         "-m", "2048", "-smp", "2", "-display", "none", "-no-reboot",
         "-monitor", f"unix:{moniteur},server,nowait", "-serial", f"file:{serie}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    budget_total = 180 + sum(e.budget_s for e in etapes)
    debut = time.monotonic()
    derniere_taille, derniere_activite = 0, time.monotonic()
    fin_machine = "eteinte"
    debut_etape: dict[str, float] = {}
    try:
        while qemu.poll() is None:
            time.sleep(1)
            taille = serie.stat().st_size
            if taille != derniere_taille:
                derniere_taille, derniere_activite = taille, time.monotonic()
            texte = serie.read_bytes().decode("utf-8", "replace")
            if FATAL.search(texte):
                time.sleep(3)
                fin_machine = "panique"
                break
            if "UNIX_SCENARIO_FIN" in texte:
                qemu.wait(timeout=60)
                break
            # Budget de l'etape en cours : depuis son DEBUT.
            vus = [e for e in etapes if f"UNIX_DEBUT {e.id}" in texte]
            courante = vus[-1] if vus else None
            limite = courante.budget_s if courante else 180
            silence = time.monotonic() - derniere_activite
            if courante and f"UNIX_TEMOIN {courante.id}" not in texte:
                ecoule = time.monotonic() - debut_etape.setdefault(courante.id, time.monotonic())
                v = verdict_attente(ecoule, silence, limite)
                if v:
                    fin_machine = v
                    if v == "muette":
                        sysrq(moniteur)
                        time.sleep(8)
                    break
            if time.monotonic() - debut > budget_total:
                fin_machine = "bavarde"
                break
    finally:
        if qemu.poll() is None:
            qemu.terminate()
            try:
                qemu.wait(timeout=10)
            except subprocess.TimeoutExpired:
                qemu.kill()
        moniteur.unlink(missing_ok=True)
    brut = serie.read_bytes().decode("utf-8", "replace").splitlines()
    return classe(etapes, brut, fin_machine), fin_machine


def main(argv: list[str]) -> int:
    args = argv[1:]
    kvm = "--kvm" in args
    global TEMOIN_ZONE_EFFACEE
    TEMOIN_ZONE_EFFACEE = "--temoin-zone-effacee" in args
    nom = "find"
    if "--scenario" in args:
        nom = args[args.index("--scenario") + 1]
    pos = [a for k, a in enumerate(args) if not a.startswith("--") and (k == 0 or args[k - 1] != "--scenario")]
    if len(pos) != 2 or (nom not in SCENARIOS and nom not in DEMARRAGES):
        print(__doc__, file=sys.stderr)
        return 2
    boot, sortie = Path(pos[0]).resolve(), Path(pos[1]).resolve() / nom
    res, fin = lance(boot, sortie, nom, kvm)
    accel = "kvm" if kvm else "tcg"
    for r in res:
        extra = (" raisons=" + ";".join(r["raisons"])) if "raisons" in r else ""
        extra += f" panique={r['panique']}" if "panique" in r else ""
        print(f"UNIX_ETAPE scenario={nom} id={r['id']} statut={r['statut']}{extra}")
    compte = {s: sum(1 for r in res if r["statut"] == s) for s in STATUTS}
    (sortie / "rapport.json").write_text(json.dumps({"scenario": nom, "accel": accel, "fin_machine": fin,
                                                    "etapes": res, "compte": compte}, indent=2))
    resume = " ".join(f"{s.lower()}={n}" for s, n in compte.items())
    if compte["PASS"] == len(res):
        print(f"UNIX_FIABILITE_OK scenario={nom} accel={accel} {resume}")
        return 0
    print(f"UNIX_FIABILITE_ECHEC scenario={nom} accel={accel} fin_machine={fin} {resume}")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
