#!/usr/bin/env python3
"""Epreuves BRDP contre un VRAI Bouchaud OS (QEMU ou machine physique).

Ce fichier n'est PAS une suite hote : il ouvre de vraies connexions TCP vers
le serveur BRDP. Il complete `test_bouchaud_lab.py`, qui teste le client contre
un faux serveur local.

Usage (BOUCHAUD_DEBUG_TOKEN doit deja etre defini dans l'environnement) :
  python3 tools/remote/brdp_qemu_live.py \
      --host 169.254.x.y --out target/brdp-qemu-e2e/live.json

Aucun secret n'est imprime ni ecrit dans le rapport.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import socket
import struct
import time
from dataclasses import dataclass, asdict
from pathlib import Path

PORT = 2222
VERSION = 1
MAX_LINE = 8192


class Echec(RuntimeError):
    pass


class Lecteur:
    def __init__(self, sock: socket.socket):
        self.sock = sock
        self.buf = bytearray()

    def ligne(self) -> bytes:
        while True:
            pos = self.buf.find(b"\n")
            if pos >= 0:
                raw = bytes(self.buf[:pos])
                del self.buf[: pos + 1]
                return raw.rstrip(b"\r")
            morceau = self.sock.recv(4096)
            if not morceau:
                raise EOFError("fin de flux avant la prochaine ligne")
            self.buf.extend(morceau)
            if len(self.buf) > MAX_LINE:
                raise Echec("ligne serveur trop grande")

    def objet(self) -> dict:
        raw = self.ligne()
        try:
            obj = json.loads(raw.decode("utf-8"))
        except Exception as exc:
            raise Echec(f"JSON serveur invalide: {exc}") from exc
        if not isinstance(obj, dict):
            raise Echec("la ligne serveur n'est pas un objet JSON")
        return obj


@dataclass
class Mesure:
    nom: str
    ok: bool
    ms: float | None = None
    detail: str | None = None


class BrdpBrut:
    def __init__(self, host: str, token: str, port: int = PORT, timeout: float = 3.0,
                 rcvbuf: int | None = None):
        self.host = host
        self.token = token.encode("utf-8")
        self.port = port
        self.timeout = timeout
        self.rcvbuf = rcvbuf
        self.sock: socket.socket | None = None
        self.lecteur: Lecteur | None = None
        self.nonce_hex: str | None = None

    def connecte(self) -> dict:
        # Le serveur BRDP n'a volontairement qu'une chaussette d'ecoute.
        # Apres un `quit`, smoltcp doit finir la fermeture TCP avant de pouvoir
        # remettre cette meme chaussette en LISTEN. Un second processus CLI a
        # naturellement le temps de demarrer entre les deux, mais les scenarios
        # de ce banc s'enchainent en quelques microsecondes et peuvent tomber
        # exactement dans cette courte fenetre avec ECONNREFUSED.
        #
        # On tolere donc UNIQUEMENT ce refus transitoire, pendant au plus
        # 1,5 s. Le temps d'attente reste inclus dans `connect_ms` : une pile
        # qui mettrait plusieurs secondes a redevenir joignable ferait toujours
        # echouer `latence-sans-reveil-dhcp`, et une panne durable echoue ici.
        debut = time.monotonic()
        fin_recyclage = debut + min(self.timeout, 1.5)
        while True:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            if self.rcvbuf:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, self.rcvbuf)
            sock.settimeout(self.timeout)
            try:
                sock.connect((self.host, self.port))
                break
            except ConnectionRefusedError:
                sock.close()
                if time.monotonic() >= fin_recyclage:
                    raise
                time.sleep(0.01)
            except BaseException:
                sock.close()
                raise

        self.sock = sock
        self.lecteur = Lecteur(sock)
        annonce = self.lecteur.objet()
        self.connect_ms = (time.monotonic() - debut) * 1000.0
        if annonce.get("brdp") != VERSION:
            raise Echec(f"version BRDP inattendue: {annonce.get('brdp')!r}")
        if annonce.get("auth") != "hmac-sha256":
            raise Echec(f"auth inattendue: {annonce.get('auth')!r}")
        nonce_hex = annonce.get("nonce")
        if not isinstance(nonce_hex, str) or len(nonce_hex) != 64:
            raise Echec("nonce annonce invalide")
        try:
            bytes.fromhex(nonce_hex)
        except ValueError as exc:
            raise Echec("nonce annonce non hexadecimal") from exc
        self.nonce_hex = nonce_hex
        return annonce

    def auth(self) -> dict:
        if self.sock is None or self.lecteur is None or self.nonce_hex is None:
            raise Echec("connexion non ouverte")
        mac = hmac.new(self.token, bytes.fromhex(self.nonce_hex), hashlib.sha256).hexdigest()
        self.envoie_objet({"cmd": "hello", "mac": mac})
        rep = self.lecteur.objet()
        if rep.get("ok") is not True or rep.get("cmd") != 1:
            raise Echec(f"authentification refusee: {rep!r}")
        return rep

    def ouvre_auth(self) -> float:
        debut = time.monotonic()
        self.connecte()
        self.auth()
        return (time.monotonic() - debut) * 1000.0

    def envoie_objet(self, obj: dict) -> None:
        self.envoie_brut(json.dumps(obj, separators=(",", ":")).encode() + b"\n")

    def envoie_brut(self, data: bytes) -> None:
        if self.sock is None:
            raise Echec("connexion non ouverte")
        self.sock.sendall(data)

    def reponse(self, cmd: int | None = None) -> dict:
        if self.lecteur is None:
            raise Echec("connexion non ouverte")
        rep = self.lecteur.objet()
        if cmd is not None:
            if rep.get("ok") is not True or rep.get("cmd") != cmd:
                raise Echec(f"reponse inattendue pour cmd={cmd}: {rep!r}")
        return rep

    def ferme_brutalement(self) -> None:
        if self.sock is not None:
            try:
                # `close()` seul est une fermeture TCP ordinaire : le noyau
                # hote envoie un FIN et la socket smoltcp peut rester dans son
                # cycle de fermeture avant de pouvoir repasser en LISTEN.
                #
                # Ce banc veut au contraire simuler un client qui DISPARAIT
                # brutalement. SO_LINGER(on, 0) transforme le close en RST.
                # C'est a la fois plus fidele au nom du scenario et plus utile :
                # on verifie que le serveur abandonne immediatement la session
                # morte et accepte une nouvelle connexion.
                try:
                    self.sock.setsockopt(
                        socket.SOL_SOCKET,
                        socket.SO_LINGER,
                        struct.pack("ii", 1, 0),
                    )
                except OSError:
                    # Le close doit quand meme avoir lieu ; le scenario suivant
                    # dira si le serveur est redevenu joignable.
                    pass
                self.sock.close()
            finally:
                self.sock = None
                self.lecteur = None

    def ferme(self) -> None:
        if self.sock is None:
            return
        try:
            self.envoie_objet({"cmd": "quit"})
            try:
                self.reponse(18)
            except Exception:
                pass
        finally:
            self.ferme_brutalement()

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        self.ferme_brutalement()


def verifie(cond: bool, message: str) -> None:
    if not cond:
        raise Echec(message)


def scenario_latence(host: str, token: str, port: int, timeout: float,
                     rounds: int, plafond_ms: float) -> Mesure:
    valeurs = []
    for _ in range(rounds):
        c = BrdpBrut(host, token, port, timeout)
        try:
            ms = c.ouvre_auth()
            c.envoie_objet({"cmd": "status"})
            c.reponse(2)
            valeurs.append(ms)
        finally:
            c.ferme()
        time.sleep(0.05)
    pire = max(valeurs)
    verifie(pire <= plafond_ms,
            f"poignee BRDP trop lente: pire={pire:.1f} ms > {plafond_ms:.1f} ms")
    return Mesure("latence-sans-reveil-dhcp", True, round(pire, 3),
                  f"{rounds} connexions, pire poignee complete")


def scenario_coalescence(host: str, token: str, port: int, timeout: float) -> Mesure:
    c = BrdpBrut(host, token, port, timeout)
    try:
        c.ouvre_auth()
        charge = b''.join([
            b'{"cmd":"status"}\n',
            b'{"cmd":"net status"}\n',
            b'{"cmd":"audit status"}\n',
        ])
        c.envoie_brut(charge)
        codes = [c.reponse().get("cmd") for _ in range(3)]
        verifie(codes == [2, 6, 3], f"ordre coalescence incorrect: {codes}")
        return Mesure("trois-commandes-un-send", True, detail=str(codes))
    finally:
        c.ferme()


def scenario_fragmentation(host: str, token: str, port: int, timeout: float) -> Mesure:
    c = BrdpBrut(host, token, port, timeout)
    try:
        c.ouvre_auth()
        ligne = b'{"cmd":"status"}\n'
        for octet in ligne:
            c.envoie_brut(bytes([octet]))
            time.sleep(0.001)
        c.reponse(2)
        return Mesure("commande-fragmentee", True)
    finally:
        c.ferme()


def scenario_avant_auth(host: str, token: str, port: int, timeout: float) -> Mesure:
    c = BrdpBrut(host, token, port, timeout)
    try:
        c.connecte()
        c.envoie_objet({"cmd": "status"})
        rep = c.reponse()
        verifie(rep.get("ok") is False and rep.get("code") == 8,
                f"commande avant auth non refusee comme attendu: {rep!r}")

        # Un refus avant authentification NE FERME PAS la session cote serveur.
        # Le scenario precedent fermait brutalement le client ici, laissant
        # l'unique socket smoltcp traverser son cycle TCP juste avant le test
        # suivant. Le banc mesurait alors son propre teardown, pas BRDP.
        #
        # On verifie plus utilement que la session reste exploitable : hello
        # doit encore fonctionner, puis on quitte proprement pour rendre la
        # socket d'ecoute deterministe au scenario suivant.
        c.auth()
        return Mesure("commande-avant-auth", True,
                      detail="refus code=8 puis authentification sur la meme session")
    finally:
        c.ferme()


def scenario_mauvais_hmac_et_reconnexion(host: str, token: str, port: int,
                                          timeout: float) -> Mesure:
    mauvais = BrdpBrut(host, token, port, timeout)
    nonce1 = None
    try:
        mauvais.connecte()
        nonce1 = mauvais.nonce_hex
        mauvais.envoie_objet({"cmd": "hello", "mac": "00" * 32})
        rep = mauvais.reponse()
        verifie(rep.get("ok") is False and rep.get("code") == 9,
                f"mauvais HMAC non refuse: {rep!r}")
        # Le serveur ferme apres avoir rendu le refus. Laisser un petit delai
        # a smoltcp pour pousser FIN, puis exiger EOF ou erreur de socket.
        time.sleep(0.05)
        ferme = False
        try:
            mauvais.sock.settimeout(1.0)
            data = mauvais.sock.recv(1)
            ferme = data == b""
        except socket.timeout:
            ferme = False
        except (ConnectionError, OSError):
            ferme = True
        verifie(ferme, "connexion mauvais HMAC reste exploitable")
    finally:
        mauvais.ferme_brutalement()

    bon = BrdpBrut(host, token, port, timeout)
    try:
        bon.connecte()
        nonce2 = bon.nonce_hex
        verifie(nonce1 != nonce2, "nonce reutilise apres reconnexion")
        bon.auth()
        bon.envoie_objet({"cmd": "status"})
        bon.reponse(2)
        return Mesure("hmac-ko-puis-reconnexion", True)
    finally:
        bon.ferme()


def scenario_deconnexion_brutale(host: str, token: str, port: int, timeout: float) -> Mesure:
    c = BrdpBrut(host, token, port, timeout)
    c.ouvre_auth()
    c.ferme_brutalement()
    time.sleep(0.05)
    d = BrdpBrut(host, token, port, timeout)
    try:
        d.ouvre_auth()
        d.envoie_objet({"cmd": "status"})
        d.reponse(2)
        return Mesure("deconnexion-brutale-reconnexion", True)
    finally:
        d.ferme()


def scenario_client_lent(host: str, token: str, port: int, timeout: float) -> Mesure:
    # Petit buffer de reception + rafale de commandes productrices d'evenements,
    # puis tail sans lecture pendant un court moment. Le critere observable est
    # l'absence de JSON corrompu et la disponibilite du serveur apres le client
    # lent. Le chemin interne `reste` n'est pas affirme sans compteur serveur.
    c = BrdpBrut(host, token, port, timeout, rcvbuf=2048)
    recus = 0
    try:
        c.ouvre_auth()
        for _ in range(24):
            c.envoie_objet({"cmd": "audit run"})
            c.reponse(4)
        c.envoie_objet({"cmd": "events tail", "n": 128})
        c.reponse(13)
        time.sleep(0.4)
        c.sock.settimeout(0.25)
        while recus < 128:
            try:
                obj = c.lecteur.objet()
            except (socket.timeout, EOFError):
                break
            verifie("seq" in obj or "lost" in obj,
                    f"ligne inattendue dans events tail: {obj!r}")
            recus += 1
    finally:
        c.ferme_brutalement()

    d = BrdpBrut(host, token, port, timeout)
    try:
        d.ouvre_auth()
        d.envoie_objet({"cmd": "status"})
        d.reponse(2)
    finally:
        d.ferme()
    return Mesure("client-lent", True, detail=f"{recus} lignes event/lost valides")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", required=True)
    ap.add_argument("--port", type=int, default=PORT)
    ap.add_argument("--timeout", type=float, default=3.0)
    ap.add_argument("--rounds", type=int, default=12)
    ap.add_argument("--max-handshake-ms", type=float, default=1500.0)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    token = os.environ.get("BOUCHAUD_DEBUG_TOKEN")
    if not token:
        raise SystemExit("BOUCHAUD_DEBUG_TOKEN absent")

    scenarios = [
        lambda: scenario_latence(args.host, token, args.port, args.timeout,
                                 args.rounds, args.max_handshake_ms),
        lambda: scenario_coalescence(args.host, token, args.port, args.timeout),
        lambda: scenario_fragmentation(args.host, token, args.port, args.timeout),
        lambda: scenario_avant_auth(args.host, token, args.port, args.timeout),
        lambda: scenario_mauvais_hmac_et_reconnexion(args.host, token, args.port, args.timeout),
        lambda: scenario_deconnexion_brutale(args.host, token, args.port, args.timeout),
        lambda: scenario_client_lent(args.host, token, args.port, args.timeout),
    ]

    mesures: list[Mesure] = []
    try:
        for lance in scenarios:
            mesure = lance()
            mesures.append(mesure)
            suffixe = f" ({mesure.ms:.1f} ms)" if mesure.ms is not None else ""
            print(f"OK  {mesure.nom}{suffixe}")
    except Exception as exc:
        mesures.append(Mesure("echec", False, detail=str(exc)))
        rapport = {
            "host": args.host,
            "port": args.port,
            "ok": False,
            "mesures": [asdict(m) for m in mesures],
        }
        if args.out:
            Path(args.out).parent.mkdir(parents=True, exist_ok=True)
            Path(args.out).write_text(json.dumps(rapport, indent=2, sort_keys=True), encoding="utf-8")
        print(f"ECHEC BRDP LIVE: {exc}")
        return 1

    rapport = {
        "host": args.host,
        "port": args.port,
        "ok": True,
        "mesures": [asdict(m) for m in mesures],
    }
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(rapport, indent=2, sort_keys=True), encoding="utf-8")
    print("BOUCHAUD_BRDP_LIVE_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
