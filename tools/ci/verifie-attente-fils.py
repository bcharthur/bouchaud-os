#!/usr/bin/env python3
"""`wait4` se declare en attente AVANT de reverifier, et pas apres.

BOUCHAUD_C70_SE_DECLARER_EN_ATTENTE_AVANT_DE_REVERIFIER

Le reveilleur, `notify_parent_of_exit`, exige DEUX conditions :

    waiting_for_child.compare_exchange(true, false).is_ok()
 && state.echange(Blocked, Ready)

Trois facons de perdre le reveil, et ce fichier les refuse toutes :

  1. chercher les zombies, ne rien trouver, PUIS se declarer en attente.
     Un fils qui meurt dans l'intervalle trouve le drapeau a faux : son
     reveil tombe dans le vide. C'est le blocage de `qemu / os primitives`
     apres `SESSION_PERE_SORT fils=4` -- machine vivante, shell jamais
     repris, `pretes=2` avec `au_repos=4`, dix minutes jusqu'a l'echeance.

  2. poser le drapeau AVANT l'etat. Le reveilleur gagne alors le
     `compare_exchange`, puis echoue sur `echange(Blocked, Ready)` parce que
     la tache est encore `Ready` -- et le drapeau est consomme.

  3. ne pas reverifier du tout apres s'etre declare. Un reveil emis avant la
     declaration n'atteint personne, mais le zombie qu'il annonce reste
     visible : c'est la seule chose qui rattrape ce cas.

La course est ANTERIEURE a C70 ; les sondes de sortie de processus n'ont fait
que deplacer le timing dedans. Elle etait donc atteignable avant, et le
restera si cet ordre se defait.
"""
from pathlib import Path
import re
import tempfile

RACINE = Path(__file__).resolve().parents[2]
PROC = "src/compat/linux/proc.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", l) for l in texte.splitlines())


def corps_wait4(texte: str) -> str | None:
    debut = texte.find("pub fn sys_wait4(")
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
    c = corps_wait4(sans_commentaires((racine / PROC).read_text()))
    if c is None:
        return ["sys_wait4 introuvable dans " + PROC]

    etat = c.find("state.range(task::TaskState::Blocked)")
    drapeau = c.find("waiting_for_child.range(true)")
    if etat < 0 or drapeau < 0:
        return ["sys_wait4 ne se declare plus en attente : motif introuvable"]

    if etat > drapeau:
        erreurs.append(
            "le drapeau waiting_for_child est pose AVANT l'etat Blocked : le "
            "reveilleur consomme le drapeau puis echoue sur l'etat"
        )

    # Une reverification doit suivre la declaration et preceder schedule().
    sched = c.find("task::schedule()", drapeau)
    fenetre = c[drapeau: sched if sched > 0 else len(c)]
    if "zombie_children" not in fenetre:
        erreurs.append(
            "aucune reverification des zombies entre la declaration d'attente "
            "et schedule() : un reveil emis avant la declaration est perdu"
        )
    elif "continue" not in fenetre:
        erreurs.append(
            "la reverification ne reprend pas la boucle : elle ne rattrape "
            "donc pas le fils deja mort"
        )
    return erreurs


def test_negatif() -> list:
    """Le controle doit REFUSER les trois regressions."""
    txt = (RACINE / PROC).read_text()
    bloc = """        {
            let task = task::current();
            task.state.range(task::TaskState::Blocked);
            task.waiting_for_child.range(true);
        }
        if !task::zombie_children(parent_pid).is_empty() {
            let task = task::current();
            task.waiting_for_child.range(false);
            task.state.range(task::TaskState::Ready);
            continue;
        }
"""
    if bloc not in txt:
        return ["test negatif impossible : le bloc d'attente a change de forme"]
    mutations = [
        ("ordre inverse", bloc.replace(
            "            task.state.range(task::TaskState::Blocked);\n"
            "            task.waiting_for_child.range(true);\n",
            "            task.waiting_for_child.range(true);\n"
            "            task.state.range(task::TaskState::Blocked);\n")),
        ("reverification retiree", """        {
            let task = task::current();
            task.state.range(task::TaskState::Blocked);
            task.waiting_for_child.range(true);
        }
"""),
    ]
    inertes = []
    for nom, remplacement in mutations:
        with tempfile.TemporaryDirectory() as tmp:
            faux = Path(tmp)
            (faux / PROC).parent.mkdir(parents=True, exist_ok=True)
            (faux / PROC).write_text(txt.replace(bloc, remplacement, 1))
            if not controle(faux):
                inertes.append(f"test negatif inerte : « {nom} » non detecte")
    return inertes


def main() -> int:
    erreurs = controle(RACINE) + test_negatif()
    if erreurs:
        for e in erreurs:
            print(f"ATTENTE_FILS_FAIL {e}")
        print(f"ATTENTE_FILS_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print("ATTENTE_FILS_VERDICT ok ordre=etat_puis_drapeau reverif=posee negatifs=2")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
