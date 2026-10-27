#!/usr/bin/env python3
"""Un morceau de console cede le coeur a une interactive qui attend
(P13_CONSOLE_MORCEAU_CEDE_V1).

Le fil `console` emet ses morceaux preemption coupee. Sous KVM imbrique un
octet coute deux sorties de VM : 16 octets tenaient le coeur jusqu'a
785-1638 us pendant qu'une interactive prete attendait son election.

Regles :

  1. `avance_tete` ne s'arrete tot que pour un budget borne (le morceau du
     fil) ; les chemins synchrone et de panique (`usize::MAX`) emettent tout ;
  2. au moins un octet est emis avant tout arret (progres garanti) ;
  3. l'enregistrement reprend la ou il s'est arrete (`a = de + emis`, puis
     `CONSOLE_ENTAME`) : memes octets, meme ordre ;
  4. le predicat `interactive_attend_ici` ne lit que des atomiques (aucun
     registre, aucun verrou) : il est appele preemption coupee, jeton tenu ;
  5. `point_cooperatif` recalcule une borne interactive echue meme au-dela
     du plancher, pour qu'une borne perimee ne coupe pas chaque morceau.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "uart": RACINE / "src/drivers/serial/uart16550.rs",
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
}


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
                return source[ouverture:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    at = corps(t["uart"], "fn avance_tete(")
    if "let cede = budget != usize::MAX;" not in at:
        fautes.append("avance_tete : l'arret anticipe n'est plus reserve au budget borne")
    ecrit = at.find("ecris_octets_sans_prefixe(core::slice::from_ref(octet));")
    compte = at.find("emis += 1;")
    predicat = at.find("arret = crate::kernel::task::interactive_attend_ici();")
    if not 0 <= at.find("if arret {") < ecrit < compte < predicat:
        fautes.append("avance_tete : un arret peut survenir avant le premier octet, ou n'est pas compte")
    if "if !cede {\n            ecris_octets_sans_prefixe(morceau);\n            emis += morceau.len();" not in at:
        fautes.append("avance_tete : les chemins synchrone/panique n'emettent plus tout")
    if "let a = de + emis;" not in at or at.find("let a = de + emis;") > at.find("if a < n {"):
        fautes.append("avance_tete : la reprise ne suit plus les octets reellement emis")
    ici = corps(t["sommeil"], "pub fn interactive_attend_ici()")
    if not ici or "tasks()" in ici or "borne_interactive_a_jour" in ici or ".lock()" in ici:
        fautes.append("interactive_attend_ici : lit autre chose que des atomiques")
    point = corps(t["sommeil"], "pub fn point_cooperatif(")
    garde = point[point.find("let mut borne_interactive"):point.find("borne_interactive_a_jour(cpu, maintenant)")]
    if "segment < SEGMENT_MIN_NS" in garde or "if en_file == 0\n        && prochaine_echeance_interactive_ns(cpu) <= maintenant" not in garde:
        fautes.append("point_cooperatif : une borne echue perimee peut survivre au point")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("panique coupee", "uart", "let cede = budget != usize::MAX;", "let cede = true;"),
        ("arret avant le premier octet", "uart",
         "            ecris_octets_sans_prefixe(core::slice::from_ref(octet));\n            emis += 1;\n            arret = crate::kernel::task::interactive_attend_ici();",
         "            arret = crate::kernel::task::interactive_attend_ici();\n            if arret { return; }\n            ecris_octets_sans_prefixe(core::slice::from_ref(octet));\n            emis += 1;"),
        ("reprise fausse", "uart", "let a = de + emis;", "let a = a;"),
        ("predicat sur le registre", "sommeil",
         "        en_file, prochaine_echeance_interactive_ns(cpu), crate::kernel::timer::monotonic_ns())",
         "        en_file, borne_interactive_a_jour(cpu, crate::kernel::timer::monotonic_ns()), crate::kernel::timer::monotonic_ns())"),
        ("borne perimee conservee", "sommeil",
         "    if en_file == 0\n        && prochaine_echeance_interactive_ns(cpu) <= maintenant",
         "    if en_file == 0 && segment < SEGMENT_MIN_NS\n        && prochaine_echeance_interactive_ns(cpu) <= maintenant"),
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
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t) + negatifs(t)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("morceau de console qui cede : ok (5 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
