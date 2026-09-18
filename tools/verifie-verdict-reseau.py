#!/usr/bin/env python3
"""Garde-fou : un verdict reseau ne se pose qu'a UN endroit.

# Le defaut, releve en photo et en archive le 18 septembre 2026

Sur la TRIGKEY, la fenetre Services et la barre du haut annoncaient :

    verdict=sans-configuration   ip=10.0.2.15   gw=10.0.2.2   dns=10.0.2.3

Une adresse SLIRP de QEMU, sur un cable de bureau. Chaque paquet sortant
cherchait par ARP une passerelle qui n'existe pas sur ce reseau -- quatre
tentatives, deux secondes, un echec mis en cache -- et l'application Services
affichait `ipv4 Actif` au-dessus d'un reseau injoignable.

La regle « carte reelle + aucun bail => on oublie la presomption SLIRP »
existait pourtant. Elle etait appliquee a UN des deux endroits qui posent le
verdict :

- `demarre()` l'appliquait ;
- le veilleur de lien, qui retente le DHCP quand le cable monte, ne
  l'appliquait pas.

Or sur cette machine c'est TOUJOURS le veilleur qui pose `SansConfiguration` :
au demarrage le lien est encore bas, donc le verdict initial est `LienBas`.
La seule branche qui tenait la regle etait celle qui ne s'executait jamais.

# L'invariant defendu ici

`DEMARRAGE` ne s'ecrit que dans `pose_le_verdict`, et `pose_le_verdict` efface
la presomption SLIRP avant de poser `SansConfiguration`.

Une regle qui vit a un seul endroit ne peut plus etre oubliee a l'autre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
NET = RACINE / "src/net/mod.rs"

POSEUR = "fn pose_le_verdict(nouvel_etat: Demarrage) {"


def code_seul(source):
    """Le code sans les commentaires : cette garde s'accuserait elle-meme,
    puisque l'en-tete du module explique le defaut qu'elle defend."""
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def corps(source, signature):
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:const |static |fn |struct |impl |enum |mod )", reste)
    return reste[: fin.start()] if fin else reste


def main():
    fautes = []
    try:
        brut = NET.read_text(encoding="utf-8", errors="replace")
    except OSError:
        print("  - src/net/mod.rs est illisible.")
        return 1
    pur = code_seul(brut)

    # ------------------------------------------------------------------ 1
    #
    # UN SEUL ECRIVAIN. Le compte porte sur le code, pas sur la prose.
    ecritures = [
        (no, ligne.strip())
        for no, ligne in enumerate(pur.splitlines(), 1)
        if re.search(r"\bDEMARRAGE\s*=", ligne)
    ]
    if not ecritures:
        fautes.append(
            "personne ne pose plus le verdict de demarrage : `DEMARRAGE` "
            "n'est plus ecrit nulle part."
        )
    elif len(ecritures) > 1:
        fautes.append(
            "`DEMARRAGE` est ecrit a %d endroits (lignes %s) : la regle qui "
            "efface la presomption SLIRP sera oubliee a l'un d'eux, comme "
            "elle l'a ete par le veilleur de lien."
            % (len(ecritures), ", ".join(str(no) for no, _ in ecritures))
        )

    # ------------------------------------------------------------------ 2
    #
    # ET CET ECRIVAIN TIENT LA REGLE.
    corps_poseur = corps(pur, POSEUR)
    if corps_poseur is None:
        fautes.append(
            "`%s` est introuvable : le verdict n'a plus de point de passage "
            "unique." % POSEUR
        )
    else:
        if "SansConfiguration" not in corps_poseur:
            fautes.append(
                "`pose_le_verdict` ne distingue plus `SansConfiguration` : "
                "une carte reelle sans bail garderait l'adresse SLIRP."
            )
        if "oublie_la_presomption_slirp" not in corps_poseur:
            fautes.append(
                "`pose_le_verdict` n'efface plus la presomption SLIRP : la "
                "machine annoncerait 10.0.2.15 sur un cable de bureau."
            )
        if "DEMARRAGE" not in corps_poseur:
            fautes.append("`pose_le_verdict` ne pose plus le verdict.")

    # ------------------------------------------------------------------ 3
    #
    # LA PRESOMPTION RESTE CANTONNEE. Aucune adresse SLIRP en dur ailleurs.
    for chemin in sorted(RACINE.joinpath("src").rglob("*.rs")):
        try:
            pur_fichier = code_seul(chemin.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
        if chemin == NET:
            continue
        if re.search(r"\[\s*10\s*,\s*0\s*,\s*2\s*,\s*(15|2|3)\s*\]", pur_fichier):
            fautes.append(
                "%s code une adresse SLIRP en dur : elle ne vaut que sous "
                "QEMU, et `net/mod.rs` est le seul endroit qui a le droit de "
                "la presumer." % chemin.relative_to(RACINE)
            )

    if fautes:
        print("VERDICT RESEAU : %d manquement(s)" % len(fautes))
        for f in fautes:
            print("  - %s" % f)
        return 1

    print("verdict reseau : un seul poseur, et il oublie la presomption SLIRP.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
