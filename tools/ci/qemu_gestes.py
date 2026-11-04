#!/usr/bin/env python3
"""Joue un scenario de gestes REELS sur le moniteur QEMU (BOUCHAUD_ATELIER_GESTES_V1).

    qemu_gestes.py SOCKET SORTIE < scenario

Une seule connexion au moniteur pour tout le scenario : les gestes s'enchainent
a quelques millisecondes pres, ce qui permet de saisir une animation de 200 ms
en cours de route. Une instruction par ligne :

    coin                pointeur au coin haut droit
    origine             pointeur au coin haut gauche (0, 0)
    va X Y              pointeur en (X, Y), depuis l'origine
    bouge DX DY         deplacement relatif (decoupe en pas de 120)
    clic                appui puis relachement du bouton gauche
    double              double-clic
    attente S           pause, en secondes (decimales admises)
    capture NOM         VM figee (`stop`), `screendump SORTIE/NOM.ppm`, reprise
    <autre>             commande du moniteur, telle quelle

La souris PS/2 de QEMU est relative et le noyau la borne a l'ecran sans
acceleration. Un grand deplacement d'un seul coup laisse un reliquat dans la
file PS/2 qui absorbe les gestes suivants : tout deplacement est donc decoupe
en pas de 120, chacun tenant dans un paquet.
"""
import socket
import sys
import time

PAS = 120


class Moniteur:
    def __init__(self, chemin):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(10)
        self.s.connect(chemin)
        self._jusqu_a_invite()

    def _jusqu_a_invite(self):
        recu = b""
        fin = time.time() + 10
        while b"(qemu)" not in recu and time.time() < fin:
            morceau = self.s.recv(65536)
            if not morceau:
                break
            recu += morceau
        return recu

    def cmd(self, commande):
        self.s.sendall(commande.encode() + b"\n")
        return self._jusqu_a_invite()


def bouge(m, dx, dy):
    while dx or dy:
        px = max(-PAS, min(PAS, dx))
        py = max(-PAS, min(PAS, dy))
        m.cmd(f"mouse_move {px} {py}")
        dx -= px
        dy -= py
        time.sleep(0.01)


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    m = Moniteur(sys.argv[1])
    sortie = sys.argv[2]
    for ligne in sys.stdin:
        g = ligne.strip()
        if not g:
            continue
        mots = g.split()
        if g == "coin":
            bouge(m, 4000, -4000)
        elif g == "origine":
            bouge(m, -4000, -4000)
        elif mots[0] == "va" and len(mots) == 3:
            bouge(m, -4000, -4000)
            bouge(m, int(mots[1]), int(mots[2]))
        elif mots[0] == "bouge" and len(mots) == 3:
            bouge(m, int(mots[1]), int(mots[2]))
        elif g == "clic":
            m.cmd("mouse_button 1")
            time.sleep(0.04)
            m.cmd("mouse_button 0")
        elif g == "double":
            for _ in range(2):
                m.cmd("mouse_button 1")
                time.sleep(0.03)
                m.cmd("mouse_button 0")
                time.sleep(0.06)
        elif mots[0] == "attente" and len(mots) == 2:
            time.sleep(float(mots[1]))
        elif mots[0] == "capture" and len(mots) == 2:
            m.cmd("stop")
            m.cmd(f"screendump {sortie}/{mots[1]}.ppm")
            m.cmd("cont")
            print(f"BUREAU_GESTE_CAPTURE={sortie}/{mots[1]}.ppm", flush=True)
        else:
            m.cmd(g)
    return 0


if __name__ == "__main__":
    sys.exit(main())
