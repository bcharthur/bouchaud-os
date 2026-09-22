#!/usr/bin/env python3
"""L'etat d'integration de Ladybird, MESURE et non estime.

    python3 tools/ladybird/mesure-integration.py            affiche
    python3 tools/ladybird/mesure-integration.py --ecris    ecrit le document

# Deux indicateurs, et non un seul

La premiere version de ce fichier melangeait deux questions tres differentes
sous un seul pourcentage, et le resultat etait faux dans le sens habituel :
vers le haut.

Un garde-fou statique qui rend zero prouve qu'un CONTRAT est respecte -- par
exemple qu'une fonction de telechargement existe et qu'elle est appelee au bon
endroit. Il ne prouve rien du tout sur le fait qu'un telechargement marche
dans Ladybird. Compter les deux ensemble donne un chiffre qui monte quand on
ajoute des gardes, ce qui est exactement l'inverse de ce qu'on veut mesurer.

Ce fichier rend donc DEUX chiffres :

  couverture des contrats        ce que les gardes et les bancs tiennent ;
  integration fonctionnelle      ce que le navigateur SAIT FAIRE.

Pour le second, une garde statique ne donne AUCUN point. Une fonction du
navigateur n'est verte que si un comportement reel a ete execute.

# Les quatre niveaux de preuve

    STATIC_CONTRACT   un garde-fou ou un banc hote : le code respecte un
                      contrat. Ne dit RIEN du comportement de Ladybird.
    HOST_RUNTIME      du code a reellement tourne sur l'hote -- de
                      l'arithmetique pure, pas le navigateur.
    QEMU_RUNTIME      le comportement a ete observe dans QEMU, dans une
                      execution reelle.
    PHYSICAL_RUNTIME  observe sur la TRIGKEY.

Seuls `QEMU_RUNTIME` et `PHYSICAL_RUNTIME` comptent pour l'integration
fonctionnelle, et seulement s'ils portent sur le NAVIGATEUR.

# Pourquoi une observation physique ancienne ne vaut pas OK

Une fonction vue une fois sur la machine, dans une build d'il y a une semaine,
n'est pas une fonction qui marche dans la branche courante. Elle est marquee
`PHYSICAL_OLD` avec la date et le commit de sa derniere validation, et elle
compte comme A REVALIDER -- pas comme verte. C'est la seule facon d'empecher
le tableau de vieillir vers le haut.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
DOCUMENT = RACINE / "docs/ladybird/INTEGRATION_STATUS.md"

DEBUT = "<!-- MESURE:DEBUT -->"
FIN = "<!-- MESURE:FIN -->"


# CE QUE LE DERNIER RUN DE CI A REELLEMENT MONTRE.
#
# C'est la seule partie de ce fichier qui ne se mesure pas toute seule : les
# journaux vivent chez GitHub, pas dans le depot. Elle est donc RECOPIEE a la
# main depuis le rapport de jalons du run, et elle porte le numero du run pour
# qu'on puisse la verifier.
#
# Rapport du run 35742940872, workflow `ladybird-native-browser`,
# job « browser-host smoke », branche `main`, commit 3c7e726 :
#
#     == jalons apres 250s (verdict: rendu) ==
#       atteint a T+22  s  BROWSER_HOST_START
#       atteint a T+36  s  BROWSER_HOST_INITIALIZED
#       atteint a T+145 s  M11_GUI_HANDSHAKE_OK
#       atteint a T+173 s  BROWSER_HOST_M11_FRAME_PRESENTED
#       atteint a T+183 s  HOST_CANVAS OK
#       atteint a T+187 s  M11_DOCUMENT_LOADED
#       atteint a T+248 s  HOST_IMAGE OK 1x1
#       atteint a T+250 s  HOST_IFRAME OK
#       JAMAIS ATTEINT     HOST_WORKER OK pong
#                          « HOST_WORKER FAIL Error: worker timeout »
#       JAMAIS ATTEINT     HOST_SMOKE_OK canvas=1 worker=1 image=1 frame=1
#
# Le build de Ladybird lui-meme a REUSSI : l'artefact
# `bouchaud-ladybird-native-browser` (433 Mio) a ete publie. Ce qui echoue est
# le smoke test, et sur un seul point.
DERNIER_RUN_CI = "35742940872"
DATE_RUN_CI = "2026-09-22, main @ 3c7e726"
ARTEFACT_LADYBIRD_PRESENT = True

JALONS_CI = {
    "BROWSER_HOST_START": True,
    "BROWSER_HOST_INITIALIZED": True,
    "M11_GUI_HANDSHAKE_OK": True,
    "M11_DOCUMENT_LOADED": True,
    "BROWSER_HOST_M11_FRAME_PRESENTED": True,
    "HOST_CANVAS OK": True,
    "HOST_IMAGE OK": True,
    "HOST_IFRAME OK": True,
    "HOST_WORKER OK": False,
}


# LES QUATRE NIVEAUX DE PREUVE.
#
# Ils sont ORDONNES : chacun prouve strictement plus que le precedent, et
# seuls les deux derniers disent quelque chose du navigateur lui-meme.
STATIC_CONTRACT = "STATIC_CONTRACT"
HOST_RUNTIME = "HOST_RUNTIME"
QEMU_RUNTIME = "QEMU_RUNTIME"
PHYSICAL_RUNTIME = "PHYSICAL_RUNTIME"

#: Les niveaux qui comptent pour l'INTEGRATION FONCTIONNELLE du navigateur.
#:
#: Ni `STATIC_CONTRACT` ni `HOST_RUNTIME` n'y figurent, et c'est toute la
#: correction apportee a la premiere version : un garde-fou qui rend zero
#: prouve qu'une fonction existe et qu'elle est appelee au bon endroit. Il ne
#: prouve rien sur le fait que Ladybird sache s'en servir.
NIVEAUX_FONCTIONNELS = (QEMU_RUNTIME, PHYSICAL_RUNTIME)


class Item:
    """Une ligne du tableau.

    `fonctionnel` dit si cette ligne decrit une capacite DU NAVIGATEUR. Les
    lignes d'infrastructure -- topologie CPU, comptabilite des fautes, cycle
    de vie des processus -- sont vraies et utiles, mais elles n'appartiennent
    pas au denominateur de « ce que le navigateur sait faire ».
    """

    def __init__(self, nom, preuve, note="", fonctionnel=False, derniere_validation=None):
        self.nom = nom
        self.preuve = preuve
        self.note = note
        self.fonctionnel = fonctionnel
        #: (date, commit) de la derniere validation physique, quand il y en a une.
        self.derniere_validation = derniere_validation
        self.etat = "NON MESURE"
        self.niveau = None
        self.detail = ""


# L'ORDRE EST CELUI DE LA CHAINE, du materiel vers l'ecran. Une ligne rouge
# haut dans la liste explique souvent toutes celles d'en dessous.
#
# `fonctionnel=True` marque les lignes qui decrivent une capacite du
# NAVIGATEUR. Les autres sont de l'infrastructure : elles comptent dans la
# couverture des contrats, jamais dans l'integration fonctionnelle.
ITEMS = [
    # --- Infrastructure noyau et portage ---------------------------------
    Item("Reseau physique (RTL8168)", "garde:verifie-pilote-rtl8168",
         "chaine RTL8168 -> DHCP -> DNS"),
    Item("Verdict reseau", "garde:verifie-verdict-reseau"),
    Item("Supervision des processus", "hote:test_supervision",
         "courtier, rendu, reseau, decodeur, travailleur, composition"),
    Item("Roles des binaires livres", "hote:test_roles_livres",
         "securite et supervision s'accordent sur les chemins reels"),
    Item("Fautes de page par processus", "hote:test_fautes",
         "sept categories, total et pire faute"),
    Item("Fautes de page raccordees au noyau",
         "qemu:tools/ci/run_fautes_demande.sh:FAUTES_DEMANDE_OK",
         "charge d'epreuve en anneau 3, le livre se remplit"),
    Item("Topologie CPU annoncee", "hote:test_cpu_topologie"),
    Item("Topologie CPU raccordee",
         "qemu:tools/ci/run_topologie_cpu.sh:TOPOLOGIE_CPU_OK",
         "la plage suit le -smp, deux tailles de machine"),
    Item("Ce que voit l'anneau 3",
         "qemu:tools/ci/run_topologie_cpu.sh:verdict=coherent",
         "sonde ring 3 : sysfs, cpuinfo, procstat et CPUID s'accordent"),
    Item("Fenetre de pile initiale", "hote:test_pile_initiale",
         "budget derive, postcondition, gros argv/envp"),
    Item("Cout d'un exec", "qemu:tools/ci/run_cout_exec.sh:COUT_EXEC_OK",
         "54 ms -> 0,9 ms, budget 10 ms"),
    Item("Profil de demarrage", "hote:test_demarrage"),
    Item("Vue Services", "garde:verifie-fenetre-services"),
    Item("Cycle de vie des onglets", "garde:verifie-lifecycle-pages"),

    # --- Capacites du navigateur -----------------------------------------
    #
    # Celles-ci, et elles seules, comptent dans l'integration fonctionnelle.
    Item("Ladybird construit",
         "ci:ladybird-native-browser.yml:bouchaud-ladybird-native-browser",
         "artefact de 433 Mio produit par la CI", fonctionnel=True),
    Item("BrowserHost demarre", "ci-jalon:BROWSER_HOST_START", fonctionnel=True),
    Item("BrowserHost initialise", "ci-jalon:BROWSER_HOST_INITIALIZED", fonctionnel=True),
    Item("Pont GUI etabli", "ci-jalon:M11_GUI_HANDSHAKE_OK", fonctionnel=True),
    Item("Document charge", "ci-jalon:M11_DOCUMENT_LOADED", fonctionnel=True),
    Item("Trame presentee", "ci-jalon:BROWSER_HOST_M11_FRAME_PRESENTED", fonctionnel=True),
    Item("Canvas 2D", "ci-jalon:HOST_CANVAS OK", fonctionnel=True),
    Item("Image PNG decodee et affichee", "ci-jalon:HOST_IMAGE OK", fonctionnel=True),
    Item("iframe", "ci-jalon:HOST_IFRAME OK", fonctionnel=True),
    Item("JavaScript", "ci-jalon:HOST_CANVAS OK", fonctionnel=True,
         note="le canvas et le worker sont pilotes en JS : leur execution le prouve"),
    Item("WebWorker", "ci-jalon:HOST_WORKER OK", fonctionnel=True,
         note="ECHEC en CI : worker timeout apres 60 s"),
    Item("JPEG", "aucune", "aucun banc ne l'exerce", fonctionnel=True),
    Item("GIF", "aucune", "aucun banc ne l'exerce", fonctionnel=True),
    Item("WebP", "aucune", "aucun banc ne l'exerce", fonctionnel=True),
    Item("background-image CSS", "aucune", "aucun banc ne l'exerce", fonctionnel=True),
    Item("Image redimensionnee", "aucune", "aucun banc ne l'exerce", fonctionnel=True),
    Item("HTTPS / TLS", "physique:2026-09-18:bb(8)", fonctionnel=True,
         note="wikipedia.org servi en 200, 119573 octets",
         derniere_validation=("2026-09-18", "a36b3e4")),
    Item("HTTP / RequestServer", "physique:2026-09-18:bb(8)", fonctionnel=True,
         note="M9_RS_REQUEST_FINISHED id=1 taille=119573",
         derniere_validation=("2026-09-18", "a36b3e4")),
    Item("Plusieurs onglets", "physique:2026-09-19:photo", fonctionnel=True,
         note="trois onglets, le premier ferme, navigation poursuivie",
         derniere_validation=("2026-09-19", "3c7e726")),
    Item("Cookies", "aucune", "--disable-sql-database", fonctionnel=True),
    Item("Cache disque", "aucune", "--disable-http-disk-cache", fonctionnel=True),
    Item("Stockage / profil", "aucune", "pots upstream en memoire seulement", fonctionnel=True),
    Item("Isolation de site", "aucune", "--site-isolation=disable", fonctionnel=True),
    Item("Audio", "aucune", "aucun backend", fonctionnel=True),
    Item("GPU", "aucune", "--force-cpu-painting", fonctionnel=True),
    Item("Latence interactive sous charge", "aucune", "non mesuree", fonctionnel=True),
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
        return "ABSENT", STATIC_CONTRACT, "%s n'existe pas" % fichier
    if texte in chemin.read_text(encoding="utf-8", errors="replace"):
        return "CABLE", STATIC_CONTRACT, "%s present dans %s" % (texte, fichier)
    return "ECHEC", STATIC_CONTRACT, "%s absent de %s" % (texte, fichier)


def mesure_qemu(reste):
    """Un banc QEMU : le script existe et porte son marqueur de succes.

    ATTENTION A CE QUE CETTE MESURE PROUVE. Elle ne relance pas le banc -- il
    demande une image d'amorcage et plusieurs minutes. Elle verifie que le banc
    EXISTE et qu'il porte le marqueur qu'il imprime en cas de succes.

    Le niveau rendu est donc `QEMU_RUNTIME` seulement parce que ce banc a ete
    execute au moins une fois lors de son ajout, et qu'il est rejoue par la CI
    Integration. S'il cessait de l'etre, cette ligne mentirait -- et c'est la
    limite honnete de cette sorte de preuve.
    """
    script, _, marqueur = reste.partition(":")
    chemin = RACINE / script
    if not chemin.exists():
        return "ABSENT", STATIC_CONTRACT, "%s n'existe pas" % script
    if marqueur not in chemin.read_text(encoding="utf-8", errors="replace"):
        return "ECHEC", STATIC_CONTRACT, "%s ne porte plus %s" % (script, marqueur)
    return "OK", QEMU_RUNTIME, "banc %s (marqueur %s)" % (script, marqueur)


def mesure_ci_jalon(reste):
    """Un jalon que le smoke test du navigateur atteint -- ou pas.

    La source est `tools/ci/run_ladybird_browser_host.sh`, qui liste les
    jalons et echoue si l'un manque. L'etat REEL vient du dernier run de la
    CI, et il est recopie dans `JALONS_CI` ci-dessous par la personne qui lit
    ce run. C'est la seule partie du tableau qui ne se mesure pas toute seule,
    et elle porte donc la date et le run d'ou elle vient.
    """
    jalon = reste
    etat = JALONS_CI.get(jalon)
    if etat is None:
        return "NON MESURE", None, "jalon absent du dernier run lu"
    if etat:
        return "OK", QEMU_RUNTIME, "atteint au run %s" % DERNIER_RUN_CI
    return "ECHEC", QEMU_RUNTIME, "JAMAIS ATTEINT au run %s" % DERNIER_RUN_CI


def mesure_ci_artefact(reste):
    workflow, _, artefact = reste.partition(":")
    chemin = RACINE / ".github/workflows" / workflow
    if not chemin.exists():
        return "ABSENT", STATIC_CONTRACT, "%s n'existe pas" % workflow
    if artefact not in chemin.read_text(encoding="utf-8", errors="replace"):
        return "ECHEC", STATIC_CONTRACT, "%s ne publie plus %s" % (workflow, artefact)
    if ARTEFACT_LADYBIRD_PRESENT:
        return "OK", QEMU_RUNTIME, "artefact %s produit au run %s" % (artefact, DERNIER_RUN_CI)
    return "NON MESURE", None, "aucun artefact recent connu"


def mesure_physique(item, reste):
    """Une observation faite sur la TRIGKEY, a une date, sur un commit.

    ELLE NE VAUT PAS « OK ». Une fonction vue une fois dans une build d'il y a
    une semaine n'est pas une fonction qui marche dans la branche courante :
    la seule chose qu'on sache est qu'elle a marche ce jour-la. Elle est donc
    marquee `PHYSICAL_OLD` et compte comme A REVALIDER.
    """
    date, _, source = reste.partition(":")
    commit = item.derniere_validation[1] if item.derniere_validation else "?"
    return "PHYSICAL_OLD", PHYSICAL_RUNTIME, "%s (%s, commit %s)" % (source, date, commit)


def mesure(item):
    sorte, _, reste = item.preuve.partition(":")
    if sorte == "garde":
        item.etat, item.detail = mesure_garde(reste)
        item.niveau = STATIC_CONTRACT
    elif sorte == "hote":
        item.etat, item.detail = mesure_hote(reste)
        item.niveau = HOST_RUNTIME
    elif sorte == "marqueur":
        item.etat, item.niveau, item.detail = mesure_marqueur(reste)
    elif sorte == "qemu":
        item.etat, item.niveau, item.detail = mesure_qemu(reste)
    elif sorte == "ci-jalon":
        item.etat, item.niveau, item.detail = mesure_ci_jalon(reste)
    elif sorte == "ci":
        item.etat, item.niveau, item.detail = mesure_ci_artefact(reste)
    elif sorte == "physique":
        item.etat, item.niveau, item.detail = mesure_physique(item, reste)
    else:
        item.etat, item.niveau = "NON MESURE", None
        item.detail = item.note or "aucune preuve"


def bloc():
    for item in ITEMS:
        mesure(item)

    # --- INDICATEUR 1 : la couverture des contrats -----------------------
    #
    # Ce que les gardes et les bancs tiennent. Il monte quand on ajoute des
    # verifications, et c'est normal : c'est ce qu'il mesure.
    contrats = [i for i in ITEMS if i.preuve.split(":")[0] in ("garde", "hote", "marqueur", "qemu")]
    contrats_verts = [i for i in contrats if i.etat in ("OK", "CABLE")]
    part_contrats = (100 * len(contrats_verts) // len(contrats)) if contrats else 0

    # --- INDICATEUR 2 : l'integration fonctionnelle ----------------------
    #
    # Ce que le NAVIGATEUR sait faire. Une garde statique n'y donne aucun
    # point ; il faut qu'un comportement reel ait ete execute, et dans la
    # branche courante.
    fonctionnels = [i for i in ITEMS if i.fonctionnel]
    fonctionnels_verts = [
        i for i in fonctionnels
        if i.etat == "OK" and i.niveau in NIVEAUX_FONCTIONNELS
    ]
    a_revalider = [i for i in fonctionnels if i.etat == "PHYSICAL_OLD"]
    fonctionnels_rouges = [i for i in fonctionnels if i.etat == "ECHEC"]
    part_fonctionnelle = (
        (100 * len(fonctionnels_verts) // len(fonctionnels)) if fonctionnels else 0
    )

    lignes = [DEBUT, ""]
    lignes.append("    couverture des contrats     %d/%d  (%d %%)"
                  % (len(contrats_verts), len(contrats), part_contrats))
    lignes.append("    integration fonctionnelle   %d/%d  (%d %%)"
                  % (len(fonctionnels_verts), len(fonctionnels), part_fonctionnelle))
    lignes.append("")
    lignes.append("    dont a revalider physiquement   %d" % len(a_revalider))
    lignes.append("    dont en ECHEC                   %d" % len(fonctionnels_rouges))
    lignes.append("")
    lignes.append("    dernier run CI lu : %s  (%s)" % (DERNIER_RUN_CI, DATE_RUN_CI))
    lignes.append("")
    lignes.append("| Element | Etat | Niveau | Preuve | Ce qu'elle dit |")
    lignes.append("|---|---|---|---|---|")
    for item in ITEMS:
        preuve = item.preuve if item.preuve != "aucune" else "—"
        detail = item.detail if item.etat != "NON MESURE" else (item.note or "—")
        portee = "navigateur" if item.fonctionnel else "infrastructure"
        lignes.append("| %s%s | **%s** | `%s` | `%s` | %s |" % (
            item.nom,
            "" if item.fonctionnel else " *(infra)*",
            item.etat,
            item.niveau or "—",
            preuve,
            detail,
        ))
        del portee
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

## Deux chiffres, et non un seul

Une premiere version melangeait deux questions tres differentes sous un seul
pourcentage, et le resultat etait faux dans le sens habituel : vers le haut.

Un garde-fou statique qui rend zero prouve qu'un **contrat** est respecte --
qu'une fonction existe, qu'elle est appelee au bon endroit. Il ne prouve rien
du tout sur le fait que Ladybird sache s'en servir. Les compter ensemble
donne un chiffre qui monte quand on ajoute des gardes, c'est-a-dire l'inverse
de ce qu'on veut mesurer.

| Indicateur | Ce qu'il compte |
|---|---|
| **couverture des contrats** | ce que les gardes et les bancs tiennent |
| **integration fonctionnelle** | ce que le NAVIGATEUR sait faire |

Pour le second, une garde statique ne donne **aucun** point.

## Les quatre niveaux de preuve

| Niveau | Ce qu'il prouve |
|---|---|
| `STATIC_CONTRACT` | le code respecte un contrat. Rien sur le comportement |
| `HOST_RUNTIME` | du code a tourne sur l'hote -- pas le navigateur |
| `QEMU_RUNTIME` | le comportement a ete observe dans une execution reelle |
| `PHYSICAL_RUNTIME` | observe sur la TRIGKEY |

Seuls les deux derniers comptent pour l'integration fonctionnelle.

## Les etats

| Etat | Ce qu'il veut dire |
|---|---|
| `OK` | la preuve a ete faite, au niveau indique |
| `CABLE` | le chemin existe dans la chaine de portage -- **pas** qu'il marche |
| `PHYSICAL_OLD` | vu sur la machine a la date indiquee. **A REVALIDER** : une |
| | build d'il y a une semaine ne dit rien de la branche courante |
| `NON MESURE` | aucune preuve. Pas « ca ne marche pas » : « on ne sait pas » |
| `ECHEC` | la preuve a ete rejouee et elle est rouge |

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
