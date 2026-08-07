"""Sonde du client leger : parler au vrai service de rendu.

    cd tools/render-proxy && npm start          # dans un autre terminal
    BO_RENDU=http://127.0.0.1:8080 python3 navigateur/distant-probe.py [url]

    exec /bo-navigateur /usr/share/bo-navigateur/distant-probe.py   # sous l'OS

`test_moteur.py` substitue le reseau : il etablit que le client forme les
bonnes adresses et fait ce qu'il faut d'une reponse donnee. Cette sonde-ci
parle au service reel — donc a un vrai Chromium — et etablit ce que l'autre ne
peut pas : qu'une page du web moderne arrive effectivement en image, que le
sondage ne retransmet rien quand rien ne bouge, et que les entrees agissent.
"""

import os
import sys
import types

try:
    import bo  # noqa: F401
except ImportError:
    bo = types.ModuleType("bo")
    bo.largeur_texte = lambda t, s, g=False, f=False: len(t) * s * 0.6
    bo.hauteur_ligne = lambda s, f=False: s * 1.35
    bo.taille = lambda: (1280, 720)
    bo.titre = lambda _: None
    bo.redessiner = lambda: None
    bo.image = lambda _: None
    bo.formats_images = lambda: ["png", "jpeg"]
    sys.modules["bo"] = bo

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from moteur import distant  # noqa: E402

_echecs = []
_reussites = 0


def verifie(nom, condition, detail=""):
    global _reussites
    if condition:
        _reussites += 1
        print("  ok    %s" % nom)
    else:
        _echecs.append(nom)
        print("  ECHEC %s%s" % (nom, (" — " + str(detail)) if detail else ""))


def principal():
    cible = sys.argv[1] if len(sys.argv) > 1 else "https://example.com"
    session = distant.Session(journal=lambda n, t: print("        [%s] %s" % (n, t)))
    print("service : %s" % session.service)
    print("cible   : %s\n" % cible)

    if not session.joignable():
        print("  ECHEC le service ne repond pas")
        print("        Lancer `npm start` dans tools/render-proxy, et pointer")
        print("        BO_RENDU dessus (10.0.2.2:8080 depuis QEMU).")
        print("\nRESULTAT : 1 verification(s) en echec sur 1")
        return 1
    verifie("le service repond", True)

    verifie("la page s'ouvre", session.ouvre(cible, 1024, 640))
    verifie("une image est arrivee", len(session.octets) > 1000, len(session.octets))
    verifie("c'est bien du JPEG", session.octets[:2] == b"\xff\xd8",
            session.octets[:4].hex())
    verifie("le service a signe l'image", bool(session.signature), session.signature)
    print("        %d octets, signature %s" % (len(session.octets), session.signature))

    session.etat()
    verifie("l'adresse courante remonte", bool(session.url), session.url)
    print("        url   : %s" % session.url[:70])
    print("        titre : %s" % session.titre[:60])

    # Rien n'a bouge : le service doit repondre 304, donc `False` ici.
    verifie("une page immobile ne retransmet rien", session.rafraichis() is False)

    # Un defilement doit changer l'image.
    avant = session.signature
    session.defile(500)
    verifie("le defilement change l'image", session.signature != avant,
            "%s -> %s" % (avant[:8], session.signature[:8]))

    verifie("un clic est accepte", session.clic(20, 20))
    verifie("une touche nommee est acceptee", session.touche("End"))
    verifie("une touche inconnue est refusee", session.touche("Truc") is False)

    print("")
    if _echecs:
        print("RESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1
    print("RESULTAT : 0 verification(s) en echec (%d passees)" % _reussites)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
