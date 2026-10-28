#!/usr/bin/env python3
"""Le temoin console ne suspend le coeur du fil que si sa precondition est
reunie (P13_TEMOIN_CONSOLE_PRECONDITION_V1).

C16 : le bras temoin `verrou_cible=fil secours=off` doit faire revenir le
repli de securite (le defaut du run 39667691144). Il ne l'a pas fait une fois
sur 9311ad3c (40672022728) et une fois sur 70a2a313 (40701963055) : le
verrou de l'anneau etait pris sur un anneau VIDE, le fil endormi -- la file
d'un fil endormi reste renseignee --, les ecrivains renoncaient et
differaient, et le chien de garde, qui ne surveille qu'un anneau non vide,
n'avait rien a voir (`soupcons=0`, `verrou_abandons=1287`).

Regles :

  1. `fil_console_attend_avec_travail` : le fil est PRET (`etat == 0`), hors
     d'execution (`on_cpu < 0`), dans la file du coeur, et l'anneau n'est pas
     vide ;
  2. `fil_verrou_suspendu`, cible `fil` : le verrou n'est pris qu'apres cette
     precondition, attendue en tenant le coeur, sous une borne nommee ;
     sans elle, HORS_CIBLE et resultat 2 -- jamais le verrou ;
  3. l'attente est publiee (`precondition_us=` sur la ligne FAIT) ;
  4. l'exigence du temoin est intacte dans le banc : sans secours, le repli
     DOIT revenir (`active=0 repli_securite=1`).

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "uart": RACINE / "src/drivers/serial/uart16550.rs",
    "shell": RACINE / "src/shell/mod.rs",
    "banc": RACINE / "tools/ci/run_serie_entrelacement.sh",
}
REGLE = "(etat == 0 && on_cpu < 0 && file as usize == coeur && occupation > 0, etat, on_cpu, file, occupation)"
BOUCLE = ("        while !vu.0 && crate::kernel::timer::monotonic_ns() < borne {\n"
          "            core::hint::spin_loop();\n"
          "            vu = crate::drivers::serial::fil_console_attend_avec_travail(ici);\n        }")
BORNE = "        let borne = debut.saturating_add(VERROU_PRECONDITION_BORNE_NS);"
A_MOI = "        let a_moi = vu.0 && !VERROU_SUSPENDU_FAIT.swap(true, Ordering::AcqRel);"
REFUS = "            VERROU_RESULTAT.store(2, Ordering::Release);\n            loop {"
PRISE = "    let pris = crate::drivers::serial::banc_verrou_console_suspendu(duree);"
PUBLIE = "\"CONSOLE_VERROU_SUSPENDU_FAIT pris={} duree_ms={} coeur={} coeur_du_fil_apres={} precondition_us={}\","
TEMOIN = ("exige \"sans secours, le repli revient (temoin du defaut)\" bash -c \"printf '%s\\n' \\\"\\$1\\\" "
          "| grep -qE 'BOUCHAUD_CONSOLE_BILAN active=0 repli_securite=1 '\" _ \"$BILAN\"")


def corps(source: str, signature: str) -> str:
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
                return source[debut:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    if REGLE not in corps(t["uart"], "pub fn fil_console_attend_avec_travail("):
        fautes.append("uart16550.rs : la precondition n'exige plus un fil PRET, hors d'execution, "
                      "dans la file du coeur, anneau non vide.")
    fil = corps(t["shell"], "fn fil_verrou_suspendu()")
    ordre = [fil.find(j) for j in (BORNE, BOUCLE, A_MOI, REFUS, PRISE)]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("fil_verrou_suspendu : le verrou n'est plus pris APRES la precondition bornee, "
                      "ou un refus ne rend plus HORS_CIBLE.")
    if "const VERROU_PRECONDITION_BORNE_NS: u64 = 2_000_000_000;" not in t["shell"]:
        fautes.append("shell : la borne de la precondition n'est plus nommee (2 s).")
    if PUBLIE not in fil:
        fautes.append("fil_verrou_suspendu : l'attente de la precondition n'est plus publiee.")
    if TEMOIN not in t["banc"]:
        fautes.append("run_serie_entrelacement.sh : l'exigence du temoin (le repli revient) a change.")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("fil endormi accepte", "uart", "etat == 0 && on_cpu < 0", "on_cpu < 0"),
        ("fil en cours accepte", "uart", " && on_cpu < 0 && file", " && file"),
        ("anneau vide accepte", "uart", " && occupation > 0, etat", ", etat"),
        ("verrou pris sans precondition", "shell", A_MOI, "        let a_moi = !VERROU_SUSPENDU_FAIT.swap(true, Ordering::AcqRel);"),
        ("attente sans borne", "shell", " && crate::kernel::timer::monotonic_ns() < borne {", " {"),
        ("refus sans resultat", "shell", REFUS, "            loop {"),
        ("attente non publiee", "shell", " precondition_us={}\",", "\","),
        ("temoin assoupli", "banc", "active=0 repli_securite=1 '\" _ \"$BILAN\"", "active=[01] repli_securite=[01] '\" _ \"$BILAN\""),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in F.items()}
    echecs, n = negatifs(t)
    fautes = verifie(t) + echecs
    if fautes:
        for f in fautes:
            print(f"verifie-temoin-console-precondition: {f}")
        return 1
    print(f"verifie-temoin-console-precondition: OK (fil pret en file, anneau non vide, attente bornee "
          f"et publiee, exigence du temoin intacte ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
