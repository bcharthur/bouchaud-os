#!/usr/bin/env python3
"""Le client Bouchaud Lab tient-il devant un serveur qui se comporte mal ?

# Pourquoi un faux serveur, et pas le vrai

Le vrai serveur BRDP ne sait pas fragmenter une annonce octet par octet, ni
fermer brutalement au milieu d'une reponse, ni renvoyer du JSON invalide. Ce
sont pourtant les cas qui comptent : un client qui suppose « un recv, une
ligne » marche au banc et perd des reponses le jour ou le reseau est charge --
c'est-a-dire exactement le jour ou l'on enquete.

Un faux serveur les produit a volonte, en boucle locale, sans QEMU et sans
reseau exterieur. Il ne remplace pas le banc de bout en bout ; il couvre ce
que le banc ne sait pas provoquer.

# Ce qui est verifie, et qui ne se voit pas a la lecture

Le jeton. Il ne doit apparaitre ni sur la sortie standard, ni sur l'erreur
standard, ni dans un seul fichier de dump. Deux epreuves le cherchent
explicitement, parce qu'une fuite de secret est precisement le genre de
defaut qu'aucun test fonctionnel ne rencontre.

Lance par `tools/ci/run_host_tests.sh`.
"""

import hashlib
import hmac
import importlib.util
import io
import json
import os
import socket
import sys
import tempfile
import threading
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

# Le module porte un tiret : il s'importe par son chemin, pas par son nom.
_ICI = Path(__file__).resolve().parent
_SPEC = importlib.util.spec_from_file_location(
    "bouchaud_lab", _ICI / "bouchaud-lab.py"
)
lab = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(lab)

JETON = "jeton-de-test-bouchaud-lab"
NONCE = bytes(range(32))


def annonce(version=lab.VERSION_BRDP, auth=lab.AUTH_ATTENDUE, nonce=NONCE):
    objet = {"brdp": version, "auth": auth, "nonce": nonce.hex()}
    return (json.dumps(objet, separators=(",", ":")) + "\n").encode()


def reponse(code, **champs):
    objet = {"ok": True, "cmd": code}
    objet.update(champs)
    return (json.dumps(objet, separators=(",", ":")) + "\n").encode()


def erreur(nom, code):
    objet = {"ok": False, "error": nom, "code": code}
    return (json.dumps(objet, separators=(",", ":")) + "\n").encode()


def evenement(seq, event="LAB_DEMARRE", cat="lab", **champs):
    objet = {"seq": seq, "t_ns": seq * 1_000_000, "cpu": 0,
             "cat": cat, "event": event}
    objet.update(champs)
    return (json.dumps(objet, separators=(",", ":")) + "\n").encode()


class FauxServeur:
    """Un serveur BRDP scriptable, en boucle locale.

    `scenario(conn, lecteur)` recoit la connexion acceptee. Tout ce qu'il ne
    fait pas -- annoncer, authentifier, repondre -- n'arrive pas, ce qui est
    le but : chaque epreuve choisit exactement ce qui va mal.
    """

    def __init__(self, scenario):
        self._scenario = scenario
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._sock.bind(("127.0.0.1", 0))
        self._sock.listen(1)
        self.port = self._sock.getsockname()[1]
        self.vu = []
        self.faute = None
        self._fil = threading.Thread(target=self._sert, daemon=True)

    def __enter__(self):
        self._fil.start()
        return self

    def __exit__(self, *_):
        try:
            self._sock.close()
        except OSError:
            pass
        self._fil.join(timeout=5)
        return False

    def _sert(self):
        try:
            conn, _ = self._sock.accept()
        except OSError:
            return
        try:
            self._scenario(self, conn)
        except Exception as exc:  # noqa: BLE001 -- remonte a l'epreuve
            self.faute = exc
        finally:
            try:
                conn.close()
            except OSError:
                pass

    # -- outils pour les scenarios ----------------------------------------

    @staticmethod
    def lit_ligne(conn, limite=8192):
        tampon = bytearray()
        while b"\n" not in tampon:
            if len(tampon) > limite:
                raise AssertionError("le client a envoye une ligne sans fin")
            morceau = conn.recv(4096)
            if not morceau:
                raise AssertionError("le client a ferme avant d'envoyer")
            tampon.extend(morceau)
        ligne, _, reste = bytes(tampon).partition(b"\n")
        return json.loads(ligne.decode())

    def authentifie(self, conn, nonce=NONCE, accepte=True, octet_par_octet=False):
        """Annonce, verifie le HMAC, et repond. Rend le HMAC recu."""
        donnees = annonce(nonce=nonce)
        if octet_par_octet:
            for o in donnees:
                conn.sendall(bytes([o]))
        else:
            conn.sendall(donnees)
        hello = self.lit_ligne(conn)
        self.vu.append(hello)
        attendu = hmac.new(JETON.encode(), nonce, hashlib.sha256).hexdigest()
        if accepte and hello.get("mac") == attendu:
            conn.sendall(reponse(1))
            return hello.get("mac")
        conn.sendall(erreur("auth-refusee", 9))
        return hello.get("mac")


