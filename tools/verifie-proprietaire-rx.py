#!/usr/bin/env python3
"""Garde-fou : UNE seule fonction lit physiquement l'anneau de reception.

# Le defaut, releve le 17 septembre 2026

TRIGKEY, lien a un gigabit, Ladybird vivant. Pendant trente secondes :

    [NET-ROUTAGE] trames=100 arp=13 dhcp=2 arp_resolus=1 arp_echoues=0
    [NET-TCP]     poignees=0 echantillons_rtt=0 syn_retransmis=0

Pas une trame de plus, pas un ARP de plus, pas une poignee TCP.

DEUX consommateurs lisaient l'anneau : le routage maison
(`draine_verrouille`) et le peripherique `smoltcp` (`E1000Device::receive`).
Le commentaire de tete de `smol_device.rs` posait deja la regle -- « les deux
ne doivent JAMAIS tourner en meme temps » -- et rien ne la faisait respecter.

Ce n'est pas seulement une course memoire. C'est la PROPRIETE DES PAQUETS :
une trame retiree de la carte par l'un n'existe plus pour l'autre. Une reponse
ARP ne se retransmet pas ; il suffit que le mauvais consommateur la prenne
pour que la resolution echoue, et qu'elle echoue POUR DE BON puisque l'echec
est mis en cache.

# L'invariant defendu ici

    NIC RX
      |
      v
    ingress unique
      |
      +--> ARP maison
      +--> DHCP
      +--> files IPv4 maison
      +--> file smoltcp

1. `e1000::receive` n'a qu'UN appelant dans tout l'arbre : le drainage
   verrouille.
2. `rtl8168::receive` n'a qu'UN appelant : la facade `e1000`.
3. Le peripherique smoltcp ne touche plus au materiel.
4. La copie vers la file smoltcp se fait sur le flux BRUT, avant tout tri.
5. L'abonnement se referme par un garde `Drop`, pas a la main.
6. La file reste BORNEE et ce qu'elle perd se compte.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
NET = RACINE / "src/net/mod.rs"
DEVICE = RACINE / "src/net/transport/smol_device.rs"
SMOLTCP = RACINE / "src/net/transport/smol_tcp.rs"
FILE = RACINE / "src/net/file_trames.rs"
E1000 = RACINE / "src/drivers/network/e1000.rs"

# Le SEUL appelant autorise du receive materiel, et la fonction qui l'abrite.
LECTEUR_AUTORISE = "fn draine_verrouille() -> usize {"
FACADE_AUTORISEE = "pub fn receive(out: &mut [u8]) -> Option<usize> {"


def code_seul(source):
    """Le code sans les commentaires.

    Une garde qui lit les commentaires se satisfait de sa propre prose : les
    appels qu'elle interdit figurent dans l'explication du defaut qu'elle
    defend, et dans l'en-tete de `smol_device.rs`.
    """
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    lignes = []
    for ligne in sans.splitlines():
        ligne = re.sub(r"//.*$", "", ligne)
        lignes.append(ligne)
    return "\n".join(lignes)


def corps(source, signature):
    """Le corps d'une fonction, jusqu'a la declaration suivante -- INDENTEE
    COMPRISE, sans quoi une regle sur une fonction se satisfait d'un jeton
    trouve trois fonctions plus bas."""
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:const |static |fn |struct |impl |enum |mod )", reste)
    return reste[: fin.start()] if fin else reste


def bloc(source, entete):
    """Le bloc complet d'un `impl`, accolades comptees."""
    d = source.find(entete)
    if d < 0:
        return None
    ouvre = source.find("{", d)
    if ouvre < 0:
        return None
    profondeur = 0
    for i in range(ouvre, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvre : i + 1]
    return source[ouvre:]


def lit(chemin, fautes):
    try:
        return chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        fautes.append("%s est illisible." % chemin.name)
        return None


