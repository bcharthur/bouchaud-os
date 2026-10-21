#!/usr/bin/env python3
"""Toute disparition d'une extremite de tube ou de canal reveille ses attentes.

BOUCHAUD_EOF_HORS_CLOSE_V1

# Le defaut

`FdKind::release` decremente les extremites d'un tube sans reveiller
personne ; un lecteur bloque n'apprend la fin de fichier (un ecrivain bloque
EPIPE) que par `notify_readiness`. `close()` l'appelait. `execve` (FD_CLOEXEC,
le motif de `posix_spawn`), `dup2` / `dup3` sur un numero ouvert et la mort
d'un orphelin (aucun SIGCHLD) ne l'appelaient pas : eof-fermeture-probe, TCG
SMP4, 12 cas sur 18 -- fin de fichier a la mort du fils (3,2 s) ou a l'alarme
(5 s) au lieu de 0,15 s.

# La regle

Un descripteur retire d'une table se ferme HORS du verrou de la table, par
`ferme_hors_verrou`, qui reveille ensuite (le reveil prend la table des
taches, de rang inferieur a `FdTable`) :
  1. `FdTable::set` rend l'ancien descripteur et `close_on_exec` les retires,
     tous deux `#[must_use]` ;
  2. `sys_dup2` / `sys_dup3` relachent la table PUIS ferment l'ancien ;
  3. `execve` ferme les retires de `close_on_exec` ;
  4. `ferme_descripteurs` (mort) ne compte plus sur le SIGCHLD du pere ;
  5. `recvmsg` ferme les droits recus qu'il abandonne sur EFAULT ;
  6. la sonde tourne dans os-primitives.

Fail-closed ; tests negatifs.
"""

import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
FD = "src/kernel/object/fd.rs"
FICHIER = "src/compat/linux/file.rs"
PROC = "src/compat/linux/proc.rs"
VIE = "src/kernel/process/thread/lifecycle.rs"
NET = "src/compat/linux/net.rs"
PRIMITIVES = "tools/ci/run_os_primitives.sh"
# BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : le marqueur est exige par le verdict a part.
VERDICT = "tools/ci/verdict_os_primitives.sh"
FICHIERS = (VERDICT, FD, FICHIER, PROC, VIE, NET, PRIMITIVES)


def corps(texte: str, entete: str) -> str | None:
    debut = texte.find(entete)
    if debut < 0:
        return None
    ouverture = texte.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return None


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def avant_fn(texte: str, entete: str, lignes: int = 3) -> str:
    """Les quelques lignes qui precedent `entete` (ses attributs)."""
    debut = texte.find(entete)
    return "\n".join(texte[:max(debut, 0)].splitlines()[-lignes:]) if debut >= 0 else ""


def verifie(racine: Path) -> list[str]:
    try:
        t = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fd, fichier, proc, vie, net = (sans_commentaires(t[f]) for f in (FD, FICHIER, PROC, VIE, NET))
    fautes = []

    ferme = corps(fd, "pub fn ferme_hors_verrou(")
    if ferme is None or "notify_readiness()" not in ferme:
        fautes.append("ferme_hors_verrou absent ou ne reveille plus")
    if "pub fn set(&mut self, fd: usize, desc: FileDesc) -> Option<FileDesc>" not in fd:
        fautes.append("FdTable::set ne rend plus l'ancien descripteur")
    elif "#[must_use" not in avant_fn(fd, "pub fn set(&mut self, fd: usize"):
        fautes.append("FdTable::set n'est plus #[must_use]")
    if "pub fn close_on_exec(&mut self) -> Vec<FileDesc>" not in fd:
        fautes.append("close_on_exec ne rend plus les descripteurs retires")
    elif "#[must_use" not in avant_fn(fd, "pub fn close_on_exec(&mut self)"):
        fautes.append("close_on_exec n'est plus #[must_use]")

    for nom in ("pub fn sys_dup2(", "pub fn sys_dup3("):
        c = corps(fichier, nom)
        if c is None:
            fautes.append(f"{FICHIER} : {nom} introuvable")
            continue
        pose = c.find("let ancien = borrowed.set(")
        relache = c.find("drop(borrowed)", pose)
        ferme_ = c.find("ferme_hors_verrou(ancien)", relache)
        if pose < 0 or relache < 0 or ferme_ < 0:
            fautes.append(f"{nom.strip('(')} : l'ancien descripteur n'est plus ferme hors verrou puis reveille")

    exec_ = corps(proc, "pub fn sys_execve(")
    if exec_ is None:
        fautes.append(f"{PROC} : sys_execve introuvable")
    else:
        retire = exec_.find("let fermes = process.files.lock().close_on_exec();")
        if retire < 0 or exec_.find("ferme_hors_verrou(fermes)", retire) < 0:
            fautes.append("execve : les descripteurs FD_CLOEXEC ne sont plus fermes par ferme_hors_verrou")

    mort = corps(vie, "pub fn ferme_descripteurs(")
    if mort is None or "ferme_hors_verrou(" not in mort:
        fautes.append("ferme_descripteurs : la mort ne reveille plus d'elle-meme")

    recv = corps(net, "pub fn sys_recvmsg(")
    if recv is None:
        fautes.append(f"{NET} : sys_recvmsg introuvable")
    else:
        efault = recv.find("if !user_write(base, &data) {")
        if efault < 0 or recv.find("ferme_hors_verrou(recus)", efault) < 0:
            fautes.append("recvmsg : les droits abandonnes sur EFAULT ne reveillent plus")

    if "/bin/eof-fermeture-probe" not in t[PRIMITIVES] or "EOF_FERMETURE_OK" not in t[VERDICT]:
        fautes.append("os-primitives ne lance plus eof-fermeture-probe")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("EOF hors close : une extremite disparait sans reveil")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (FD, "    if !aucun {\n        notify_readiness();\n    }", "    let _ = aucun;"),
        (FD, "    #[must_use = \"l'ancien descripteur", "    #[allow(unused)] // \"l'ancien descripteur"),
        (FICHIER, "    let ancien = borrowed.set(new as usize, desc);\n    drop(borrowed);\n    // BOUCHAUD",
         "    let _ = borrowed.set(new as usize, desc);\n    drop(borrowed);\n    // BOUCHAUD"),
        (FICHIER, "    crate::kernel::fd::ferme_hors_verrou(ancien); // BOUCHAUD_EOF_HORS_CLOSE_V1",
         "    drop(ancien);"),
        (PROC, "    crate::kernel::fd::ferme_hors_verrou(fermes);", "    drop(fermes);"),
        (VIE, "    crate::kernel::fd::ferme_hors_verrou(fermes.into_iter().flatten());", "    drop(fermes);"),
        (NET, "            crate::kernel::fd::ferme_hors_verrou(recus);\n", ""),
        (PRIMITIVES, "/bin/eof-fermeture-probe", "/bin/true"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"EOF hors close : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"EOF_HORS_CLOSE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
