#!/usr/bin/env python3
"""Garde-fou : la page d'accueil du navigateur doit arriver dans l'image.

# Le defaut, constate le 16 septembre 2026

Ladybird a demarre de bout en bout pour la premiere fois : services lances,
DHCP, DNS, TLS, HTTP 200, 286 467 octets recus de google.com. Et la fenetre
affichait ceci :

    Failed to load file:///usr/share/ladybird/bouchaud-start.html
    Load failed: No such file or directory (errno=2), Duration: 9470ms

Le navigateur etait parfaitement fonctionnel. C'est la page qui manquait.

# Pourquoi elle manquait

`tools/ladybird/start.html` est un fichier A NOUS. Il n'etait installe que par
`tools/ladybird/browser-upstream.sh`, c'est-a-dire uniquement par la
reconstruction integrale de Ladybird -- des heures de vcpkg, Skia, ICU,
HarfBuzz et LibWeb.

Or la voie normale n'est pas celle-la : c'est l'artefact que la CI publie, et
le workflow `ladybird-native-browser.yml` ne construit que sur `main`. La page
a ete ajoutee le 13 septembre par `cf4d63b`, qui n'a jamais ete poussee sur
`main` ; le dernier commit de `main` touchant `tools/ladybird/**` date du
5 septembre. L'artefact telecharge precedait donc la page de huit jours, et
son `resources/` ne pouvait pas la contenir -- pendant que le noyau, lui,
exportait deja son URL.

Deux fichiers voisins avaient deja ce probleme et l'avaient deja resolu :
`fonts.conf` et le bundle CA sont copies DEPUIS LE DEPOT par
`prepare-reference-ladybird.ps1`, apres `resources/`, precisement pour ne pas
dependre de l'age de l'artefact. La page d'accueil ne l'etait pas.

# Ce qui est verifie ici

1. `tools/ladybird/start.html` existe et est une vraie page.
2. Le noyau ne porte qu'UNE definition du chemin, et l'URL en decoule.
3. `prepare-reference-ladybird.ps1` installe la page DEPUIS LE DEPOT, et le
   fait APRES avoir copie `resources/` -- sinon un artefact ancien la
   recouvrirait, ou l'absence d'un fichier suffirait a la faire disparaitre.
4. `verify-reference-ladybird-image.py` EXIGE la page dans l'image. Ce point
   n'est pas cosmetique : `prepare-reference-ladybird.ps1` REUTILISE une
   `ladybird-browser.img` existante des que ce verificateur la declare valide.
   Sans cette exigence, une image deja fabriquee sans page d'accueil survit a
   la correction, et l'utilisateur ne voit rien changer.
5. `browser-upstream.sh` continue de l'installer, pour que la voie longue et
   la voie courte produisent la meme image.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

PAGE = RACINE / "tools/ladybird/start.html"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
PREPARE = RACINE / "tools/reference/prepare-reference-ladybird.ps1"
VERIFICATEUR = RACINE / "tools/reference/verify-reference-ladybird-image.py"
UPSTREAM = RACINE / "tools/ladybird/browser-upstream.sh"
RUN = RACINE / "run.ps1"


def lit(chemin, fautes):
    """Le contenu d'un fichier, ou None avec une faute deja enregistree."""
    if not chemin.exists():
        fautes.append(
            "fichier absent : %s" % chemin.relative_to(RACINE).as_posix()
        )
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []

    page = lit(PAGE, fautes)
    stage2 = lit(STAGE2, fautes)
    prepare = lit(PREPARE, fautes)
    verificateur = lit(VERIFICATEUR, fautes)
    upstream = lit(UPSTREAM, fautes)
    run = lit(RUN, fautes)

    if page is not None and "<html" not in page.lower():
        fautes.append(
            "tools/ladybird/start.html n'est plus une page HTML. C'est elle "
            "que le navigateur ouvre au demarrage : si elle cesse d'etre une "
            "page, la premiere chose que voit l'utilisateur cesse d'en etre "
            "une aussi."
        )

    # 2. Le noyau : une seule definition, et l'URL qui en decoule.
    chemin = url = None
    if stage2 is not None:
        m_chemin = re.search(
            r'const\s+CHEMIN_ACCUEIL\s*:\s*&str\s*=\s*"([^"]+)"\s*;', stage2
        )
        m_url = re.search(
            r'const\s+URL_ACCUEIL\s*:\s*&str\s*=\s*"([^"]+)"\s*;', stage2
        )
        if m_chemin is None:
            fautes.append(
                "src/platform/pc/stage2.rs : CHEMIN_ACCUEIL a disparu. Sans "
                "lui le noyau ne peut plus CONSTATER la presence de la page "
                "avant de lancer le navigateur, et l'absence se redecouvre "
                "quatre-vingts secondes plus tard dans le journal de "
                "WebContent."
            )
        else:
            chemin = m_chemin.group(1)
        if m_url is None:
            fautes.append(
                "src/platform/pc/stage2.rs : URL_ACCUEIL a disparu."
            )
        else:
            url = m_url.group(1)

        if chemin and url and url != "file://" + chemin:
            fautes.append(
                "src/platform/pc/stage2.rs : URL_ACCUEIL (%s) ne designe plus "
                "CHEMIN_ACCUEIL (%s). Le noyau verifierait alors la presence "
                "d'un fichier et en ouvrirait un autre -- exactement le "
                "desaccord que cette garde existe pour empecher."
                % (url, chemin)
            )

        if chemin and 'set_exported_for_boot("BOUCHAUD_M9_URL", URL_ACCUEIL)' not in stage2:
            fautes.append(
                "src/platform/pc/stage2.rs : BOUCHAUD_M9_URL n'est plus "
                "exporte depuis URL_ACCUEIL. Une URL litterale reintroduirait "
                "la deuxieme copie du chemin."
            )

        if "BOUCHAUD_STAGE2_PAGE_ACCUEIL_ABSENTE" not in stage2:
            fautes.append(
                "src/platform/pc/stage2.rs : l'amorcage ne signale plus une "
                "page d'accueil manquante. C'est la ligne qui aurait nomme ce "
                "defaut en une seconde au lieu de quatre-vingt-trois."
            )

    relatif = chemin.lstrip("/") if chemin else "usr/share/ladybird/bouchaud-start.html"
    base = relatif.rsplit("/", 1)[-1]

    # 3. La preparation de l'image : depuis le depot, et APRES resources/.
    if prepare is not None:
        pos_page = prepare.find(base)
        pos_ressources = prepare.find("$Resources")
        pos_copie_ressources = prepare.find("-Path (Join-Path $Resources")

        if "$PageAccueil" not in prepare or pos_page < 0:
            fautes.append(
                "tools/reference/prepare-reference-ladybird.ps1 : la page "
                "d'accueil n'est plus installee depuis le depot. Elle "
                "redeviendrait alors tributaire de l'age de l'artefact CI, "
                "qui est construit sur `main` et peut preceder de plusieurs "
                "jours le fichier que le noyau reclame. C'est litteralement "
                "le defaut du 16 septembre."
            )
        elif pos_copie_ressources >= 0 and pos_page < pos_copie_ressources:
            fautes.append(
                "tools/reference/prepare-reference-ladybird.ps1 : la page "
                "d'accueil est installee AVANT la copie de `resources/`. Un "
                "artefact portant sa propre version la recouvrirait, et le "
                "depot cesserait de faire autorite sur son propre fichier."
            )

        if pos_ressources < 0:
            fautes.append(
                "tools/reference/prepare-reference-ladybird.ps1 : la copie de "
                "`resources/` a disparu ; l'ordre ne peut plus etre verifie."
            )

        if "tools\\ladybird\\start.html" not in prepare:
            fautes.append(
                "tools/reference/prepare-reference-ladybird.ps1 : la source "
                "de la page n'est plus tools/ladybird/start.html."
            )

    # 4. Le verificateur d'image, qui est aussi la cle d'invalidation du cache.
    if verificateur is not None and '"%s"' % relatif not in verificateur:
        fautes.append(
            "tools/reference/verify-reference-ladybird-image.py : `%s` n'est "
            "plus exige. Une image sans page d'accueil serait declaree valide "
            "-- et comme prepare-reference-ladybird.ps1 REUTILISE toute image "
            "que ce verificateur accepte, une image deja fabriquee sans la "
            "page survivrait indefiniment a la correction." % relatif
        )

    # 4bis. run.ps1 assemble SA PROPRE arborescence, et accepte lui aussi
    # l'artefact de la CI : il porte donc le meme defaut, et a besoin de la
    # meme copie depuis le depot, elle aussi APRES resources/.
    if run is not None:
        pos_page = run.find(base)
        # ANCRER SUR LA COPIE, PAS SUR LA VARIABLE.
        # `$ResourcesDir` est defini des centaines de lignes plus haut : s'y
        # accrocher faisait passer pour "apres resources/" une installation
        # placee juste avant la copie. La mutation correspondante n'etait pas
        # attrapee.
        pos_ressources = run.find('(Join-Path $ResourcesDir "*")')
        if "$StartPageSource" not in run or pos_page < 0:
            fautes.append(
                "run.ps1 : la page d'accueil n'est plus installee depuis le "
                "depot. `run.ps1 -Ladybird` accepte un artefact CI comme "
                "`-NativeBrowser`, et un artefact anterieur a la page ne la "
                "contient pas : le navigateur rouvrirait sur errno=2."
            )
        elif pos_ressources >= 0 and pos_page < pos_ressources:
            fautes.append(
                "run.ps1 : la page d'accueil est installee AVANT la copie de "
                "resources/. Un artefact portant sa propre version la "
                "recouvrirait -- c'est exactement le defaut que le commentaire "
                "BOUCHAUD_FONTCONFIG_DEPOT_FAIT_FOI_V1 decrit pour fonts.conf."
            )

    # 5. La voie longue doit produire la meme image que la voie courte.
    if upstream is not None and base not in upstream:
        fautes.append(
            "tools/ladybird/browser-upstream.sh : la reconstruction integrale "
            "n'installe plus `%s`. Les deux voies produiraient des images "
            "differentes, et le defaut ne se verrait que sur l'une des deux."
            % base
        )

    if fautes:
        print("page d'accueil du navigateur : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "page d'accueil du navigateur : %s installee depuis le depot, exigee "
        "dans l'image, et annoncee au demarrage." % relatif
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
