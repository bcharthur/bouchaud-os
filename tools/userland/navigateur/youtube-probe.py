"""Sonde YouTube : eprouver la chaine entiere contre le vrai service.

    python3 navigateur/youtube-probe.py [adresse ou identifiant]
    exec /bo-navigateur /usr/share/bo-navigateur/youtube-probe.py [adresse]

Les verifications de `test_moteur.py` substituent le reseau : elles etablissent
que le moteur fait ce qu'il faut d'une reponse donnee, jamais que YouTube rend
cette reponse-la aujourd'hui. Cette sonde-ci fait l'inverse — elle parle au vrai
service, et dit **a quelle etape** cela s'arrete.

C'est ce qui la rend utile : « YouTube ne marche pas » ne se corrige pas, alors
que « le client ANDROID_VR est refuse pour preuve d'origine » ou « le flux rend
403 sous cet agent » se corrigent l'un et l'autre.

## Les etapes, dans l'ordre ou elles peuvent lacher

1. **Joignabilite** — la machine atteint-elle YouTube ? Un refus de connexion
   ici n'est pas un defaut du moteur : c'est une politique reseau, et la sonde
   le dit plutot que de laisser croire a une panne.
2. **Reponse du lecteur** — quel client repond, lequel refuse, et pourquoi.
3. **Choix du flux** — un format decodable a-t-il ete retenu ?
4. **Signature** — l'adresse arrive-t-elle signee, et sait-on la resoudre ?
5. **Lecture reelle** — une tranche du flux est-elle servie ? C'est l'etape
   qui prouve le plus : c'est la que Google verifie que l'agent est bien celui
   du client qui a obtenu l'adresse, et la ou un 403 se manifeste.
"""

import sys
import types

# L'hote Qt n'existe pas sur une machine de developpement. Le moteur ne s'en
# sert ici que pour mesurer du texte, ce dont la sonde n'a aucun besoin.
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
    bo.formats_images = lambda: ["png"]
    sys.modules["bo"] = bo

import os  # noqa: E402

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from moteur import reseau, youtube  # noqa: E402

# `signature` a besoin de QuickJS : sur une machine de developpement, le module
# n'existe que si `test-moteur.sh` l'a construit. Son absence n'empeche pas les
# quatre autres etapes, et la sonde le dit plutot que de refuser de demarrer.
try:
    from moteur import signature
except ImportError as _erreur:
    signature = None
    _SANS_QUICKJS = str(_erreur)
else:
    _SANS_QUICKJS = ""

DEFAUT = "https://www.youtube.com/watch?v=aqz-KE-bpKQ"

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


def note(texte):
    print("        %s" % texte)


def etape_joignable():
    """YouTube est-il seulement atteignable depuis cette machine ?"""
    print("\n1. Joignabilite")
    reponse = reseau.charge("https://www.youtube.com/robots.txt", brut=True)
    if reponse.code == 200:
        verifie("youtube.com repond", True)
        return True

    verifie("youtube.com repond", False, "code %s / %s" % (reponse.code, reponse.erreur))
    note("Un refus de connexion ici ne vient pas du moteur. Si la machine passe")
    note("par un mandataire a liste blanche, il faut y autoriser :")
    note("    www.youtube.com, youtubei.googleapis.com,")
    note("    *.googlevideo.com, s.ytimg.com, i.ytimg.com")
    note("Sans `*.googlevideo.com`, l'extraction reussit et la lecture echoue :")
    note("les noms d'hote du media sont tires au sort a chaque requete.")
    return False


def etape_lecteur(video):
    """Quel client repond, lequel refuse, et pourquoi."""
    print("\n2. Reponse du lecteur")
    journal = []
    try:
        donnees, client = youtube.reponse_lecteur(
            video, lambda niveau, texte: journal.append((niveau, texte)))
    except LookupError as e:
        verifie("un client rend un flux", False, e)
        for _, texte in journal:
            note(texte)
        return None, None
    for _, texte in journal:
        note(texte)
    verifie("un client rend un flux", True)
    note("client retenu : %s" % client)
    details = youtube.details(donnees)
    note("titre : %s — %s" % (details["titre"], details["auteur"]))
    return donnees, client


