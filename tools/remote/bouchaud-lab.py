#!/usr/bin/env python3
"""Bouchaud Lab -- le client PC du canal d'enquete.

# A quoi sert cet outil

La machine de reference perd sa reception apres soixante-quatre trames. Quand
cela arrive, il ne reste rien pour l'interroger : la trace serie n'existe pas
(`com1=bus-flottant`, zero octet), le disque a cesse d'ecrire, et DHCP attend
une trame ENTRANTE, c'est-a-dire precisement la chose en panne.

Deux canaux survivent, et cet outil parle aux deux :

    BRDP      TCP 2222   bidirectionnel, authentifie par HMAC-SHA256
    telemetry UDP 2223   emission seule, en diffusion, NON authentifiee

Le premier meurt avec la reception -- un TCP sans RX ne s'etablit pas. Le
second n'a besoin que du sens qui reste vivant. C'est pour cela qu'ils sont
deux, et c'est pour cela que `telemetry --watch` est la commande a lancer EN
PREMIER lors d'une campagne physique : elle est la seule qui continuera de
dire quelque chose apres la panne.

# Bibliotheque standard uniquement

Aucun `pip install`. Cet outil doit fonctionner sur le PC qu'on a sous la
main le jour ou la machine tombe, y compris un PowerShell sans rien
d'installe.

    python .\\tools\\remote\\bouchaud-lab.py telemetry --watch
    python3 tools/remote/bouchaud-lab.py status --host 169.254.12.34

# Le jeton ne s'affiche jamais

Ni en clair, ni tronque, ni dans un message d'erreur, ni dans un dump. Il ne
traverse pas non plus le reseau : le protocole prouve qu'on le connait par un
HMAC du nonce. Voir `src/net/diag_distant/brdp.rs`.
"""

from __future__ import annotations

import argparse
import binascii
import datetime
import hashlib
import hmac
import json
import os
import socket
import sys
import time
from pathlib import Path

# ---------------------------------------------------------------------------
# LE PROTOCOLE, TEL QU'IL EST -- PAS TEL QU'ON L'IMAGINE
# ---------------------------------------------------------------------------
#
# Chaque constante ci-dessous a une contrepartie dans le noyau. Les faire
# diverger, c'est fabriquer un client qui marche au banc et se tait sur le
# terrain.

#: `diag_distant::PORT_BRDP`
PORT_BRDP = 2222
#: `diag_distant::PORT_TELEMETRIE`
PORT_TELEMETRIE = 2223
#: `brdp::VERSION`
VERSION_BRDP = 1
#: L'algorithme annonce par `serveur::annonce`.
AUTH_ATTENDUE = "hmac-sha256"
#: `brdp::NONCE_LEN`
NONCE_LEN = 32
#: `brdp::DESCRIPTEURS_MAX`
DESCRIPTEURS_MAX = 64
#: `brdp::EVENTS_TAIL_MAX`
EVENTS_TAIL_MAX = 1024
# HOTFIX11_SERIAL_CAPTURE
SERIAL_READ_MAX = 1536
SERIAL_STATUS_CODE = 90
SERIAL_READ_CODE = 91

# BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
SERVICES_PAGE_CODE = 92


#: `reponses::REPONSE_MAX` vaut 4096 ; le terminateur et un peu de marge
#: au-dessus evitent de refuser une ligne que le serveur a le droit d'emettre.
#: La borne EXISTE parce qu'un pair qui n'envoie jamais de `\n` ferait grandir
#: le tampon du client sans fin -- la meme panne memoire que le serveur refuse
#: de son cote, dans l'autre sens.
LIGNE_MAX = 8192

#: `telemetrie::CHARGE_MAX`, plus l'en-tete UDP. Un datagramme plus gros a ete
#: fragmente ou n'est pas de nous.
DATAGRAMME_MAX = 2048

# Le serveur BRDP V1 utilise une seule socket smoltcp. Apres un `quit`, cette
# socket peut rester tres brievement indisponible pendant son recyclage avant
# de repasser en LISTEN. Une seconde invocation CLI peut donc recevoir un
# ECONNREFUSED transitoire alors que le service est sain.
#
# On ne masque PAS une panne : seul ConnectionRefusedError est rejoue, pendant
# au plus 1,5 s (et jamais au-dela du --timeout demande). Un timeout, une route
# absente, une erreur d'authentification ou un protocole invalide remontent
# immediatement.
RECYCLAGE_REFUS_MAX = 1.5
RECYCLAGE_REFUS_PAUSE = 0.01


class ErreurLab(Exception):
    """Tout ce qui empeche l'enquete d'aboutir."""


class ErreurProtocole(ErreurLab):
    """Le pair ne parle pas BRDP/1, ou pas comme il le devrait."""


class ErreurAuth(ErreurLab):
    """L'authentification a ete refusee, ou n'a pas pu etre tentee."""


class FinDeFlux(ErreurLab):
    """Le pair a ferme. Attendu apres `quit`, brutal partout ailleurs."""


class ErreurCommande(ErreurLab):
    """Le serveur a repondu `ok:false`. Il dit pourquoi, on le repete."""

    def __init__(self, nom: str, code: int):
        super().__init__(f"{nom} (code {code})")
        self.nom = nom
        self.code = code


# ---------------------------------------------------------------------------
# LA TABLE DES COMMANDES : UNE SEULE SOURCE DE VERITE
# ---------------------------------------------------------------------------
#
# Le nom CLI, la ligne exacte du protocole et le code attendu vivent ENSEMBLE.
# Une table separee de la documentation diverge le jour ou l'on ajoute une
# commande ; ici, il n'y a rien a synchroniser.


class Commande:
    __slots__ = ("cli", "fil", "code", "argument", "flux")

    def __init__(self, cli, fil, code, argument=None, flux=False):
        #: le nom tape par l'operateur
        self.cli = cli
        #: la valeur EXACTE du champ `cmd`, telle que `brdp::analyse` l'attend
        self.fil = fil
        #: `Commande::code()` cote noyau, repete dans la reponse
        self.code = code
        #: le nom du champ entier, s'il en faut un
        self.argument = argument
        #: cette commande est-elle suivie de lignes hors enveloppe ?
        self.flux = flux


