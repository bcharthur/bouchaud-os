#!/usr/bin/env python3
"""Envoie UNE commande au moniteur HMP de QEMU (socket Unix) et imprime la reponse.

    qemu_moniteur.py SOCKET "screendump -d vgpu /chemin/capture.ppm"

Remplace `echo ... | socat - unix-connect:SOCKET` la ou socat n'est pas
installe. Code de sortie 1 si le socket ne repond pas.
"""
import socket
import sys
import time


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: qemu_moniteur.py SOCKET COMMANDE", file=sys.stderr)
        return 2
    chemin, commande = sys.argv[1], sys.argv[2]
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5)
    try:
        s.connect(chemin)
    except OSError as e:
        print(f"moniteur injoignable : {e}", file=sys.stderr)
        return 1
    recu = b""
    fin = time.time() + 5
    # Le moniteur annonce sa banniere puis l'invite « (qemu) ».
    while b"(qemu)" not in recu and time.time() < fin:
        try:
            morceau = s.recv(4096)
        except socket.timeout:
            break
        if not morceau:
            break
        recu += morceau
    s.sendall(commande.encode() + b"\n")
    recu = b""
    fin = time.time() + 5
    while recu.count(b"(qemu)") < 1 and time.time() < fin:
        try:
            morceau = s.recv(4096)
        except socket.timeout:
            break
        if not morceau:
            break
        recu += morceau
    s.close()
    texte = recu.decode(errors="replace").replace("\r", "")
    # L'ECHO de la frappe arrive avec des sequences d'effacement du terminal
    # (ESC [ K, ESC [ D) : ces lignes ne sont pas la reponse.
    for ligne in texte.splitlines():
        if "\x1b[" in ligne or not ligne.strip() or ligne.strip() == "(qemu)":
            continue
        print(ligne)
    return 0


if __name__ == "__main__":
    sys.exit(main())
