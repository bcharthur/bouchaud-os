#!/usr/bin/env python3
"""Garde-fou : le jeton BRDP ne fuit par aucun chemin, et n'entre que par un.

# Ce que ce garde-fou defend

`BOUCHAUD_DEBUG_TOKEN` est le secret partage qui authentifie le debugger
distant. Il a trois proprietes a tenir, et aucune ne se verifie a la lecture :

1. **UN SEUL CHEMIN D'ENTREE.** Le jeton est lu par `option_env!`, une fois.
   Un second point de lecture -- une variable d'execution, un fichier, une
   valeur de repli -- ferait exister deux verites, et la plus permissive
   gagnerait toujours.

2. **AUCUN CHEMIN DE SORTIE.** Le jeton ne part ni sur la serie, ni dans la
   boite noire, ni dans une reponse BRDP, ni dans la telemetrie. Le protocole
   est concu pour qu'il ne traverse JAMAIS le reseau : le client prouve qu'il
   le connait par un HMAC du nonce. Un `serial_println!` de confort
   annulerait tout ce travail d'un coup, et personne ne le verrait -- sur la
   machine de reference la serie ne produit pas un octet.

3. **RIEN DANS GIT.** Aucun litteral de jeton dans l'arbre, a l'exception du
   secret ephemere de banc, qui n'ouvre aucune machine et sert seulement a
   prouver que la presence d'un jeton change l'image.

# Ce que ce garde-fou NE pretend PAS

Le jeton est compile DANS l'image de laboratoire : `strings` sur cette image
le retrouve. C'est inherent a un secret partage lu a la construction, et
c'est assume -- une image LAB est un outil de banc, pas un artefact a
distribuer. Ce qui est defendu ici, c'est que le jeton ne sorte pas de
l'image par un canal d'execution, et qu'il n'entre jamais dans le depot.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
VARIABLE = "BOUCHAUD_DEBUG_TOKEN"

# Le secret ephemere de banc. Il est ECRIT dans le depot, volontairement : il
# ne protege rien, et le nommer ici permet de refuser tous les autres.
JETON_DE_BANC = "bouchaud-qemu-lab-test"

echecs = []


def echec(message):
    echecs.append(message)


def sources():
    """Les fichiers suivis qui peuvent porter du code ou un reglage."""
    for motif in ("*.rs", "*.py", "*.sh", "*.yml", "*.yaml", "*.toml", "*.ps1"):
        for chemin in RACINE.rglob(motif):
            texte = str(chemin.relative_to(RACINE))
            if texte.startswith(("target/", ".git/", "userland/quickjs")):
                continue
            yield chemin, texte


# ---------------------------------------------------------------------------
# 1. UN SEUL CHEMIN D'ENTREE
# ---------------------------------------------------------------------------

LECTURES = []
for chemin, relatif in sources():
    if chemin.suffix != ".rs":
        continue
    contenu = chemin.read_text(encoding="utf-8", errors="replace")
    for trouve in re.finditer(rf'option_env!\s*\(\s*"{VARIABLE}"\s*\)', contenu):
        LECTURES.append(relatif)
    # Une lecture a l'EXECUTION n'existe pas dans un noyau sans processus
    # parent, mais la refuser garde la propriete vraie si un jour il y en a un.
    # `(?<!option_)` : sans lui, cette regle accuse `option_env!`, qui est
    # precisement le chemin autorise.
    if re.search(rf'(?<!option_)\benv!\s*\(\s*"{VARIABLE}"', contenu):
        echec(f"{relatif}: `env!` rendrait la construction impossible sans jeton")
    if re.search(rf'var(_os)?\s*\(\s*"{VARIABLE}"', contenu):
        echec(f"{relatif}: lecture a l'execution du jeton -- un seul chemin, et il est `option_env!`")

if len(LECTURES) != 1:
    echec(f"le jeton doit etre lu en UN seul point, trouve {len(LECTURES)}: {LECTURES}")
elif LECTURES[0] != "src/net/diag_distant/serveur.rs":
    echec(f"le jeton est lu depuis {LECTURES[0]}, attendu src/net/diag_distant/serveur.rs")

# ---------------------------------------------------------------------------
# 2. AUCUN CHEMIN DE SORTIE
# ---------------------------------------------------------------------------

SERVEUR = RACINE / "src/net/diag_distant/serveur.rs"
if not SERVEUR.exists():
    echec("src/net/diag_distant/serveur.rs est introuvable")
else:
    contenu = SERVEUR.read_text(encoding="utf-8", errors="replace")

    # Les sorties possibles depuis ce fichier. `JETON` ne doit apparaitre que
    # dans `arme()`, dans `demarre()` et dans la verification du HMAC.
    SORTIES = (
        "serial_println", "println", "print!", "dmesg", "blackbox",
        "emets", "write!", "lab::", "journal",
    )
    for numero, ligne in enumerate(contenu.splitlines(), 1):
        nue = ligne.strip()
        if nue.startswith("//") or nue.startswith("///"):
            continue
        if "JETON" not in nue and "jeton" not in nue:
            continue
        # `jeton.is_empty()` et `let Some(jeton)` sont des tests, pas des
        # sorties ; ce qui compte est qu'aucune fonction d'ECRITURE ne recoive
        # la valeur sur la meme ligne.
        for sortie in SORTIES:
            if sortie in nue and "raison=sans-jeton" not in nue and "jeton-vide" not in nue:
                echec(
                    f"serveur.rs:{numero}: le jeton cotoie une sortie `{sortie}` -- "
                    f"il ne doit ni s'imprimer, ni s'archiver, ni partir sur le fil"
                )

    # LE JETON NE SERT QU'A TROIS CHOSES : etre declare, decider de
    # l'armement, et etre compare par `brdp::verifie`. Toute autre
    # consommation demande un examen -- c'est le seul secret du module.
    #
    # LA LIGNE ENTIERE EST EXAMINEE, pas seulement ce qui suit `JETON` : un
    # `const JETON` ou un commentaire en capitales se reconnaissent a ce qui
    # PRECEDE le mot, et une premiere redaction qui partait du mot les
    # accusait tous les trois.
    for numero, ligne in enumerate(contenu.splitlines(), 1):
        if not re.search(r"\bJETON\b", ligne):
            continue
        nue = ligne.strip()
        if nue.startswith("//"):
            continue
        autorise = (
            "const JETON" in nue                 # -> la declaration
            or "JETON.unwrap_or" in nue          # -> brdp::verifie
            or "let Some(jeton) = JETON" in nue  # -> decision d'armement
            or "JETON.map(" in nue               # -> arme()
        )
        if not autorise:
            echec(f"serveur.rs:{numero}: usage inattendu du jeton -- `{nue[:70]}`")

# Le jeton ne doit apparaitre dans AUCUNE reponse du protocole.
REPONSES = RACINE / "src/net/diag_distant/reponses.rs"
if REPONSES.exists():
    contenu = REPONSES.read_text(encoding="utf-8", errors="replace")
    if "JETON" in contenu or VARIABLE in contenu:
        echec("reponses.rs mentionne le jeton -- une reponse BRDP ne le porte jamais")

TELEMETRIE = RACINE / "src/net/diag_distant/telemetrie.rs"
if TELEMETRIE.exists():
    contenu = TELEMETRIE.read_text(encoding="utf-8", errors="replace")
    if "JETON" in contenu or VARIABLE in contenu:
        echec("telemetrie.rs mentionne le jeton -- ce canal part en clair et en diffusion")

# ---------------------------------------------------------------------------
# 3. RIEN DANS GIT
# ---------------------------------------------------------------------------

AFFECTATION = re.compile(rf'{VARIABLE}\s*[=:]\s*["\']?([A-Za-z0-9_\-.]+)')
for chemin, relatif in sources():
    if relatif == "tools/verifie-jeton-brdp.py":
        continue
    contenu = chemin.read_text(encoding="utf-8", errors="replace")
    for valeur in AFFECTATION.findall(contenu):
        # Une reference a un secret de CI (`${{ secrets.X }}`) est reduite par
        # la regex a un mot sans interet ; on ne garde que les litteraux.
        if valeur in ("", "secrets", "env"):
            continue
        if valeur == JETON_DE_BANC:
            continue
        echec(
            f"{relatif}: un litteral de jeton est ecrit dans le depot "
            f"(`{valeur}`) -- seul le secret ephemere de banc est tolere"
        )

# Un fichier d'environnement suivi porterait le jeton sans qu'on le relise.
for nom in (".env", ".env.local", "tools/remote/.env"):
    if (RACINE / nom).exists():
        echec(f"{nom} existe dans l'arbre -- un jeton ne se range pas dans un fichier suivi")

# ---------------------------------------------------------------------------

if echecs:
    print(f"[ECHEC] jeton BRDP : {len(echecs)} probleme(s)")
    for message in echecs:
        print(f"  - {message}")
    sys.exit(1)

print("[OK] jeton BRDP : une seule lecture, aucune sortie, aucun litteral dans l'arbre")