COMMANDES = [
    Commande("status", "status", 2),
    Commande("audit", "audit status", 3),
    Commande("audit-run", "audit run", 4),
    Commande("audit-last", "audit last", 5),
    Commande("net", "net status", 6),
    Commande("rtl8168", "rtl8168 status", 7),
    Commande("rtl8168-ring", "rtl8168 ring", 8),
    Commande("rtl8168-desc", "rtl8168 desc", 9, argument="n"),
    Commande("dhcp", "dhcp status", 10),
    Commande("blackbox", "blackbox status", 11),
    Commande("checkpoint", "blackbox checkpoint", 12),
    Commande("events-tail", "events tail", 13, argument="n", flux=True),
    Commande("events-watch", "events watch", 14, flux=True),
    Commande("services", "services snapshot", 15),
    # BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
    Commande("services-page", "services page", SERVICES_PAGE_CODE, argument="start"),
    Commande("processes", "processes snapshot", 16),
    Commande("memory", "memory snapshot", 17),
    # HOTFIX11_SERIAL_CAPTURE
    Commande("serial-status", "serial status", 90),
    Commande("quit", "quit", 18),
    Commande("internet-start", "internet proof start", 19),
    Commande("internet", "internet proof status", 20),

    # BOUCHAUD_P0_REMOTE_CONTROL_V1
    Commande("system-reboot", "system reboot", 100),
    Commande("system-shutdown", "system shutdown", 101),
    Commande("browser-start", "browser start", 110),
    Commande("browser-stop", "browser stop", 111),
    Commande("browser-restart", "browser restart", 112),
    Commande("process-kill", "process kill", 120, argument="pid"),
    Commande("process-kill-tree", "process kill-tree", 121, argument="pid"),
]

PAR_NOM = {c.cli: c for c in COMMANDES}


# ---------------------------------------------------------------------------
# LIRE UN FLUX, PAS DES MESSAGES
# ---------------------------------------------------------------------------


class LecteurLignes:
    """Recompose des lignes JSON a partir d'un flux TCP quelconque.

    # Ce que TCP ne promet pas

    Ni qu'une reponse arrive en un seul `recv`, ni qu'un `recv` n'en contienne
    qu'une. Les deux se produisent : la premiere sur un reseau charge, la
    seconde des que le serveur enchaine une reponse et des evenements. Un
    client qui suppose « un recv, une ligne » marche au banc et perd des
    reponses sur le terrain -- c'est exactement le defaut que le serveur a
    corrige de son cote.
    """

    def __init__(self, sock: socket.socket, ligne_max: int = LIGNE_MAX):
        self._sock = sock
        self._tampon = bytearray()
        self._ligne_max = ligne_max
        self._fini = False

    def ligne(self) -> bytes:
        """Rend la prochaine ligne complete, sans son terminateur."""
        while True:
            coupe = self._tampon.find(b"\n")
            if coupe >= 0:
                ligne = bytes(self._tampon[:coupe])
                del self._tampon[: coupe + 1]
                return ligne.rstrip(b"\r")
            if len(self._tampon) > self._ligne_max:
                # LA BORNE SE DIT, ELLE NE SE SUBIT PAS. Un pair qui n'envoie
                # jamais de terminateur ferait grandir ce tampon sans fin.
                raise ErreurProtocole(
                    f"ligne de plus de {self._ligne_max} octets sans terminateur"
                )
            if self._fini:
                raise FinDeFlux("le pair a ferme au milieu d'une ligne")
            morceau = self._sock.recv(4096)
            if not morceau:
                self._fini = True
                if not self._tampon:
                    raise FinDeFlux("le pair a ferme")
            self._tampon.extend(morceau)

    def objet(self) -> dict:
        """La prochaine ligne, analysee. Une ligne illisible est une faute."""
        brut = self.ligne()
        try:
            objet = json.loads(brut.decode("utf-8", errors="replace"))
        except json.JSONDecodeError as exc:
            raise ErreurProtocole(f"ligne JSON invalide : {exc}") from exc
        if not isinstance(objet, dict):
            raise ErreurProtocole("une ligne du protocole est toujours un objet")
        return objet

    # BOUCHAUD_HOTFIX12_1_SERVICES_JSON_COMPAT
    def objet_services_v1_compat(self) -> dict:
        # Compatibilite ciblee avec le JSON coupe de Hotfix12 V1.
        morceaux = []
        continuations = 0
        while True:
            brut = self.ligne()
            if brut.endswith(b"\\"):
                morceaux.append(brut[:-1])
                continuations += 1
                if continuations > 4:
                    raise ErreurProtocole("services page V1: trop de continuations JSON")
                continue
            morceaux.append(brut)
            break
        brut = b"".join(morceaux)
        try:
            objet = json.loads(brut.decode("utf-8", errors="replace"))
        except json.JSONDecodeError as exc:
            raise ErreurProtocole(f"services page: JSON invalide apres compat V1 : {exc}") from exc
        if not isinstance(objet, dict):
            raise ErreurProtocole("services page: reponse non objet")
        return objet


# ---------------------------------------------------------------------------
# LE CLIENT BRDP
# ---------------------------------------------------------------------------


