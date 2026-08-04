"""Sonde Python : verifie l'OS depuis un interprete reel, pas depuis un test.

Les sondes en C interrogent l'ABI appel par appel. Celle-ci fait l'inverse :
elle laisse CPython utiliser le systeme comme il en a l'habitude, et regarde ce
qui casse. Une bibliotheque standard entiere passe par des chemins qu'aucune
sonde ecrite a la main ne pense a emprunter — c'est precisement la qu'on trouve
les manques.

Lancee par `exec /usr/bin/python3 /python-probe.py`. Meme format de bilan que
les sondes C, pour que tools/test.sh la compte comme les autres.
"""

import errno
import io
import json
import os
import platform
import re
import select
import socket
import struct
import sys
import tempfile
import threading
import time

echecs = 0


def verifie(quoi, condition, detail=""):
    global echecs
    print("  %-44s %s" % (quoi, "ok" if condition else "ECHEC"))
    if detail:
        print("    " + str(detail))
    if not condition:
        echecs += 1


def section(nom):
    print("\n[%s]" % nom)


# ----------------------------------------------------------------- interprete

def teste_interprete():
    section("interprete")
    verifie("l'interprete demarre", True, "Python " + platform.python_version())
    verifie("sys.platform == linux", sys.platform == "linux")
    verifie("la bibliotheque standard est importable", json and re and struct)
    # Le zip de la bibliotheque standard est lu par zipimport, qui a besoin de
    # mmap et de lectures a offset. Qu'un module non gele s'importe le prouve.
    import textwrap
    verifie("import depuis l'archive zip", textwrap.dedent("  a") == "a")
    verifie("uname() renvoie le noyau", os.uname().sysname == "Linux",
            " ".join(os.uname()[:3]))
    verifie("compilation a la volee", eval(compile("2**16", "<sonde>", "eval")) == 65536)


# --------------------------------------------------------------- systeme de fichiers

def teste_fichiers():
    section("systeme de fichiers")
    chemin = "/tmp/sonde-python.txt"
    contenu = "ligne un\nligne deux\naccents : eaiou\n"

    with open(chemin, "w") as f:
        f.write(contenu)
    verifie("ecriture d'un fichier", os.path.exists(chemin))

    with open(chemin) as f:
        relu = f.read()
    verifie("relecture identique", relu == contenu, "%d octets" % len(relu))

    infos = os.stat(chemin)
    verifie("stat() donne la bonne taille", infos.st_size == len(contenu.encode()))

    with open(chemin, "rb") as f:
        f.seek(9)
        verifie("seek() puis read()", f.read(5) == b"ligne")

    os.makedirs("/tmp/sonde/a/b", exist_ok=True)
    verifie("makedirs() en cascade", os.path.isdir("/tmp/sonde/a/b"))

    entrees = sorted(os.listdir("/tmp"))
    verifie("listdir() voit le fichier cree", "sonde-python.txt" in entrees,
            ", ".join(entrees[:6]))

    trouves = [n for _, _, fichiers in os.walk("/tmp/sonde") for n in fichiers]
    verifie("os.walk() parcourt l'arborescence", trouves == [])

    os.remove(chemin)
    verifie("remove() efface", not os.path.exists(chemin))

    # tempfile enchaine mkdir, open(O_EXCL), fstat, unlink : un raccourci pour
    # verifier d'un coup ce qu'une bibliotheque tierce fera de toute facon.
    with tempfile.NamedTemporaryFile(mode="w", suffix=".tmp", delete=False) as f:
        temporaire = f.name
        f.write("temporaire")
    verifie("tempfile cree un fichier unique", os.path.exists(temporaire), temporaire)
    os.remove(temporaire)

    try:
        open("/introuvable-absolument", "r")
        ouvert = True
    except FileNotFoundError:
        ouvert = False
    verifie("un fichier absent leve FileNotFoundError", not ouvert)


# ------------------------------------------------------------------- processus

def teste_processus():
    section("processus")
    verifie("getpid() rend un pid plausible", os.getpid() > 0, "pid=%d" % os.getpid())
    verifie("getcwd()", os.getcwd().startswith("/"), os.getcwd())

    lecture, ecriture = os.pipe()
    os.write(ecriture, b"par le tube")
    os.close(ecriture)
    verifie("os.pipe() transporte des octets", os.read(lecture, 64) == b"par le tube")
    os.close(lecture)

    # fork() depuis Python : l'enfant reprend l'interprete la ou il en etait,
    # avec son propre espace d'adressage.
    lecture, ecriture = os.pipe()
    enfant = os.fork()
    if enfant == 0:
        os.close(lecture)
        os.write(ecriture, ("enfant pid=%d" % os.getpid()).encode())
        os.close(ecriture)
        os._exit(7)
    os.close(ecriture)
    message = os.read(lecture, 64).decode()
    os.close(lecture)
    _, statut = os.waitpid(enfant, 0)
    verifie("fork() donne un enfant vivant", message.startswith("enfant pid="), message)
    verifie("waitpid() rend le code de sortie", os.WIFEXITED(statut) and os.WEXITSTATUS(statut) == 7)

    verifie("environ est peuple", "PATH" in os.environ, "PATH=" + os.environ.get("PATH", ""))
    verifie("PYTHONHOME oriente le prefixe", sys.prefix == "/usr", sys.prefix)


