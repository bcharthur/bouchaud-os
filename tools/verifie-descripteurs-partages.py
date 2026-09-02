#!/usr/bin/env python3
"""Verifie le mode d'acces des fichiers destines au partage inscriptible."""

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def lit(relatif: str) -> str:
    return (ROOT / relatif).read_text(encoding="utf-8")


def exige(condition: bool, message: str, erreurs: list[str]) -> None:
    if not condition:
        erreurs.append(message)


def main() -> int:
    erreurs: list[str] = []
    fd = lit("src/kernel/object/fd.rs")
    memoire = lit("src/compat/linux/file.rs")
    client = lit("src/gui/client.rs")
    surface = lit("src/gui/surface.rs")
    securite = lit("src/kernel/security/filesystem.rs")
    sonde = lit("tools/userland/shm-probe.c")

    exige(
        "pub fn fichier_partage_inscriptible(node: usize) -> Self" in fd,
        "le constructeur des fichiers partages inscriptibles a disparu",
        erreurs,
    )
    exige(
        "desc.flags = ACCES_LECTURE_ECRITURE;" in fd,
        "le constructeur ne publie plus le mode lecture/ecriture",
        erreurs,
    )
    exige(
        "flags: self.flags," in fd,
        "une duplication ou un transfert perdrait le mode d'acces",
        erreurs,
    )
    exige(
        "FileDesc::fichier_partage_inscriptible(idx)" in memoire,
        "les tampons anonymes ne portent plus leur droit d'ecriture",
        erreurs,
    )
    exige(
        "FileDesc::fichier_partage_inscriptible(surface_node)" in client,
        "la surface injectee au client ne porte plus son droit d'ecriture",
        erreurs,
    )
    exige(
        "FileDesc::fichier_partage_inscriptible(self.node)" in surface,
        "Surface::descripteur ne porte plus son droit d'ecriture",
        erreurs,
    )

    oublis = []
    for chemin in (ROOT / "src").rglob("*.rs"):
        texte = chemin.read_text(encoding="utf-8")
        if "FileDesc::new(FdKind::File" in texte:
            oublis.append(str(chemin.relative_to(ROOT)))
    exige(
        not oublis,
        "creation directe d'un descripteur de fichier partage : "
        + ", ".join(oublis),
        erreurs,
    )

    exige(
        "if open_flags & O_ACCMODE != O_RDWR" in securite
        and "return Err(FsDeny::MappingAccess);" in securite,
        "la protection des projections partagees inscriptibles a ete relachee",
        erreurs,
    )
    exige(
        "(drapeaux & O_ACCMODE) == O_RDWR" in sonde,
        "la sonde ne controle plus le mode d'acces du descripteur anonyme",
        erreurs,
    )

    if erreurs:
        for erreur in erreurs:
            print(f"erreur: {erreur}", file=sys.stderr)
        return 1

    print(
        "ok  memoire et surfaces partagees : descripteurs lecture/ecriture ; "
        "controle mmap conserve"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