class ClientBrdp:
    """Une connexion authentifiee au debugger distant.

    S'utilise comme gestionnaire de contexte :

        with ClientBrdp("169.254.12.34", jeton=jeton) as c:
            print(c.commande("status"))
    """

    def __init__(self, hote: str, jeton: str, port: int = PORT_BRDP,
                 delai: float = 5.0):
        if not jeton:
            # Le serveur refuse un jeton vide ; le dire ICI evite une
            # connexion, un nonce brule et un refus qu'on prendrait pour un
            # probleme reseau.
            raise ErreurAuth("aucun jeton : rien a prouver au serveur")
        self.hote = hote
        self.port = port
        self._jeton = jeton.encode("utf-8")
        self.delai = delai
        self._sock = None
        self._lecteur = None
        #: la version annoncee par le serveur, une fois la main serree
        self.version = None
        self.authentifiee = False

    # -- cycle de vie ------------------------------------------------------

    def __enter__(self):
        self.ouvre()
        return self

    def __exit__(self, *_):
        self.ferme()
        return False

    def ouvre(self):
        # Une socket unique cote serveur implique une courte fenetre de
        # recyclage apres `quit`. C'est visible surtout quand plusieurs
        # commandes CLI sont lancees a la suite par un banc : le processus
        # suivant peut arriver avant que smoltcp ait remis la socket en LISTEN.
        #
        # Retenter uniquement ECONNREFUSED rend le vrai client robuste a ce
        # comportement documente sans transformer `--timeout` en attente
        # silencieuse. Le temps total de recyclage est borne a 1,5 s.
        debut = time.monotonic()
        fin_recyclage = debut + min(self.delai, RECYCLAGE_REFUS_MAX)
        while True:
            try:
                self._sock = socket.create_connection(
                    (self.hote, self.port), self.delai
                )
                break
            except ConnectionRefusedError:
                if time.monotonic() >= fin_recyclage:
                    raise
                time.sleep(RECYCLAGE_REFUS_PAUSE)

        self._sock.settimeout(self.delai)
        self._lecteur = LecteurLignes(self._sock)
        try:
            self._poignee_de_main()
        except BaseException:
            # UNE POIGNEE DE MAIN RATEE NE LAISSE PAS UN SOCKET OUVERT. `dump`
            # enchaine les connexions et `discover` peut en tenter plusieurs :
            # un descripteur perdu a chaque echec finit par epuiser le
            # processus, et cela n'arriverait qu'en campagne, quand la machine
            # refuse deja les connexions.
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None
            self._lecteur = None
            raise

    def ferme(self):
        if self._sock is None:
            return
        try:
            if self.authentifiee:
                # On part proprement : le serveur date la deconnexion dans son
                # anneau, et la prochaine connexion ne trouve pas un socket a
                # moitie ferme.
                self._envoie({"cmd": "quit"})
                try:
                    self._lecteur.objet()
                except ErreurLab:
                    pass
        except OSError:
            pass
        finally:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None
            self._lecteur = None
            self.authentifiee = False

    def abandonne(self):
        # Fermeture locale immediate d'un transport deja douteux.
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
        self._sock = None
        self._lecteur = None
        self.authentifiee = False

    # -- la poignee de main ------------------------------------------------

    def _poignee_de_main(self):
        annonce = self._lecteur.objet()

        version = annonce.get("brdp")
        if version is None:
            raise ErreurProtocole("l'annonce ne porte pas de version BRDP")
        if version != VERSION_BRDP:
            # ON NE TENTE PAS DE S'ADAPTER. Un client qui devine une version
            # qu'il ne connait pas envoie des commandes dont il ignore l'effet,
            # sur une machine qu'on essaie justement de ne pas perturber.
            raise ErreurProtocole(
                f"version BRDP {version} ; ce client parle {VERSION_BRDP}"
            )
        self.version = version

        auth = annonce.get("auth")
        if auth != AUTH_ATTENDUE:
            raise ErreurProtocole(
                f"authentification annoncee « {auth} » ; attendu « {AUTH_ATTENDUE} »"
            )

        nonce_hex = annonce.get("nonce")
        if not isinstance(nonce_hex, str):
            raise ErreurProtocole("l'annonce ne porte pas de nonce")
        try:
            nonce = binascii.unhexlify(nonce_hex)
        except (binascii.Error, ValueError) as exc:
            raise ErreurProtocole(f"nonce illisible : {exc}") from exc
        if len(nonce) != NONCE_LEN:
            raise ErreurProtocole(
                f"nonce de {len(nonce)} octets ; le protocole en demande {NONCE_LEN}"
            )

        # LE JETON NE PART PAS. On prouve qu'on le connait, et c'est tout ce
        # que le serveur a besoin de savoir.
        preuve = hmac.new(self._jeton, nonce, hashlib.sha256).hexdigest()
        self._envoie({"cmd": "hello", "mac": preuve})

        reponse = self._lecteur.objet()
        if not reponse.get("ok"):
            nom = reponse.get("error", "inconnue")
            # UN HMAC FAUX FERME LA CONNEXION cote serveur : inutile de
            # reessayer sur ce nonce, il ne resservira jamais.
            raise ErreurAuth(f"authentification refusee par la machine : {nom}")
        self.authentifiee = True

    # -- l'envoi -----------------------------------------------------------

    def _envoie(self, objet: dict):
        # `separators` sans espace : la ligne reste courte, et `brdp::analyse`
        # tolere les espaces mais n'en a pas besoin.
        ligne = json.dumps(objet, separators=(",", ":")).encode("utf-8") + b"\n"
        self._sock.sendall(ligne)

    # -- les commandes -----------------------------------------------------

    def commande(self, nom: str, valeur: int | None = None) -> dict:
        """Envoie une commande de la table et rend sa reponse."""
        commande = PAR_NOM.get(nom)
        if commande is None:
            raise ErreurLab(f"commande inconnue du client : {nom}")
        if not self.authentifiee:
            raise ErreurAuth("commande envoyee avant authentification")

        objet = {"cmd": commande.fil}
        if commande.argument is not None:
            if valeur is None:
                raise ErreurLab(f"« {nom} » demande un argument")
            objet[commande.argument] = int(valeur)
        self._envoie(objet)

        if nom == "services-page":
            reponse = self._lecteur.objet_services_v1_compat()
        else:
            reponse = self._lecteur.objet()
        if not reponse.get("ok"):
            raise ErreurCommande(
                reponse.get("error", "inconnue"), reponse.get("code", 0)
            )
        rendu = reponse.get("cmd")
        if rendu != commande.code:
            # LES REPONSES NE SONT PAS NUMEROTEES : leur seul rattachement est
            # l'ordre. Un code qui ne correspond pas veut dire qu'on lit la
            # reponse d'une AUTRE commande, et tout ce qui suivra sera decale.
            raise ErreurProtocole(
                f"reponse pour cmd={rendu}, attendu cmd={commande.code}"
            )
        return reponse


    # HOTFIX11_SERIAL_CAPTURE
    def serial_read(self, start: int, n: int) -> tuple[dict, bytes]:
        "Lit une tranche bornee du journal serie RAM."
        if not self.authentifiee:
            raise ErreurAuth("commande envoyee avant authentification")
        if start < 0:
            raise ErreurLab("serial read: start negatif")
        if not 1 <= n <= SERIAL_READ_MAX:
            raise ErreurLab(
                f"serial read demande entre 1 et {SERIAL_READ_MAX} octets, recu {n}"
            )
        self._envoie({"cmd": "serial read", "start": int(start), "n": int(n)})
        reponse = self._lecteur.objet()
        if not reponse.get("ok"):
            raise ErreurCommande(
                reponse.get("error", "inconnue"), reponse.get("code", 0)
            )
        if reponse.get("cmd") != SERIAL_READ_CODE:
            raise ErreurProtocole(
                f"reponse pour cmd={reponse.get('cmd')}, attendu cmd={SERIAL_READ_CODE}"
            )
        hexa = reponse.get("data_hex")
        if not isinstance(hexa, str):
            raise ErreurProtocole("serial read: data_hex absent")
        try:
            donnees = binascii.unhexlify(hexa)
        except (binascii.Error, ValueError) as exc:
            raise ErreurProtocole(f"serial read: hexadecimal invalide: {exc}") from exc
        debut = reponse.get("start")
        suivant = reponse.get("next")
        if not isinstance(debut, int) or not isinstance(suivant, int):
            raise ErreurProtocole("serial read: bornes absentes")
        if suivant < debut or len(donnees) != suivant - debut:
            raise ErreurProtocole(
                "serial read: longueur incoherente avec start/next"
            )
        return reponse, donnees

    def evenements(self, tail: int | None = None, limite: int | None = None):
        """Demande le flux et rend les lignes au fur et a mesure.

        Rend des couples `(genre, objet)` ou `genre` vaut `evenement` ou
        `perte`. Un `tail` s'arrete de lui-meme ; un `watch` tourne jusqu'a
        l'interruption ou jusqu'a la fermeture du pair.
        """
        if tail is not None:
            if not 1 <= tail <= EVENTS_TAIL_MAX:
                raise ErreurLab(
                    f"events tail demande entre 1 et {EVENTS_TAIL_MAX}, recu {tail}"
                )
            self.commande("events-tail", tail)
            plafond = tail if limite is None else min(tail, limite)
        else:
            self.commande("events-watch")
            plafond = limite

        rendus = 0
        while plafond is None or rendus < plafond:
            try:
                objet = self._lecteur.objet()
            except socket.timeout:
                # UN SILENCE N'EST PAS UNE PANNE. Sur un `tail`, l'anneau peut
                # contenir moins d'evenements qu'on n'en a demande : le serveur
                # en rend ce qu'il a puis se tait, et c'est la bonne reponse.
                if tail is not None:
                    return
                continue
            except FinDeFlux:
                return
            if "lost" in objet:
                # LE TROU EST UNE INFORMATION, pas un incident a masquer.
                yield "perte", objet
                continue
            if "seq" not in objet:
                raise ErreurProtocole(
                    "ligne inattendue dans le flux d'evenements : "
                    + json.dumps(objet)[:120]
                )
            rendus += 1
            yield "evenement", objet