# ----------------------------------------------------------------------- threads

def teste_threads():
    section("threads")
    resultats = []
    verrou = threading.Lock()

    def travail(n):
        with verrou:
            resultats.append(n * n)

    fils = [threading.Thread(target=travail, args=(i,)) for i in range(4)]
    for f in fils:
        f.start()
    for f in fils:
        f.join()
    verifie("4 threads demarrent et se terminent", sorted(resultats) == [0, 1, 4, 9],
            sorted(resultats))

    # Un evenement traverse deux threads : c'est un futex sous le capot, avec
    # une attente a echeance — le cas ou une erreur d'unite (relative contre
    # absolue) se voit tout de suite.
    signal_ = threading.Event()
    debut = time.monotonic()
    threading.Timer(0.15, signal_.set).start()
    obtenu = signal_.wait(timeout=5.0)
    ecoule = time.monotonic() - debut
    verifie("Event.wait() reveille par un autre thread", obtenu)
    verifie("le reveil arrive dans le bon ordre de grandeur", 0.1 < ecoule < 2.0,
            "%.3f s" % ecoule)


# ------------------------------------------------------------------------ temps

def teste_temps():
    section("temps")
    debut = time.monotonic()
    time.sleep(0.2)
    ecoule = time.monotonic() - debut
    verifie("sleep(0.2) dure environ 0,2 s", 0.15 < ecoule < 1.0, "%.3f s" % ecoule)

    verifie("time.time() est posterieur a 2020", time.time() > 1577836800,
            time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime()))

    import datetime
    maintenant = datetime.datetime.now()
    verifie("datetime.now() fonctionne", maintenant.year >= 2020, str(maintenant))

    # Deux lectures consecutives de l'horloge monotone ne doivent jamais
    # reculer : c'est ce sur quoi reposent tous les delais d'une boucle
    # d'evenements.
    lectures = [time.monotonic() for _ in range(50)]
    verifie("l'horloge monotone ne recule pas",
            all(b >= a for a, b in zip(lectures, lectures[1:])))


# ------------------------------------------------------------------ descripteurs

def teste_select():
    section("multiplexage")
    lecture, ecriture = os.pipe()

    prets, _, _ = select.select([lecture], [], [], 0)
    verifie("select() sur un tube vide ne rend rien", prets == [])

    os.write(ecriture, b"x")
    prets, _, _ = select.select([lecture], [], [], 1.0)
    verifie("select() voit le tube devenu lisible", prets == [lecture])
    os.read(lecture, 1)

    sondeur = select.poll()
    sondeur.register(lecture, select.POLLIN)
    verifie("poll() sans donnee expire", sondeur.poll(0) == [])
    os.write(ecriture, b"y")
    verifie("poll() signale la donnee", len(sondeur.poll(1000)) == 1)
    os.read(lecture, 1)
    os.close(lecture)
    os.close(ecriture)


# ----------------------------------------------------------------------- reseau

def teste_reseau():
    section("reseau")
    try:
        adresse = socket.gethostbyname("example.com")
        resolu = True
    except OSError as e:
        adresse, resolu = str(e), False
    verifie("gethostbyname() resout un nom", resolu, adresse)
    if not resolu:
        return

    prise = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    prise.settimeout(20)
    try:
        prise.connect((adresse, 80))
        prise.sendall(b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n")
        reponse = prise.recv(512)
        joint = True
    except OSError as e:
        reponse, joint = str(e).encode(), False
    finally:
        prise.close()
    verifie("connect() + sendall() vers le port 80", joint)
    verifie("recv() rend une reponse HTTP", reponse.startswith(b"HTTP/1."),
            reponse[:40])


# ------------------------------------------------------------------ aleatoire

def teste_aleatoire():
    section("aleatoire")
    # os.urandom passe par getrandom() : une source qui rendrait toujours la
    # meme chose casserait silencieusement tout ce qui hache ou chiffre.
    tirages = {os.urandom(16) for _ in range(8)}
    verifie("os.urandom() ne se repete pas", len(tirages) == 8)
    verifie("os.urandom() rend la bonne longueur", len(os.urandom(32)) == 32)

    import random
    verifie("random tire dans l'intervalle demande",
            all(0 <= random.randrange(10) < 10 for _ in range(50)))

    import hashlib
    empreinte = hashlib.sha256(b"bouchaud").hexdigest()
    verifie("hashlib.sha256() (implementation interne)", len(empreinte) == 64, empreinte[:16])


# ------------------------------------------------------------------------- main

def main():
    print("Sonde Python — l'OS vu par un interprete reel\n")
    for test in (teste_interprete, teste_fichiers, teste_processus, teste_threads,
                 teste_temps, teste_select, teste_reseau, teste_aleatoire):
        try:
            test()
        except Exception as e:  # une exception non rattrapee est un echec en soi
            global echecs
            echecs += 1
            print("  %-44s %s" % (test.__name__, "ECHEC"))
            import traceback
            traceback.print_exc(file=sys.stdout)

    print("\nRESULTAT : %d verification(s) en echec" % echecs)
    return 1 if echecs else 0


if __name__ == "__main__":
    sys.exit(main())
