#!/usr/bin/env python3
"""Verifie le magasin du chrome : historique, favoris, completion.

CE QUI EST EN JEU
-----------------
Le magasin vit dans un sous-arbre auquel le moteur de rendu a acces -- c'est
ce que `src/kernel/security/chemins.rs` accorde, et c'est un elargissement
assume tant que le chrome vit dans WebContent. Ce qui en est relu n'est donc
pas de la donnee de confiance : c'est une entree, au meme titre qu'un en-tete
HTTP.

Deux consequences, et ce fichier garde les deux.

D'abord, une adresse relue peut porter n'importe quel schema. `javascript:`
evalue du script dans le document courant ; un `data:` de premier niveau
fabrique un document a l'origine opaque. Les deux sont l'auto-XSS classique,
celui qu'on fait coller a quelqu'un dans sa barre d'adresse -- sauf qu'ici il
suffit de l'ecrire dans un fichier.

Ensuite, une adresse relue peut porter des octets qu'un clavier ne produit
pas : un saut de ligne qui coupe l'enregistrement en deux, une tabulation qui
est le separateur de champs, un marqueur bidirectionnel qui fait lire
l'adresse a l'envers.

LES REGLES
----------
1. Le depot du chrome et celui du noyau sont le meme. Deux constantes pour un
   seul chemin divergent, et c'est alors le bac a sable qui refuse ce que le
   chrome vient d'ouvrir -- silencieusement.

2. Toute ligne relue est VERIFIEE avant d'entrer dans une liste.

3. Toute adresse notee est verifiee avant d'entrer dans l'historique. La
   verification a la relecture ne suffit pas : ce qui est ecrit aujourd'hui est
   relu demain.

4. La barre d'adresse ne connait plus sa propre liste de schemas. Elle demande
   a `BouchaudUrl`, ou la liste est exercee sur l'hote -- et ou `data:` n'est
   plus.

5. L'ecriture est DIFFEREE, depuis le tic. Ecrire depuis la navigation
   reecrirait le fichier trois fois pour une redirection en chaine.

6. L'ecriture est synchronisee. `/persist` est adosse au RAMFS : sans `fsync`,
   l'historique n'atteint le disque qu'a l'extinction.

7. La completion COPIE l'adresse avant de naviguer. Ses pointeurs designent des
   elements de l'historique, et naviguer en ajoute un.

8. Ctrl+D et l'entree de menu passent par la meme fonction. Deux chemins vers
   le meme geste finissent par le faire differemment.

Code de retour : 0 si les huit regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
URL = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudUrl.h"
BANC = RACINE / "tools" / "ladybird" / "chrome" / "test_url.cpp"
CHEMINS = RACINE / "src" / "kernel" / "security" / "chemins.rs"


def sans_commentaires(source):
    """Commentaires de ligne retires, chaines a guillemets doubles preservees."""
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


def regle_meme_depot(chrome, chemins, fautes):
    """1."""
    noyau = re.search(r'pub const MAGASIN_DU_CHROME: &str = "([^"]+)";', chemins)
    if not noyau:
        fautes.append(
            "chemins.rs : `MAGASIN_DU_CHROME` a disparu ; le bac a sable "
            "refuserait l'historique et les favoris."
        )
        return
    attendu = 'magasin_dossier = "%s"' % noyau.group(1)
    if attendu not in chrome:
        fautes.append(
            "BouchaudChrome.h : le magasin (%s attendu) ne correspond plus a "
            "celui que le noyau autorise. Le chrome ouvrirait un chemin que le "
            "bac a sable refuse, et l'historique disparaitrait sans un mot."
            % noyau.group(1)
        )


def regle_relecture(chrome, fautes):
    """2 et 3."""
    for fonction, quoi in (
        ("inline void adopte_entree(", "une ligne relue du fichier"),
        ("inline void note_visite(", "une adresse ajoutee a l'historique"),
        ("inline void bascule_favori(", "une adresse mise en favori"),
    ):
        bloc = corps(chrome, fonction)
        if bloc is None:
            fautes.append("BouchaudChrome.h : `%s` a disparu." % fonction.strip())
            continue
        if "acceptable_pour_le_magasin(" not in bloc:
            fautes.append(
                "BouchaudChrome.h : %s n'est plus verifiee. Un `javascript:` "
                "ecrit dans le fichier deviendrait une entree de completion "
                "que l'utilisateur peut activer." % quoi
            )


def regle_schemas(chrome, url, fautes):
    """4."""
    bloc = corps(chrome, "inline ByteString normalize_input(")
    if bloc is None:
        fautes.append("BouchaudChrome.h : `normalize_input` a disparu.")
        return
    if "BouchaudUrl::schema_navigable(" not in bloc:
        fautes.append(
            "BouchaudChrome.h : la barre d'adresse a de nouveau sa propre "
            "liste de schemas. Celle de `BouchaudUrl` est exercee sur l'hote ; "
            "une seconde liste ne l'est pas, et c'est elle qui laissera passer "
            "le prochain schema qui execute."
        )
    for schema in ("data:", "javascript:", "vbscript:"):
        if '"%s"' % schema in bloc:
            fautes.append(
                "BouchaudChrome.h : `normalize_input` mentionne `%s`. Ce "
                "schema execute ; il n'a pas a etre nomme dans une barre "
                "d'adresse autrement que pour etre refuse." % schema
            )

    liste = corps(url, "constexpr bool schema_navigable(")
    if liste is None:
        fautes.append("BouchaudUrl.h : `schema_navigable` a disparu.")
        return
    for schema in ("javascript:", "data:", "vbscript:", "blob:"):
        if '"%s"' % schema in liste:
            fautes.append(
                "BouchaudUrl.h : `%s` est entre dans la liste blanche des "
                "schemas navigables." % schema
            )


def regle_ecriture(chrome, fautes):
    """5 et 6."""
    tic = corps(chrome, "inline void tick()")
    if tic is None or "ecrit_le_magasin()" not in tic:
        fautes.append(
            "BouchaudChrome.h : le magasin ne s'ecrit plus depuis le tic. "
            "Ecrire depuis la navigation reecrirait le fichier trois fois pour "
            "une seule redirection en chaine."
        )
    for fonction in ("inline void note_visite(", "inline void note_titre("):
        bloc = corps(chrome, fonction)
        if bloc is not None and "ecrit_le_magasin()" in bloc:
            fautes.append(
                "BouchaudChrome.h : `%s` ecrit le fichier elle-meme ; "
                "l'ecriture doit rester differee." % fonction.strip()
            )

    ecriture = corps(chrome, "inline bool ecrit_fichier(")
    if ecriture is None:
        fautes.append("BouchaudChrome.h : `ecrit_fichier` a disparu.")
        return
    # La SEQUENCE, et non deux positions : le chemin d'erreur de la boucle
    # d'ecriture ferme lui aussi le descripteur, bien avant le `fsync` du
    # chemin normal. Comparer la premiere occurrence de chacun donnait un faux
    # positif -- et une regle qui crie a tort finit par etre desactivee.
    if "fsync(" not in ecriture:
        fautes.append(
            "BouchaudChrome.h : le magasin n'est plus synchronise. `/persist` "
            "est adosse au RAMFS : l'historique n'atteindrait le disque qu'a "
            "l'extinction."
        )
    elif not re.search(r"fsync\(fd\);\s*close\(fd\);", ecriture):
        fautes.append(
            "BouchaudChrome.h : la fermeture ne suit plus immediatement la "
            "synchronisation ; un `fsync` sur un descripteur deja ferme ne "
            "fait rien."
        )


def regle_completion(chrome, fautes):
    """7."""
    # Les deux endroits qui NAVIGUENT depuis une proposition. Chacun doit
    # copier l'adresse : les pointeurs designent des elements de l'historique,
    # et `commit_address` finit par en ajouter un.
    for fonction in ("inline void handle_key(", "inline void handle_pointer("):
        bloc = corps(chrome, fonction)
        if bloc is None:
            continue
        if "entrees_de_completion(" not in bloc:
            continue
        if "auto const cible =" not in bloc:
            fautes.append(
                "BouchaudChrome.h : `%s` navigue depuis une proposition sans "
                "en copier l'adresse. Le pointeur designe un element de "
                "l'historique, et naviguer en ajoute un." % fonction.strip()
            )


def regle_favori(chrome, fautes):
    """8."""
    raccourcis = corps(chrome, "inline bool raccourci_navigateur(")
    if raccourcis is None or "bascule_favori()" not in raccourcis:
        fautes.append(
            "BouchaudChrome.h : Ctrl+D ne met plus rien de cote."
        )
    menu = corps(chrome, "inline void active_entree_menu(")
    if menu is None or "bascule_favori()" not in menu:
        fautes.append(
            "BouchaudChrome.h : l'entree de menu des favoris ne passe plus par "
            "`bascule_favori`. Deux chemins vers le meme geste finissent par le "
            "faire differemment."
        )


def main():
    fautes = []
    for chemin in (CHROME, URL, BANC, CHEMINS):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    chrome = sans_commentaires(CHROME.read_text(encoding="utf-8"))
    url = sans_commentaires(URL.read_text(encoding="utf-8"))
    chemins = sans_commentaires(CHEMINS.read_text(encoding="utf-8"))

    regle_meme_depot(chrome, chemins, fautes)
    regle_relecture(chrome, fautes)
    regle_schemas(chrome, url, fautes)
    regle_ecriture(chrome, fautes)
    regle_completion(chrome, fautes)
    regle_favori(chrome, fautes)

    if fautes:
        print("historique et favoris : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print("historique et favoris : depot unique, toute adresse relue verifiee, "
          "ecriture differee et synchronisee")
    return 0


if __name__ == "__main__":
    sys.exit(main())