def main():
    fautes = []

    # ------------------------------------------------------------------ 1
    #
    # Le balayage porte sur TOUT `src/`, et pas sur une liste de fichiers :
    # une regle qui n'examine que les fichiers connus ne voit pas le prochain
    # consommateur, qui est justement celui qu'elle doit arreter.
    appelants = []
    for chemin in sorted(RACINE.joinpath("src").rglob("*.rs")):
        try:
            pur = code_seul(chemin.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
        if chemin == E1000:
            continue
        for ligne_no, ligne in enumerate(pur.splitlines(), 1):
            if "e1000::receive(" in ligne:
                appelants.append((chemin, ligne_no))

    if not appelants:
        fautes.append(
            "personne ne lit plus la carte : le drainage a disparu, et le "
            "reseau ne recevrait plus rien du tout."
        )
    elif len(appelants) > 1:
        ou = ", ".join(
            "%s:%d" % (c.relative_to(RACINE), n) for c, n in appelants
        )
        fautes.append(
            "plusieurs appelants de `e1000::receive` : %s. Une trame retiree "
            "par l'un n'existe plus pour l'autre, et une reponse ARP ne se "
            "retransmet pas. UN seul lecteur physique, puis une repartition."
            % ou
        )
    else:
        chemin, _ = appelants[0]
        if chemin != NET:
            fautes.append(
                "l'unique lecteur de la carte est %s, et non le drainage "
                "verrouille de `net/mod.rs`." % chemin.relative_to(RACINE)
            )
        else:
            pur = code_seul(lit(NET, fautes) or "")
            drainage = corps(pur, LECTEUR_AUTORISE)
            if drainage is None or "e1000::receive(" not in drainage:
                fautes.append(
                    "net/mod.rs : `e1000::receive` n'est plus appele depuis "
                    "`draine_verrouille`, qui est le seul endroit tenant le "
                    "verrou de reception."
                )

    # ------------------------------------------------------------------ 2
    facade = lit(E1000, fautes)
    if facade is not None:
        pur = code_seul(facade)
        if pur.count("rtl8168::receive(") > 1:
            fautes.append(
                "e1000.rs : plusieurs appels a `rtl8168::receive`. La facade "
                "doit en avoir exactement un, celui qu'elle expose."
            )
        receive = corps(pur, FACADE_AUTORISEE)
        if receive is None or "rtl8168::receive(" not in receive:
            fautes.append(
                "e1000.rs : la facade `receive` ne delegue plus au pilote "
                "RTL8168."
            )

    # ------------------------------------------------------------------ 3
    device = lit(DEVICE, fautes)
    if device is not None:
        pur = code_seul(device)
        impl = bloc(pur, "impl Device for E1000Device")
        if impl is None:
            fautes.append("smol_device.rs : l'implementation de `Device` est introuvable.")
        else:
            if "e1000::receive" in impl:
                fautes.append(
                    "smol_device.rs : le peripherique smoltcp lit de nouveau la "
                    "carte. Il doit consommer la file alimentee par l'ingress "
                    "unique, sans quoi il vole les reponses ARP de la pile "
                    "maison -- et reciproquement."
                )
            if "retire_trame_smoltcp" not in impl:
                fautes.append(
                    "smol_device.rs : le peripherique ne consomme plus la file "
                    "logicielle."
                )

    # ------------------------------------------------------------------ 4
    reseau = lit(NET, fautes)
    if reseau is not None:
        pur = code_seul(reseau)
        route = corps(pur, "fn route_trame(trame: &[u8]) {")
        if route is None:
            fautes.append("net/mod.rs : `route_trame` est introuvable.")
        else:
            copie = route.find("file_smoltcp().pose(")
            analyse = route.find("ethernet::parse_header")
            if copie < 0:
                fautes.append(
                    "net/mod.rs : le routage ne recopie plus rien pour smoltcp. "
                    "Le peripherique ne recevrait jamais rien."
                )
            elif analyse >= 0 and copie > analyse:
                fautes.append(
                    "net/mod.rs : la copie smoltcp se fait APRES l'analyse. Le "
                    "routage maison jette ce qu'il ne sait pas traiter -- IPv6, "
                    "VLAN, controle de flux -- et une trame jetee la serait "
                    "perdue pour smoltcp aussi. La copie porte sur le flux BRUT."
                )

    # ------------------------------------------------------------------ 5
    fetch = lit(SMOLTCP, fautes)
    if fetch is not None:
        pur = code_seul(fetch)
        if bloc(pur, "impl Drop for AbonnementSmoltcp") is None:
            fautes.append(
                "smol_tcp.rs : l'abonnement ne se referme plus par un garde. "
                "`fetch` a une dizaine de sorties ; poser la fermeture a la "
                "main la perd a la premiere oubliee, et le routage recopierait "
                "chaque trame pour personne jusqu'au prochain demarrage."
            )
        corps_fetch = corps(pur, "pub fn fetch(dst: Ipv4Addr, port: u16, request: &[u8], out: &mut Vec<u8>) -> bool {")
        if corps_fetch is None:
            fautes.append("smol_tcp.rs : `fetch` est introuvable.")
        else:
            ouverture = corps_fetch.find("AbonnementSmoltcp::ouvre()")
            device_pos = corps_fetch.find("E1000Device")
            if ouverture < 0:
                fautes.append(
                    "smol_tcp.rs : `fetch` n'ouvre plus la file smoltcp."
                )
            elif device_pos >= 0 and ouverture > device_pos:
                fautes.append(
                    "smol_tcp.rs : la file est ouverte APRES le peripherique. "
                    "Une trame recue entre les deux serait routee par la pile "
                    "maison et jamais recopiee."
                )

    # ------------------------------------------------------------------ 6
    file_source = lit(FILE, fautes)
    if file_source is not None:
        pur = code_seul(file_source)
        if "crate::" in pur:
            fautes.append(
                "file_trames.rs n'est plus pur : la discipline de la file "
                "redeviendrait inverifiable autrement qu'en demarrant la "
                "machine."
            )
        if "unsafe" in pur:
            fautes.append("file_trames.rs contient de l'unsafe.")
        pose = corps(pur, "pub fn pose(&mut self, trame: &[u8]) -> bool {")
        if pose is None:
            fautes.append("file_trames.rs : `pose` est introuvable.")
        else:
            if "perdues_pleine" not in pose:
                fautes.append(
                    "file_trames.rs : une trame perdue faute de place ne se "
                    "compte plus. C'est la seule chose qui distingue « smoltcp "
                    "n'a rien recu » de « smoltcp n'a pas lu ce qu'on lui a "
                    "donne »."
                )
            if "TRAMES" not in pose:
                fautes.append(
                    "file_trames.rs : la file n'est plus bornee. Une file sans "
                    "borne dans un noyau est une panne memoire deguisee en "
                    "fonctionnalite."
                )

    if fautes:
        for f in fautes:
            print("FAUTE: %s" % f)
        return 1
    print(
        "proprietaire RX unique : un seul lecteur physique, copie brute avant "
        "tri, abonnement referme par un garde, file bornee et pertes comptees."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