# ---------------------------------------------------------------------------
# LA TELEMETRIE : ECOUTER, SANS RIEN DEMANDER
# ---------------------------------------------------------------------------


def ecoute_telemetrie(port: int, duree: float | None, rappel,
                      taille_max: int = DATAGRAMME_MAX):
    """Ecoute les datagrammes de diffusion et rend chaque ligne au rappel.

    `rappel(source_ip, objet, brut)` est appele une fois par ligne valide.
    `duree` en secondes, ou `None` pour ne jamais s'arreter.

    # RIEN N'EST ENVOYE

    Pas de sonde, pas de requete, pas d'ARP. C'est le point : ce canal existe
    pour les moments ou la machine ne peut PLUS recevoir. Lui parler pour la
    trouver reviendrait a dependre de ce qui est en panne.

    # UN DATAGRAMME MALFORME NE TUE PAS L'ECOUTE

    On note, on jette, on continue. Un listener qui meurt sur une ligne
    tronquee s'arrete exactement quand la machine commence a mal aller.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        sock.bind(("", port))
    except OSError as exc:
        sock.close()
        raise ErreurLab(f"impossible d'ecouter sur UDP {port} : {exc}") from exc

    import time as _time

    fin = None if duree is None else _time.monotonic() + duree
    rejetes = 0
    try:
        while True:
            if fin is not None:
                reste = fin - _time.monotonic()
                if reste <= 0:
                    break
                sock.settimeout(min(reste, 1.0))
            else:
                sock.settimeout(1.0)
            try:
                charge, origine = sock.recvfrom(taille_max)
            except socket.timeout:
                continue
            except OSError:
                break
            source = origine[0]
            for brut in charge.split(b"\n"):
                brut = brut.strip()
                if not brut:
                    continue
                try:
                    objet = json.loads(brut.decode("utf-8", errors="replace"))
                except (json.JSONDecodeError, UnicodeDecodeError):
                    rejetes += 1
                    continue
                if not isinstance(objet, dict):
                    rejetes += 1
                    continue
                rappel(source, objet, brut)
    finally:
        sock.close()
    return rejetes


# ---------------------------------------------------------------------------
# LA MISE EN FORME
# ---------------------------------------------------------------------------

# BOUCHAUD_HOTFIX9_RX_PROOF_GATE_V1
#: Les compteurs que la campagne RTL8168 regarde, dans l'ordre ou on les lit.
#: Aucun verdict n'est calcule ici : les deux criteres viennent du noyau.
CHAMPS_RTL8168 = [
    "rx_packets", "rx_octets", "rx_cur", "desc_nic", "desc_cpu",
    "rx_tours_cpu", "rx_rendus_tour1", "rx_rendus_tour2",
    "rx_reutilises_tour2", "isr_rx_ok", "isr_rx_err",
    "rx_ok_sans_progres", "repair_degre", "repair_raison",
    "repair_sans_preuve", "repair_preuves_consommees",
    "repair_differee_pending", "repair_verdict_pending",
    "reprises_differees", "reprises_sans_effet",
    "reinitialisations", "reinitialisations_ok",
    "critere_rx_au_dela_de_64", "critere_tour2",
]


def _valeur(v):
    if isinstance(v, bool):
        return "oui" if v else "non"
    if isinstance(v, list):
        return f"[{len(v)} valeurs]"
    return v


def imprime_objet(objet: dict, titre: str = "", ordre=None):
    """Rend un objet lisible, sans rien inventer."""
    if titre:
        print(f"=== {titre} ===")
    cles = list(ordre) if ordre else [k for k in objet if k not in ("ok", "cmd")]
    largeur = max((len(k) for k in cles), default=0)
    for cle in cles:
        if cle not in objet:
            # Le noyau ne l'a pas rendu. On le DIT plutot que de l'omettre :
            # un champ absent d'un releve se remarque, un champ escamote non.
            print(f"  {cle:<{largeur}} : (absent de la reponse)")
            continue
        valeur = objet[cle]
        if isinstance(valeur, dict):
            print(f"  {cle}:")
            for sous, sv in valeur.items():
                print(f"      {sous:<16} : {_valeur(sv)}")
        else:
            print(f"  {cle:<{largeur}} : {_valeur(valeur)}")


def texte_evenement(objet: dict) -> str:
    """Une ligne d'evenement, dans la ponctuation du catalogue."""
    t_ns = objet.get("t_ns", 0)
    secondes = t_ns // 1_000_000_000
    micros = (t_ns % 1_000_000_000) // 1_000
    tete = f"[{secondes:5d}.{micros:06d}] "
    tete += f"{objet.get('cat', '?')} {objet.get('event', '?')}"
    ignore = {"seq", "t_ns", "cpu", "cat", "event"}
    reste = " ".join(f"{k}={_valeur(v)}" for k, v in objet.items() if k not in ignore)
    return f"{tete} {reste}".rstrip()


