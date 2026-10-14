#!/usr/bin/env python3
"""ordonnanceur-probe ne rend jamais un vert qu'elle n'a pas mesure
(BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1, BOUCHAUD_ATTRIBUTION_REVEIL_V2).

Run 38569175760 (KVM SMP1) : trois passes « RESULTAT : 0 » sans aucune
comparaison -- le temoin au repos, contamine par des tics perdus, etait plus
lent que la phase sous charge. Regles :

  1. la sonde rend 2 sur NON_QUALIFIANT, et sa ligne RESULTAT y commence par
     `RESULTAT : NON_QUALIFIANT` : la forme `RESULTAT : <n> verification(s)
     en echec (<p> passees)` n'est imprimee qu'apres ce retour ;
  2. chaque comparaison est jugee sur tous les reveils ET sur les reveils
     propres (`juge(nom, tout, propre, ...)`), et les reveils contamines sont
     exclus des propres ;
  3. une porte non qualifiee (charge sans effet, ou porte contaminee) rend
     les deux comparaisons de priorite NON_QUALIFIANTES ;
  4. le noyau ventile chaque sommeil en retard (`ventile` dans
     `dort_jusqua`), journalise les absences du PIT et du quantum local, et
     `/proc/self/reveil` rend la ligne `ventilation` et la liste ;
  5. le banc d'attribution sort en 2 sur un NON_QUALIFIANT, sauf
     BANC_TEMOIN_INSTRUMENT=1, dont le marqueur dit qu'il ne qualifie pas
     P13 ; l'endurance n'imprime son marqueur que sur le code 0 de la sonde ;
     le rapport d'endurance lit la forme NON_QUALIFIANT.

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
FICHIERS = {
    "sonde": RACINE / "tools/userland/ordonnanceur-probe.c",
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
    "attribution": RACINE / "src/kernel/scheduler/attribution_reveil.rs",
    "replanif": RACINE / "src/arch/x86_64/idt/reschedule.rs",
    "fd": RACINE / "src/kernel/object/fd.rs",
    "banc": RACINE / "tools/ci/run_ordonnanceur_attribution.sh",
    "endurance": RACINE / "tools/ci/fabrique-scenario-endurance.sh",
    "rapport": RACINE / "tools/ci/reliability/budgets_endurance.py",
    "flux": RACINE / ".github/workflows/os-primitives.yml",
}

RETOUR_NQ = re.compile(
    r'printf\("RESULTAT : NON_QUALIFIANT [^"]*"[^;]*;\s*return 2;', re.S)
RESULTAT_QUALIFIE = 'printf("RESULTAT : %d verification(s) en echec (%d passees)\\n"'


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\b" + re.escape(nom) + r"\s*\([^;{]*\)[^;{()]*\{", texte)
    if not m:
        return ""
    o = m.end() - 1
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    s = t["sonde"]
    # 1. Code 2 et forme NON_QUALIFIANT, avant toute forme qualifiee.
    m = RETOUR_NQ.search(s)
    if not m:
        fautes.append("ordonnanceur-probe.c : NON_QUALIFIANT ne rend plus 2, ou sa ligne RESULTAT a change de forme.")
    if s.count(RESULTAT_QUALIFIE) != 1 or (m and s.find(RESULTAT_QUALIFIE) < m.end()):
        fautes.append("ordonnanceur-probe.c : la forme « RESULTAT : n verification(s) » peut etre imprimee pour un NON_QUALIFIANT.")
    # 2. Deux jugements, reveils contamines exclus.
    fin = corps(s, "finalise_phase")
    if "if (!ph->contamine[i])" not in fin or "quantiles(propres, n, &ph->propre)" not in fin:
        fautes.append("ordonnanceur-probe.c : les reveils contamines ne sont plus exclus des reveils propres.")
    juge = corps(s, "juge")
    if "} else if (tout == propre) {" not in juge or not re.search(
            r'\} else \{\s*v = NON_QUALIFIANT;\s*snprintf\(raison, taille, "%s : %s sur tous les reveils', juge):
        fautes.append("ordonnanceur-probe.c : un desaccord tous/propres ne rend plus NON_QUALIFIANT.")
    ev = corps(s, "evalue")
    for expression in ("normale->propre.p95 > repos->propre.p95 + plancher",
                       "prio->propre.p99 < normale->propre.p99 || prio->propre.pire < normale->propre.pire",
                       "prio->propre.pire <= normale->propre.pire + repos->propre.pire"):
        if expression not in ev:
            fautes.append(f"ordonnanceur-probe.c : comparaison sur les reveils propres absente : {expression}")
    # 3. Porte non qualifiee : les deux comparaisons ne passent pas.
    if not re.search(r"if \(porte != QUALIFIE_OK\) \{.*?compte\(b, n1, NON_QUALIFIANT, pourquoi\);"
                     r"\s*compte\(b, n2, NON_QUALIFIANT, pourquoi\);", ev, re.S):
        fautes.append("ordonnanceur-probe.c : une porte non qualifiee ne rend plus les comparaisons NON_QUALIFIANTES.")
    # 4. Le noyau ventile et le dit.
    if "ar::ventile(&mesure, &d, local_cpu() as u32, &absences[..n], incomplet)" not in t["sommeil"] \
            or "current().ventilation_reveil.note(&v, mesure.echeance_ns)" not in t["sommeil"]:
        fautes.append("sommeil.rs : un sommeil en retard n'est plus ventile.")
    if "journalise_absence(&a)" not in corps(t["attribution"], "note_tic_tsc"):
        fautes.append("attribution_reveil.rs : les absences du PIT ne sont plus journalisees.")
    if "crate::kernel::attribution_reveil::note_quantum(" not in t["replanif"]:
        fautes.append("reschedule.rs : les absences du quantum local ne sont plus journalisees.")
    pr = corps(t["fd"], "proc_reveil")
    if "ventilation_reveil.releve()" not in pr or '"ventilation seuil_us={}' not in pr or "liste={}" not in pr:
        fautes.append("fd.rs : /proc/self/reveil ne rend plus la ventilation et la liste des contamines.")
    # 5. Consommateurs.
    banc = t["banc"]
    if 'echo "ORDONNANCEUR_ATTRIBUTION_NON_QUALIFIANT $BILAN"\n  exit 2' not in banc \
            or "(verdicts explicites et prouves ; ne qualifie pas P13)" not in banc:
        fautes.append("run_ordonnanceur_attribution.sh : un NON_QUALIFIANT passe pour une reussite.")
    flux = t["flux"]
    if "run_ordonnanceur_attribution.sh" in flux and "ne qualifie pas P13" not in flux:
        fautes.append("os-primitives.yml : le temoin d'instrument ne dit plus qu'il ne qualifie pas P13.")
    if "/bin/ordonnanceur-probe 8 && echo ENDURANCE_ORDONNANCEUR_OK" not in t["endurance"]:
        fautes.append("fabrique-scenario-endurance.sh : le marqueur de la sonde ne depend plus de son code de sortie.")
    if "(NON_QUALIFIANT )?" not in t["rapport"]:
        fautes.append("budgets_endurance.py : le rapport ne lit plus la forme NON_QUALIFIANT.")
    return fautes


def main() -> int:
    t = {}
    for nom, chemin in FICHIERS.items():
        if not chemin.is_file():
            print(f"sonde ordonnanceur : {chemin} absent")
            return 1
        t[nom] = chemin.read_text(encoding="utf-8")
    fautes = verifie(t)
    if fautes:
        print("sonde ordonnanceur : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    negatifs = [
        # l'ancien faux vert : charge non mesurable, et rien ne rougit
        ("sonde", "        compte(b, n1, NON_QUALIFIANT, pourquoi);\n        compte(b, n2, NON_QUALIFIANT, pourquoi);",
         "        (void)pourquoi;"),
        # NON_QUALIFIANT rendu 0
        ("sonde", "               b.echecs, b.non_qualifiantes, b.ok);\n        return 2;",
         "               b.echecs, b.non_qualifiantes, b.ok);\n        return 0;"),
        # un desaccord tranche par « tous »
        ("sonde", "    } else {\n        v = NON_QUALIFIANT;", "    } else {\n        v = tout ? QUALIFIE_OK : QUALIFIE_ECHEC;"),
        # les contamines gardes dans les propres
        ("sonde", "        if (!ph->contamine[i])\n            propres[n++]", "        if (1)\n            propres[n++]"),
        # la marge de la seconde verification jugee sur le seul temoin brut
        ("sonde", "prio->propre.pire <= normale->propre.pire + repos->propre.pire",
         "prio->propre.pire <= normale->propre.pire + repos->tout.pire"),
        # le noyau ne ventile plus
        ("sommeil", "            current().ventilation_reveil.note(&v, mesure.echeance_ns);\n", ""),
        # le PIT ne journalise plus
        ("attribution", "            journalise_absence(&a);\n        }\n    }\n", "        }\n    }\n"),
        # le quantum ne journalise plus
        ("replanif", "    crate::kernel::attribution_reveil::note_quantum(", "    let _ = ("),
        # la liste disparait de /proc/self/reveil
        ("fd", "liste_tronquee={} liste={}", "liste_tronquee={} liste_absente={}"),
        # le banc prend un NON_QUALIFIANT pour une reussite
        ("banc", 'echo "ORDONNANCEUR_ATTRIBUTION_NON_QUALIFIANT $BILAN"\n  exit 2',
         'echo "ORDONNANCEUR_ATTRIBUTION_NON_QUALIFIANT $BILAN"\n  exit 0'),
        # l'endurance imprime son marqueur quel que soit le code
        ("endurance", "/bin/ordonnanceur-probe 8 && echo ENDURANCE_ORDONNANCEUR_OK",
         "/bin/ordonnanceur-probe 8 ; echo ENDURANCE_ORDONNANCEUR_OK"),
        # le rapport ne lit plus la forme NON_QUALIFIANT
        ("rapport", "(NON_QUALIFIANT )?", ""),
    ]
    for n, (cle, a, b) in enumerate(negatifs, 1):
        if t[cle].count(a) < 1:
            print(f"sonde ordonnanceur : le test negatif {n} ne s'applique plus ({cle}) -- garde inoperant")
            return 1
        mute = dict(t)
        mute[cle] = t[cle].replace(a, b, 1)
        if not verifie(mute):
            print(f"sonde ordonnanceur : le test negatif {n} ne rougit pas ({cle}) -- garde inoperant")
            return 1
    print(f"[OK] sonde ordonnanceur : NON_QUALIFIANT rend 2 et jamais « RESULTAT : 0 », chaque comparaison "
          f"jugee sur tous et sur les propres, porte non qualifiee non comptee, noyau ventile, consommateurs "
          f"alignes (negatifs={len(negatifs)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
