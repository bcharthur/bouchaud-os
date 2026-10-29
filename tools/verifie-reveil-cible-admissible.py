#!/usr/bin/env python3
"""Le banc du reveil derriere un coeur muet ne masque qu'un coeur ou la
reproduction est atteignable (P13_REVEIL_CIBLE_ADMISSIBLE_V1, C21).

KVM cible 40812092788 (2d850401) : en SMP2, le pilote masquait le coeur du
dormeur tel qu'il le trouvait ; trois tirages de suite sur le coeur un -- dont
le seul autre coeur est le BSP, tenu hors ordonnanceur par le shell -- ont
donne trois `autres_coeurs_au_repos=0`, donc NON_QUALIFIANT, alors que le
dormeur progressait (824 a 1203 tours). Defaut du BANC, pas du produit.

Regles :

  1. `cible_masquage_admissible(coeur, en_ligne)` : vrai si et seulement si
     un AP (1..en_ligne) autre que `coeur` existe ;
  2. le pilote verifie la cible AVANT de lancer le masqueur et le fil
     epingle, sur cette cible ; il re-lit le coeur du dormeur (borne 5 s) ;
     un coeur impose n'est jamais remplace ;
  3. un refus publie `SERIE_DORMEUR_MASQUE_ECHEC raison=cible-inadmissible`,
     rend le resultat 2 (barriere en echec) et ne masque rien ;
  4. le banc : le test de reproduction (`autres_coeurs_au_repos >= 1`) est
     INCHANGE ; un refus ajoute `cible-inadmissible` au NON_QUALIFIANT ; la
     remesure reste bornee a 3 et ne remesure que le code 2 ;
  5. p13-cible-kvm : le coeur un impose en SMP2 exige le code 2 exactement
     et la ligne de refus, et la preuve est publiee.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "shell": RACINE / "src/shell/mod.rs",
    "banc": RACINE / "tools/ci/run_reveil_coeur_muet.sh",
    "ci": RACINE / ".github/workflows/p13-cible-kvm.yml",
}
REGLE = ("fn cible_masquage_admissible(coeur: usize, en_ligne: usize) -> bool {\n"
         "    coeur < en_ligne && (1..en_ligne).any(|c| c != coeur)\n}")
BOUCLE = "    while index != usize::MAX && etat != 9 && !cible_masquage_admissible(cible, en_ligne) {"
FORCE = "        if force != usize::MAX || crate::kernel::timer::monotonic_ns() >= limite {"
BORNE = "    let limite = debut.saturating_add(5_000_000_000);"
RELU = "        (etat, _, coeur) = crate::kernel::task::etat_tache_brut(index);\n        cible = coeur as usize;"
REFUS = "    if index != usize::MAX && etat != 9 && !cible_masquage_admissible(cible, en_ligne) {"
REFUS_LIGNE = '"SERIE_DORMEUR_MASQUE_ECHEC raison=cible-inadmissible coeur={} coeurs={} ecartees={} attente_ms={} force={}",'
REFUS_RESULTAT = "        DORMEUR_RESULTAT.store(2, Ordering::Release);\n        loop {"
MASQUEUR = 'crate::kernel::task::spawn_noyau_epingle(fil_dormeur_masqueur, "dormeur-masqueur", cible)'
EPINGLE = 'crate::kernel::task::spawn_noyau_epingle(fil_dormeur_epingle, "dormeur-epingle", cible)'
REPRODUCTION = ('[ "${ligne:+x}" = x ] && [ "$(champ autres_coeurs_au_repos)" -ge 1 ] 2>/dev/null '
                '|| nq="$nq aucun-autre-coeur-au-repos"')
NQ_REFUS = "grep -aq 'SERIE_DORMEUR_MASQUE_ECHEC raison=cible-inadmissible' \"$LOG\" && nq=\"$nq cible-inadmissible\""
NQ_SORTIE = '  echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"\n  exit 2'
BORNE_ESSAIS = 'case "$TENTATIVES" in 1|2|3) ;; *)'
REMESURE = '    if [ "$code" != 2 ]; then exit "$code"; fi'
AUTORUN = '  echo "serie-dormeur-masque $MASQUE ${BANC_DORMEUR_COEUR:-}"'
CI_ARME = "BANC_DORMEUR_COEUR=1 BO_SMP=2 BO_QEMU_KVM=1 tools/ci/run_reveil_coeur_muet.sh boot-livree.bin 3000"
CI_LIGNE = "grep -aq 'SERIE_DORMEUR_MASQUE_ECHEC raison=cible-inadmissible coeur=1 coeurs=2' precondition-reveil-kvm-smp2-coeur1.log"
CI_CODE = 'if [ "$code" != 2 ]; then echo "PRECONDITION_REVEIL code=$code (NON_QUALIFIANT attendu : 2)" >&2; exit 1; fi'
CI_PREUVE = "            precondition-reveil-kvm-*.log"


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


def admissible_selon_source(shell: str):
    """La regle telle que la source l'ecrit, traduite en Python, ou None."""
    regle = corps(shell, "fn cible_masquage_admissible(")
    if REGLE not in shell or not regle:
        return None
    return lambda coeur, en_ligne: coeur < en_ligne and any(c != coeur for c in range(1, en_ligne))


