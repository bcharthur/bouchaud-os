#!/usr/bin/env python3
"""Garde-fou : le reseau doit marcher sur un AUTRE reseau demain.

# Ce que ce garde-fou interdit, et pourquoi

La machine de reference a une passerelle a `192.168.1.254`, dont l'adresse
materielle est `70:fc:8f:31:68:8d`, et un serveur de noms a la meme adresse.
QEMU, lui, a une passerelle a `10.0.2.2` et un serveur de noms a `10.0.2.3`.

Chacune de ces valeurs, ecrite en dur, ferait passer un test physique sans
que rien ne fonctionne :

  * une adresse materielle de passerelle codee en dur supprime ARP du chemin
    -- et c'est precisement ARP qui est casse ;
  * une duree de vie de cache ARP infinie, ou de dix minutes, transforme une
    reception morte en reseau qui marche pendant dix minutes ;
  * une adresse IP de service codee en dur supprime DNS du chemin ;
  * un repli sur `10.0.2.3` fait croire que le serveur de noms repond sur une
    machine ou il n'existe pas.

Aucune de ces choses ne serait vue par un test QEMU, et toutes feraient
echouer la machine chez quelqu'un d'autre.

# Ce qui est verifie

1. Aucune adresse materielle litterale dans la pile reseau.
2. Aucune adresse IP de reseau prive ecrite en dur hors des tables de
   diffusion et de masque.
3. La duree de vie du cache ARP reste BORNEE et modeste.
4. `netdiag` oublie le voisin avant chaque tour -- sans quoi il ne mesure
   qu'un cache.
5. Le pilote acquitte `IntrStatus` au RUNTIME, et pas seulement au demarrage.
6. La reprise n'appelle pas la reinitialisation complete en premier.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
RESEAU = [
    RACINE / "src/net/mod.rs",
    RACINE / "src/net/diagnostic.rs",
    RACINE / "src/net/resolveur.rs",
]
PILOTE = RACINE / "src/drivers/network/rtl8168.rs"
ANNEAU = RACINE / "src/drivers/network/anneau_rx.rs"
DIAG = RACINE / "src/net/diagnostic.rs"

# Une adresse materielle litterale : six octets hexadecimaux separes.
MAC_LITTERALE = re.compile(
    r"0x[0-9a-fA-F]{2}\s*,\s*0x[0-9a-fA-F]{2}\s*,\s*0x[0-9a-fA-F]{2}\s*,"
    r"\s*0x[0-9a-fA-F]{2}\s*,\s*0x[0-9a-fA-F]{2}\s*,\s*0x[0-9a-fA-F]{2}"
)

# Les adresses que le releve physique et QEMU utilisent reellement.
IP_INTERDITES = [
    (r"\b192\s*,\s*168\s*,\s*1\s*,\s*254\b", "la passerelle de la machine de reference"),
    (r"\b192\s*,\s*168\s*,\s*1\s*,\s*97\b", "l'adresse obtenue par DHCP ce jour-la"),
    (r"\b10\s*,\s*0\s*,\s*2\s*,\s*3\b", "le serveur de noms de QEMU"),
    (r"\b10\s*,\s*0\s*,\s*2\s*,\s*2\b", "la passerelle de QEMU"),
    (r"\b8\s*,\s*8\s*,\s*8\s*,\s*8\b", "un serveur de noms public"),
    (r"\b1\s*,\s*1\s*,\s*1\s*,\s*1\b", "un serveur de noms public"),
]

# Ce que le cache ARP positif ne doit jamais depasser, en millisecondes.
ARP_TTL_MAXIMAL_MS = 300_000


def sans_presomption_slirp(source):
    """Le code, MOINS le module qui a le droit de porter ces adresses.

    La presomption SLIRP doit exister quelque part -- sans elle, QEMU n'a plus
    de configuration d'usine et tous les bancs d'emulateur tombent. Elle vit
    donc dans UN module nomme, et ce garde-fou verifie qu'elle n'en sort pas.
    """
    debut = source.find("mod presomption_slirp {")
    if debut < 0:
        return source
    profondeur = 0
    ouvre = source.find("{", debut)
    for i in range(ouvre, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[:debut] + source[i + 1:]
    return source[:debut]


def code_seul(source):
    """Le code sans les commentaires.

    Une garde qui lit les commentaires se satisfait de sa propre prose : les
    adresses qu'elle interdit figurent dans l'explication du defaut qu'elle
    defend, et dans celle-ci meme.
    """
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    lignes = []
    for ligne in sans.splitlines():
        ligne = re.sub(r"//.*$", "", ligne)
        ligne = re.sub(r"^\s*///.*$", "", ligne)
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


def lit(chemin, fautes):
    try:
        return chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        fautes.append("%s est illisible." % chemin.name)
        return None


def main():
    fautes = []

    # ------------------------------------------------------------------ 1, 2
    for chemin in RESEAU + [PILOTE]:
        source = lit(chemin, fautes)
        if source is None:
            continue
        pur = sans_presomption_slirp(code_seul(source))
        if MAC_LITTERALE.search(pur):
            fautes.append(
                "%s porte une adresse materielle litterale. Coder la MAC de la "
                "passerelle supprime ARP du chemin -- et c'est ARP qui est "
                "casse." % chemin.name
            )
        for motif, quoi in IP_INTERDITES:
            if re.search(motif, pur):
                fautes.append(
                    "%s code en dur %s. La machine doit fonctionner sur un "
                    "autre reseau Ethernet demain." % (chemin.name, quoi)
                )

    # --------------------------------------------- le repli compile, sur le vrai fer
    mod = lit(RACINE / "src/net/mod.rs", fautes)
    if mod is not None:
        repli = corps(code_seul(mod), "pub fn resolveur_compile() -> Ipv4Addr {")
        if repli is None:
            fautes.append(
                "net/mod.rs : `resolveur_compile` a disparu. Le resolveur du NAT "
                "de QEMU serait de nouveau remis au navigateur sur la machine "
                "physique, ou il ne mene nulle part."
            )
        elif "using_rtl8168()" not in repli or "[0, 0, 0, 0]" not in repli:
            fautes.append(
                "net/mod.rs : `resolveur_compile` ne rend plus RIEN sur une "
                "carte reelle. Une adresse qui ne mene nulle part vaut moins "
                "que pas d'adresse : sans resolveur le navigateur le dit, avec "
                "un faux il attend un delai d'attente et accuse le reseau."
            )
        oubli = corps(code_seul(mod), "fn oublie_la_presomption_slirp() {")
        if oubli is None:
            fautes.append(
                "net/mod.rs : la presomption SLIRP n'est plus effacee sur une "
                "carte reelle sans bail. `10.0.2.2` serait resolu par ARP a "
                "chaque paquet sortant, sur un reseau ou il n'existe pas."
            )

    # ------------------------------------------------------------------ 3
    if mod is not None:
        pur = code_seul(mod)
        trouve = re.search(r"const ARP_TTL_MS:\s*u64\s*=\s*([0-9_]+)", pur)
        if trouve is None:
            fautes.append(
                "net/mod.rs : la duree de vie du cache ARP positif n'est plus "
                "une constante bornee."
            )
        else:
            valeur = int(trouve.group(1).replace("_", ""))
            if valeur > ARP_TTL_MAXIMAL_MS:
                fautes.append(
                    "net/mod.rs : la duree de vie du cache ARP est passee a "
                    "%d ms. Allonger ce cache ne repare pas une reception "
                    "morte, il la CACHE -- pendant exactement cette duree."
                    % valeur
                )
        # Le voisin doit pouvoir etre oublie UN PAR UN, sinon un banc ne peut
        # pas mesurer une resolution reelle.
        if corps(pur, "pub fn oublie_voisin(") is None:
            fautes.append(
                "net/mod.rs : on ne peut plus oublier un voisin seul. Un banc "
                "ne mesurerait alors qu'une resolution suivie de cent lectures "
                "de cache."
            )

    # ------------------------------------------------------------------ 4
    diag = lit(DIAG, fautes)
    if diag is not None:
        pur = code_seul(diag)
        tour = corps(pur, "fn tour_arp(cible: Ipv4Addr) -> (bool, u64) {")
        if tour is None:
            fautes.append("diagnostic.rs : `tour_arp` est introuvable.")
        else:
            if "oublie_voisin" not in tour:
                fautes.append(
                    "diagnostic.rs : un tour ARP n'oublie plus le voisin. Les "
                    "tours deux a cent ne feraient que relire un cache, et le "
                    "banc passerait sur une carte dont la reception est morte."
                )
            if "voisin_en_cache" not in tour:
                fautes.append(
                    "diagnostic.rs : le banc ne verifie plus que l'oubli a eu "
                    "lieu. Un oubli qui echoue en silence rend la mesure sans "
                    "objet."
                )
        # L'ANCRE EST LA VALEUR, PAS LE NOM DE LA CONSTANTE.
        #
        # Une premiere version cherchait `HOTE_EPREUVE` et se satisfaisait donc
        # d'une constante videe : le nom survit a la mutation, la valeur non.
        hote = re.search(
            r'const HOTE_EPREUVE:\s*&str\s*=\s*"([^"]*)"', pur
        )
        if hote is None:
            fautes.append(
                "diagnostic.rs : l'hote de l'epreuve n'est plus une constante "
                "lisible."
            )
        else:
            valeur = hote.group(1)
            octets = valeur.split(".")
            if "." not in valeur or len(valeur) < 4:
                fautes.append(
                    "diagnostic.rs : « %s » n'est pas un nom d'hote. L'epreuve "
                    "de couche 3 doit passer par DNS, sinon elle ne le teste "
                    "pas." % valeur
                )
            elif len(octets) == 4 and all(o.isdigit() for o in octets):
                fautes.append(
                    "diagnostic.rs : « %s » est une adresse, pas un nom. Une "
                    "adresse en dur supprime DNS du chemin." % valeur
                )

    # ------------------------------------------------------------------ 5
    pilote = lit(PILOTE, fautes)
    if pilote is not None:
        pur = code_seul(pilote)
        maintenance = corps(pur, "unsafe fn maintenance_isr() -> u16 {")
        if maintenance is None:
            fautes.append("rtl8168.rs : `maintenance_isr` est introuvable.")
        elif "write16(REG_INTR_STATUS" not in maintenance:
            fautes.append(
                "rtl8168.rs : la maintenance ne reecrit plus `IntrStatus`. Un "
                "bit verrouille reste verrouille, et le pilote scrute : "
                "personne d'autre ne le lira jamais."
            )
        recevoir = corps(pur, "pub fn receive(out: &mut [u8]) -> Option<usize> {")
        if recevoir is None:
            fautes.append("rtl8168.rs : `receive` est introuvable.")
        elif "maintenance_anneau_vide()" not in recevoir:
            fautes.append(
                "rtl8168.rs : le passage a vide ne fait plus de maintenance. "
                "C'est EXACTEMENT la ou U-Boot la fait, et le seul instant ou "
                "elle ne coute rien : le descripteur courant porte encore OWN."
            )

        # ------------------------------------------------------------- 6
        repare = corps(pur, "unsafe fn repare_reception() -> Option<bool> {")
        if repare is None:
            fautes.append("rtl8168.rs : `repare_reception` est introuvable.")
        else:
            ordre = [
                ("maintenance_isr()", "acquitter les statuts"),
                ("Degre::Rearme", "rearmer les descripteurs rendus"),
                ("Degre::RelanceRx", "relancer le moteur de reception"),
                ("Degre::ReconstruitAnneau", "reconstruire l'anneau"),
                ("Degre::ReinitialiseCarte", "reinitialiser la carte"),
            ]
            positions = []
            for jeton, quoi in ordre:
                place = repare.find(jeton)
                if place < 0:
                    fautes.append(
                        "rtl8168.rs : la reprise ne sait plus %s." % quoi
                    )
                positions.append(place)
            if all(p >= 0 for p in positions) and positions != sorted(positions):
                fautes.append(
                    "rtl8168.rs : l'echelle de reprise n'est plus ordonnee du "
                    "moins au plus invasif. Reinitialiser la carte coupe le "
                    "lien et oblige a refaire DHCP : ce prix est le DERNIER a "
                    "payer, pas le premier."
                )

        # ------------------------------ la course que le verrou existe pour fermer
        #
        # Reconstruire l'anneau, c'est le reecrire en entier et remettre
        # `RX_CUR` a zero. Le faire hors du chemin de drainage -- qui tient
        # `VERROU_RECEPTION` -- pendant que `receive` lit un descripteur serait
        # exactement ce que ce verrou interdit : « un seul point sort les
        # trames de la carte ».
        appels = pur.count("repare_reception()")
        # Une declaration, un appel : celui de `repare_si_demande`.
        if appels > 2:
            fautes.append(
                "rtl8168.rs : `repare_reception` est appelee depuis plus d'un "
                "endroit. Elle ne doit l'etre que par `repare_si_demande`, que "
                "seul le drainage verrouille appelle."
            )
        vide = corps(pur, "unsafe fn maintenance_anneau_vide() {")
        if vide is None:
            fautes.append("rtl8168.rs : `maintenance_anneau_vide` est introuvable.")
        elif "repare_reception" in vide:
            fautes.append(
                "rtl8168.rs : le passage a vide repare lui-meme. `receive` est "
                "aussi appelee par le peripherique smoltcp, qui ne tient pas le "
                "verrou de reception : y reconstruire l'anneau rouvrirait la "
                "course que ce verrou ferme."
            )
        demande = corps(pur, "pub fn demande_reparation_si_arretee() -> bool {")
        if demande is None:
            fautes.append(
                "rtl8168.rs : `demande_reparation_si_arretee` est introuvable."
            )
        else:
            for ecriture in ("write8(", "write16(", "write32(", "desc_write"):
                if ecriture in demande:
                    fautes.append(
                        "rtl8168.rs : `demande_reparation_si_arretee` ecrit dans "
                        "le materiel (`%s`). Elle est appelee par le veilleur de "
                        "lien, qui ne tient pas le verrou de reception : elle "
                        "doit ARMER, pas agir." % ecriture
                    )
            if "repare_reception" in demande:
                fautes.append(
                    "rtl8168.rs : le veilleur de lien repare lui-meme. La "
                    "reparation doit avoir lieu sous le verrou du drainage."
                )

    # ------------------------- la reparation doit avoir UN executant, et locke
    if mod is not None:
        drainage = corps(code_seul(mod), "fn draine_verrouille() -> usize {")
        if drainage is None:
            fautes.append("net/mod.rs : `draine_verrouille` est introuvable.")
        elif "repare_si_demande()" not in drainage:
            fautes.append(
                "net/mod.rs : le drainage verrouille n'execute plus les "
                "reparations armees. Personne d'autre n'a le droit de le faire "
                "-- le veilleur de lien et le peripherique smoltcp ne tiennent "
                "pas le verrou --, et une reception morte le resterait."
            )

    anneau = lit(ANNEAU, fautes)
    if anneau is not None:
        pur = code_seul(anneau)
        if "crate::" in pur:
            fautes.append(
                "anneau_rx.rs n'est plus pur : la discipline de l'anneau "
                "redeviendrait inverifiable autrement qu'en flashant la "
                "machine."
            )
        if "unsafe" in pur:
            fautes.append("anneau_rx.rs contient de l'unsafe : de l'arithmetique n'en a pas besoin.")

    if fautes:
        for f in fautes:
            print("FAUTE: %s" % f)
        return 1
    print(
        "reseau sans triche : aucune adresse en dur, cache ARP borne, banc qui "
        "oublie avant de demander, statuts acquittes au runtime, reprise "
        "ordonnee du moins au plus invasif."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
