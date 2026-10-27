#!/usr/bin/env python3
"""L'ordonnanceur reloge lui-meme les taches pretes en file sur un coeur muet
(P13_REVEIL_COEUR_MUET_V2).

KVM SMP2, run 40582472007 : le dormeur, publie sur le coeur zero que le banc
masquait, y restait 193 a 420 ms -- le coeur un elisait (battements a l'heure,
vol de l'hote nul) mais ne volait que sur file vide. Le secours des coeurs
muets existait ; seul le chien de garde de la console l'appelait.

Regles :

  1. le tic du BSP (`idt/timer.rs`) et le vecteur local des AP
     (`idt/reschedule.rs`) appellent `secours_coeurs_muets()` ;
  2. `secours_coeurs_muets` (hors temoin) designe les coeurs par
     `coeurs_muets(..., SECOURS_SEUIL_NS)`, sort sans parcours quand aucun ne
     l'est, n'entre qu'un passage a la fois, ne reloge que par
     `reloge_tache_coeur_muet` et compte dans SON compteur ;
  3. `reloge_tache_coeur_muet` garde ses gardes : fil noyau epingle jamais
     deplace, `secours_permis`, affinite (`allowed_on`), propriete par
     `remove`, barriere puis `request_cpu` et IPI ; il ne compte rien --
     `secours_tache_coeur_muet` (console) compte SES relogements seul ;
  4. `coeurs_muets` exige file non vide ET `coeur_muet` ;
  5. le temoin `banc-reveil-historique` n'a pas de secours par l'ordonnanceur
     (fonction vide sous l'option) ;
  6. tests hote de `coeurs_muets` presents.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "timer": RACINE / "src/arch/x86_64/idt/timer.rs",
    "resched": RACINE / "src/arch/x86_64/idt/reschedule.rs",
    "creation": RACINE / "src/kernel/process/thread/creation.rs",
    "secours": RACINE / "src/kernel/scheduler/secours.rs",
    "test": RACINE / "tools/smp/test_secours_coeur_muet.rs",
}
APPEL = "crate::kernel::task::secours_coeurs_muets();"
PROD = '#[cfg(not(feature = "banc-reveil-historique"))]\npub fn secours_coeurs_muets() {'
TEMOIN = '#[cfg(feature = "banc-reveil-historique")]\npub fn secours_coeurs_muets() {}'


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", source.find(")", debut))
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
    for cle in ("timer", "resched"):
        if APPEL not in t[cle]:
            fautes.append(f"{cle} : le battement ne secourt pas les coeurs muets")
    c = t["creation"]
    if PROD not in c:
        fautes.append("secours_coeurs_muets : version livree absente ou hors option")
    p = corps(c, PROD)
    if "coeurs_muets(&coeurs[..en_ligne], maintenant, SECOURS_SEUIL_NS)" not in p:
        fautes.append("secours_coeurs_muets : designation hors de la regle pure et de son seuil")
    if "if muets == 0 || SECOURS_ORDONNANCEUR_EN_COURS.swap(true, Ordering::AcqRel) {\n        return;" not in p:
        fautes.append("secours_coeurs_muets : parcours sans coeur muet, ou passages concurrents")
    if "SECOURS_ORDONNANCEUR_EN_COURS.store(false, Ordering::Release);" not in p:
        fautes.append("secours_coeurs_muets : garde jamais rendue")
    if "reloge_tache_coeur_muet(index, SECOURS_SEUIL_NS).is_some()" not in p \
            or "SECOURS_ORDONNANCEUR_RELOGEES.fetch_add" not in p:
        fautes.append("secours_coeurs_muets : relogement hors du chemin garde, ou non compte")
    if "SECOURS_RELOGEES" in p:
        fautes.append("secours_coeurs_muets : compte dans le compteur de la console")
    r = corps(c, "fn reloge_tache_coeur_muet(")
    for garde, nom in (
        ("if tache.noyau && !tache.migrable {\n        return None;", "fil noyau epingle deplace"),
        ("if !secours_permis(occupant_depuis, tache.ready_since_ns.charge(), maintenant, seuil_ns) {", "regle du secours ignoree"),
        ("allowed_on(tache, cpu)", "affinite ignoree"),
        (".remove(identite.en_mot())", "propriete de l'entree ignoree"),
        ("core::sync::atomic::fence(Ordering::SeqCst);", "barriere du motif croise retiree"),
        ("smp::reschedule_cpu(cible);", "coeur d'accueil non prevenu"),
    ):
        if garde not in r:
            fautes.append(f"reloge_tache_coeur_muet : {nom}")
    if "SECOURS_RELOGEES" in r:
        fautes.append("reloge_tache_coeur_muet : compte les relogements de tous dans celui de la console")
    s = corps(c, "pub fn secours_tache_coeur_muet(")
    if "reloge_tache_coeur_muet(index, seuil_ns)?" not in s or "SECOURS_RELOGEES.fetch_add" not in s:
        fautes.append("secours_tache_coeur_muet : ne compte plus ses propres relogements")
    m = corps(t["secours"], "pub fn coeurs_muets(")
    if "if file && coeur_muet(depuis, maintenant_ns, seuil_ns) {" not in m:
        fautes.append("coeurs_muets : designe un coeur sans file, ou sans la regle coeur_muet")
    if TEMOIN not in c:
        fautes.append("temoin : l'ordonnanceur y secourt (le banc ne prouverait plus rien)")
    for test in ("le_coeur_masque_par_un_fil_noyau_est_designe_des_qu_il_a_une_file",
                 "un_coeur_muet_sans_file_n_a_rien_a_rendre",
                 "ni_le_repos_ni_le_quantum_ne_designent_un_coeur"):
        if f"fn {test}()" not in t["test"]:
            fautes.append(f"test hote absent : {test}")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("tic du BSP sans secours", "timer", APPEL, ""),
        ("vecteur local sans secours", "resched", APPEL, ""),
        ("seuil remplace", "creation", "coeurs_muets(&coeurs[..en_ligne], maintenant, SECOURS_SEUIL_NS)",
         "coeurs_muets(&coeurs[..en_ligne], maintenant, 0)"),
        ("passages concurrents", "creation", "if muets == 0 || SECOURS_ORDONNANCEUR_EN_COURS.swap(true, Ordering::AcqRel) {",
         "if muets == 0 {"),
        ("garde jamais rendue", "creation", "    SECOURS_ORDONNANCEUR_EN_COURS.store(false, Ordering::Release);\n", ""),
        ("fil epingle deplace", "creation", "if tache.noyau && !tache.migrable {\n        return None;\n    }\n    let donneur",
         "let donneur"),
        ("affinite ignoree", "creation", "*c = accueil(allowed_on(tache, cpu),", "*c = accueil(true,"),
        ("barriere retiree", "creation", "    // Meme motif croise que `publish_ready` : mise en file, barriere, puis\n"
         "    // lecture de l'etat du coeur d'accueil.\n    core::sync::atomic::fence(Ordering::SeqCst);\n", ""),
        ("compteur console partage", "creation", "            SECOURS_ORDONNANCEUR_RELOGEES.fetch_add(1, Ordering::Relaxed);",
         "            SECOURS_RELOGEES.fetch_add(1, Ordering::Relaxed);"),
        ("coeur sans file designe", "secours", "if file && coeur_muet(depuis, maintenant_ns, seuil_ns) {",
         "if coeur_muet(depuis, maintenant_ns, seuil_ns) {"),
        ("temoin secouru", "creation", TEMOIN, '#[cfg(feature = "banc-reveil-historique")]\npub fn secours_coeurs_muets() { secours_coeurs_muets_livre(); }'),
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
    return echecs, len(mutations)


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    echecs, n = negatifs(t)
    fautes = verifie(t) + echecs
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print(f"secours des coeurs muets par l'ordonnanceur : ok ({n} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
