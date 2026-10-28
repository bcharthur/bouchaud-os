#!/usr/bin/env python3
"""La demande differee est jugee quand elle est servie, avec l'horloge de
l'activation en cours (P13_ACTIVATION_MINIMALE_V1).

C1g : `console`, fil noyau normal publie en demande differee, faisait ceder
la sonde au repos deux fois de suite, ~1,1 ms chaque fois, et P1 tombait.
Une regle prise A LA PUBLICATION (d8095136) a ete mesuree inefficace : la
demande etait posee contre un autre occupant, ou contre une sonde endormie
restee installee dont la « residence » comptait le sommeil, puis servie plus
tard a la sonde reprise -- en fin d'appel systeme, sans aucune regle, puis au
`safe_point` qui suit (run 40694203600).

Regles :

  1. `reveil::activation_garde` est pure et testee sur l'hote : une tache
     UTILISATEUR, aucune interactive en attente, au moins une normale, moins
     de `RESIDENCE_MINIMALE_NS` d'activation -- pas une constante a part ;
  2. `courante_garde_le_coeur` la consulte quand la regle de classe ne garde
     pas, avec la residence de `profil_occupant` (l'horloge de l'activation),
     et compte ses gardes ;
  3. `schedule()` : la tache reveillee qui reprend sur place apres
     `commit_scheduler_idle_sauf` date le debut de son activation
     (`note_reprise_installee`), interruptions masquees ;
  4. `sert_demande_differee` : la demande prise, la regle consultee, une
     cession refusee REPOSE la demande, sinon `yield_now` ;
  5. les trois fins d'appel systeme la servent par `sert_demande_differee`,
     et `take_need_resched` n'est appele nulle part ailleurs.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "reveil": RACINE / "src/kernel/scheduler/reveil.rs",
    "ordo": RACINE / "src/kernel/process/thread/ordonnancement.rs",
    "courant": RACINE / "src/kernel/process/thread/courant.rs",
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
    "usermode": RACINE / "src/arch/x86_64/usermode.rs",
    "linux": RACINE / "src/compat/linux/mod.rs",
    "test": RACINE / "tools/smp/test_reveil_cible.rs",
}
REGLE = ("    utilisateur && attente_interactive == 0 && attente_normale > 0 "
         "&& residence_ns < RESIDENCE_MINIMALE_NS")
CONSULTE = ("        if !activation_garde(!tache.noyau, residence, interactives, normales) {\n"
            "            return false;\n        }")
HORLOGE = "    let depuis = profil_occupant(cpu).2;"
RESIDENCE = "    let residence = if depuis == 0 { u64::MAX } else { maintenant.saturating_sub(depuis) };"
COMPTE = "            ACTIVATIONS_GARDEES.fetch_add(1, Ordering::Relaxed);"
REPRISE = ("                cpu::commit_scheduler_idle_sauf(|| tasks()[cur].state == TaskState::Ready);\n"
           "                // P13_ACTIVATION_MINIMALE_V1 : reveillee, elle reprend sur\n"
           "                // place -- son activation commence ici, pas a son `install`.\n"
           "                if tasks()[cur].state == TaskState::Ready {\n"
           "                    note_reprise_installee();\n                }")
MASQUE = ("    ::x86_64::instructions::interrupts::without_interrupts(|| {\n"
          "        CURRENT_DEPUIS_NS[local_cpu()].store(crate::kernel::timer::monotonic_ns(), Ordering::Release);\n"
          "    });")
SERT = ("    if !take_need_resched() {\n        return;\n    }\n"
        "    if courante_garde_le_coeur_local() {\n        set_need_resched();\n        return;\n    }\n"
        "    yield_now();")
TESTS = ("une_tache_utilisateur_reprise_garde_son_coeur_face_a_des_normales",
         "l_activation_minimale_ne_protege_pas_un_fil_noyau",
         "une_interactive_en_attente_coupe_l_activation_minimale",
         "une_activation_qui_dure_redevient_preemptible",
         "rien_a_garder_quand_personne_n_attend",
         "la_sonde_reprise_ne_cede_plus_a_console")


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


def sans_commentaires(texte: str) -> str:
    return "\n".join(l for l in texte.splitlines() if not l.strip().startswith("//"))


def verifie(t: dict, src: str) -> list:
    fautes = []
    regle = corps(t["reveil"], "pub fn activation_garde(")
    if REGLE not in regle:
        fautes.append("reveil.rs : activation_garde n'est plus la regle (utilisateur, aucune interactive, "
                      "une normale, moins de RESIDENCE_MINIMALE_NS).")
    garde = corps(t["ordo"], "fn courante_garde_le_coeur(")
    ordre = [garde.find(j) for j in (HORLOGE, RESIDENCE, "if cession(classe, consomme, interactives, normales) != Cession::Garde {",
                                     CONSULTE, COMPTE, "    if cession_par_classe() {\n        CESSIONS_GARDEES")]
    if min(ordre) < 0 or ordre != sorted(ordre):
        fautes.append("courante_garde_le_coeur : l'activation minimale n'est plus consultee avec l'horloge "
                      "de l'activation, ou ses gardes ne sont plus comptees.")
    schedule = corps(t["ordo"], "pub fn schedule() -> bool")
    if REPRISE not in schedule or schedule.find(REPRISE) > schedule.find("rearme_compta_apres_idle();"):
        fautes.append("schedule() : la reprise sur place ne date plus le debut de l'activation.")
    if MASQUE not in corps(t["courant"], "fn note_reprise_installee()"):
        fautes.append("courant.rs : note_reprise_installee ne lit plus le coeur interruptions masquees.")
    if SERT not in corps(t["sommeil"], "pub fn sert_demande_differee()"):
        fautes.append("sommeil.rs : sert_demande_differee ne juge plus la demande, ou perd la demande refusee.")
    if sans_commentaires(t["linux"]).count("task::sert_demande_differee();") != 1:
        fautes.append("compat/linux : la fin d'appel systeme ne sert plus la demande par la regle.")
    if sans_commentaires(t["usermode"]).count("crate::kernel::task::sert_demande_differee();") != 2:
        fautes.append("usermode.rs : les fins d'appel natives ne servent plus la demande par la regle.")
    appels = [l for l in sans_commentaires(src).splitlines()
              if re.search(r"\btake_need_resched\(\)", l) and "pub fn take_need_resched" not in l]
    if len(appels) != 1 or "if !take_need_resched() {" not in appels[0]:
        fautes.append("take_need_resched est consomme hors de sert_demande_differee : une cession sans regle.")
    for nom in TESTS:
        if f"fn {nom}()" not in t["test"]:
            fautes.append(f"test hote absent : {nom}.")
    return fautes


def sources() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in sorted((RACINE / "src").rglob("*.rs")))


def negatifs(t: dict, src: str) -> list:
    mutations = [
        ("fil noyau protege", "reveil", "    utilisateur && attente_interactive", "    attente_interactive"),
        ("garde face a une interactive", "reveil", " && attente_interactive == 0 && attente_normale", " && attente_normale"),
        ("sans borne", "reveil", " && residence_ns < RESIDENCE_MINIMALE_NS", ""),
        ("horloge de la tranche", "ordo", HORLOGE, "    let depuis = debut;"),
        ("regle non consultee", "ordo", CONSULTE, "        return false;"),
        ("gardes non comptees", "ordo", COMPTE, ""),
        ("reprise non datee", "ordo", "                    note_reprise_installee();\n", ""),
        ("coeur lu interruptions ouvertes", "courant",
         "    ::x86_64::instructions::interrupts::without_interrupts(|| {\n", "    ({\n"),
        ("demande perdue au refus", "sommeil", "        set_need_resched();\n        return;", "        return;"),
        ("cession sans regle", "sommeil", "    if courante_garde_le_coeur_local() {", "    if false {"),
        ("fin d'appel linux sans regle", "linux", "    task::sert_demande_differee();",
         "    if task::take_need_resched() {\n        task::yield_now();\n    }"),
        ("fin d'appel native sans regle", "usermode", "            crate::kernel::task::sert_demande_differee();",
         "            if crate::kernel::task::take_need_resched() { crate::kernel::task::yield_now(); }"),
        ("cas mesure non teste", "test", "fn la_sonde_reprise_ne_cede_plus_a_console()", "fn x()"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = t[cle].replace(avant, apres, 1)
        s = src.replace(t[cle], m[cle]) if cle != "test" else src
        if not verifie(m, s):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {k: p.read_text(encoding="utf-8") for k, p in F.items()}
    src = sources()
    echecs, n = negatifs(t, src)
    fautes = verifie(t, src) + echecs
    if fautes:
        for f in fautes:
            print(f"verifie-activation-minimale: {f}")
        return 1
    print(f"verifie-activation-minimale: OK (regle pure bornee par RESIDENCE_MINIMALE_NS, horloge de "
          f"l'activation, reprise sur place datee, trois fins d'appel jugees ; {n} negatifs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
