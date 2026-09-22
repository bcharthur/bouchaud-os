#!/usr/bin/env python3
"""L'etat d'integration de Ladybird, MESURE et non estime.

    python3 tools/ladybird/mesure-integration.py            affiche
    python3 tools/ladybird/mesure-integration.py --ecris    ecrit le document

# Pourquoi ce fichier plutot qu'un document tenu a la main

Un tableau d'avancement ecrit a la main vieillit mal et se trompe toujours
dans le meme sens : vers le haut. « Le binaire demarre » devient « le
navigateur marche », puis « 95 % ». Personne ne ment ; c'est la forme du
document qui invite a cela, parce que rien ne coute de laisser une ligne
verte apres qu'elle a cesse d'etre vraie.

Chaque ligne du document produit ici porte donc une PREUVE, et la preuve est
rejouee a chaque mesure. Quatre sortes seulement :

  hote:<suite>        un banc d'essai hote, compile et execute maintenant ;
  garde:<script>      un garde-fou d'architecture, execute maintenant ;
  marqueur:<f>:<t>    un texte present dans la chaine de portage -- il prouve
                      qu'une fonction est CABLEE, pas qu'elle marche ;
  physique:<archive>  une observation faite sur la TRIGKEY, avec la date et
                      l'archive de boite noire qui la porte.

Une ligne sans preuve vaut NON MESURE. Jamais « OK », jamais zero pour cent,
jamais une estimation.

# Ce que le pourcentage compte, et ce qu'il ne compte pas

Il compte les lignes prouvees PAR UNE PREUVE REJOUABLE -- hote, garde,
marqueur. Les observations physiques sont comptees a part : elles sont la
seule preuve qui vaille pour le materiel, et elles sont aussi celles qui
vieillissent sans prevenir, puisque rien ne les rejoue.

Deux chiffres, donc, et jamais un seul : ce qui est verifie en continu, et ce
qui a ete vu une fois sur la machine.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
DOCUMENT = RACINE / "docs/ladybird/INTEGRATION_STATUS.md"

DEBUT = "<!-- MESURE:DEBUT -->"
FIN = "<!-- MESURE:FIN -->"


class Item:
    def __init__(self, nom, preuve, note=""):
        self.nom = nom
        self.preuve = preuve
        self.note = note
        self.etat = "NON MESURE"
        self.detail = ""


# L'ORDRE EST CELUI DE LA CHAINE, du materiel vers l'ecran. Une ligne rouge
# haut dans la liste explique souvent toutes celles d'en dessous.
ITEMS = [
    Item("Reseau physique (RTL8168)", "garde:verifie-pilote-rtl8168",
         "chaine RTL8168 -> DHCP -> DNS validee physiquement"),
    Item("DNS", "garde:verifie-lien-reseau",
         "echelle de onze barreaux, bb(7) et bb(8)"),
    Item("Verdict reseau", "garde:verifie-verdict-reseau"),
    Item("Supervision des processus", "hote:test_supervision",
         "courtier, rendu, reseau, decodeur, travailleur, composition"),
    Item("Roles des binaires livres", "hote:test_roles_livres",
         "securite et supervision s'accordent sur les chemins reels"),
    Item("Fautes de page par processus", "hote:test_fautes",
         "sept categories, total et pire faute"),
    Item("Fautes de page raccordees au noyau",
         "marqueur:tools/ci/run_fautes_demande.sh:FAUTES_DEMANDE_OK",
         "banc QEMU : charge d'epreuve en anneau 3, le livre se remplit"),
    Item("Cycle de vie des onglets", "garde:verifie-lifecycle-pages"),
    Item("Enregistrement des pages chez l'hote",
         "marqueur:tools/ladybird/prepare-m11-page-registry.py:M11_TAB_STAGE 80 READY"),
    Item("Vue Services", "garde:verifie-fenetre-services"),
    Item("Onglets du chrome", "garde:verifie-onglets"),
    Item("Clavier du navigateur", "garde:verifie-clavier-navigateur"),
    Item("Repeinture partielle", "garde:verifie-repeinture-partielle"),
    Item("Polices", "garde:verifie-polices-navigateur"),
    Item("Telechargements", "garde:verifie-telechargements"),
    Item("Compositor supervise",
         "marqueur:src/kernel/navigateur/supervision_corps.rs:Role::Composition"),
    Item("ImageDecoder cable",
         "marqueur:tools/ladybird/prepare-image-decoder.py:ImageDecoder"),
    Item("WebWorker cable",
         "marqueur:tools/ladybird/prepare-full-browser-host.py:WebWorker"),
    Item("HTTPS / TLS", "physique:bb(8) 2026-09-18",
         "wikipedia.org servi en 200, 119573 octets"),
    Item("HTTP / RequestServer", "physique:bb(8) 2026-09-18",
         "M9_RS_REQUEST_FINISHED id=1 taille=119573"),
    Item("Document charge", "physique:bb(8) 2026-09-18",
         "M11_DOCUMENT_LOADED page=2"),
    Item("Plusieurs onglets", "physique:2026-09-19",
         "trois onglets ouverts, le premier ferme, navigation poursuivie"),
    Item("Images affichees", "aucune",
         "telechargees mais non affichees ; chaine de decodage non instrumentee"),
    Item("JavaScript", "aucune", "aucun banc ne l'exerce"),
    Item("WebWorker a l'oeuvre", "aucune", "packagé, jamais observe en service"),
    Item("Cookies", "aucune", "--disable-sql-database"),
    Item("Cache disque", "aucune", "--disable-http-disk-cache"),
    Item("Stockage / profil", "aucune", "pots upstream en memoire seulement"),
    Item("Isolation de site", "aucune", "--site-isolation=disable"),
    Item("Audio", "aucune", "aucun backend"),
    Item("GPU", "aucune", "--force-cpu-painting"),
    Item("Temps de demarrage", "aucune", "non profile"),
    Item("Latence interactive sous charge", "aucune", "non mesuree"),
]


def execute(commande, timeout=300):
    try:
        acheve = subprocess.run(
            commande, cwd=RACINE, capture_output=True, text=True, timeout=timeout
        )
        return acheve.returncode, (acheve.stdout + acheve.stderr)
    except Exception as exc:
        return 127, str(exc)


def mesure_garde(nom):
    chemin = RACINE / "tools" / (nom + ".py")
    if not chemin.exists():
        return "ABSENT", "%s n'existe pas" % chemin.relative_to(RACINE).as_posix()
    code, sortie = execute([sys.executable, str(chemin)])
    if code == 0:
        return "OK", "garde verte"
    return "ECHEC", sortie.strip().splitlines()[0] if sortie.strip() else "code %d" % code


def mesure_hote(nom):
    sources = list(RACINE.glob("tools/**/%s.rs" % nom))
    if not sources:
        return "ABSENT", "aucun tools/**/%s.rs" % nom
    binaire = RACINE / "target" / ("mesure-integration-" + nom)
    binaire.parent.mkdir(parents=True, exist_ok=True)
    code, sortie = execute(
        ["rustc", "--edition", "2021", "--test", "-o", str(binaire), str(sources[0])],
        timeout=600,
    )
    if code != 0:
        return "ECHEC", "ne compile pas"
    code, sortie = execute([str(binaire), "--test-threads=1"], timeout=600)
    resume = ""
    for ligne in sortie.splitlines():
        if ligne.startswith("test result:"):
            resume = ligne.strip()
    if code == 0:
        return "OK", resume or "banc vert"
    return "ECHEC", resume or "banc rouge"


def mesure_marqueur(reste):
    fichier, _, texte = reste.partition(":")
    chemin = RACINE / fichier
    if not chemin.exists():
        return "ABSENT", "%s n'existe pas" % fichier
    if texte in chemin.read_text(encoding="utf-8", errors="replace"):
        # CABLE, et pas « marche ». La nuance est le seul interet de cette
        # sorte de preuve : elle dit qu'un chemin existe dans la chaine de
        # portage, et rien de ce qu'il fait a l'execution.
        return "CABLE", "%s present dans %s" % (texte, fichier)
    return "ECHEC", "%s absent de %s" % (texte, fichier)


def mesure(item):
    sorte, _, reste = item.preuve.partition(":")
    if sorte == "garde":
        item.etat, item.detail = mesure_garde(reste)
    elif sorte == "hote":
        item.etat, item.detail = mesure_hote(reste)
    elif sorte == "marqueur":
        item.etat, item.detail = mesure_marqueur(reste)
    elif sorte == "physique":
        # Rejouee par personne. Elle vaut pour la date qu'elle porte et pour
        # rien d'autre, et c'est pourquoi elle est comptee a part.
        item.etat, item.detail = "PHYSIQUE", reste
    else:
        item.etat, item.detail = "NON MESURE", item.note or "aucune preuve"


def bloc():
    for item in ITEMS:
        mesure(item)

    # `CABLE` NE COMPTE PAS COMME VERT, et c'est la regle la plus importante
    # de ce compteur.
    #
    # La premiere version l'y mettait, et rendait 94 % -- le chiffre exact que
    # ce fichier existe pour eviter. « Le chemin est present dans la chaine de
    # portage » et « la fonction marche » sont deux affirmations tres
    # differentes : ImageDecoder est cable, et les images ne s'affichent pas.
    # Une preuve par marqueur a sa valeur, elle ne vaut simplement pas une
    # preuve d'execution.
    execute = [i for i in ITEMS if i.preuve.split(":")[0] in ("garde", "hote")]
    verts = [i for i in execute if i.etat == "OK"]
    cables = [i for i in ITEMS if i.etat == "CABLE"]
    physiques = [i for i in ITEMS if i.etat == "PHYSIQUE"]
    non_mesures = [i for i in ITEMS if i.etat == "NON MESURE"]
    rouges = [i for i in ITEMS if i.etat in ("ECHEC", "ABSENT")]

    part = (100 * len(verts) // len(ITEMS)) if ITEMS else 0

    lignes = [DEBUT, ""]
    lignes.append("    prouve par execution  %d/%d elements  (%d %%)"
                  % (len(verts), len(ITEMS), part))
    lignes.append("    cable, non prouve     %d ligne(s)" % len(cables))
    lignes.append("    vu sur la machine     %d ligne(s), non rejouees" % len(physiques))
    lignes.append("    non mesure            %d ligne(s)" % len(non_mesures))
    lignes.append("    en echec              %d ligne(s)" % len(rouges))
    lignes.append("")
    lignes.append("| Element | Etat | Preuve | Ce qu'elle dit |")
    lignes.append("|---|---|---|---|")
    for item in ITEMS:
        preuve = item.preuve if item.preuve != "aucune" else "—"
        detail = item.detail if item.etat != "NON MESURE" else (item.note or "—")
        lignes.append("| %s | **%s** | `%s` | %s |" % (item.nom, item.etat, preuve, detail))
    lignes.append("")
    lignes.append(FIN)
    return "\n".join(lignes)


ENTETE = """# Etat d'integration de Ladybird

