#!/usr/bin/env python3
"""Garde-fou : un ecart de scrutation HID doit designer UN responsable.

# Le defaut, mesure le 17 septembre 2026

La session physique montre un pic de 638 ms sur `hid_poll_gap_max_ms`. Trois
causes l'expliquaient aussi bien -- l'ordonnanceur, le verrou xHCI, le corps
de la scrutation -- et le chiffre ne les separait pas. Trois hypotheses pour
un chiffre, c'est zero diagnostic, et c'est ce qui a coute une passe entiere.

`note_poll_servi` n'etait appele qu'APRES l'acquisition du verrou : tout ce
qui precedait etait dans le meme sac.

# Le second defaut, trouve par la mesure elle-meme

Une fois les trois intervalles poses, leur somme faisait 31 ms pour un ecart
mesure de 144 ms. Les 113 manquantes etaient une SERIE de refus : chaque tour
refuse est court, donc `run_to_lock` restait petit pendant que l'attente
reelle explosait. Une mesure qui ne compte que les succes ne voit pas la
famine.

# Le troisieme, trouve en corrigeant le second

La cession existait et etait active. Elle faisait attendre le systeme de
fichiers AVANT de prendre le verrou -- et rien ne l'empechait de le reprendre
juste apres. Trente-six refus consecutifs, cession active. Une politesse
n'est pas une barriere.

# Ce qui est verifie ici

1. `chrono_hid.rs` et `equite_pilote.rs` restent PURS.
2. Les quatre instants sont pris, et T3 par un garde `Drop` -- le corps de la
   scrutation a plusieurs sorties et en gagnera d'autres.
3. La famine est mesuree du PREMIER refus au succes, et elle compte dans la
   designation du responsable. Sans cela une famine de 140 ms se cache
   derriere un `run_to_lock` de 2.
4. Le tourniquet ferme l'ACQUISITION aux autres consommateurs, dans les deux
   chemins de prise -- instantane et patient. Le fermer dans un seul ne
   protege de rien : le systeme de fichiers utilise le chemin patient.
5. La reservation est bornee dans les deux sens : levee des que la scrutation
   passe, et expiree au bout de la borne. Un fil HID mort ne doit pas emporter
   le disque.
6. La scrutation elle-meme n'est JAMAIS bloquee par le tourniquet.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
CHRONO = RACINE / "src/drivers/usb/chrono_hid.rs"
EQUITE = RACINE / "src/drivers/usb/equite_pilote.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"


def code_seul(source):
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def corps(source, signature):
    """Le corps d'une fonction, jusqu'a la suivante -- INDENTEE COMPRISE.

    La premiere version ne coupait que sur une declaration en debut de ligne.
    Toutes les methodes d'un `impl` etant indentees, elle rendait le reste du
    bloc entier : une regle sur `note_verrou_pris` se satisfaisait alors d'un
    jeton trouve dans `note_echec_verrou`, trois methodes plus bas. Une garde
    qui lit trop large ne verifie plus ce qu'elle nomme.
    """
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:const |static |fn |struct |impl |enum )", reste)
    return reste[: fin.start()] if fin else reste


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    chrono = lit(CHRONO, fautes)
    equite = lit(EQUITE, fautes)
    xhci = lit(XHCI, fautes)

    # 1. La purete.
    for nom, source in (("chrono_hid.rs", chrono), ("equite_pilote.rs", equite)):
        if source is None:
            continue
        pur = code_seul(source)
        for jeton, pourquoi in (
            ("use crate::", "il ne se compile plus seul avec `rustc --test`"),
            ("unsafe", "cette comptabilite se raisonne sans pointeur"),
            ("monotonic_ns", "les instants doivent etre des ARGUMENTS, sinon les "
                             "tests hote ne peuvent fabriquer ni famine ni retard"),
        ):
            if jeton in pur:
                fautes.append("%s contient `%s` : %s." % (nom, jeton, pourquoi))

    if chrono is not None:
        pur = code_seul(chrono)
        # 3. La famine se mesure, et elle pese dans le verdict.
        pris = corps(pur, "pub fn note_verrou_pris(")
        if pris is None or "famine_depuis_ns" not in pris:
            fautes.append(
                "chrono_hid.rs : la famine n'est plus fermee a l'acquisition. "
                "C'est le SEUL instant ou sa duree est connue -- et sans elle, "
                "les trois intervalles totalisaient 31 ms pour un ecart mesure "
                "de 144."
            )
        resp = corps(pur, "pub fn responsable(")
        if resp is None or "lock_starve_max_us" not in resp:
            fautes.append(
                "chrono_hid.rs : la famine ne compte plus dans la designation "
                "du responsable. Une famine de 140 ms se cacherait derriere un "
                "`run_to_lock` de 2, et le correctif viserait le mauvais tiers."
            )
        echec = corps(pur, "pub fn note_echec_verrou(")
        if echec is None or "compare_exchange" not in echec:
            fautes.append(
                "chrono_hid.rs : la famine est datee du DERNIER refus et non du "
                "premier. Elle paraitrait perpetuellement naissante."
            )

    if equite is not None:
        pur = code_seul(equite)
        # 5. La reservation est bornee des deux cotes.
        ceder = corps(pur, "pub fn doit_ceder(")
        if ceder is None:
            fautes.append("equite_pilote.rs : le tourniquet a disparu.")
        else:
            # L'USAGE, PAS LA SIGNATURE : `maximale_ns` est un parametre, donc
            # toujours present. Seule la COMPARAISON prouve que la borne sert.
            if ">= maximale_ns" not in ceder:
                fautes.append(
                    "equite_pilote.rs : la reservation n'expire plus. Une famine "
                    "du clavier deviendrait un blocage du stockage, et un fil "
                    "HID mort emporterait le disque avec lui."
                )
        if corps(pur, "pub fn libere(") is None:
            fautes.append(
                "equite_pilote.rs : la reservation ne se leve plus quand la "
                "scrutation a eu son tour."
            )
        # SIGNATURE COMPLETE : `Equite` porte deja un `reclame(&self)`, et
        # s'arreter au nom trouvait le sien -- la garde jugeait alors une
        # fonction qui n'est pas celle qu'elle defend.
        reclame = corps(pur, "pub fn reclame(&self, maintenant_ns: u64)")
        if reclame is not None and "compare_exchange" not in reclame:
            fautes.append(
                "equite_pilote.rs : chaque reclamation rajeunit la reservation. "
                "Une scrutation affamee en continu la garderait ouverte pour "
                "toujours, et la borne ne serait jamais atteinte."
            )

    if xhci is not None:
        pur = code_seul(xhci)
        # 2. Les quatre instants, et T3 par Drop.
        fil = corps(pur, "fn fil_hid() -> ! {")
        if fil is None or "note_reveil" not in fil or "echeance_pour" not in fil:
            fautes.append(
                "xhci_active.rs : le fil HID ne mesure plus son retard de "
                "reveil, ou recopie la formule de l'echeance au lieu de "
                "l'appeler. Une formule recopiee mentirait le jour ou la cadence "
                "du timer change -- et le mensonge accuserait l'ordonnanceur."
            )
        if corps(pur, "impl Drop for CorpsMesure {") is None:
            fautes.append(
                "xhci_active.rs : T3 n'est plus pris par un garde. Le corps de "
                "la scrutation a plusieurs sorties et en gagnera d'autres ; la "
                "mesure se perdrait a la premiere oubliee."
            )

        # 4 et 6. Le tourniquet dans LES DEUX chemins, jamais contre HID.
        for nom, fonction in (
            ("prends_le_pilote", corps(pur, "fn prends_le_pilote(")),
            ("attends_le_pilote", corps(pur, "fn attends_le_pilote(")),
        ):
            if fonction is None:
                fautes.append("xhci_active.rs : `%s` est introuvable." % nom)
                continue
            if "TOURNIQUET.doit_ceder" not in fonction:
                fautes.append(
                    "xhci_active.rs : `%s` ne respecte plus le passage reserve. "
                    "Le systeme de fichiers utilise le chemin PATIENT et la "
                    "scrutation le chemin INSTANTANE : fermer un seul des deux "
                    "ne protege de rien." % nom
                )
            if "qui != Proprietaire::Hid" not in fonction:
                fautes.append(
                    "xhci_active.rs : `%s` peut bloquer la scrutation elle-meme "
                    "sur sa propre reservation." % nom
                )

    if fautes:
        for f in fautes:
            print("FAUTE: %s" % f)
        return 1
    print(
        "chrono HID : quatre instants, famine mesuree et comptee dans le "
        "verdict, tourniquet borne ferme aux deux chemins et jamais a la "
        "scrutation."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
