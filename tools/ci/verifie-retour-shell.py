#!/usr/bin/env python3
"""La racine de premier plan ne passe pas par la boucle d'attente.

BOUCHAUD_C71_LA_RACINE_NE_S_ATTEND_PAS_ELLE_MEME

INVARIANT. Dans `exit_current`, le chemin de la RACINE de premier plan
(`pid_sortant == racine`) atteint `switch_to_kernel()` SANS traverser
`commute_sortie_definitive_si_possible`.

Pourquoi cet invariant et pas un autre. La boucle d'attente de `exit_current`
tourne sur la pile d'une tache DEJA MORTE, et son corps appelle
`commute_sortie_definitive_si_possible`, qui part vers toute tache executable
et NE REVIENT PAS (`unreachable!`). Deux tests de sortie la precedent ; s'ils
sont faux ne serait-ce qu'un instant -- un descendant pas encore marque zombie
-- la pile est abandonnee, `switch_to_kernel` n'est jamais atteint, et le
`run` synchrone reste gare a vie : le shell ne revient plus.

Quand c'est la racine elle-meme qui meurt, elle n'a personne a attendre. La
faire entrer dans cette boucle etait donc un risque pur, sans contrepartie.

MESURE, banc contendu (QEMU epingle sur 2 coeurs d'une machine qui en a 4),
temoins `RETOUR_SHELL*` :

    avant   pid=23, pid=24 (commandes qui aboutissent)  SAUT puis REPRIS
            pid=25         (commande qui bloque)        SAUT ABSENT, 3/3

Sans contention le test est vrai du premier coup : c'est ce qui faisait passer
le defaut pour intermittent, et une passe verte pour une preuve.

EVOLUTION (BOUCHAUD_CONTINUATION_SYNCHRONE_V1). La boucle d'attente sur la
pile morte a disparu : `switch_to_kernel` mene desormais a l'IDLE du coeur, et
la continuation de `run` / `run_noyau` n'est reprise que par
`reprend_continuation`, quand sa racine est terminee. L'invariant est le meme,
dit avec les nouveaux noms : la voie de la continuation est decidee AVANT toute
commutation definitive, et la pile de la racine ne peut plus etre abandonnee
avant d'avoir rendu le shell.
"""
from pathlib import Path
import re
import tempfile

RACINE = Path(__file__).resolve().parents[2]
LIFECYCLE = "src/kernel/process/thread/lifecycle.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", l) for l in texte.splitlines())


def corps_exit(texte: str) -> str | None:
    debut = texte.find("pub fn exit_current(")
    if debut < 0:
        return None
    i = texte.find("{", debut)
    profondeur, j = 0, i
    while j < len(texte):
        if texte[j] == "{":
            profondeur += 1
        elif texte[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[i: j + 1]
        j += 1
    return None


def controle(racine: Path) -> list:
    erreurs = []
    c = corps_exit(sans_commentaires((racine / LIFECYCLE).read_text()))
    if c is None:
        return ["exit_current introuvable : le garde-fou ne controle plus rien"]

    voie = c.find("reprend_continuation(")
    if voie < 0:
        return ["la voie de la continuation a disparu de exit_current : la mort "
                "de la racine ne rendrait plus le shell"]

    # Decidee AVANT la premiere commutation definitive : sinon la pile de la
    # racine part vers une autre tache et la continuation reste garee.
    premiere_commutation = c.find("commute_sortie_definitive_si_possible")
    if premiere_commutation >= 0 and premiere_commutation < voie:
        erreurs.append(
            "une commutation definitive precede la voie de la continuation : "
            "la pile de la racine peut encore etre abandonnee"
        )
    if "Destination::Continuation" not in c[:voie]:
        erreurs.append("la voie de la continuation n'est plus gardee par "
                       "`continuation::destination`")

    for temoin in ("RETOUR_SHELL_SAUT", "RETOUR_SHELL_REPRIS"):
        if temoin not in (racine / LIFECYCLE).read_text():
            erreurs.append(
                f"{temoin} a disparu : sans lui, un shell perdu redevient "
                f"indiscernable d'un shell lent"
            )
    return erreurs


def test_negatif() -> list:
    """Le controle doit REFUSER les deux regressions qui reperdent le shell."""
    txt = (RACINE / LIFECYCLE).read_text()
    mutations = [
        ("voie de la continuation retiree", lambda t: t.replace(
            '        reprend_continuation("racine_terminee");', "", 1)),
        ("commutation avant la continuation", lambda t: t.replace(
            "    let garee = CONTINUATION_ETAT.load(Ordering::Acquire) == CONTINUATION_GAREE;\n    let sortie",
            "    commute_sortie_definitive_si_possible(cur, cpu_id);\n"
            "    let garee = CONTINUATION_ETAT.load(Ordering::Acquire) == CONTINUATION_GAREE;\n    let sortie", 1)),
        # La mutation ne doit pas CONTENIR la chaine cherchee, sinon le test
        # negatif passe pour vert sans rien avoir prouve.
        ("temoin de reprise retire", lambda t: t.replace(
            "RETOUR_SHELL_REPRIS", "TEMOIN_SUPPRIME", 1)),
    ]
    inertes = []
    for nom, muter in mutations:
        with tempfile.TemporaryDirectory() as tmp:
            faux = Path(tmp)
            (faux / LIFECYCLE).parent.mkdir(parents=True, exist_ok=True)
            (faux / LIFECYCLE).write_text(muter(txt))
            if not controle(faux):
                inertes.append(f"test negatif inerte : « {nom} » non detecte")
    return inertes


def main() -> int:
    erreurs = controle(RACINE) + test_negatif()
    if erreurs:
        for e in erreurs:
            print(f"RETOUR_SHELL_FAIL {e}")
        print(f"RETOUR_SHELL_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("RETOUR_SHELL_VERDICT ok voie=continuation_avant_commutation temoins=2 negatifs=3")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