def horodatage_local() -> str:
    return datetime.datetime.now().strftime("%H:%M:%S.%f")[:-3]


# ---------------------------------------------------------------------------
# LE JETON
# ---------------------------------------------------------------------------


def lis_le_jeton(args) -> str:
    """`--token`, puis `BOUCHAUD_DEBUG_TOKEN`. Jamais affiche.

    Le message d'echec nomme les deux sources SANS jamais montrer la moindre
    portion d'une valeur -- pas meme tronquee : un prefixe divise l'espace de
    recherche, ce qui est exactement ce qu'un secret ne doit pas faire.
    """
    jeton = getattr(args, "token", None) or os.environ.get("BOUCHAUD_DEBUG_TOKEN")
    if not jeton:
        raise ErreurAuth(
            "aucun jeton BRDP.\n"
            "  Passez --token, ou posez BOUCHAUD_DEBUG_TOKEN dans "
            "l'environnement.\n"
            "  C'est le meme secret que celui injecte a la construction de "
            "l'image LAB."
        )
    return jeton


# ---------------------------------------------------------------------------
# LES SOUS-COMMANDES
# ---------------------------------------------------------------------------

#: `dump` interroge tout ce qui se lit sans effet de bord. `checkpoint` n'y
#: est PAS : c'est la seule commande du protocole qui ecrit, et un releve ne
#: doit pas modifier ce qu'il releve.
PLAN_DUMP = [
    ("summary.json", "status", None),
    ("audit.json", "audit", None),
    ("audit-last.json", "audit-last", None),
    ("net.json", "net", None),
    ("rtl8168.json", "rtl8168", None),
    ("rtl8168-ring.json", "rtl8168-ring", None),
    ("rtl8168-desc-0.json", "rtl8168-desc", 0),
    ("rtl8168-desc-63.json", "rtl8168-desc", DESCRIPTEURS_MAX - 1),
    ("dhcp.json", "dhcp", None),
    ("blackbox.json", "blackbox", None),
    ("services.json", "services", None),
    ("processes.json", "processes", None),
    ("memory.json", "memory", None),
    ("internet-proof.json", "internet", None),
]


def racine_depot() -> Path:
    """La racine du depot, deduite de l'emplacement du script."""
    return Path(__file__).resolve().parents[2]