<!--
  CE FICHIER EST GENERE. Ne pas le modifier a la main entre les balises
  MESURE:DEBUT et MESURE:FIN : `tools/verifie-integration-ladybird.py` les
  compare a une mesure fraiche et echoue si elles different.

      python3 tools/ladybird/mesure-integration.py --ecris

  Le tableau et les items vivent dans `tools/ladybird/mesure-integration.py`.
-->

## Comment lire ce tableau

Chaque ligne porte une preuve, et la preuve est rejouee a chaque mesure.

| Etat | Ce qu'il veut dire |
|---|---|
| `OK` | un banc ou un garde-fou vert, execute a l'instant |
| `CABLE` | le chemin existe dans la chaine de portage -- **pas** qu'il marche |
| `PHYSIQUE` | vu une fois sur la TRIGKEY, a la date indiquee, rejoue par personne |
| `NON MESURE` | aucune preuve. Pas « ca ne marche pas » : « on ne sait pas » |
| `ECHEC` | la preuve a ete rejouee et elle est rouge |

Les deux premiers chiffres ne se melangent pas, et c'est deliberé. Ce qui est
verifie en continu ne peut pas regresser sans qu'on le sache ; ce qui a ete vu
une fois sur la machine peut avoir cesse d'etre vrai depuis, et aucune CI ne le
dira. Les confondre donnerait le « 95 % » qui ne veut rien dire.

`NON MESURE` est la colonne la plus utile du tableau : c'est la liste de ce
qu'il reste a instrumenter.

## Mesure

"""


def main():
    contenu = ENTETE + bloc() + "\n"
    if "--ecris" in sys.argv:
        DOCUMENT.parent.mkdir(parents=True, exist_ok=True)
        DOCUMENT.write_text(contenu, encoding="utf-8")
        print("ecrit :", DOCUMENT.relative_to(RACINE).as_posix())
        return 0
    print(contenu)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
