"""Un client WebSocket : la premiere connexion que la page garde ouverte.

## Pourquoi c'est different de tout le reste du reseau

Tout ce que le navigateur faisait jusqu'ici suivait le meme rythme : la page
demande, le serveur repond, la connexion se ferme ou retourne a la reserve.
`fetch` et `XMLHttpRequest` sont des allers-retours.

WebSocket renverse cela. La connexion reste ouverte, et **le serveur parle
quand il veut**. Il ne suffit donc pas de savoir emettre : il faut ecouter en
permanence sans occuper le fil de l'interface, et livrer ce qui arrive au
rythme de la boucle d'evenements, pas a celui du reseau.

L'architecture reprend exactement celle du reseau asynchrone deja en place :
un fil par connexion qui lit, et une file de complements que le battement du
document vide. C'est le meme contrat — rien ne remonte au JavaScript en dehors
d'un `tic()` — et donc la meme garantie : un serveur bavard ne peut pas
interrompre un script au milieu de son execution.

## Ce qui est fait, et ce qui ne l'est pas

Fait : la poignee de main RFC 6455 avec sa cle et sa verification, les trames
texte et binaires, le masquage cote client (obligatoire, et un serveur correct
refuse une trame non masquee), la fragmentation en reception, `ping`/`pong`,
la fermeture des deux cotes avec son code, et `ws://` comme `wss://`.

Pas fait : les extensions negociees (`permessage-deflate`), les
sous-protocoles multiples avec selection, et le decoupage a l'emission — un
message part en une trame, ce qui reste conforme mais moins souple sur les
tres gros envois.
"""

import base64
import hashlib
import os
import socket
import ssl
import struct
import threading
import urllib.parse

from . import reseau, securite, telemetrie

# La constante de la RFC 6455. Le serveur concatene la cle du client avec elle,
# hache le tout en SHA-1 et rend le resultat en base64 : c'est ce qui prouve
# qu'on parle bien a un serveur WebSocket et non a un cache HTTP qui aurait
# repondu 101 par accident.
MAGIE = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

# Codes d'operation.
CONTINUATION, TEXTE, BINAIRE = 0x0, 0x1, 0x2
FERMETURE, PING, PONG = 0x8, 0x9, 0xA

# Etats, tels que la page les voit.
CONNEXION, OUVERT, FERMETURE_EN_COURS, FERME = 0, 1, 2, 3

# Bornes. Un serveur hostile ne doit pas pouvoir epuiser la memoire du
# navigateur avec une seule trame annoncee enorme.
MESSAGE_MAX = 8 * 1024 * 1024
DELAI_POIGNEE = 15.0


class Erreur(Exception):
    """Une connexion qui n'a pas pu s'etablir, ou qui s'est rompue."""


