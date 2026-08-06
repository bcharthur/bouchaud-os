"""Sonde de persistance du navigateur : ce qu'il retient d'une session a l'autre.

Lancee par
`exec /bo-navigateur /usr/share/bo-navigateur/persist-probe.py`.

`test_moteur.py` verifie deja la logique des temoins, du cache et du stockage
local — mais sur la machine de developpement, ou `/persist` n'existe pas : tout
y tourne en memoire, et rien n'est jamais ecrit sur un disque.

Cette sonde-ci tourne sous l'OS, la ou la zone existe. Elle etablit ce que
l'autre ne peut pas : que les fichiers atteignent reellement le disque, et
qu'ils y sont encore au demarrage suivant. Comme `persist-probe.c`, elle
reconnait seule son passage — le temoin qu'elle cherche est celui qu'elle a
depose la fois d'avant.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from moteur import stockage  # noqa: E402

_echecs = []
_reussites = 0


def verifie(nom, condition, detail=""):
    global _reussites
    if condition:
        _reussites += 1
    else:
        _echecs.append("%s%s" % (nom, (" — " + str(detail)) if detail else ""))


def egal(nom, obtenu, attendu):
    verifie(nom, obtenu == attendu, "obtenu %r, attendu %r" % (obtenu, attendu))


ORIGINE = "https://exemple.test/page"
TEMOIN = "session=depuis-le-disque"


def ecrit():
    print("persist-probe.py: passage 1 — depot")

    bocal = stockage.temoins()
    bocal.oublie_tout()
    bocal.absorbe("https://exemple.test/connexion",
                  {"set-cookie": "session=depuis-le-disque; Path=/"})
    egal("le temoin est rendu tout de suite", bocal.pour(ORIGINE), TEMOIN)

    boite = stockage.cache()
    boite.vide()
    egal("une reponse cachable est acceptee",
         boite.depose("https://exemple.test/logo.png", b"PNGDISQUE",
                      {"cache-control": "max-age=86400", "content-type": "image/png"}),
         True)

    local = stockage.local()
    local.efface(ORIGINE)
    local.ecrit(ORIGINE, "theme", "sombre")
    egal("le stockage local repond", local.lit(ORIGINE, "theme"), "sombre")

    # Les fichiers doivent exister dans la zone, pas seulement en memoire.
    verifie("les temoins sont sur le disque",
            os.path.exists(os.path.join(stockage.RACINE, "temoins.json")),
            stockage.RACINE)
    verifie("l'index du cache aussi",
            os.path.exists(os.path.join(stockage.RACINE, "cache-index.json")))
    verifie("le stockage local aussi",
            os.path.exists(os.path.join(stockage.RACINE, "stockage-local.json")))

    # Et l'ensemble doit atteindre le disque, pas seulement le RAMFS.
    os.sync()
    verifie("sync accepte", True)


def relit():
    print("persist-probe.py: passage 2 — relecture apres redemarrage")

    egal("le temoin a survecu", stockage.temoins().pour(ORIGINE), TEMOIN)

    garde = stockage.cache().lit("https://exemple.test/logo.png")
    verifie("l'entree de cache a survecu", garde is not None)
    if garde:
        egal("son contenu est intact", garde[0], b"PNGDISQUE")
        egal("son type aussi", garde[1].get("content-type"), "image/png")

    egal("le stockage local a survecu",
         stockage.local().lit(ORIGINE, "theme"), "sombre")
    # Le cloisonnement tient aussi apres un redemarrage : ce n'est pas un etat
    # de session, c'est une propriete du rangement.
    egal("et reste cloisonne",
         stockage.local().lit("https://autre.test/page", "theme"), None)


def principal():
    print("persist-probe.py: zone = %s (%s)"
          % (stockage.RACINE, "disponible" if stockage.disponible() else "absente"))
    if not stockage.disponible():
        print("RESULTAT : 0 verification(s) en echec (0 passees, zone absente)")
        return 0

    if stockage.temoins().pour(ORIGINE) == TEMOIN:
        relit()
    else:
        ecrit()

    if _echecs:
        for echec in _echecs:
            print("  - %s" % echec)
        print("RESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1
    print("RESULTAT : 0 verification(s) en echec (%d passees)" % _reussites)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
