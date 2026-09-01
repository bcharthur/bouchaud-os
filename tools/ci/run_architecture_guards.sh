#!/usr/bin/env bash
#
# Tous les garde-fous d'architecture, DECOUVERTS et non enumeres.
#
# # Pourquoi la decouverte, et pas une liste
#
# Les garde-fous de ce depot encodent des invariants qu'aucun test ne peut
# retrouver apres coup : l'ordre des acces qui ferme un reveil perdu, le fait
# qu'une empreinte de disque ne soit adoptee qu'apres une ecriture complete,
# l'attribution de chaque prise du gros verrou. Ils sont ecrits au moment ou
# l'on comprend le defaut -- et c'est le seul moment ou on le comprend.
#
# Une liste codee en dur dans un workflow ne suit pas ce rythme. Le depot en a
# fait l'experience : cinq garde-fous ont cesse de fonctionner lorsque `bkl.rs`,
# `thread.rs` et `persistance.rs` ont ete fragmentes en arbres. Trois tombaient
# sur une exception Python, deux accusaient a tort. Aucun ne protegeait plus
# rien, et personne ne l'a vu -- parce que rien ne les executait.
#
# La decouverte retire cette facon de casser : un garde-fou ajoute est
# immediatement execute, et un garde-fou casse est immediatement rouge.

set -uo pipefail
cd "$(dirname "$0")/../.."

# `verifie-scripts-windows.py` a son propre travail dans la barriere : il ne
# controle que les `.ps1` MODIFIES par la PR, la dette historique du depot
# n'etant pas bloquante. L'executer ici sur tout l'arbre le rendrait rouge en
# permanence, ce qui reviendrait a l'eteindre.
EXCLUS="tools/verifie-scripts-windows.py"

echecs=0
total=0
for garde in $(find tools -name 'verifie-*.py' -o -name 'test_atlas.py' | sort); do
    case " $EXCLUS " in *" $garde "*) continue ;; esac
    total=$((total + 1))
    printf '%-52s ' "$garde"
    if sortie=$(python3 "$garde" 2>&1); then
        echo "ok"
    else
        echo "ECHEC"
        echo "$sortie" | sed 's/^/    /'
        echecs=$((echecs + 1))
    fi
done

echo
echo "garde-fous d'architecture : $((total - echecs))/$total"
[ "$echecs" -eq 0 ] || exit 1
