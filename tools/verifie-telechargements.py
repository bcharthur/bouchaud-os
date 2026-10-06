#!/usr/bin/env python3
"""Verifie le chemin d'un telechargement, de l'en-tete au disque.

CE QUI EST EN JEU
-----------------
Le nom du fichier vient d'un en-tete `Content-Disposition`, c'est-a-dire de la
machine d'en face. Les octets viennent du reseau. Le resultat survit au
redemarrage.

BOUCHAUD_UI_V1 : l'ecriture n'a plus lieu dans WebContent. Le processus
navigateur (UI/Bouchaud) reprend la requete et `WebView::FileDownloader`
(upstream) ecrit un fichier temporaire exclusif (`MustBeNew`) puis le RENOMME
a sa place. Le chrome ne fait plus que MONTRER.

LES REGLES
----------
1. Le nom propose passe par `BouchaudNomFichier::assainit` AVANT de toucher un
   chemin (`Application::default_path_for_downloaded_file`).
2. Un homonyme est NUMEROTE, pas ecrase (`chemin_de_telechargement`).
3. Le depot du chrome et celui du noyau sont le meme.
4. AUCUN role sandboxe n'ecrit ni ne lit le depot ou le magasin du chrome.
5. Le navigateur repond a `ask_user_for_download_path` (sinon ECANCELED).
6. Un `FileDownloaderObserver` remonte ajout et progression au chrome.
7. La fin est SYNCHRONISEE (fichier ET dossier : `FileDownloader` renomme).
8. Le banc d'essai hote du nom existe.

Code de retour : 0 si les huit regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
NOM = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudNomFichier.h"
BANC = RACINE / "tools" / "ladybird" / "chrome" / "test_nom_fichier.cpp"
CHEMINS = RACINE / "src" / "kernel" / "security" / "chemins.rs"
APPLICATION = RACINE / "tools" / "ladybird" / "ui-bouchaud" / "Application.cpp"


def sans_commentaires(source):
    """Commentaires de ligne retires, chaines a guillemets doubles preservees.

    Ce fichier cherche des APPELS. Les commentaires de ce depot citent le code
    qu'ils expliquent ; sans depouillement, une regle se laisse satisfaire par
    la phrase qui decrit ce qu'il faudrait faire.
    """
    sortie = []
    index = 0
    taille = len(source)
    while index < taille:
        caractere = source[index]
        if caractere == '"':
            sortie.append(caractere)
            index += 1
            while index < taille:
                sortie.append(source[index])
                if source[index] == "\\" and index + 1 < taille:
                    sortie.append(source[index + 1])
                    index += 2
                    continue
                if source[index] == '"':
                    index += 1
                    break
                index += 1
            continue
        if source.startswith("//", index):
            fin = source.find("\n", index)
            index = taille if fin < 0 else fin
            continue
        sortie.append(caractere)
        index += 1
    return "".join(sortie)


def corps(source, signature):
    """Le corps de la FONCTION dont la signature est donnee.

    Une signature peut apparaitre d'abord comme DECLARATION anticipee. On les
    distingue par le point-virgule qui la termine -- mais SEULEMENT celui qui
    est a profondeur nulle : `-> [u8; 16]` en contient un, et une regle qui le
    prenait pour une fin de declaration cherchait la fonction suivante sans le
    dire, puis rapportait « introuvable » sur une fonction bien presente.
    """
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        # On repart du DEBUT de la signature : certaines « signatures »
        # recherchees incluent deja leur accolade (`struct Champ {`), et
        # repartir apres elle ferait manquer la seule accolade qui compte.
        i = trouve
        while i < len(source):
            c = source[i]
            if c in "([<":
                profondeur += 1
            elif c in ")]>":
                if profondeur > 0:
                    profondeur -= 1
            elif profondeur == 0:
                if c == "{":
                    ouvrante = i
                    break
                if c == ";":
                    declaration = i
                    break
            i += 1
        if ouvrante >= 0:
            break
        if declaration < 0:
            return None
        debut = declaration + 1
    profondeur = 0
    i = ouvrante
    while i < len(source):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
        i += 1
    return None

def regle_assainissement(application, chrome, fautes):
    """1 et 2."""
    bloc = corps(application, "ErrorOr<LexicalPath> Application::default_path_for_downloaded_file(")
    if bloc is None:
        fautes.append("Application.cpp : `default_path_for_downloaded_file` a disparu.")
        return
    assainit = bloc.find("BouchaudNomFichier::assainit(")
    chemin = bloc.find("chemin_de_telechargement(")
    if assainit < 0 or chemin < 0 or assainit > chemin:
        fautes.append(
            "Application.cpp : le nom propose par le SERVEUR n'est plus "
            "assaini AVANT de former un chemin. Un `Content-Disposition` "
            "choisirait alors ou l'on ecrit."
        )
    numerote = corps(chrome, "inline ByteString chemin_de_telechargement(")
    if numerote is None:
        fautes.append("BouchaudChrome.h : `chemin_de_telechargement` a disparu.")
    elif "access(" not in numerote:
        fautes.append(
            "BouchaudChrome.h : un homonyme n'est plus detecte ; le second "
            "telechargement du meme nom ecraserait le premier."
        )


def regle_meme_depot(chrome, chemins, fautes):
    """3."""
    noyau = re.search(
        r'pub const DOSSIER_TELECHARGEMENTS: &str = "([^"]+)";', chemins
    )
    if not noyau:
        fautes.append(
            "chemins.rs : `DOSSIER_TELECHARGEMENTS` a disparu ; le bac a sable "
            "n'accorderait plus le depot."
        )
        return
    bloc = corps(chrome, "inline ByteString dossier_de_telechargement(")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `dossier_de_telechargement` a disparu.")
        return
    if noyau.group(1) not in bloc:
        fautes.append(
            "BouchaudChrome.h : le depot de repli (%s attendu) ne correspond "
            "plus a celui que le noyau autorise. Le chrome ouvrirait un chemin "
            "que le bac a sable refuse." % noyau.group(1)
        )
    if "XDG_DOWNLOAD_DIR" not in bloc:
        fautes.append(
            "BouchaudChrome.h : le depot n'est plus lu dans l'environnement. "
            "La couche plateforme calcule deja la reponse -- `/tmp` quand le "
            "profil est ephemere -- et la recalculer ferait deux verites."
        )


def regle_droits(chemins, fautes):
    """4. Aucun role sandboxe n'atteint le depot ni le magasin du chrome."""
    for fonction in ("pub fn ecriture_permise(", "pub fn lecture_permise("):
        bloc = corps(chemins, fonction)
        if bloc is None:
            fautes.append("chemins.rs : `%s` introuvable." % fonction)
            continue
        for constante in ("DOSSIER_TELECHARGEMENTS", "MAGASIN_DU_CHROME"):
            if constante in bloc:
                fautes.append(
                    "chemins.rs : `%s` accorde de nouveau %s a un role "
                    "sandboxe. Le chrome et l'ecriture des telechargements sont "
                    "dans le processus navigateur ; un rendu compromis n'a plus "
                    "a y toucher." % (fonction.split()[2].rstrip("("), constante)
                )
    if "depose_les_telechargements" in chemins:
        fautes.append("chemins.rs : le predicat `depose_les_telechargements` est revenu.")
    profil = corps(chemins, "const fn possede_le_profil(")
    if profil is not None and "BrowserContent" in profil:
        fautes.append(
            "chemins.rs : le profil persistant du navigateur -- cookies, HSTS, "
            "cache -- vient d'etre ouvert a un role de RENDU."
        )


