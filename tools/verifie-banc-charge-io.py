#!/usr/bin/env python3
"""Garde-fou : le banc de charge mesure bien la concurrence qu'il pretend.

# Pourquoi cette garde existe

Un banc qui passe sans exercer ce qu'il annonce est pire qu'aucun banc : il
transforme une regression en verdict vert. Ce banc-ci a deja produit deux
verdicts trompeurs pendant sa propre mise au point :

  * avec une SEULE cle USB, le fil de charge n'avait rien a lire -- une
    interface Mass Storage dont le disque porte la partition
    BOUCHAUD-BLACKBOX est cedee a l'enregistreur et ne devient pas un volume
    bloc. `lectures=0`, et le banc mesurait un pilote sans concurrence ;
  * avec `-smp 4`, les quinze autres coeurs de la machine cible n'etaient pas
    representes, et les sondes multicoeur ne voyaient rien.

# Ce qui est verifie ici

1. Le banc presente DEUX supports de masse : celui de l'enregistreur et celui
   de la charge. Sans le second, il n'y a aucune concurrence a mesurer.
2. Il presente un clavier et une souris USB : la scrutation HID doit avoir
   quelque chose a scruter, sinon `hid_poll_gap_max_ms` ne mesure rien.
3. Il tourne sur seize coeurs, comme la machine cible.
4. Il exige les QUATRE criteres A, B, C, D -- pas seulement la couverture.
5. Il exige un echantillon dans CHAQUE TIERS de la session. Une archive qui
   garde le debut et la fin sans le milieu est exactement le symptome qu'on
   corrige.
6. Le drapeau `banc-io` reste une option de compilation. Il lit le volume en
   boucle et ETEINT la machine : un reglage a l'execution laisserait la
   possibilite de l'armer par accident sur une image livree.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

BANC = RACINE / "tools/ci/run_trigkey_ladybird_io_stress.sh"
MODULE = RACINE / "src/platform/pc/banc_io.rs"
CARGO = RACINE / "Cargo.toml"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    banc = lit(BANC, fautes)
    module = lit(MODULE, fautes)
    cargo = lit(CARGO, fautes)
    stage2 = lit(STAGE2, fautes)

    if banc is not None:
        # 1. Deux supports de masse.
        supports = len(re.findall(r"-device usb-storage", banc))
        if supports < 2:
            fautes.append(
                "le banc ne presente que %d support(s) de masse. Celui qui "
                "porte la partition BOUCHAUD-BLACKBOX est CEDE a "
                "l'enregistreur et ne devient pas un volume bloc : sans un "
                "second support, le fil de charge n'a rien a lire, et le banc "
                "mesure un pilote sans concurrence -- c'est-a-dire le "
                "contraire de ce qu'il existe pour mesurer." % supports
            )

        # 2. De l'entree a scruter.
        for peripherique, pourquoi in (
            ("usb-kbd", "sans clavier, la scrutation HID n'a rien a lire et "
                        "`hid_poll_gap_max_ms` ne mesure rien"),
            ("usb-tablet", "sans pointeur, la moitie du trafic HID manque"),
        ):
            if peripherique not in banc:
                fautes.append("le banc ne presente plus de `%s` : %s." % (peripherique, pourquoi))

        # 3. Seize coeurs, comme la cible.
        smp = re.search(r"-smp (\d+)", banc)
        if smp is None or int(smp.group(1)) < 16:
            fautes.append(
                "le banc ne tourne plus sur seize coeurs. La machine cible en "
                "a seize, et les sondes multicoeur ne voient rien de ce qui se "
                "passe sur les coeurs qu'on ne simule pas."
            )

        # 4. Les quatre criteres.
        for critere in ("A :", "B :", "C :", "D :"):
            if critere not in banc:
                fautes.append(
                    "le banc n'exige plus le critere %s. Les quatre repondent a "
                    "quatre questions differentes, et trois ne remplacent pas "
                    "le quatrieme." % critere.strip(" :")
                )

        # 5. Le milieu, pas seulement les deux bouts.
        # ANCRAGE SUR LA VERIFICATION, PAS SUR LE MOT.
        #
        # `tiers` apparait aussi dans le calcul qui decoupe la session : s'y
        # fier laissait passer la suppression de la verification elle-meme.
        if "bas <= t <= haut" not in banc:
            fautes.append(
                "le banc n'exige plus un echantillon dans chaque tiers de la "
                "session. Une archive qui garde le debut et la fin sans le "
                "milieu est exactement le symptome de depart."
            )
        if "run_trigkey_ladybird_io_stress" in banc and "BOUCHAUD_TRIGKEY_IO_STRESS_OK" not in banc:
            fautes.append("le banc n'emet plus son verdict final.")

    # 6. Le drapeau reste une option de compilation.
    if cargo is not None and not re.search(r"^banc-io = \[\]", cargo, re.M):
        fautes.append(
            "Cargo.toml : la fonctionnalite `banc-io` a disparu. Le banc lit le "
            "volume en boucle et ETEINT la machine : un reglage a l'execution "
            "laisserait la possibilite de l'armer par accident sur une image "
            "livree."
        )
    if stage2 is not None:
        lancement = re.search(
            r'#\[cfg\(feature = "banc-io"\)\]\s*\n\s*crate::platform::pc::banc_io::demarre\(\);',
            stage2,
        )
        if lancement is None:
            fautes.append(
                "stage2.rs : le banc n'est plus lance sous `#[cfg(feature = "
                '"banc-io")]`. Soit il ne part plus du tout, soit il part sur '
                "une image livree."
            )
    if module is not None:
        if "power::shutdown" not in module:
            fautes.append(
                "banc_io.rs : le banc n'eteint plus proprement. Le vidage de "
                "l'enregistreur n'a lieu qu'a l'extinction volontaire : sans "
                "elle, le tambour reste en RAM et l'archive est vide."
            )
        # L'ARRET DOIT PRECEDER LE VERDICT, ET PAS SEULEMENT EXISTER.
        #
        # `ARRET_DEMANDE` apparait aussi dans sa declaration et dans le fil de
        # charge : chercher le nom laissait passer la suppression de l'ordre
        # d'arret lui-meme.
        demande = module.find("ARRET_DEMANDE.store(true")
        rendu = module.find("verdict(depart)")
        if demande < 0 or rendu < 0 or demande > rendu or "CHARGE_ARRETEE" not in module:
            fautes.append(
                "banc_io.rs : la charge ne s'arrete plus avant le verdict. "
                "« Le verrou revient a aucun » veut dire qu'il n'y a pas de "
                "fuite ; le lire pendant qu'un lecteur legitime le tient "
                "mesure seulement qu'il y avait du trafic."
            )
        for bit in (
            "INJECTE_ECHEANCE_DONNEES",
            "INJECTE_ECHEANCE_STATUT",
            "INJECTE_VERROU_TENU",
        ):
            if bit not in module:
                fautes.append(
                    "banc_io.rs : `%s` n'est plus arme. Cette panne-la ne se "
                    "fabrique pas sur commande avec une vraie cle, et c'est "
                    "pourtant exactement ce qu'il faut prouver." % bit
                )

    if fautes:
        for faute in fautes:
            print("FAUTE: %s" % faute)
        return 1
    print(
        "banc de charge : deux supports, entree reelle, seize coeurs, quatre "
        "criteres, pannes injectees, drapeau de compilation."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