TABLE = [((0, 2), True), ((1, 2), False), ((0, 1), False), ((2, 2), False),
         ((0, 4), True), ((1, 4), True), ((3, 4), True), ((4, 4), False), ((7, 8), True)]


def verifie(t: dict) -> list:
    fautes = []
    regle = admissible_selon_source(t["shell"])
    if regle is None or any(regle(*e) != attendu for e, attendu in TABLE):
        fautes.append("shell : la regle d'admissibilite n'est plus « un AP electeur hors du coeur masque ».")
    pilote = corps(t["shell"], "fn fil_dormeur_pilote()")
    ordre = [pilote.find(j) for j in (BORNE, BOUCLE, FORCE, RELU, REFUS, REFUS_LIGNE, REFUS_RESULTAT, MASQUEUR, EPINGLE)]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("fil_dormeur_pilote : la cible n'est plus verifiee (bornee, coeur impose jamais remplace) "
                      "AVANT le masqueur, ou un refus ne rend plus 2 sans masquer.")
    if pilote.count("spawn_noyau_epingle(") != 2 or "coeur as usize)" in pilote.split(REFUS)[-1]:
        fautes.append("fil_dormeur_pilote : le masqueur ou le fil epingle ne visent plus la cible verifiee.")
    b = t["banc"]
    for quoi, extrait in (("le test de reproduction a change", REPRODUCTION),
                          ("un refus n'est plus NON_QUALIFIANT", NQ_REFUS),
                          ("le NON_QUALIFIANT ne sort plus en code 2", NQ_SORTIE),
                          ("la remesure n'est plus bornee a 3", BORNE_ESSAIS),
                          ("la remesure rejoue autre chose que le code 2", REMESURE),
                          ("le coeur impose n'est plus transmis", AUTORUN)):
        if extrait not in b:
            fautes.append(f"run_reveil_coeur_muet.sh : {quoi}.")
    for quoi, extrait in (("le bras coeur un impose a disparu", CI_ARME),
                          ("la ligne de refus n'est plus exigee", CI_LIGNE),
                          ("le code 2 n'est plus exige", CI_CODE),
                          ("la preuve n'est plus publiee", CI_PREUVE)):
        if extrait not in t["ci"]:
            fautes.append(f"p13-cible-kvm.yml : {quoi}.")
    return fautes


def negatifs(t: dict) -> tuple:
    mutations = [
        ("BSP compte electeur", "shell", "(1..en_ligne).any(", "(0..en_ligne).any("),
        ("toujours admissible", "shell", "    coeur < en_ligne && (1..en_ligne)", "    true || (1..en_ligne)"),
        ("coeur masque compte", "shell", ".any(|c| c != coeur)", ".any(|c| c == c)"),
        ("cible non verifiee", "shell", BOUCLE, "    while false {"),
        ("coeur impose remplace", "shell", FORCE, "        if crate::kernel::timer::monotonic_ns() >= limite {"),
        ("attente sans borne", "shell", FORCE, "        if force != usize::MAX {"),
        ("coeur jamais relu", "shell", RELU, "        cible = cible;"),
        ("refus sans resultat", "shell", REFUS_RESULTAT, "        loop {"),
        ("refus masque quand meme", "shell", REFUS_RESULTAT,
         "        DORMEUR_RESULTAT.store(2, Ordering::Release);\n        if false { loop {} }\n        loop {"),
        ("masqueur sur le coeur brut", "shell", MASQUEUR, MASQUEUR.replace(", cible)", ", coeur as usize)")),
        ("refus non publie", "shell", "raison=cible-inadmissible coeur={}", "raison=autre coeur={}"),
        ("reproduction assouplie", "banc", '"$(champ autres_coeurs_au_repos)" -ge 1', '"$(champ autres_coeurs_au_repos)" -ge 0'),
        ("refus compte vert", "banc", NQ_REFUS, "true"),
        ("non qualifiant vert", "banc", NQ_SORTIE, '  echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"\n  exit 0'),
        ("essais sans borne", "banc", BORNE_ESSAIS, 'case "$TENTATIVES" in [0-9]*) ;; *)'),
        ("echec remesure", "banc", REMESURE, '    if [ "$code" = 0 ]; then exit 0; fi'),
        ("coeur impose perdu", "banc", AUTORUN, '  echo "serie-dormeur-masque $MASQUE"'),
        ("ci accepte le vert", "ci", CI_CODE, 'if [ "$code" = 1 ]; then exit 1; fi'),
        ("ci sans ligne de refus", "ci", CI_LIGNE, "true"),
        ("ci sans preuve", "ci", CI_PREUVE + "\n", ""),
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
            print(f"verifie-reveil-cible-admissible: {f}")
        return 1
    print(f"verifie-reveil-cible-admissible: OK (cible a AP electeur, verifiee avant le masquage, refus "
          f"NON_QUALIFIANT, reproduction inchangee ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