def regle_navigateur(application, fautes):
    """5, 6 et 7."""
    if "Optional<ByteString> Application::ask_user_for_download_path(" not in application:
        fautes.append(
            "Application.cpp : `ask_user_for_download_path` n'est plus "
            "redefini : chaque telechargement serait annule (ECANCELED)."
        )
    for signature in ("Application::ObservateurTelechargements::download_added(",
                      "Application::ObservateurTelechargements::download_updated("):
        bloc = corps(application, signature)
        if bloc is None or "observe(" not in bloc:
            fautes.append("Application.cpp : %s ne remonte plus au chrome." % signature)
    if "make<ObservateurTelechargements>()" not in application:
        fautes.append("Application.cpp : l'observateur n'est plus cree ; le panneau resterait vide.")
    observe = corps(application, "static void observe(")
    if observe is None or "BouchaudChrome::observe_telechargement(" not in observe:
        fautes.append("Application.cpp : `observe` ne montre plus le telechargement.")
    elif "synchronise_fichier_termine(" not in observe:
        fautes.append(
            "Application.cpp : la fin d'un telechargement n'est plus "
            "synchronisee. `/persist` est adosse au RAMFS : le fichier "
            "n'atteindrait le disque qu'a l'extinction."
        )
    sync = corps(application, "static void synchronise_fichier_termine(")
    if sync is None or "fsync(" not in sync or "dirname()" not in sync:
        fautes.append(
            "Application.cpp : `synchronise_fichier_termine` ne synchronise "
            "plus le fichier ET son dossier (le renommage est une ecriture du "
            "dossier)."
        )


def main():
    fautes = []
    for chemin in (CHROME, NOM, BANC, CHEMINS, APPLICATION):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    chrome = sans_commentaires(CHROME.read_text(encoding="utf-8"))
    chemins = sans_commentaires(CHEMINS.read_text(encoding="utf-8"))
    application = sans_commentaires(APPLICATION.read_text(encoding="utf-8"))

    regle_assainissement(application, chrome, fautes)
    regle_meme_depot(chrome, chemins, fautes)
    regle_droits(chemins, fautes)
    regle_navigateur(application, fautes)

    if fautes:
        print("telechargements : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("telechargements : nom assaini, depot unique ferme aux roles "
          "sandboxes, ecriture par le navigateur, fin synchronisee")
    return 0


if __name__ == "__main__":
    sys.exit(main())