class Connexion:
    """Une connexion WebSocket vivante.

    Elle possede son fil de lecture. Tout ce qu'elle recoit va dans
    `evenements`, que l'appelant vide au rythme qui l'arrange — jamais
    l'inverse. `verrou` protege la file, pas la prise : l'emission se fait
    depuis le fil appelant, la reception depuis le fil de lecture, et les deux
    ecrivent sur des moities differentes de la socket.
    """

    def __init__(self, url, protocoles=(), document=None):
        self.url = url
        self.protocoles = list(protocoles or ())
        self.protocole = ""
        self.etat = CONNEXION
        self.code_fermeture = 0
        self.raison_fermeture = ""
        self.propre = False

        self._prise = None
        self._fil = None
        self._verrou = threading.Lock()
        self._evenements = []
        self._emission = threading.Lock()
        # Les octets deja lus mais pas encore consommes.
        #
        # La poignee de main se lit par blocs, et un serveur qui parle tout de
        # suite — ou qui ferme aussitot — envoie ses premieres trames dans le
        # meme paquet TCP que ses en-tetes. Jeter ce qui suit `\r\n\r\n`
        # perdait donc le premier message : le defaut ne se voyait que sur un
        # serveur pressé, et c'est exactement ce que fait `/ws/ferme`.
        self._reste = b""
        # Fragments d'un message en cours de reception.
        self._morceaux = []
        self._code_morceaux = TEXTE
        self._document = document

    # -- Ouverture -------------------------------------------------------------

    def ouvre(self):
        """Etablit la connexion et lance le fil de lecture. Peut lever `Erreur`."""
        morceaux = urllib.parse.urlsplit(self.url)
        schema = morceaux.scheme.lower()
        if schema not in ("ws", "wss"):
            raise Erreur("schema inattendu pour un WebSocket : %s" % schema)
        hote = morceaux.hostname
        if not hote:
            raise Erreur("adresse WebSocket sans hote")
        securise = schema == "wss"
        port = morceaux.port or (443 if securise else 80)
        chemin = morceaux.path or "/"
        if morceaux.query:
            chemin += "?" + morceaux.query

        cle = base64.b64encode(os.urandom(16)).decode("ascii")
        try:
            prise = socket.create_connection((reseau.resout(hote), port),
                                             timeout=DELAI_POIGNEE)
            if securise:
                # Le meme contexte que le reste du navigateur : fail-closed. Un
                # `wss://` qui n'est pas verifiable ne doit pas se degrader en
                # connexion en clair, pas plus qu'un `https://`.
                prise = reseau.contexte_tls().wrap_socket(prise,
                                                          server_hostname=hote)
        except Exception as e:  # noqa: BLE001
            raise Erreur("connexion impossible : %s" % e)

        entetes = [
            "GET %s HTTP/1.1" % chemin,
            "Host: %s" % morceaux.netloc,
            "Upgrade: websocket",
            "Connection: Upgrade",
            "Sec-WebSocket-Key: %s" % cle,
            "Sec-WebSocket-Version: 13",
            "User-Agent: %s" % reseau.AGENT,
        ]
        # L'origine du document qui ouvre la connexion. Un serveur s'en sert
        # pour refuser les pages tierces ; l'omettre reviendrait a lui retirer
        # ce moyen de defense.
        if self._document:
            origine = securite.origine(self._document)
            if origine is not None:
                entetes.append("Origin: %s://%s%s" % (
                    origine[0], origine[1],
                    "" if origine[2] in (80, 443) else ":%d" % origine[2]))
        if self.protocoles:
            entetes.append("Sec-WebSocket-Protocol: %s" % ", ".join(self.protocoles))

        try:
            prise.sendall(("\r\n".join(entetes) + "\r\n\r\n").encode("ascii"))
            reponse = self._lis_entetes(prise)
        except Exception as e:  # noqa: BLE001
            prise.close()
            raise Erreur("poignee de main interrompue : %s" % e)

        statut, champs, self._reste = reponse
        if statut != 101:
            prise.close()
            raise Erreur("le serveur a repondu %d au lieu de 101" % statut)
        attendu = base64.b64encode(
            hashlib.sha1((cle + MAGIE).encode("ascii")).digest()).decode("ascii")
        if champs.get("sec-websocket-accept", "").strip() != attendu:
            # Sans ce controle, n'importe quel intermediaire repondant 101
            # passerait pour un serveur WebSocket.
            prise.close()
            raise Erreur("la reponse du serveur ne prouve pas la poignee de main")

        self.protocole = champs.get("sec-websocket-protocol", "").strip()
        prise.settimeout(None)
        self._prise = prise
        self.etat = OUVERT
        self._pose("open", None)
        telemetrie.compte("websocket.ouvertures")

        self._fil = threading.Thread(target=self._ecoute, daemon=True,
                                     name="bo-websocket")
        self._fil.start()
        return True

    def _lis_entetes(self, prise):
        """Lit la reponse de la poignee de main, sans toucher au corps."""
        tampon = b""
        while b"\r\n\r\n" not in tampon:
            bloc = prise.recv(1024)
            if not bloc:
                raise Erreur("connexion fermee pendant la poignee de main")
            tampon += bloc
            if len(tampon) > 65536:
                raise Erreur("en-tetes de poignee de main demesures")
        tete_octets, _, suite = tampon.partition(b"\r\n\r\n")
        tete = tete_octets.decode("latin-1")
        lignes = tete.split("\r\n")
        try:
            statut = int(lignes[0].split()[1])
        except (IndexError, ValueError):
            raise Erreur("ligne de statut illisible")
        champs = {}
        for ligne in lignes[1:]:
            if ":" in ligne:
                nom, _, valeur = ligne.partition(":")
                champs[nom.strip().lower()] = valeur.strip()
        # `suite` porte ce que le serveur a envoye juste apres ses en-tetes.
        return statut, champs, suite

    # -- Reception -------------------------------------------------------------

    def _ecoute(self):
        """Le fil de lecture. Ne touche jamais au JavaScript directement."""
        try:
            while self.etat in (OUVERT, FERMETURE_EN_COURS):
                trame = self._lis_trame()
                if trame is None:
                    break
                self._traite(*trame)
        except Exception as e:  # noqa: BLE001
            self._pose("error", str(e))
        finally:
            self._termine()

    def _traite(self, fin, code, charge):
        if code == FERMETURE:
            self.code_fermeture = struct.unpack(">H", charge[:2])[0] if len(charge) >= 2 else 1005
            self.raison_fermeture = charge[2:].decode("utf-8", "replace")
            self.propre = True
            if self.etat == OUVERT:
                # Le serveur ferme : on lui renvoie sa fermeture, comme le veut
                # la norme, avant de couper.
                self.etat = FERMETURE_EN_COURS
                self._envoie(FERMETURE, charge[:2])
            self.etat = FERME
            return
        if code == PING:
            self._envoie(PONG, charge)
            return
        if code == PONG:
            return

        if code in (TEXTE, BINAIRE):
            self._morceaux = [charge]
            self._code_morceaux = code
        elif code == CONTINUATION:
            self._morceaux.append(charge)
        else:
            return
        if not fin:
            return

        complet = b"".join(self._morceaux)
        self._morceaux = []
        if self._code_morceaux == TEXTE:
            self._pose("message", complet.decode("utf-8", "replace"))
        else:
            self._pose("message", list(complet))
        telemetrie.compte("websocket.messages_recus")

    def _lis_trame(self):
        entete = self._lis(2)
        if entete is None:
            return None
        fin = bool(entete[0] & 0x80)
        code = entete[0] & 0x0F
        masque = bool(entete[1] & 0x80)
        longueur = entete[1] & 0x7F
        if longueur == 126:
            suite = self._lis(2)
            if suite is None:
                return None
            longueur = struct.unpack(">H", suite)[0]
        elif longueur == 127:
            suite = self._lis(8)
            if suite is None:
                return None
            longueur = struct.unpack(">Q", suite)[0]
        if longueur > MESSAGE_MAX:
            raise Erreur("trame de %d octets : au-dela de la limite" % longueur)
        cles = self._lis(4) if masque else None
        if masque and cles is None:
            return None
        charge = self._lis(longueur) if longueur else b""
        if charge is None:
            return None
        if masque:
            charge = bytes(o ^ cles[i % 4] for i, o in enumerate(charge))
        return fin, code, charge

    def _lis(self, combien):
        morceaux = b""
        if self._reste:
            morceaux = self._reste[:combien]
            self._reste = self._reste[combien:]
        while len(morceaux) < combien:
            try:
                bloc = self._prise.recv(combien - len(morceaux))
            except (OSError, ValueError):
                return None
            if not bloc:
                return None
            morceaux += bloc
        return morceaux

    # -- Emission --------------------------------------------------------------

    def envoie(self, donnees):
        """`send()`. Rend `False` si la connexion n'est plus ouverte."""
        if self.etat != OUVERT:
            return False
        if isinstance(donnees, str):
            return self._envoie(TEXTE, donnees.encode("utf-8"))
        return self._envoie(BINAIRE, bytes(bytearray(donnees)))

    def _envoie(self, code, charge=b""):
        prise = self._prise
        if prise is None:
            return False
        entete = bytes([0x80 | code])
        taille = len(charge)
        # Le masquage est obligatoire cote client : un serveur conforme rejette
        # une trame non masquee, et la raison n'est pas la confidentialite mais
        # les caches intermediaires qu'un attaquant pourrait empoisonner.
        cles = os.urandom(4)
        if taille < 126:
            entete += bytes([0x80 | taille])
        elif taille < 65536:
            entete += bytes([0x80 | 126]) + struct.pack(">H", taille)
        else:
            entete += bytes([0x80 | 127]) + struct.pack(">Q", taille)
        masquee = bytes(o ^ cles[i % 4] for i, o in enumerate(charge))
        try:
            with self._emission:
                prise.sendall(entete + cles + masquee)
        except (OSError, ValueError):
            return False
        if code in (TEXTE, BINAIRE):
            telemetrie.compte("websocket.messages_emis")
        return True

    def ferme(self, code=1000, raison=""):
        """`close()`. La fermeture propre demande un aller-retour."""
        if self.etat in (FERME, FERMETURE_EN_COURS):
            return False
        self.etat = FERMETURE_EN_COURS
        charge = struct.pack(">H", int(code)) + str(raison).encode("utf-8")[:123]
        self._envoie(FERMETURE, charge)
        return True

    def _termine(self):
        self.etat = FERME
        prise, self._prise = self._prise, None
        if prise is not None:
            try:
                prise.close()
            except Exception:  # noqa: BLE001
                pass
        self._pose("close", {"code": self.code_fermeture or 1006,
                             "raison": self.raison_fermeture,
                             "propre": self.propre})

    # -- File d'evenements -----------------------------------------------------

    def _pose(self, type_, charge):
        with self._verrou:
            self._evenements.append((type_, charge))

    def evenements(self):
        """Vide la file et rend ce qui s'y trouvait.

        C'est le seul chemin par lequel une connexion parle au JavaScript, et il
        est emprunte depuis le fil de l'interface — donc jamais au milieu de
        l'execution d'un script.
        """
        with self._verrou:
            arrivees, self._evenements = self._evenements, []
        return arrivees
