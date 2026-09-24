#!/usr/bin/env python3
"""Le gestionnaire de faute doit compter son temps du cote NOYAU.

BOUCHAUD_C66_LE_TEMPS_DE_FAUTE_ETAIT_COMPTE_EN_UTILISATEUR

`account_kernel_enter`/`account_kernel_exit` n'etaient appelees que depuis
`usermode.rs`, autour du corps d'un appel systeme. Le gestionnaire de faute de
page n'en contenait aucune : tout ce qu'il fait -- attendre le cache de pages,
DECLENCHER ET ATTENDRE UNE LECTURE ATA, recopier la page, poser la traduction
-- tombait dans `user_ns`.

Ce que cela a coute : le releve `user_ms=92250 sys_ms=671` du run 35912027322
a ete lu comme « ce segment est limite par le CPU en espace utilisateur, donc
ce n'est pas de l'attente disque », et cette lecture a oriente toute une
campagne vers les relocations de demarrage. Elle ne tenait pas : `sys_ms` ne
mesurait que le corps des appels systeme.

Mesure de la correction, banc `run_faute_fichier.sh`, trois executions par
variante, compteurs REPLIES (`CUMUL_*`, sans tranche en vol) :

    variante     user_ms (min/med/max)    noyau_ms (min/med/max)
    avant        1319 / 1320 / 1322          13 /   14 /   14
    apres          56 /   61 /   74        1174 / 1204 / 1278

Le total est conserve : le temps a change d'etiquette, il n'a pas change de
valeur, et rien n'est devenu plus rapide.

Ce fichier empeche la regression de revenir en silence.
"""
from pathlib import Path
import re
import subprocess
import sys
import tempfile

RACINE = Path(__file__).resolve().parents[2]
EXCEPTIONS = "src/arch/x86_64/idt/exceptions.rs"
COMPTA = "src/kernel/process/thread/comptabilite.rs"


def sans_commentaires(texte: str) -> str:
    """Retire les commentaires de ligne.

    Sans cela, une explication qui NOMME `account_fault_enter` compterait
    comme un appel -- et le test negatif deviendrait inerte, parce que
    supprimer le vrai appel laisserait le commentaire derriere lui.
    """
    return "\n".join(re.sub(r"//.*", "", l) for l in texte.splitlines())


def corps_du_gestionnaire(texte: str) -> str | None:
    """Le corps de `page_fault_handler`, du prototype a son accolade finale."""
    debut = texte.find('extern "x86-interrupt" fn page_fault_handler(')
    if debut < 0:
        return None
    fin = texte.find("\n}\n", debut)
    if fin < 0:
        return None
    return sans_commentaires(texte[debut:fin])


def controle(racine: Path) -> list:
    erreurs = []

    exc = (racine / EXCEPTIONS).read_text()
    corps = corps_du_gestionnaire(exc)
    if corps is None:
        return ["page_fault_handler introuvable dans " + EXCEPTIONS]

    entrees = corps.count("account_fault_enter")
    if entrees != 1:
        erreurs.append(
            f"page_fault_handler appelle account_fault_enter {entrees} fois, attendu 1"
        )

    # Chaque `return;` qui rend la main doit avoir rendu le mur d'abord. Les
    # sorties qui ne rendent PAS la main (kill, panic) n'en ont pas besoin :
    # `install()` reecrit COMPTA_EN_NOYAU a la commutation suivante.
    lignes = corps.splitlines()
    for i, ligne in enumerate(lignes):
        if ligne.strip() != "return;":
            continue
        fenetre = "\n".join(lignes[max(0, i - 6): i])
        if "account_fault_exit" not in fenetre:
            erreurs.append(
                f"un `return;` du gestionnaire (ligne relative {i}) ne rend pas "
                "le mur user/noyau : account_fault_exit manque avant lui"
            )

    cmp_txt = (racine / COMPTA).read_text()

    # La sortie doit RESTAURER. Un `frontiere_compta(false)` en dur ferait
    # basculer le CPU du cote utilisateur pour la suite de l'appel systeme
    # englobant, quand la faute a ete prise en noyau.
    m = re.search(
        r"pub fn account_fault_exit\(([^)]*)\)\s*\{(.*?)\n\}", cmp_txt, re.S
    )
    if not m:
        erreurs.append("account_fault_exit introuvable dans " + COMPTA)
    else:
        arg, corps_exit = m.group(1), m.group(2)
        nom = arg.split(":")[0].strip()
        if nom.startswith("_") or f"{nom}.0" not in corps_exit:
            erreurs.append(
                "account_fault_exit n'utilise pas l'etat sauvegarde : il ecrase "
                "le mur au lieu de le restaurer"
            )

    m = re.search(r"pub fn account_fault_enter\(\)[^{]*\{(.*?)\n\}", cmp_txt, re.S)
    if not m:
        erreurs.append("account_fault_enter introuvable dans " + COMPTA)
    elif "frontiere_compta(true)" not in m.group(1):
        erreurs.append(
            "account_fault_enter ne pose pas la frontiere : le temps de faute "
            "retomberait dans user_ns"
        )

    if not re.search(r"fn frontiere_compta\(vers_noyau: bool\) -> bool", cmp_txt):
        erreurs.append(
            "frontiere_compta ne rend plus le cote precedent : la restauration "
            "ne peut plus etre exacte"
        )

    return erreurs


def test_negatif() -> list:
    """Le controle doit REFUSER chacune des trois regressions connues.

    Sans cela, ce fichier pourrait etre vert parce qu'il ne verifie rien.
    """
    exc = (RACINE / EXCEPTIONS).read_text()
    cmp_txt = (RACINE / COMPTA).read_text()

    mutations = [
        ("frontiere retiree du gestionnaire", EXCEPTIONS,
         lambda t: t.replace(
             "let mur_avant_faute = crate::kernel::task::account_fault_enter();", "", 1)),
        ("sortie retiree avant le return", EXCEPTIONS,
         lambda t: t.replace(
             "crate::kernel::task::account_fault_exit(mur_avant_faute);", "", 1)),
        ("sortie qui ecrase au lieu de restaurer", COMPTA,
         lambda t: t.replace(
             "pub fn account_fault_exit(avant: MurAvantFaute) {\n    frontiere_compta(avant.0);",
             "pub fn account_fault_exit(_avant: MurAvantFaute) {\n    frontiere_compta(false);", 1)),
    ]

    inertes = []
    for nom, cible, muter in mutations:
        with tempfile.TemporaryDirectory() as tmp:
            faux = Path(tmp)
            for rel, contenu in ((EXCEPTIONS, exc), (COMPTA, cmp_txt)):
                (faux / rel).parent.mkdir(parents=True, exist_ok=True)
                (faux / rel).write_text(muter(contenu) if rel == cible else contenu)
            if not controle(faux):
                inertes.append(f"test negatif inerte : « {nom} » n'a pas ete detecte")
    return inertes


def main() -> int:
    erreurs = controle(RACINE) + test_negatif()
    if erreurs:
        for e in erreurs:
            print(f"COMPTA_FAUTE_FAIL {e}")
        print(f"COMPTA_FAUTE_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("COMPTA_FAUTE_VERDICT ok frontiere=posee restauration=exacte negatifs=3")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
