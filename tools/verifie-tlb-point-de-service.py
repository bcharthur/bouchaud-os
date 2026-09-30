#!/usr/bin/env python3
"""Une ecriture serie interruptions masquees sert les shootdowns TLB en attente.

BOUCHAUD_TLB_POINT_DE_SERVICE_V1

# Le defaut

Le releve complet de l'ordonnanceur s'imprime depuis l'IRQ du minuteur du BSP,
interruptions masquees. Sa duree est celle du port serie (87 us par octet a
115 200 bauds) ; pendant ce temps le coeur 0 n'acquitte aucun IPI, et
l'emetteur d'un shootdown TLB atteint sa borne fail-closed de deux secondes :
panique `smp.rs` « acquittement manquant ». Reproduit a la demande avec un
COM1 a debit borne (tube lu lentement) : releves de 1,8 a 2,9 s, panique en
phase memoire ; avec le point de service, releves jusqu'a 4,8 s et aucune
panique, toutes les phases passent.

# La regle

  * `write_lot` (uart16550.rs) appelle `sert_shootdowns_en_attente()` dans sa
    boucle d'emission, sous la condition « interruptions masquees » ;
  * `handle_tlb_shootdown` delegue a `sert_shootdowns` (un seul traitement,
    celui du gestionnaire) ;
  * ni `sert_shootdowns_en_attente` ni `sert_shootdowns` n'emettent d'EOI :
    hors IPI, aucun vecteur n'est en service.

Fail-closed : une fonction introuvable est une faute. Deux tests negatifs.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UART = "src/drivers/serial/uart16550.rs"
SMP = "src/arch/x86_64/smp.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", ligne) for ligne in texte.splitlines())


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
                return texte[ouverture : i + 1]
    return None


def verifie(racine: Path) -> list[str]:
    fautes = []
    try:
        uart = sans_commentaires((racine / UART).read_text(encoding="utf-8"))
        smp = sans_commentaires((racine / SMP).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]

    ecriture = corps(uart, "fn write_lot(")
    if ecriture is None:
        fautes.append(f"{UART} : `fn write_lot` introuvable")
    else:
        boucle = ecriture[ecriture.find("while pose") :] if "while pose" in ecriture else ""
        if not re.search(r"interrupts::are_enabled\s*\(\s*\)", ecriture):
            fautes.append(f"{UART} : write_lot ne teste plus l'etat des interruptions")
        if "sert_shootdowns_en_attente" not in boucle:
            fautes.append(f"{UART} : write_lot ne sert plus les shootdowns dans sa boucle d'emission")

    gestionnaire = corps(smp, "pub fn handle_tlb_shootdown(")
    if gestionnaire is None:
        fautes.append(f"{SMP} : `handle_tlb_shootdown` introuvable")
    elif not re.search(r"\bsert_shootdowns\s*\(", gestionnaire):
        fautes.append(f"{SMP} : handle_tlb_shootdown ne delegue plus a sert_shootdowns")

    for entete in ("pub fn sert_shootdowns_en_attente(", "fn sert_shootdowns("):
        bloc = corps(smp, entete)
        if bloc is None:
            fautes.append(f"{SMP} : `{entete.strip('(')}` introuvable")
        elif "eoi" in bloc.lower():
            fautes.append(f"{SMP} : `{entete.strip('(')}` emet un EOI hors IPI")
    return fautes


def copie_mutee(remplacements: list[tuple[str, str, str]]) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for fichier in (UART, SMP):
            dest = copie / fichier
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / fichier).read_text(encoding="utf-8"), encoding="utf-8")
        for fichier, avant, apres in remplacements:
            cible = copie / fichier
            texte = cible.read_text(encoding="utf-8")
            if avant not in texte:
                return False
            cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("point de service TLB : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        [(UART, "crate::arch::x86_64::smp::sert_shootdowns_en_attente();", "")],
        [(SMP, "fn sert_shootdowns(cpu: usize) {\n", "fn sert_shootdowns(cpu: usize) {\n    eoi_local();\n")],
    ]
    for n, mutation in enumerate(negatifs, 1):
        if not copie_mutee(mutation):
            print(f"point de service TLB : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"TLB_POINT_DE_SERVICE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