def etape_flux(donnees):
    """Un format decodable a-t-il ete retenu ?"""
    print("\n3. Choix du flux")
    video, audio = youtube.choisit_flux(donnees)
    verifie("un flux image ou son est retenu", video is not None or audio is not None)
    if video is not None:
        note("image : itag %s, %s, %sp%s"
             % (video.itag, video.codec_video, video.hauteur,
                " (progressif)" if video.progressif else " (adaptatif)"))
    if audio is not None:
        note("son separe : itag %s, %s" % (audio.itag, audio.codec_audio))
    if video is not None and not video.progressif and audio is None:
        note("Attention : piste image sans piste son — la video sera muette.")
    return video, audio


def etape_signature(video_id, flux):
    """L'adresse arrive-t-elle signee, et sait-on la resoudre ?"""
    print("\n4. Signature")
    if flux is None:
        return None
    if signature is None:
        note("QuickJS absent (%s) : etape sautee." % _SANS_QUICKJS)
        note("Sous l'OS il est toujours la ; ici, lancer ./test-moteur.sh une")
        note("fois suffit a le construire.")
        return flux.url if not flux.signature_a_dechiffrer else None

    if not flux.signature_a_dechiffrer and "&n=" not in flux.url \
            and "?n=" not in flux.url:
        verifie("aucune signature a resoudre", True)
        return flux.url

    dechiffreur = None
    if flux.signature_a_dechiffrer:
        dechiffreur = signature.dechiffreur_pour(video_id, journal=_journal)
        verifie("le dechiffreur est construit depuis base.js",
                dechiffreur is not None and dechiffreur.utilisable())
    resolue = signature.resout(flux.url, flux.signature_a_dechiffrer, dechiffreur)
    verifie("l'adresse est resolue", resolue is not None)
    if resolue is None:
        return None
    # Le parametre `n` decide du debit : sans sa transformation, le flux est
    # servi au compte-gouttes et la lecture ne suit pas.
    finale = signature.applique_n(resolue, dechiffreur)
    verifie("le parametre n est transforme", finale is not None)
    return finale


def etape_lecture(url, client):
    """Une tranche du flux est-elle reellement servie ?

    C'est l'etape qui prouve le plus. Google lie l'adresse au client qui l'a
    obtenue : la meme adresse reclamee sous un autre agent rend 403. On demande
    donc les deux, pour que le resultat soit lisible dans les deux sens.
    """
    print("\n5. Lecture reelle")
    if not url:
        return

    agent = youtube.agent_du_client(client)
    avec = reseau.tranche(url, 0, 65536, {"User-Agent": agent})
    verifie("une tranche est servie avec l'agent du client",
            avec.code in (200, 206) and len(avec.octets) > 0,
            "code %s, %d octets" % (avec.code, len(avec.octets)))
    if avec.code in (200, 206):
        note("%d octets recus, type %s" % (len(avec.octets), avec.type_mime))

    sans = reseau.tranche(url, 0, 65536, {"User-Agent": "sonde/1.0"})
    if sans.code == 403 and avec.code in (200, 206):
        note("Sous un autre agent, le meme flux rend 403 : c'est exactement le")
        note("defaut que la propagation de l'agent corrige.")
    elif sans.code in (200, 206):
        note("Ce flux ne verifie pas l'agent ; d'autres le font.")


def _journal(niveau, texte):
    note("%s: %s" % (niveau, texte))


def principal():
    cible = sys.argv[1] if len(sys.argv) > 1 else DEFAUT
    video = youtube.identifiant(cible) or (
        cible if len(cible) == 11 and "/" not in cible else None)
    if not video:
        print("adresse incomprehensible : %s" % cible)
        return 2

    print("sonde YouTube — video %s" % video)

    if not etape_joignable():
        print("\nRESULTAT : 1 verification(s) en echec sur %d" % (_reussites + 1))
        return 1

    donnees, client = etape_lecteur(video)
    if donnees is None:
        print("\nRESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1

    flux_video, flux_audio = etape_flux(donnees)
    principal_flux = flux_video or flux_audio
    url = etape_signature(video, principal_flux)
    etape_lecture(url, client)

    print("")
    if _echecs:
        print("RESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1
    print("RESULTAT : 0 verification(s) en echec (%d passees)" % _reussites)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