def fait_dump(args, jeton) -> int:
    quand = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = Path(args.out) if args.out else racine_depot() / "target"
    dossier = base / f"remote-dump-{quand}"
    dossier.mkdir(parents=True, exist_ok=True)

    reussies, echouees = [], []
    version = None
    criteres = {}

    def ecris(nom, contenu):
        (dossier / nom).write_text(
            json.dumps(contenu, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    try:
        client = ClientBrdp(args.host, jeton, args.port, args.timeout)
        client.ouvre()
    except (ErreurLab, OSError) as exc:
        # MEME UN DUMP QUI NE SE CONNECTE PAS LAISSE UNE TRACE. Le jour de la
        # panne, « le client n'a pas pu se connecter a telle heure » est deja
        # un fait, et c'est souvent LE fait.
        ecris("metadata.json", {
            "host": args.host,
            "date": datetime.datetime.now().isoformat(timespec="seconds"),
            "brdp": None,
            "erreur": str(exc),
            "commandes_reussies": 0,
            "commandes_echouees": len(PLAN_DUMP),
        })
        print(f"dump partiel dans {dossier}")
        print(f"  connexion impossible : {exc}", file=sys.stderr)
        return 1

    try:
        version = client.version
        for nom_fichier, nom_cmd, valeur in PLAN_DUMP:
            try:
                reponse = client.commande(nom_cmd, valeur)
                ecris(nom_fichier, reponse)
                reussies.append(nom_cmd)
                if nom_cmd == "rtl8168":
                    for cle in ("critere_rx_au_dela_de_64", "critere_tour2"):
                        if cle in reponse:
                            criteres[cle] = reponse[cle]
            except (ErreurLab, OSError) as exc:
                # UNE COMMANDE PERDUE N'EMPORTE PAS LES AUTRES. Apres une panne
                # RX, certaines reponses peuvent manquer ; celles qui passent
                # restent exploitables, et c'est tout l'interet du dump.
                ecris(nom_fichier, {"ok": False, "erreur_client": str(exc),
                                    "commande": nom_cmd})
                echouees.append(nom_cmd)
                if isinstance(exc, (FinDeFlux, ErreurProtocole, OSError)):
                    # Le flux n'est plus fiable : inutile d'insister, mais tout
                    # ce qui precede est deja sur le disque.
                    break

        # HOTFIX11_SERIAL_CAPTURE
        try:
            serial_meta = capture_serial_snapshot(
                client, dossier / "serial-live.log", 512 * 1024
            )
            ecris("serial-live.json", serial_meta)
            reussies.append("serial-capture")
        except (ErreurLab, OSError) as exc:
            ecris("serial-live.json", {
                "ok": False,
                "erreur_client": str(exc),
            })
            echouees.append("serial-capture")

        lignes = []
        try:
            for genre, objet in client.evenements(tail=args.events):
                lignes.append(json.dumps({"genre": genre, **objet}))
            reussies.append("events-tail")
        except (ErreurLab, OSError) as exc:
            lignes.append(json.dumps({"ok": False, "erreur_client": str(exc)}))
            echouees.append("events-tail")
        (dossier / "events.jsonl").write_text(
            "\n".join(lignes) + ("\n" if lignes else ""), encoding="utf-8"
        )
    finally:
        client.ferme()

    metadonnees = {
        "host": args.host,
        "date": datetime.datetime.now().isoformat(timespec="seconds"),
        "brdp": version,
        "commandes_reussies": len(reussies),
        "commandes_echouees": len(echouees),
        "reussies": reussies,
        "echouees": echouees,
    }
    # LES DEUX CRITERES DE LA CAMPAGNE, remontes au premier niveau. Ils
    # viennent du noyau : ce fichier les recopie, il ne les calcule pas.
    metadonnees["critere_rx_au_dela_de_64"] = criteres.get("critere_rx_au_dela_de_64")
    metadonnees["critere_tour2"] = criteres.get("critere_tour2")
    ecris("metadata.json", metadonnees)

    print(f"dump dans {dossier}")
    print(f"  commandes reussies : {len(reussies)}")
    print(f"  commandes echouees : {len(echouees)}")
    for cle in ("critere_rx_au_dela_de_64", "critere_tour2"):
        print(f"  {cle} : {_valeur(metadonnees[cle])}")
    return 0 if not echouees else 2



# HOTFIX11_SERIAL_CAPTURE
def capture_serial_snapshot(client: ClientBrdp, fichier: Path,
                            max_octets: int = 512 * 1024) -> dict:
    "Capture une fenetre FIGEE du journal serie."
    statut = client.commande("serial-status")
    oldest = int(statut.get("oldest", 0))
    end = int(statut.get("end", 0))
    capacity = int(statut.get("capacity", 0))
    if oldest < 0 or end < oldest:
        raise ErreurProtocole("serial status: bornes invalides")
    if max_octets < 0:
        raise ErreurLab("--bytes doit etre positif ou nul")
    start = oldest if max_octets == 0 else max(oldest, end - max_octets)
    cursor = start
    recus = 0
    perdus = 0
    morceaux = 0

    fichier.parent.mkdir(parents=True, exist_ok=True)
    with fichier.open("wb") as sortie:
        while cursor < end:
            n = min(SERIAL_READ_MAX, end - cursor)
            reponse, donnees = client.serial_read(cursor, n)
            reel = int(reponse["start"])
            suivant = int(reponse["next"])
            if reel < cursor:
                raise ErreurProtocole("serial read: le serveur a recule le curseur")
            if reel > cursor:
                perdus += reel - cursor
            if suivant <= reel and reel < end:
                raise ErreurProtocole("serial read: aucun progres")
            if reel >= end:
                break
            utile = min(len(donnees), end - reel)
            sortie.write(donnees[:utile])
            recus += utile
            morceaux += 1
            cursor = min(suivant, end)

    return {
        "oldest_at_snapshot": oldest,
        "end_at_snapshot": end,
        "capacity": capacity,
        "requested_start": start,
        "bytes_written": recus,
        "bytes_lost_during_capture": perdus,
        "chunks": morceaux,
        "complete_window": perdus == 0 and start + recus == end,
    }


def fait_serial_capture(args, jeton) -> int:
    quand = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    sortie = Path(args.out) if args.out else (
        racine_depot() / "target" / f"serial-live-{quand}.log"
    )
    with ClientBrdp(args.host, jeton, args.port, args.timeout) as client:
        meta = capture_serial_snapshot(client, sortie, args.bytes)
    meta_path = sortie.with_suffix(sortie.suffix + ".json")
    meta_path.write_text(json.dumps(meta, indent=2, sort_keys=True) + "\n",
                         encoding="utf-8")
    if args.json:
        print(json.dumps({"file": str(sortie), "meta": meta}, indent=2,
                         sort_keys=True))
    else:
        print(f"serial capture : {sortie}")
        print(f"  bytes        : {meta['bytes_written']}")
        print(f"  chunks       : {meta['chunks']}")
        print(f"  lost         : {meta['bytes_lost_during_capture']}")
        print(f"  complete     : {meta['complete_window']}")
        print(f"  metadata     : {meta_path}")
    return 0 if meta["complete_window"] else 2


def fait_telemetrie(args) -> int:
    vus = {}

    def rappel(source, objet, brut):
        vus[source] = vus.get(source, 0) + 1
        if args.json:
            print(json.dumps(objet, sort_keys=True), flush=True)
        else:
            print(f"[{horodatage_local()}] {source} {texte_evenement(objet)}",
                  flush=True)

    duree = None if args.watch else args.timeout
    if not args.json:
        cible = "sans limite" if duree is None else f"{duree:g} s"
        print(f"ecoute UDP {args.port} ({cible}) -- aucun paquet n'est emis")
    try:
        rejetes = ecoute_telemetrie(args.port, duree, rappel)
    except KeyboardInterrupt:
        rejetes = 0
    if not args.json:
        print(f"\n{sum(vus.values())} evenement(s) de {len(vus)} machine(s)"
              f", {rejetes} ligne(s) rejetee(s)")
    return 0


def fait_discover(args) -> int:
    vus = {}

    def rappel(source, objet, brut):
        entree = vus.setdefault(source, {"evenements": 0, "premier": None})
        entree["evenements"] += 1
        if entree["premier"] is None:
            entree["premier"] = horodatage_local()

    if not args.json:
        print(f"ecoute passive UDP {args.port} pendant {args.timeout:g} s"
              " -- aucun paquet n'est emis")
    try:
        ecoute_telemetrie(args.port, args.timeout, rappel)
    except KeyboardInterrupt:
        pass

    if args.json:
        print(json.dumps(
            {"hotes": [{"host": ip, **d} for ip, d in sorted(vus.items())]},
            indent=2, sort_keys=True))
        return 0 if vus else 1

    if not vus:
        print("\naucune machine Bouchaud OS detectee.")
        print("  La telemetrie part en DIFFUSION sur le segment local :")
        print("  verifiez d'etre sur le meme segment, sans routeur entre les deux.")
        return 1
    print()
    for ip, detail in sorted(vus.items()):
        print("Bouchaud OS detected:")
        print(f"  host: {ip}")
        print("  telemetry: yes")
        print(f"  evenements recus: {detail['evenements']}")
    return 0


def fait_events(args, jeton) -> int:
    with ClientBrdp(args.host, jeton, args.port, args.timeout) as client:
        tail = None if args.watch else args.tail
        try:
            for genre, objet in client.evenements(tail=tail):
                if args.json:
                    print(json.dumps(objet, sort_keys=True), flush=True)
                elif genre == "perte":
                    print(f"  *** {objet.get('lost')} evenement(s) perdu(s), "
                          f"de {objet.get('from')} a {objet.get('to')} ***",
                          flush=True)
                else:
                    print(texte_evenement(objet), flush=True)
        except KeyboardInterrupt:
            pass
    return 0


def fait_commande_simple(args, jeton, nom: str, valeur=None,
                         ordre=None, titre=None) -> int:
    with ClientBrdp(args.host, jeton, args.port, args.timeout) as client:
        reponse = client.commande(nom, valeur)
    if args.json:
        print(json.dumps(reponse, indent=2, sort_keys=True))
    else:
        imprime_objet(reponse, titre or nom, ordre)
    return 0


# BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
def collecte_services_detail(args, jeton) -> dict:
    start = 0
    total = None
    entrees = []
    t_ns = 0
    client = None
    echecs_consecutifs = 0

    try:
        while total is None or start < total:
            try:
                if client is None:
                    client = ClientBrdp(args.host, jeton, args.port, args.timeout)
                    client.ouvre()
                page = client.commande("services-page", start)
                echecs_consecutifs = 0
            except (OSError, socket.timeout, FinDeFlux) as exc:
                if client is not None:
                    client.abandonne()
                client = None
                echecs_consecutifs += 1
                print(
                    f"services-all: transport retry {echecs_consecutifs}/5 au curseur {start}: {exc}",
                    file=sys.stderr,
                )
                if echecs_consecutifs >= 5:
                    raise
                time.sleep(min(0.25 * echecs_consecutifs, 1.0))
                continue

            page_total = page.get("total")
            prochain = page.get("next")
            lignes = page.get("entries")
            if not isinstance(page_total, int) or page_total < 0:
                raise ErreurProtocole("services page: total absent/invalide")
            if not isinstance(prochain, int) or prochain < start:
                raise ErreurProtocole("services page: curseur next invalide")
            if not isinstance(lignes, list):
                raise ErreurProtocole("services page: entries absent")
            if prochain == start and start < page_total:
                raise ErreurProtocole("services page: aucun progres")

            if total is None:
                total = page_total
            elif page_total != total:
                entrees.clear()
                start = 0
                total = page_total
                continue

            for e in lignes:
                if not isinstance(e, dict) or not isinstance(e.get("id"), str):
                    raise ErreurProtocole("services page: entree invalide")
                entrees.append(e)

            t_ns = max(t_ns, int(page.get("t_ns", 0)))
            start = prochain
            if page.get("done") is True:
                break
    finally:
        if client is not None:
            client.ferme()

    return {
        "host": args.host,
        "date": datetime.datetime.now().isoformat(timespec="seconds"),
        "t_ns": t_ns,
        "total": total if total is not None else 0,
        "services": entrees,
    }


def _profondeur_service(entree: dict, par_id: dict) -> int:
    profondeur = 0
    parent = entree.get("parent") or ""
    vus = set()
    while parent and parent in par_id and parent not in vus and profondeur < 8:
        vus.add(parent)
        profondeur += 1
        parent = par_id[parent].get("parent") or ""
    return profondeur


def fait_services_all(args, jeton) -> int:
    capture = collecte_services_detail(args, jeton)
    services = capture["services"]
    prefixe = (args.prefix or "").strip()
    if prefixe:
        services_affiches = [
            e for e in services
            if e.get("id") == prefixe or str(e.get("id", "")).startswith(prefixe + ".")
        ]
    else:
        services_affiches = services

    sortie = Path(args.out) if args.out else (
        racine_depot() / "target" /
        f"services-detail-{datetime.datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
    )
    sortie.parent.mkdir(parents=True, exist_ok=True)
    sortie.write_text(
        json.dumps(capture, indent=2, ensure_ascii=False, sort_keys=False) + "\n",
        encoding="utf-8",
    )

    if args.json:
        print(json.dumps(
            {**capture, "services": services_affiches},
            indent=2, ensure_ascii=False, sort_keys=False,
        ))
        return 0

    par_id = {e.get("id"): e for e in services if isinstance(e.get("id"), str)}
    print("=== registre des services Bouchaud OS ===")
    print(f"  total capture : {capture['total']}")
    if prefixe:
        print(f"  filtre        : {prefixe}")
    print(f"  fichier       : {sortie}")
    print()

    racine_precedente = None
    for e in services_affiches:
        ident = e.get("id", "?")
        racine = ident.split(".", 1)[0]
        if racine != racine_precedente:
            if racine_precedente is not None:
                print()
            print(f"[{racine}]")
            racine_precedente = racine

        profondeur = _profondeur_service(e, par_id)
        etat = e.get("etat") or "?"
        raison = e.get("raison") or "-"
        prerequis = e.get("prerequis") or "-"
        kpi = e.get("kpi") if isinstance(e.get("kpi"), dict) else {}
        pid = kpi.get("pid")
        cpu = kpi.get("cpu_pour_mille")
        erreurs = e.get("erreurs", 0)

        extras = []
        if pid is not None:
            extras.append(f"pid={pid}")
        if cpu is not None:
            extras.append(f"cpu={cpu/10:.1f}%")
        if erreurs:
            extras.append(f"err={erreurs}")
        suffixe = ("  " + " ".join(extras)) if extras else ""

        print(
            f"{'  ' * profondeur}{ident:<36} "
            f"{etat:<13} raison={raison} prerequis={prerequis}{suffixe}"
        )

    print()
    problematiques = [
        e for e in services
        if e.get("etat") in ("demarrage", "degrade", "reprise", "panne")
    ]
    print(f"services a examiner : {len(problematiques)}")
    for e in problematiques:
        print(f"  {e.get('id')}: {e.get('etat')} ({e.get('raison') or '-'})")
    return 0


# ---------------------------------------------------------------------------
# LA LIGNE DE COMMANDE
# ---------------------------------------------------------------------------


def construis_parseur() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="bouchaud-lab",
        description="Client du canal d'enquete Bouchaud OS "
                    "(BRDP TCP 2222, telemetrie UDP 2223).",
        epilog="Le jeton se passe par --token ou BOUCHAUD_DEBUG_TOKEN. "
               "Il n'est jamais affiche ni ecrit dans un dump.",
    )
    p.add_argument("--json", action="store_true",
                   help="rendre l'objet JSON brut au lieu du texte lisible")
    sous = p.add_subparsers(dest="commande", required=True)

    def avec_hote(sp, defaut_timeout=5.0):
        sp.add_argument("--host", required=True, help="adresse LAB de la machine")
        sp.add_argument("--port", type=int, default=PORT_BRDP)
        sp.add_argument("--token", default=None,
                        help="jeton BRDP (defaut : BOUCHAUD_DEBUG_TOKEN)")
        sp.add_argument("--timeout", type=float, default=defaut_timeout)
        return sp

    # Les commandes de releve, toutes batties sur la meme table.
    SIMPLES = [
        ("status", "status", None, "etat general"),
        ("audit", "audit", None, "auditeur LAB"),
        ("audit-run", "audit-run", None, "tour d'auditeur"),
        ("audit-last", "audit-last", None, "dernier verdict"),
        ("net", "net", None, "pile reseau et canal LAB"),
        ("rtl8168", "rtl8168", CHAMPS_RTL8168, "RTL8168"),
        ("rtl8168-ring", "rtl8168-ring", None, "anneau RTL8168"),
        ("dhcp", "dhcp", None, "DHCP"),
        ("serial-status", "serial-status", None, "journal serie RAM"),
        ("blackbox", "blackbox", None, "boite noire"),
        ("services", "services", None, "services"),
        ("processes", "processes", None, "processus"),
        ("memory", "memory", None, "memoire"),
        # BOUCHAUD_HOTFIX10_INTERNET_PROOF_CHAIN_V1
        ("internet", "internet", None, "preuve Internet (etat)"),
        ("internet-start", "internet-start", None, "preuve Internet (lancer)"),
    ]
    for nom, cible, ordre, titre in SIMPLES:
        sp = avec_hote(sous.add_parser(nom, help=titre))
        sp.set_defaults(_faire=lambda a, j, c=cible, o=ordre, t=titre:
                        fait_commande_simple(a, j, c, None, o, t))

    # BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
    sp = avec_hote(sous.add_parser(
        "services-all",
        help="capturer le registre detaille sys/net/browser sans interaction"),
        defaut_timeout=15.0)
    sp.add_argument(
        "--prefix", default=None,
        help="filtre d'affichage local: sys, net, browser ou un sous-arbre")
    sp.add_argument(
        "--out", default=None,
        help="JSON complet (defaut: target/services-detail-*.json)")
    sp.set_defaults(_faire=fait_services_all)

    sp = avec_hote(sous.add_parser(
        "rtl8168-desc", help=f"un descripteur (0..{DESCRIPTEURS_MAX - 1})"))
    sp.add_argument("index", type=int)
    sp.set_defaults(_faire=lambda a, j: fait_commande_simple(
        a, j, "rtl8168-desc", a.index, None, f"descripteur {a.index}"))

    # HOTFIX11_SERIAL_CAPTURE
    sp = avec_hote(sous.add_parser(
        "serial-capture", help="capturer le journal serie RAM via BRDP"),
        defaut_timeout=10.0)
    sp.add_argument("--bytes", type=int, default=512 * 1024,
                    help="octets les plus recents a capturer; 0 = tout l'anneau")
    sp.add_argument("--out", default=None,
                    help="fichier de sortie (defaut: target/serial-live-*.log)")
    sp.set_defaults(_faire=fait_serial_capture)

    sp = avec_hote(sous.add_parser(
        "checkpoint", help="forcer un checkpoint de boite noire (ECRIT)"))
    sp.set_defaults(_faire=lambda a, j: fait_commande_simple(
        a, j, "checkpoint", None, None, "checkpoint"))

    sp = avec_hote(sous.add_parser("events", help="les evenements de l'anneau"),
                   defaut_timeout=2.0)
    groupe = sp.add_mutually_exclusive_group()
    groupe.add_argument("--tail", type=int, default=100,
                        help=f"les N derniers (1..{EVENTS_TAIL_MAX})")
    groupe.add_argument("--watch", action="store_true",
                        help="suivre le flux jusqu'a l'interruption")
    sp.set_defaults(_faire=fait_events)

    sp = avec_hote(sous.add_parser("dump", help="tout relever dans un dossier"))
    sp.add_argument("--events", type=int, default=500,
                    help=f"evenements a capturer (1..{EVENTS_TAIL_MAX})")
    sp.add_argument("--out", default=None,
                    help="dossier parent (defaut : target/ du depot)")
    sp.set_defaults(_faire=fait_dump)

    sp = sous.add_parser("telemetry", help="ecouter la telemetrie UDP")
    sp.add_argument("--port", type=int, default=PORT_TELEMETRIE)
    sp.add_argument("--watch", action="store_true",
                    help="ecouter sans limite de duree")
    sp.add_argument("--timeout", type=float, default=30.0)
    sp.set_defaults(_faire=lambda a, j: fait_telemetrie(a), _sans_jeton=True)

    sp = sous.add_parser("discover",
                         help="trouver les machines par ecoute PASSIVE")
    sp.add_argument("--port", type=int, default=PORT_TELEMETRIE)
    sp.add_argument("--timeout", type=float, default=10.0)
    sp.set_defaults(_faire=lambda a, j: fait_discover(a), _sans_jeton=True)

    return p


def main(argv=None) -> int:
    args = construis_parseur().parse_args(argv)
    try:
        # `telemetry` et `discover` n'ouvrent aucune connexion et ne prouvent
        # rien a personne : leur demander un jeton serait une ceremonie vide.
        jeton = None if getattr(args, "_sans_jeton", False) else lis_le_jeton(args)
        return args._faire(args, jeton)
    except ErreurAuth as exc:
        print(f"authentification : {exc}", file=sys.stderr)
        return 2
    except ErreurCommande as exc:
        print(f"la machine a refuse la commande : {exc}", file=sys.stderr)
        return 3
    except ErreurLab as exc:
        print(f"erreur : {exc}", file=sys.stderr)
        return 4
    except (OSError, socket.timeout) as exc:
        print(f"reseau : {exc}", file=sys.stderr)
        return 5
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
