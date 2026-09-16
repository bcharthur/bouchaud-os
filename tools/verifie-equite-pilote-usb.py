#!/usr/bin/env python3
"""Garde-fou : le clavier et l'enregistreur ne doivent pas etre affames.

# Le defaut, releve le 16 septembre 2026

Trois consommateurs se partagent `RUNTIME_BUSY`, l'unique verrou du pilote
xHCI : la scrutation HID, l'enregistreur de vol, et le systeme de fichiers.
Les deux premiers tentent une prise INSTANTANEE et renoncent si le verrou est
pris ; le troisieme attend, jusqu'a cinquante millions de tours.

Quand le navigateur demarre, le systeme de fichiers lit quatre cents
mebioctets de binaires sur la cle d'amorcage. Il tient donc le verrou presque
en continu, et les deux autres renoncent sans fin. Cela donne exactement les
deux symptomes rapportes le meme jour :

  * « le clavier fonctionne mais quand je commence a taper dans Ladybird, il
    est deconnecte, je peux plus ecrire » ;
  * l'archive blackbox s'arrete a 7,30 s, a l'instant precis ou les services
    Ladybird demarrent.

Ni l'un ni l'autre n'etait en panne. Les deux etaient affames, par la meme
cause, au meme instant -- et c'est pourquoi trois corrections successives de
l'enregistreur seul n'avaient rien change.

# Ce qui est verifie ici

1. Le module de decision reste PUR : la famine ne se reproduit sur la machine
   qu'en lancant un navigateur de quatre cents mebioctets depuis la cle.
2. Les DEUX consommateurs prioritaires signalent leur famine. En brancher un
   seul laisserait l'autre exactement dans l'etat d'avant.
3. Le systeme de fichiers cede AVANT de prendre le verrou.
4. La cession reste BORNEE. Un consommateur qui reclame sans jamais aboutir
   -- pilote en panne, peripherique parti -- ne doit pas bloquer le systeme
   de fichiers : remplacer une famine par un blocage serait un plus mauvais
   marche.
5. Zero n'est pas la sentinelle de « pas encore date ». C'est un horodatage
   legitime, et s'en servir comme marqueur d'absence rendait indatable une
   famine commencee a `monotonic_ns() == 0`.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
EQUITE = RACINE / "src/drivers/usb/equite_pilote.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
STOCKAGE = RACINE / "src/drivers/usb/blackbox_storage.rs"
TEST = RACINE / "tools/platform/test_equite_pilote.rs"


def code_seul(source):
    sans_blocs = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans_blocs.splitlines())


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    equite = lit(EQUITE, fautes)
    xhci = lit(XHCI, fautes)
    stockage = lit(STOCKAGE, fautes)
    test = lit(TEST, fautes)

    if equite is not None:
        code = code_seul(equite)
        if "use crate::" in code:
            fautes.append(
                "equite_pilote.rs depend du reste du noyau : il ne se compile "
                "plus seul, et la famine redevient invrifiable ailleurs que "
                "sur la machine, ou elle demande un navigateur entier."
            )
        if "unsafe" in code:
            fautes.append("equite_pilote.rs contient du `unsafe` ; cette decision est pure.")
        # 5. La sentinelle.
        if not re.search(r"const JAMAIS: u64 = u64::MAX;", code):
            fautes.append(
                "equite_pilote.rs : la sentinelle de « pas encore date » n'est "
                "plus `u64::MAX`. Zero est un horodatage LEGITIME : s'en servir "
                "rend indatable une famine commencee a monotonic_ns() == 0, "
                "car `compare_exchange(0, 0)` reussit sans rien ecrire."
            )
        # 4. La borne.
        if "CESSION_MAXIMALE_NS" not in code or "cession_maximale_ns" not in code:
            fautes.append(
                "equite_pilote.rs : la cession n'est plus bornee. Un "
                "consommateur qui reclame sans aboutir bloquerait le systeme "
                "de fichiers ; ce serait echanger une famine contre un blocage."
            )

    if xhci is not None:
        code = code_seul(xhci)
        # 2. Les deux consommateurs.
        if "EQUITE_HID.saut(" not in code:
            fautes.append(
                "xhci_active.rs : la scrutation HID ne signale plus sa famine. "
                "Le clavier redeviendrait muet des que le navigateur charge, "
                "et paraitrait deconnecte."
            )
        if "EQUITE_HID.succes()" not in code:
            fautes.append(
                "xhci_active.rs : la scrutation HID ne signale plus ses "
                "reussites ; la famine ne retomberait jamais et le systeme de "
                "fichiers cederait en permanence."
            )
        if "pub fn enregistreur_a_saute" not in code or "pub fn enregistreur_a_reussi" not in code:
            fautes.append(
                "xhci_active.rs : l'enregistreur de vol n'a plus de quoi "
                "signaler sa famine. Le brancher d'un seul cote laisserait "
                "l'autre exactement dans l'etat d'avant."
            )
        # 3. La cession precede la prise.
        debut = code.find("fn avec_le_pilote_usb")
        if debut == -1:
            fautes.append("xhci_active.rs : `avec_le_pilote_usb` a disparu.")
        else:
            corps = code[debut:debut + 2500]
            # LA CONDITION, PAS SEULEMENT L'APPEL.
            #
            # Chercher `cede_encore(` laissait passer
            # `while false && eq::cede_encore(...)` : l'appel reste present,
            # a la bonne place, et la cession ne s'execute jamais. C'est la
            # mutation M4, et elle n'etait pas attrapee.
            m_cede = re.search(r"while\s+eq::cede_encore\(", corps)
            i_cede = m_cede.start() if m_cede else -1
            i_prend = corps.find("while RUNTIME_BUSY")
            if i_cede == -1:
                fautes.append(
                    "xhci_active.rs : le systeme de fichiers ne cede plus le "
                    "passage. C'est lui qui tient le verrou en continu pendant "
                    "que le navigateur charge."
                )
            elif i_prend != -1 and i_cede > i_prend:
                fautes.append(
                    "xhci_active.rs : la cession a lieu APRES la prise du "
                    "verrou. Ceder une fois qu'on le tient ne libere personne."
                )

    if stockage is not None and "enregistreur_a_saute(" not in code_seul(stockage):
        fautes.append(
            "blackbox_storage.rs : le saut de l'enregistreur n'est plus "
            "signale. C'est a cet endroit exact que l'archive du 16 septembre "
            "s'est arretee."
        )

    if test is not None and test.count("#[test]") < 10:
        fautes.append(
            "test_equite_pilote.rs : moins de dix cas pour une regle qui ne se "
            "reproduit autrement qu'en chargeant un navigateur entier."
        )

    if fautes:
        print("equite du pilote USB : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "equite du pilote USB : HID et enregistreur signalent leur famine, le "
        "systeme de fichiers cede avant de prendre, et la cession est bornee."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
