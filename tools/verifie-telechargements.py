#!/usr/bin/env python3
"""Verifie le chemin d'un telechargement, de l'en-tete au disque.

CE QUI EST EN JEU
-----------------
Le nom du fichier vient d'un en-tete `Content-Disposition`, c'est-a-dire de la
machine d'en face. Les octets viennent du reseau. L'ecriture se fait depuis
WebContent, le processus qui execute le script du site. Trois entrees hostiles
pour une seule operation, et le resultat survit au redemarrage.

LES REGLES
----------
1. Le nom propose passe par `BouchaudNomFichier::assainit` AVANT de toucher un
   chemin. La traversee (`../ladybird/profile/cookies.sqlite`), le fichier
   cache (`.bashrc`) et le nom demesure sont trois facons de choisir ou l'on
   ecrit a la place de l'utilisateur.

2. Un homonyme est NUMEROTE, pas ecrase, et l'ouverture est exclusive.
   `access()` puis `open()` laisse une fenetre entre les deux ; `O_EXCL` la
   ferme.

3. Le depot du chrome et celui du noyau sont le meme. Deux constantes pour un
   seul chemin finissent par diverger, et c'est alors le bac a sable qui
   refuse ce que le chrome vient d'ouvrir.

4. Le droit d'ecriture accorde au rendu porte sur CE sous-arbre et sur rien
   d'autre. Le profil du navigateur -- cookies, HSTS, cache -- reste ferme :
   c'est la frontiere qui compte, et elle ne doit pas bouger en meme temps.

5. Le chemin en processus est force. Sans cela, LibWeb transfere la requete a
   un hote qui n'existe pas ici, et le telechargement n'arrive jamais -- sans
   un mot.

6. Les quatre suites sont branchees : octets, fin, echec, annulation.

7. La fin est SYNCHRONISEE. `/persist` est adosse au RAMFS : ce qui n'est pas
   `fsync` n'atteint le disque qu'a l'extinction, et un telechargement annonce
   comme termine doit survivre a une coupure qui arrive juste apres.

8. Le banc d'essai hote du nom existe. C'est le seul endroit ou l'entree
   hostile est reellement exercee.

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
V19 = RACINE / "tools" / "ladybird" / "prepare-v19-navigateur.py"


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
    debut = source.find(signature)
    if debut < 0:
        return None
    ouvre = source.find("{", debut)
    if ouvre < 0:
        return None
    profondeur = 0
    for index in range(ouvre, len(source)):
        if source[index] == "{":
            profondeur += 1
        elif source[index] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvre : index + 1]
    return None


def regle_assainissement(chrome, fautes):
    """1 et 2."""
    bloc = corps(chrome, "inline Optional<u64> demarre_telechargement(")
    if bloc is None:
        fautes.append(
            "BouchaudChrome.h : `demarre_telechargement` a disparu ; plus rien "
            "n'ouvre de fichier."
        )
        return
    if "BouchaudNomFichier::assainit(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : le nom propose par le SERVEUR n'est plus "
            "assaini avant de former un chemin. Un `Content-Disposition` "
            "choisirait alors ou l'on ecrit."
        )
    if "O_EXCL" not in bloc:
        fautes.append(
            "BouchaudChrome.h : l'ouverture n'est plus exclusive. Entre le "
            "`access()` qui constate l'absence et le `open()` qui cree, un "
            "fichier a pu apparaitre -- et on ecraserait celui d'un autre."
        )

    chemin = corps(chrome, "inline ByteString chemin_de_telechargement(")
    if chemin is None:
        fautes.append("BouchaudChrome.h : `chemin_de_telechargement` a disparu.")
    elif "access(" not in chemin:
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
    """4."""
    bloc = corps(chemins, "pub fn ecriture_permise(")
    if bloc is None:
        fautes.append("chemins.rs : `ecriture_permise` introuvable.")
        return
    if "DOSSIER_TELECHARGEMENTS" not in bloc:
        fautes.append(
            "chemins.rs : le depot n'est plus inscriptible ; le navigateur ne "
            "pourrait plus rien enregistrer."
        )
    if "sous_arbre(" not in bloc:
        fautes.append(
            "chemins.rs : le droit ne porte plus sur un SOUS-ARBRE. Une "
            "comparaison de prefixe sans separateur ouvrirait les voisins de "
            "nom du depot."
        )
    predicat = corps(chemins, "const fn depose_les_telechargements(")
    if predicat is None:
        fautes.append("chemins.rs : `depose_les_telechargements` introuvable.")
    else:
        roles = set(re.findall(r"SecurityProfile::(\w+)", predicat))
        if roles != {"BrowserContent"}:
            fautes.append(
                "chemins.rs : le depot est accorde a %s. Il appartient au seul "
                "role qui tient les octets du corps de reponse ; l'elargir "
                "ouvrirait le disque a un role qui n'en a pas besoin."
                % ", ".join(sorted(roles) or ["personne"])
            )
    # Le profil du navigateur reste ferme au rendu : c'est la frontiere qui
    # compte, et elle ne doit pas bouger en meme temps que le depot.
    profil = corps(chemins, "const fn possede_le_profil(")
    if profil is not None and "BrowserContent" in profil:
        fautes.append(
            "chemins.rs : le profil persistant du navigateur -- cookies, HSTS, "
            "cache -- vient d'etre ouvert a un role de RENDU. Un rendu "
            "compromis y survivrait a un redemarrage."
        )


def regle_v19(v19, fautes):
    """5 et 6."""
    # Le NOM du drapeau n'a aucune importance ; ce qui compte est qu'il soit
    # faux sous `BOUCHAUD_PORT` et qu'il garde la branche de transfert. Une
    # regle qui cherche seulement le nom se laisse satisfaire par une moitie de
    # renommage -- c'est la mutation qui a echappe a la premiere version.
    drapeau = re.search(
        r"#if defined\(BOUCHAUD_PORT\)(?:(?!#e(?:lse|ndif)).)*?"
        r"constexpr bool (\w+) = false;",
        v19,
        re.S,
    )
    if not drapeau:
        fautes.append(
            "prepare-v19-navigateur.py : plus aucun drapeau ne desactive le "
            "transfert de requete sous `BOUCHAUD_PORT`. LibWeb transfererait "
            "la requete a un hote qui n'existe pas dans ce portage, et le "
            "telechargement n'arriverait jamais -- sans un mot."
        )
    elif not re.search(
        r"%s && request_server_request\.has_value\(\)" % re.escape(drapeau.group(1)),
        v19,
    ):
        fautes.append(
            "prepare-v19-navigateur.py : `%s` ne garde plus la branche de "
            "transfert. Il est declare et ignore." % drapeau.group(1)
        )
    for appel, quoi in (
        ("demarre_telechargement(", "l'ouverture du fichier"),
        ("recoit_telechargement(", "les octets recus"),
        ("termine_telechargement(", "la fin"),
        ("echoue_telechargement(", "l'echec"),
        ("telechargement_annule(", "l'annulation"),
    ):
        if appel not in v19:
            fautes.append(
                "prepare-v19-navigateur.py : %s n'est plus branchee." % quoi
            )


def regle_synchronisation(chrome, fautes):
    """7."""
    bloc = corps(chrome, "inline void termine_telechargement(")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `termine_telechargement` a disparu.")
        return
    fsync = bloc.find("fsync(")
    close = bloc.find("close(")
    if fsync < 0:
        fautes.append(
            "BouchaudChrome.h : la fin d'un telechargement n'est plus "
            "synchronisee. `/persist` est adosse au RAMFS : le fichier "
            "n'atteindrait le disque qu'a l'extinction, et une coupure juste "
            "apres le perdrait alors qu'il est annonce comme termine."
        )
    elif close >= 0 and close < fsync:
        fautes.append(
            "BouchaudChrome.h : le descripteur est ferme AVANT d'etre "
            "synchronise ; `fsync` sur un descripteur ferme ne fait rien."
        )


def main():
    fautes = []
    for chemin in (CHROME, NOM, BANC, CHEMINS, V19):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    chrome = sans_commentaires(CHROME.read_text(encoding="utf-8"))
    chemins = sans_commentaires(CHEMINS.read_text(encoding="utf-8"))
    v19 = sans_commentaires(V19.read_text(encoding="utf-8"))

    regle_assainissement(chrome, fautes)
    regle_meme_depot(chrome, chemins, fautes)
    regle_droits(chemins, fautes)
    regle_v19(v19, fautes)
    regle_synchronisation(chrome, fautes)

    if fautes:
        print("telechargements : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("telechargements : nom assaini, depot unique et borne au rendu, "
          "chemin en processus force, fin synchronisee")
    return 0


if __name__ == "__main__":
    sys.exit(main())