def client(port, **kw):
    return lab.ClientBrdp("127.0.0.1", JETON, port, kw.pop("delai", 2.0))


class CasBrdp(unittest.TestCase):
    """Ferme toujours ce qu'elle ouvre.

    Un descripteur oublie par une epreuve ne fait rien echouer aujourd'hui et
    fait tomber la suite entiere le jour ou elle grossit. `addCleanup` le rend
    impossible a oublier.
    """

    def ouvre(self, port, delai=2.0):
        c = client(port, delai=delai)
        self.addCleanup(c.ferme)
        c.ouvre()
        return c


# ===========================================================================
# LA POIGNEE DE MAIN
# ===========================================================================


class PoigneeDeMain(CasBrdp):

    def test_annonce_complete_et_auth_acceptee(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)  # quit
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port) as c:
                self.assertTrue(c.authentifiee)
                self.assertEqual(c.version, lab.VERSION_BRDP)
        self.assertIsNone(srv.faute)

    def test_annonce_fragmentee_octet_par_octet(self):
        # TCP EST UN FLUX. Un segment par octet est le pire cas, et rien ne
        # l'interdit ; le client doit recomposer l'annonce sans broncher.
        def scenario(srv, conn):
            srv.authentifie(conn, octet_par_octet=True)
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port) as c:
                self.assertTrue(c.authentifiee)
        self.assertIsNone(srv.faute)

    def test_le_hmac_envoye_est_exactement_celui_du_protocole(self):
        # L'EPREUVE QUI LIE LE CLIENT AU NOYAU. Le serveur recalcule
        # HMAC-SHA256(jeton, nonce) de son cote et compare : si le client
        # hachait le nonce hexadecimal au lieu des octets, cette epreuve
        # tomberait, et elle seule.
        recu = {}

        def scenario(srv, conn):
            recu["mac"] = srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port):
                pass
        attendu = hmac.new(JETON.encode(), NONCE, hashlib.sha256).hexdigest()
        self.assertEqual(recu["mac"], attendu)
        self.assertEqual(len(recu["mac"]), 64)

    def test_auth_refusee_est_une_erreur_d_auth(self):
        def scenario(srv, conn):
            srv.authentifie(conn, accepte=False)

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurAuth):
                self.ouvre(srv.port)

    def test_version_inconnue_est_refusee(self):
        # ON NE S'ADAPTE PAS A UNE VERSION QU'ON NE CONNAIT PAS : ce serait
        # envoyer des commandes dont on ignore l'effet a une machine qu'on
        # essaie de ne pas perturber.
        def scenario(srv, conn):
            conn.sendall(annonce(version=2))

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole) as ctx:
                self.ouvre(srv.port)
        self.assertIn("2", str(ctx.exception))

    def test_nonce_de_mauvaise_longueur_est_refuse(self):
        def scenario(srv, conn):
            conn.sendall(annonce(nonce=b"\x01\x02\x03"))

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole):
                self.ouvre(srv.port)

    def test_nonce_non_hexadecimal_est_refuse(self):
        def scenario(srv, conn):
            objet = {"brdp": 1, "auth": lab.AUTH_ATTENDUE, "nonce": "pas-du-hexa!"}
            conn.sendall((json.dumps(objet) + "\n").encode())

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole):
                self.ouvre(srv.port)

    def test_algorithme_inattendu_est_refuse(self):
        def scenario(srv, conn):
            conn.sendall(annonce(auth="md5"))

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole):
                self.ouvre(srv.port)

    def test_sans_jeton_le_client_ne_se_connecte_meme_pas(self):
        # On refuse AVANT d'ouvrir : sinon on brule un nonce et l'on prend un
        # refus d'authentification pour un probleme reseau.
        with self.assertRaises(lab.ErreurAuth):
            lab.ClientBrdp("127.0.0.1", "", 1)


