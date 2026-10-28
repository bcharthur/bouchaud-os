#!/usr/bin/env python3
"""Le temoin console ne suspend le coeur du fil que si sa precondition est
reunie (P13_TEMOIN_CONSOLE_PRECONDITION_V1, P13_TEMOIN_CONSOLE_SUIT_LE_FIL_V1).

C16 : le bras temoin `verrou_cible=fil secours=off` doit faire revenir le
repli de securite (le defaut du run 39667691144). Il ne l'a pas fait une fois
sur 9311ad3c (40672022728) et une fois sur 70a2a313 (40701963055) : le
verrou de l'anneau etait pris sur un anneau VIDE, le fil endormi -- la file
d'un fil endormi reste renseignee --, les ecrivains renoncaient et
differaient, et le chien de garde, qui ne surveille qu'un anneau non vide,
n'avait rien a voir (`soupcons=0`, `verrou_abandons=1287`).

C18 (KVM 40770950829, `58f7e8d8`) : dans le bras rafale masquee, la rafale
attendait le verrou et le banc attendait un anneau non vide -- interblocage,
aucune injection (`anneau=0 attente_us=2000000`) ; et le coeur lu 1,5 s avant
la suspension n'etait plus toujours celui du fil.

Regles :

  1. `fil_console_attend_avec_travail` : le fil est PRET (`etat == 0`), hors
     d'execution (`on_cpu < 0`), dans la file du coeur, et il a du travail :
     des lignes dans l'anneau, ou une rafale masquee armee ;
  2. `fil_console_attache_a` : pret ou endormi, hors d'execution, dans la
     file du coeur ;
  3. un fil de banc epingle par coeur ; celui dont le coeur porte le fil le
     REVENDIQUE (compare-exchange, un seul) ; les autres s'effacent ;
  4. le verrou n'est pris qu'apres la revendication et la precondition,
     attendues sous une borne nommee (2 s) ; sinon HORS_CIBLE, resultat 2 ;
  5. seule `serie-rafale-masquee` arme le travail a venir, et le desarme a
     chacune de ses sorties ;
  6. l'attente est publiee (`precondition_us=`) ; l'exigence du temoin est
     intacte (sans secours, le repli DOIT revenir).

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
REGLE = "(etat == 0 && on_cpu < 0 && file as usize == coeur && (occupation > 0 || travail_a_venir),"
ATTACHE = "(etat == 0 || etat == 1) && on_cpu < 0 && file as usize == coeur"
BORNE = "        let borne = debut.saturating_add(VERROU_PRECONDITION_BORNE_NS);"
CAS = "VERROU_REVENDIQUE.compare_exchange(usize::MAX, ici, Ordering::AcqRel, Ordering::Acquire).is_ok()"
REVENDIQUE = "            if crate::drivers::serial::fil_console_attache_a(ici)\n                && " + CAS
EFFACE = "        if revendique != ici && revendique != usize::MAX {"
BOUCLE = ("            while !vu.0 && crate::kernel::timer::monotonic_ns() < borne {\n"
          "                core::hint::spin_loop();\n"
          "                vu = crate::drivers::serial::fil_console_attend_avec_travail(ici, RAFALE_ARMEE.load(Ordering::Acquire));\n"
          "            }")
A_MOI = "        let a_moi = revendique == ici && vu.0 && !VERROU_SUSPENDU_FAIT.swap(true, Ordering::AcqRel);"
REFUS = "                VERROU_RESULTAT.store(2, Ordering::Release);\n            }\n            parque_fil_de_banc();"
PRISE = "    let pris = crate::drivers::serial::banc_verrou_console_suspendu(duree);"
PUBLIE = "\"CONSOLE_VERROU_SUSPENDU_FAIT pris={} duree_ms={} coeur={} coeur_du_fil_apres={} precondition_us={}\","
PAR_COEUR = 'crate::kernel::task::spawn_noyau_epingle(fil_verrou_suspendu, "verrou-suspendu", coeur) || un'
ARME = "    RAFALE_ARMEE.store(true, Ordering::Release);"
DESARME = "RAFALE_ARMEE.store(false, Ordering::Release);"
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
        fautes.append("uart16550.rs : la precondition n'exige plus un fil PRET, hors d'execution, dans la file "
                      "du coeur, avec du travail (anneau ou rafale armee).")
    if ATTACHE not in corps(t["uart"], "pub fn fil_console_attache_a("):
        fautes.append("uart16550.rs : « attache » n'exige plus un fil pret ou endormi, hors d'execution, dans la file du coeur.")
    if PAR_COEUR not in corps(t["shell"], "fn serie_verrou_suspendu("):
        fautes.append("serie_verrou_suspendu : plus un fil de banc par coeur.")
    fil = corps(t["shell"], "fn fil_verrou_suspendu()")
    ordre = [fil.find(j) for j in (BORNE, REVENDIQUE, EFFACE, BOUCLE, A_MOI, REFUS, PRISE)]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("fil_verrou_suspendu : le verrou n'est plus pris APRES la revendication et la precondition "
                      "bornees, ou un refus ne rend plus HORS_CIBLE.")
    if "const VERROU_PRECONDITION_BORNE_NS: u64 = 2_000_000_000;" not in t["shell"]:
        fautes.append("shell : la borne de la precondition n'est plus nommee (2 s).")
    if PUBLIE not in fil:
        fautes.append("fil_verrou_suspendu : l'attente de la precondition n'est plus publiee.")
    rafale = corps(t["shell"], "fn serie_rafale_masquee(")
    ecritures = [l for l in t["shell"].splitlines() if "RAFALE_ARMEE.store(" in l]
    if ARME not in rafale or rafale.count(DESARME) != 2 or len(ecritures) != 3:
        fautes.append("rafale : seule serie-rafale-masquee arme le travail a venir, et le desarme a chaque sortie.")
    if TEMOIN not in t["banc"]:
        fautes.append("run_serie_entrelacement.sh : l'exigence du temoin (le repli revient) a change.")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("fil endormi accepte", "uart", "(etat == 0 && on_cpu < 0 && file as usize == coeur && (occupation",
         "(on_cpu < 0 && file as usize == coeur && (occupation"),
        ("fil en cours accepte", "uart", "(etat == 0 && on_cpu < 0 && file", "(etat == 0 && file"),
        ("sans travail", "uart", " && (occupation > 0 || travail_a_venir),", ","),
        ("travail toujours a venir", "uart", "(occupation > 0 || travail_a_venir)", "(occupation > 0 || true)"),
        ("attache en cours d'execution", "uart", "(etat == 0 || etat == 1) && on_cpu < 0 && file",
         "(etat == 0 || etat == 1) && file"),
        ("un seul fil de banc", "shell", PAR_COEUR,
         'crate::kernel::task::spawn_noyau_epingle(fil_verrou_suspendu, "verrou-suspendu", 0) || un'),
        ("revendication sans compare-exchange", "shell", "                && " + CAS, "                && true"),
        ("verrou pris sans precondition", "shell", A_MOI,
         "        let a_moi = revendique == ici && !VERROU_SUSPENDU_FAIT.swap(true, Ordering::AcqRel);"),
        ("verrou pris sans revendication", "shell", A_MOI,
         "        let a_moi = vu.0 && !VERROU_SUSPENDU_FAIT.swap(true, Ordering::AcqRel);"),
        ("attente sans borne", "shell", "            while !vu.0 && crate::kernel::timer::monotonic_ns() < borne {",
         "            while !vu.0 {"),
        ("refus sans resultat", "shell", REFUS, "            }\n            parque_fil_de_banc();"),
        ("attente non publiee", "shell", " precondition_us={}\",", "\","),
        ("rafale jamais desarmee", "shell", "            RAFALE_ARMEE.store(false, Ordering::Release);\n            return 1;",
         "            return 1;"),
        ("rafale armee ailleurs", "shell", "fn parque_fil_de_banc() -> ! {",
         "fn x() { RAFALE_ARMEE.store(true, Ordering::Release); }\nfn parque_fil_de_banc() -> ! {"),
        ("temoin assoupli", "banc", "active=0 repli_securite=1 '\" _ \"$BILAN\"",
         "active=[01] repli_securite=[01] '\" _ \"$BILAN\""),
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
    print(f"verifie-temoin-console-precondition: OK (fil pret en file, travail present ou a venir, coeur du fil "
          f"revendique, attente bornee et publiee, exigence du temoin intacte ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
