#!/usr/bin/env python3
"""Une echeance interactive reste armable pendant un balayage
(P13_BORNE_INTERACTIVE_PENDANT_BALAYAGE_V1).

`wake_sleepers` remet les bornes interactives par coeur a JAMAIS AVANT de
lire le registre (pour qu'un armement concurrent survive au recalage par
`fetch_min`), puis les recale. Entre les deux, un coeur qui arme son
minuteur (`cible_occupee` -> `borne_interactive_a_jour`) lisait JAMAIS et
n'armait que son quantum : l'interactive attendait le balayage suivant
(`tardif_us` 997 et 1159, bras `on` de l'A/B KVM SMP4, run 40136589584).

Regles :

  1. `wake_sleepers` incremente `BALAYAGES_INTERACTIFS` AVANT la remise a
     JAMAIS et ne le decremente qu'APRES `recalcule_echeance()` ;
  2. `borne_interactive_a_jour` consulte ce compteur et, pendant un
     balayage, complete la borne par `borne_pendant_balayage` avec le
     minimum exact du registre (`minimum_interactif`), SANS le publier ;
  3. la decision est pure (`echeances.rs`) et couverte par
     `tools/smp/test_echeances.rs`, dont une mutation executable de la
     lecture historique.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
    "pur": RACINE / "src/kernel/scheduler/echeances.rs",
    "test": RACINE / "tools/smp/test_echeances.rs",
}
TESTS = (
    "un_balayage_en_cours_ne_masque_aucune_echeance_interactive",
    "la_lecture_historique_de_la_borne_est_detectee",
    "une_echeance_non_echue_reste_armable_derriere_une_echue",
)


def corps(source: str, signature: str) -> str:
    """Le corps d'une fonction, par appariement d'accolades."""
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", debut)
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    balayage = corps(t["sommeil"], "fn wake_sleepers()")
    plus = balayage.find("BALAYAGES_INTERACTIFS.fetch_add(1")
    remise = balayage.find("borne.remplace(crate::kernel::echeances::JAMAIS);")
    recalage = balayage.find("recalcule_echeance();")
    moins = balayage.find("BALAYAGES_INTERACTIFS.fetch_sub(1")
    if not 0 <= plus < remise < recalage < moins:
        fautes.append("wake_sleepers : le balayage n'est pas annonce autour de la remise a JAMAIS et du recalage")
    if balayage.count("BALAYAGES_INTERACTIFS.fetch_add(1") != 1 or balayage.count("BALAYAGES_INTERACTIFS.fetch_sub(1") != 1:
        fautes.append("wake_sleepers : increments/decrements du balayage desequilibres")
    lecture = corps(t["sommeil"], "pub fn borne_interactive_a_jour(")
    garde = lecture.find("BALAYAGES_INTERACTIFS.load(")
    completion = lecture.find("borne_pendant_balayage(")
    sortie = lecture.find("valeur > maintenant_ns")
    if not 0 <= garde < completion < sortie:
        fautes.append("borne_interactive_a_jour : la borne publiee est lue seule pendant un balayage")
    pendant = lecture[garde:sortie] if 0 <= garde < sortie else ""
    if "minimum_interactif(cpu)" not in pendant or "remplace(" in pendant:
        fautes.append("borne_interactive_a_jour : pendant un balayage, le minimum exact manque ou est publie")
    pure = corps(t["pur"], "pub fn borne_pendant_balayage(")
    if "if publiee > maintenant_ns { publiee.min(exacte) } else { exacte }" not in pure:
        fautes.append("echeances.rs : la completion pendant un balayage n'est plus celle testee")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}")
    if "fn vue_historique(publiee: u64, _exacte: u64, _maintenant: u64) -> u64 {\n    publiee\n}" not in t["test"]:
        fautes.append("test hote : la mutation historique n'est plus executee")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("balayage non annonce", "sommeil", "    BALAYAGES_INTERACTIFS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);\n", ""),
        ("fin annoncee avant le recalage", "sommeil",
         "    recalcule_echeance();\n    // Les bornes interactives sont recalees",
         "    BALAYAGES_INTERACTIFS.fetch_sub(1, core::sync::atomic::Ordering::SeqCst);\n    recalcule_echeance();\n    // Les bornes interactives sont recalees"),
        ("lecture sans completion", "sommeil",
         "        return crate::kernel::echeances::borne_pendant_balayage(\n            valeur, minimum_interactif(cpu), maintenant_ns);\n",
         ""),
        ("completion publiee", "sommeil",
         "            valeur, minimum_interactif(cpu), maintenant_ns);",
         "            valeur, { let m = minimum_interactif(cpu); borne.remplace(m); m }, maintenant_ns);"),
        ("completion sans registre", "sommeil",
         "            valeur, minimum_interactif(cpu), maintenant_ns);",
         "            valeur, crate::kernel::echeances::JAMAIS, maintenant_ns);"),
        ("completion impure", "pur",
         "if publiee > maintenant_ns { publiee.min(exacte) } else { exacte }",
         "publiee"),
        ("mutation historique retiree", "test", "fn la_lecture_historique_de_la_borne_est_detectee()", "fn x()"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs


def main() -> int:
    t = {cle: chemin.read_text(encoding="utf-8") for cle, chemin in FICHIERS.items()}
    fautes = verifie(t) + negatifs(t)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("borne interactive pendant un balayage : ok (7 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