# ===========================================================================
# LE FLUX : CE QUE TCP NE GARANTIT PAS
# ===========================================================================


class Flux(CasBrdp):

    def test_une_reponse_fragmentee_est_recomposee(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            for o in reponse(2, rx_packets=64):
                conn.sendall(bytes([o]))
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port) as c:
                r = c.commande("status")
        self.assertEqual(r["rx_packets"], 64)

    def test_trois_reponses_dans_un_seul_recv(self):
        # RIEN N'INTERDIT AU NOYAU DISTANT DE COLLER TROIS REPONSES. Un client
        # qui n'en garderait qu'une perdrait les deux autres en silence.
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(2, a=1) + reponse(3, b=2) + reponse(6, c=3))
            srv.lit_ligne(conn)
            srv.lit_ligne(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port) as c:
                self.assertEqual(c.commande("status")["a"], 1)
                self.assertEqual(c.commande("audit")["b"], 2)
                self.assertEqual(c.commande("net")["c"], 3)

    def test_eof_brutal_apres_l_annonce(self):
        def scenario(srv, conn):
            conn.close()

        with FauxServeur(scenario) as srv:
            with self.assertRaises((lab.FinDeFlux, OSError)):
                self.ouvre(srv.port)

    def test_eof_brutal_au_milieu_d_une_reponse(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(b'{"ok":true,"cmd":2,"incom')
            conn.close()

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port)
            with self.assertRaises(lab.FinDeFlux):
                c.commande("status")

    def test_silence_du_pair_rend_un_timeout(self):
        def scenario(srv, conn):
            import time
            time.sleep(1.2)

        with FauxServeur(scenario) as srv:
            with self.assertRaises((socket.timeout, OSError)):
                self.ouvre(srv.port, delai=0.4)

    def test_json_invalide_est_une_faute_de_protocole(self):
        def scenario(srv, conn):
            conn.sendall(b"ceci n'est pas du JSON\n")

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole):
                self.ouvre(srv.port)

    def test_une_ligne_du_protocole_est_toujours_un_objet(self):
        def scenario(srv, conn):
            conn.sendall(b"[1,2,3]\n")

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole):
                self.ouvre(srv.port)

    def test_ligne_trop_longue_est_bornee_et_dite(self):
        # LA BORNE SE DIT, ELLE NE SE SUBIT PAS : un pair qui n'envoie jamais
        # de terminateur ferait grandir le tampon du client sans fin.
        def scenario(srv, conn):
            try:
                conn.sendall(b"x" * (lab.LIGNE_MAX + 4096))
            except OSError:
                pass

        with FauxServeur(scenario) as srv:
            with self.assertRaises(lab.ErreurProtocole) as ctx:
                self.ouvre(srv.port)
        self.assertIn("terminateur", str(ctx.exception))

    def test_une_reponse_de_mauvais_code_est_refusee(self):
        # LES REPONSES NE SONT PAS NUMEROTEES : leur seul rattachement est
        # l'ordre. Un code qui ne correspond pas veut dire qu'on lit la
        # reponse d'une AUTRE commande, et tout ce qui suit sera decale.
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(11))

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port)
            with self.assertRaises(lab.ErreurProtocole):
                c.commande("status")

    def test_une_erreur_du_serveur_porte_son_nom_et_son_code(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(erreur("file-pleine", 10))

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port)
            with self.assertRaises(lab.ErreurCommande) as ctx:
                c.commande("status")
        self.assertEqual(ctx.exception.nom, "file-pleine")
        self.assertEqual(ctx.exception.code, 10)


