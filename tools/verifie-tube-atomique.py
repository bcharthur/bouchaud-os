#!/usr/bin/env python3
"""Une ecriture de PIPE_BUF octets ou moins dans un tube ou un canal n'est
jamais coupee ni entrelacee.

BOUCHAUD_TUBE_ATOMIQUE_V1

# Le defaut

L'ecriture d'un tube jugeait la place sous le verrou, le relachait, puis
ecrivait `min(place, n)` sous un second verrou : deux ecrivains prenaient la
meme place, et une ecriture partielle (des qu'il restait UN octet) laissait
un autre ecrivain se glisser entre ses deux moities. `writev` ecrivait ses
vecteurs un par un. tube-atomicite-probe, TCG SMP4 : 29 / 359 / 1011
enregistrements de 100 / 512 / 4096 octets melanges.

# La regle

  1. `octets_admis` : jusqu'a PIPE_BUF, tout ou rien ;
  2. tube ET canal (socketpair) : place jugee et octets ecrits DANS la meme
     fermeture passee a `attends_place`, sous un seul verrou ;
  3. `writev` rassemble ses vecteurs jusqu'a PIPE_BUF octets en une ecriture ;
  4. `poll` n'annonce POLLOUT que si une ecriture de PIPE_BUF octets passe
     (semantique Linux) : POLLOUT des qu'il reste UN octet, face a une
     ecriture tout-ou-rien qui rend EAGAIN, fait tourner une boucle
     d'evenements non bloquante entre POLLOUT et EAGAIN (defaut introduit
     par la regle 1, corrige ensuite) ;
  5. la sonde tourne dans os-primitives.

Fail-closed ; tests negatifs.
"""

import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
FD = "src/kernel/object/fd.rs"
FICHIER = "src/compat/linux/file.rs"
PRIMITIVES = "tools/ci/run_os_primitives.sh"
# BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : le marqueur est exige par le verdict a part.
VERDICT = "tools/ci/verdict_os_primitives.sh"
FICHIERS = (VERDICT, FD, FICHIER, PRIMITIVES)


def corps(texte: str, entete: str, depuis: int = 0) -> str | None:
    debut = texte.find(entete, depuis)
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


def fermeture_d_ecriture(branche: str) -> str | None:
    """Le texte de la fermeture passee a `attends_place` dans une branche."""
    debut = branche.find("attends_place(")
    if debut < 0:
        return None
    return corps(branche, "||", debut)


def verifie(racine: Path) -> list[str]:
    try:
        t = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fd, fichier = sans_commentaires(t[FD]), sans_commentaires(t[FICHIER])
    fautes = []

    admis = corps(fd, "pub fn octets_admis(")
    if "pub const PIPE_BUF: usize = 4096;" not in fd:
        fautes.append("PIPE_BUF absent ou different de 4096")
    if admis is None or "longueur <= PIPE_BUF" not in admis or "if place >= longueur { longueur } else { 0 }" not in admis:
        fautes.append("octets_admis ne rend plus tout-ou-rien jusqu'a PIPE_BUF")

    ecrit = corps(fichier, "pub fn ecrit_octets(")
    if ecrit is None:
        return fautes + [f"{FICHIER} : ecrit_octets introuvable"]
    for nom, entete, tampon in (("tube", "FdKind::Pipe(shared, readable) =>", "state.buffer.extend_from_slice"),
                                ("canal", "FdKind::SocketPair(_, outbox) =>", "canal.octets.extend_from_slice")):
        branche = corps(ecrit, entete)
        f = fermeture_d_ecriture(branche) if branche else None
        if f is None:
            fautes.append(f"ecriture {nom} : fermeture d'attends_place introuvable")
            continue
        if "octets_admis(" not in f or tampon not in f:
            fautes.append(f"ecriture {nom} : la place n'est plus jugee ET ecrite sous le meme verrou")
        apres = branche[branche.find(f) + len(f):]
        if "extend_from_slice" in apres:
            fautes.append(f"ecriture {nom} : une ecriture hors de la fermeture (place relue hors verrou)")

    writev = corps(fichier, "pub fn sys_writev(")
    rassemble = corps(fichier, "fn rassemble_court(")
    if writev is None or "rassemble_court(iov, count)" not in writev or "ecrit_octets(fd, &data)" not in writev:
        fautes.append("writev ne rassemble plus ses vecteurs courts en une ecriture")
    elif writev.find("rassemble_court(") > writev.find("for index in 0..count"):
        fautes.append("writev : le chemin vecteur par vecteur passe avant le rassemblement")
    if rassemble is None or "total > crate::kernel::fd::PIPE_BUF" not in rassemble:
        fautes.append("rassemble_court n'est plus borne a PIPE_BUF")

    pret = corps(fichier, "fn writable(fd: i32) -> bool")
    if pret is None:
        fautes.append("writable introuvable")
    else:
        for nom, entete, garde in (("tube", "FdKind::Pipe(state, false) =>",
                                    "state.readers == 0 || state.place() >= crate::kernel::fd::PIPE_BUF"),
                                   ("canal", "FdKind::SocketPair(_, outbox) =>",
                                    "canal.lecteurs == 0 || canal.place() >= crate::kernel::fd::PIPE_BUF")):
            branche = corps(pret, entete)
            if branche is None or garde not in branche:
                fautes.append(f"poll {nom} : POLLOUT n'exige plus PIPE_BUF octets libres")

    if "/bin/tube-atomicite-probe" not in t[PRIMITIVES] or "TUBE_ATOMICITE_OK" not in t[VERDICT]:
        fautes.append("os-primitives ne lance plus tube-atomicite-probe")
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
        print("tube atomique : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (FD, "if place >= longueur { longueur } else { 0 }", "place.min(longueur)"),
        (FICHIER, "                    let n = crate::kernel::fd::octets_admis(state.place(), data.len());",
         "                    let n = state.place().min(data.len());"),
        (FICHIER, "                    let n = crate::kernel::fd::octets_admis(canal.place(), data.len());",
         "                    let n = canal.place().min(data.len());"),
        (FICHIER, "                    state.buffer.extend_from_slice(&data[..n]);\n                    Capacite::Place(n)",
         "                    Capacite::Place(n)"),
        (FICHIER, "    if let Some(rassemble) = rassemble_court(iov, count) {", "    if let Some(rassemble) = None::<Option<Result<Vec<u8>, i64>>>.flatten() {"),
        (FICHIER, "state.readers == 0 || state.place() >= crate::kernel::fd::PIPE_BUF",
         "state.readers == 0 || state.place() > 0"),
        (FICHIER, "canal.lecteurs == 0 || canal.place() >= crate::kernel::fd::PIPE_BUF",
         "canal.lecteurs == 0 || canal.place() > 0"),
        (PRIMITIVES, "/bin/tube-atomicite-probe", "/bin/true"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"tube atomique : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"TUBE_ATOMIQUE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