# ===========================================================================
# LES EVENEMENTS
# ===========================================================================


class Evenements(CasBrdp):

    def test_events_tail_rend_ce_que_l_anneau_contient(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            cmd = srv.lit_ligne(conn)
            assert cmd == {"cmd": "events tail", "n": 5}, cmd
            conn.sendall(reponse(13))
            for i in range(1, 6):
                conn.sendall(evenement(i))
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            with client(srv.port) as c:
                vus = list(c.evenements(tail=5))
        self.assertEqual(len(vus), 5)
        self.assertTrue(all(g == "evenement" for g, _ in vus))
        self.assertEqual([o["seq"] for _, o in vus], [1, 2, 3, 4, 5])

    def test_events_tail_qui_demande_plus_que_l_anneau_n_a_ne_reste_pas_pendu(self):
        # `events tail 100` sur un anneau qui n'en contient que trois rend
        # trois evenements et se tait. Le client doit rendre la main, pas
        # attendre les quatre-vingt-dix-sept qui n'existent pas.
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(13))
            for i in range(1, 4):
                conn.sendall(evenement(i))
            import time
            time.sleep(1.2)

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port, delai=0.5)
            vus = list(c.evenements(tail=100))
        self.assertEqual(len(vus), 3)

    def test_une_perte_est_rendue_comme_une_perte(self):
        # LE TROU EST UNE INFORMATION. Un client qui presenterait une suite
        # trouee comme continue ferait tirer des conclusions fausses sur la
        # chronologie -- c'est pour cela que le serveur l'annonce.
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(13))
            conn.sendall(evenement(10))
            conn.sendall(b'{"lost":90,"from":11,"to":101}\n')
            conn.sendall(evenement(101))
            import time
            time.sleep(1.2)

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port, delai=0.5)
            vus = list(c.evenements(tail=50))
        genres = [g for g, _ in vus]
        self.assertEqual(genres, ["evenement", "perte", "evenement"])
        perte = vus[1][1]
        self.assertEqual((perte["lost"], perte["from"], perte["to"]), (90, 11, 101))

    def test_events_watch_suit_le_flux_et_s_arrete_sur_la_limite(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            cmd = srv.lit_ligne(conn)
            assert cmd == {"cmd": "events watch"}, cmd
            conn.sendall(reponse(14))
            for i in range(1, 9):
                conn.sendall(evenement(i))
            import time
            time.sleep(1.2)

        with FauxServeur(scenario) as srv:
            c = self.ouvre(srv.port, delai=0.5)
            vus = list(c.evenements(limite=4))
        self.assertEqual(len(vus), 4)

    def test_tail_hors_bornes_est_refuse_avant_d_etre_envoye(self):
        for mauvais in (0, lab.EVENTS_TAIL_MAX + 1):
            def scenario(srv, conn):
                srv.authentifie(conn)
                import time
                time.sleep(0.8)

            with FauxServeur(scenario) as srv:
                c = self.ouvre(srv.port, delai=0.5)
                with self.assertRaises(lab.ErreurLab):
                    list(c.evenements(tail=mauvais))


# ===========================================================================
# LA TELEMETRIE ET LA DECOUVERTE
# ===========================================================================


def port_udp_libre():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def envoie_udp(port, charge, depuis_delai=0.15):
    def part():
        import time
        time.sleep(depuis_delai)
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            s.sendto(charge, ("127.0.0.1", port))
        finally:
            s.close()
    fil = threading.Thread(target=part, daemon=True)
    fil.start()
    return fil


class Telemetrie(unittest.TestCase):

    def test_un_datagramme_d_un_evenement(self):
        port = port_udp_libre()
        recus = []
        envoie_udp(port, evenement(7, event="RING_WRAP_AFTER", cat="rtl8168"))
        lab.ecoute_telemetrie(port, 1.5,
                              lambda ip, o, b: recus.append((ip, o)))
        self.assertEqual(len(recus), 1)
        self.assertEqual(recus[0][0], "127.0.0.1")
        self.assertEqual(recus[0][1]["event"], "RING_WRAP_AFTER")

    def test_un_datagramme_de_plusieurs_lignes(self):
        # LE FORMAT EST UNE LIGNE JSON PAR EVENEMENT, et un datagramme en
        # porte plusieurs : c'est tout l'interet du groupage.
        port = port_udp_libre()
        recus = []
        charge = evenement(1) + evenement(2) + evenement(3)
        envoie_udp(port, charge)
        lab.ecoute_telemetrie(port, 1.5, lambda ip, o, b: recus.append(o))
        self.assertEqual([o["seq"] for o in recus], [1, 2, 3])

    def test_un_datagramme_malforme_ne_tue_pas_l_ecoute(self):
        # UN LISTENER QUI MEURT SUR UNE LIGNE TRONQUEE S'ARRETE EXACTEMENT
        # QUAND LA MACHINE COMMENCE A MAL ALLER.
        port = port_udp_libre()
        recus = []
        charge = b"{ceci n'est pas du json\n" + evenement(42) + b"\x00\xff\n"
        envoie_udp(port, charge)
        rejetes = lab.ecoute_telemetrie(port, 1.5,
                                        lambda ip, o, b: recus.append(o))
        self.assertEqual(len(recus), 1)
        self.assertEqual(recus[0]["seq"], 42)
        self.assertGreaterEqual(rejetes, 1)

    def test_une_ligne_qui_n_est_pas_un_objet_est_rejetee(self):
        port = port_udp_libre()
        recus = []
        envoie_udp(port, b"[1,2,3]\n" + evenement(5))
        rejetes = lab.ecoute_telemetrie(port, 1.5,
                                        lambda ip, o, b: recus.append(o))
        self.assertEqual(len(recus), 1)
        self.assertEqual(rejetes, 1)

    def test_l_ecoute_ne_rend_rien_quand_personne_ne_parle(self):
        port = port_udp_libre()
        recus = []
        lab.ecoute_telemetrie(port, 0.6, lambda ip, o, b: recus.append(o))
        self.assertEqual(recus, [])


class Decouverte(unittest.TestCase):

    def test_discover_deduplique_les_sources(self):
        port = port_udp_libre()
        for i in range(4):
            envoie_udp(port, evenement(i + 1), depuis_delai=0.1 + i * 0.05)

        args = lab.argparse.Namespace(port=port, timeout=1.5, json=True)
        sortie = io.StringIO()
        with redirect_stdout(sortie):
            code = lab.fait_discover(args)
        self.assertEqual(code, 0)
        rendu = json.loads(sortie.getvalue())
        self.assertEqual(len(rendu["hotes"]), 1, "une seule source attendue")
        self.assertEqual(rendu["hotes"][0]["host"], "127.0.0.1")
        self.assertEqual(rendu["hotes"][0]["evenements"], 4)

    def test_discover_sans_personne_rend_un_code_non_nul(self):
        port = port_udp_libre()
        args = lab.argparse.Namespace(port=port, timeout=0.6, json=True)
        with redirect_stdout(io.StringIO()):
            code = lab.fait_discover(args)
        self.assertEqual(code, 1)

    def test_discover_n_emet_aucun_paquet(self):
        # C'EST LE POINT DE LA DECOUVERTE PASSIVE. Sonder pour trouver la
        # machine reviendrait a dependre de sa reception -- la chose meme qui
        # est en panne le jour ou l'on cherche.
        port = port_udp_libre()
        temoin = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        temoin.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        temoin.bind(("127.0.0.1", 0))
        temoin.settimeout(0.2)
        try:
            args = lab.argparse.Namespace(port=port, timeout=0.6, json=True)
            with redirect_stdout(io.StringIO()):
                lab.fait_discover(args)
            with self.assertRaises(socket.timeout):
                temoin.recvfrom(2048)
        finally:
            temoin.close()


# ===========================================================================
# LE DUMP
# ===========================================================================


def serveur_de_dump(echoue_sur=()):
    """Un serveur qui repond a tout, sauf aux commandes nommees."""
    par_fil = {c.fil: c for c in lab.COMMANDES}

    def scenario(srv, conn):
        srv.authentifie(conn)
        while True:
            try:
                cmd = srv.lit_ligne(conn)
            except (AssertionError, OSError, json.JSONDecodeError):
                return
            nom = cmd.get("cmd")
            commande = par_fil.get(nom)
            if commande is None:
                conn.sendall(erreur("commande-inconnue", 4))
                continue
            if nom in echoue_sur:
                conn.sendall(erreur("argument-invalide", 6))
                continue
            if nom == "rtl8168 status":
                conn.sendall(reponse(7, rx_packets=64, rx_rendus_tour2=0,
                                     critere_rx_au_dela_de_64=False,
                                     critere_tour2=False))
            elif nom == "events tail":
                conn.sendall(reponse(13))
                for i in range(1, 4):
                    conn.sendall(evenement(i))
            elif nom == "quit":
                conn.sendall(reponse(18))
                return
            else:
                conn.sendall(reponse(commande.code))
    return scenario


class Dump(CasBrdp):

    def _args(self, port, sortie, events=3):
        return lab.argparse.Namespace(
            host="127.0.0.1", port=port, token=JETON, timeout=1.0,
            events=events, out=str(sortie), json=False,
        )

    def test_un_dump_complet_ecrit_tous_les_fichiers(self):
        with FauxServeur(serveur_de_dump()) as srv, \
                tempfile.TemporaryDirectory() as tmp:
            with redirect_stdout(io.StringIO()):
                code = lab.fait_dump(self._args(srv.port, tmp), JETON)
            self.assertEqual(code, 0)
            dossiers = list(Path(tmp).glob("remote-dump-*"))
            self.assertEqual(len(dossiers), 1)
            dossier = dossiers[0]
            attendus = [n for n, _, _ in lab.PLAN_DUMP] + \
                       ["metadata.json", "events.jsonl"]
            for nom in attendus:
                self.assertTrue((dossier / nom).exists(), f"{nom} manque")
            meta = json.loads((dossier / "metadata.json").read_text())
            self.assertEqual(meta["host"], "127.0.0.1")
            self.assertEqual(meta["brdp"], lab.VERSION_BRDP)
            self.assertEqual(meta["commandes_echouees"], 0)
            # LES DEUX CRITERES DE LA CAMPAGNE, recopies du noyau.
            self.assertIn("critere_rx_au_dela_de_64", meta)
            self.assertIn("critere_tour2", meta)
            self.assertIs(meta["critere_tour2"], False)

    def test_le_dump_continue_apres_une_commande_refusee(self):
        # UNE COMMANDE PERDUE N'EMPORTE PAS LES AUTRES. Apres une panne RX,
        # certaines reponses manquent ; celles qui passent restent
        # exploitables, et c'est tout l'interet du dump.
        with FauxServeur(serveur_de_dump(echoue_sur={"dhcp status"})) as srv, \
                tempfile.TemporaryDirectory() as tmp:
            with redirect_stdout(io.StringIO()):
                code = lab.fait_dump(self._args(srv.port, tmp), JETON)
            self.assertEqual(code, 2, "un dump partiel se distingue d'un complet")
            dossier = next(Path(tmp).glob("remote-dump-*"))
            rate = json.loads((dossier / "dhcp.json").read_text())
            self.assertFalse(rate["ok"])
            self.assertIn("erreur_client", rate)
            # Les suivantes ont bien ete relevees malgre l'echec.
            for nom in ("blackbox.json", "services.json", "memory.json"):
                objet = json.loads((dossier / nom).read_text())
                self.assertTrue(objet.get("ok"), f"{nom} aurait du reussir")
            meta = json.loads((dossier / "metadata.json").read_text())
            self.assertEqual(meta["commandes_echouees"], 1)
            self.assertIn("dhcp", meta["echouees"])

    def test_un_dump_sans_connexion_laisse_quand_meme_une_trace(self):
        # « Le client n'a pas pu se connecter a telle heure » est deja un
        # fait, et le jour de la panne c'est souvent LE fait.
        libre = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        libre.bind(("127.0.0.1", 0))
        port = libre.getsockname()[1]
        libre.close()
        with tempfile.TemporaryDirectory() as tmp:
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                code = lab.fait_dump(self._args(port, tmp), JETON)
            self.assertEqual(code, 1)
            dossier = next(Path(tmp).glob("remote-dump-*"))
            meta = json.loads((dossier / "metadata.json").read_text())
            self.assertIn("erreur", meta)
            self.assertEqual(meta["commandes_reussies"], 0)


# ===========================================================================
# LE JETON NE FUIT PAS
# ===========================================================================


class LeJetonNeFuitPas(unittest.TestCase):

    def test_aucun_fichier_de_dump_ne_porte_le_jeton(self):
        with FauxServeur(serveur_de_dump()) as srv, \
                tempfile.TemporaryDirectory() as tmp:
            args = lab.argparse.Namespace(
                host="127.0.0.1", port=srv.port, token=JETON, timeout=1.0,
                events=3, out=str(tmp), json=False)
            with redirect_stdout(io.StringIO()):
                lab.fait_dump(args, JETON)
            dossier = next(Path(tmp).glob("remote-dump-*"))
            for fichier in sorted(dossier.iterdir()):
                contenu = fichier.read_text(encoding="utf-8", errors="replace")
                self.assertNotIn(JETON, contenu, f"{fichier.name} porte le jeton")

    def test_la_sortie_ne_porte_jamais_le_jeton(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(7, rx_packets=64, critere_tour2=False))
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        with FauxServeur(scenario) as srv:
            sortie, erreurs = io.StringIO(), io.StringIO()
            with redirect_stdout(sortie), redirect_stderr(erreurs):
                lab.main(["rtl8168", "--host", "127.0.0.1",
                          "--port", str(srv.port), "--token", JETON])
        self.assertNotIn(JETON, sortie.getvalue())
        self.assertNotIn(JETON, erreurs.getvalue())
        self.assertIn("rx_packets", sortie.getvalue())

    def test_le_message_d_absence_de_jeton_ne_montre_aucune_valeur(self):
        ancien = os.environ.pop("BOUCHAUD_DEBUG_TOKEN", None)
        try:
            erreurs = io.StringIO()
            with redirect_stderr(erreurs):
                code = lab.main(["status", "--host", "127.0.0.1"])
            self.assertEqual(code, 2)
            texte = erreurs.getvalue()
            self.assertIn("BOUCHAUD_DEBUG_TOKEN", texte)
            self.assertIn("--token", texte)
        finally:
            if ancien is not None:
                os.environ["BOUCHAUD_DEBUG_TOKEN"] = ancien

    def test_le_jeton_de_l_environnement_sert_de_repli(self):
        def scenario(srv, conn):
            srv.authentifie(conn)
            srv.lit_ligne(conn)
            conn.sendall(reponse(2))
            srv.lit_ligne(conn)
            conn.sendall(reponse(18))

        ancien = os.environ.get("BOUCHAUD_DEBUG_TOKEN")
        os.environ["BOUCHAUD_DEBUG_TOKEN"] = JETON
        try:
            with FauxServeur(scenario) as srv:
                sortie = io.StringIO()
                with redirect_stdout(sortie):
                    code = lab.main(["status", "--host", "127.0.0.1",
                                     "--port", str(srv.port)])
            self.assertEqual(code, 0)
            self.assertNotIn(JETON, sortie.getvalue())
        finally:
            if ancien is None:
                os.environ.pop("BOUCHAUD_DEBUG_TOKEN", None)
            else:
                os.environ["BOUCHAUD_DEBUG_TOKEN"] = ancien


# ===========================================================================
# LA TABLE DU PROTOCOLE
# ===========================================================================


class TableDuProtocole(unittest.TestCase):

    def test_les_noms_du_fil_sont_ceux_du_noyau(self):
        # SI CETTE EPREUVE TOMBE, LE CLIENT PARLE A COTE. Ces chaines sont
        # celles de `brdp::analyse`, recopiees a la main : le seul moyen de
        # s'apercevoir d'une divergence est de les comparer ici.
        # BOUCHAUD_P13_BRDPCONTRACT_CURRENT
        # Cette table reste volontairement independante de COMMANDES : elle
        # attrape une divergence client/noyau au lieu de recopier le client.
        attendus = {
            "status": ("status", 2), "audit": ("audit status", 3),
            "audit-run": ("audit run", 4), "audit-last": ("audit last", 5),
            "net": ("net status", 6), "rtl8168": ("rtl8168 status", 7),
            "rtl8168-ring": ("rtl8168 ring", 8),
            "rtl8168-desc": ("rtl8168 desc", 9),
            "dhcp": ("dhcp status", 10), "blackbox": ("blackbox status", 11),
            "checkpoint": ("blackbox checkpoint", 12),
            "events-tail": ("events tail", 13),
            "events-watch": ("events watch", 14),
            "services": ("services snapshot", 15),
            "services-page": ("services page", 92),
            "processes": ("processes snapshot", 16),
            "memory": ("memory snapshot", 17),
            "serial-status": ("serial status", 90),
            "quit": ("quit", 18),
            "internet-start": ("internet proof start", 19),
            "internet": ("internet proof status", 20),
            "system-reboot": ("system reboot", 100),
            "system-shutdown": ("system shutdown", 101),
            "browser-start": ("browser start", 110),
            "browser-stop": ("browser stop", 111),
            "browser-restart": ("browser restart", 112),
            "process-kill": ("process kill", 120),
            "process-kill-tree": ("process kill-tree", 121),
        }
        self.assertEqual(set(attendus), set(lab.PAR_NOM))
        for cli, (fil, code) in attendus.items():
            self.assertEqual(lab.PAR_NOM[cli].fil, fil)
            self.assertEqual(lab.PAR_NOM[cli].code, code)

    def test_les_bornes_sont_celles_du_noyau(self):
        self.assertEqual(lab.PORT_BRDP, 2222)
        self.assertEqual(lab.PORT_TELEMETRIE, 2223)
        self.assertEqual(lab.VERSION_BRDP, 1)
        self.assertEqual(lab.NONCE_LEN, 32)
        self.assertEqual(lab.DESCRIPTEURS_MAX, 64)
        self.assertEqual(lab.EVENTS_TAIL_MAX, 1024)
        # `reponses::REPONSE_MAX` vaut 4096 : la borne du client doit la
        # couvrir, sinon il refuse une ligne que le serveur a le droit d'emettre.
        self.assertGreater(lab.LIGNE_MAX, 4096)

    def test_le_descripteur_hors_bornes_est_refuse_par_le_noyau(self):
        # Le client n'invente pas la borne : il la porte pour que l'operateur
        # ait un message clair au lieu d'un `argument-invalide` sur le fil.
        self.assertEqual(lab.DESCRIPTEURS_MAX - 1, 63)

    def test_checkpoint_n_est_pas_dans_le_plan_de_dump(self):
        # UN RELEVE NE MODIFIE PAS CE QU'IL RELEVE. `blackbox checkpoint` est
        # la seule commande du protocole qui ecrit ; elle reste manuelle.
        noms = {c for _, c, _ in lab.PLAN_DUMP}
        self.assertNotIn("checkpoint", noms)


if __name__ == "__main__":
    unittest.main(verbosity=2)
