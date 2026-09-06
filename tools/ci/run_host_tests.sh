#!/usr/bin/env bash
#
# Toutes les suites de tests hote, DECOUVERTES et non enumerees.
#
# Le noyau est bare-metal : les tests Rust hote sont compiles individuellement
# avec `rustc --test`. Les outils de fiabilite Python ont leur propre decouverte
# unittest. Ajouter une suite dans l'un ou l'autre arbre suffit pour qu'elle
# devienne bloquante dans CI Fast.
set -uo pipefail
cd "$(dirname "$0")/../.."

SORTIE="${TMPDIR:-/tmp}/bouchaud-host-tests.$$"
mkdir -p "$SORTIE"
trap 'rm -rf "$SORTIE"' EXIT

if python3 tools/exec/fabrique-hello-exe.py "$SORTIE/bo-hello.exe" >/dev/null 2>&1; then
    export BO_HELLO_EXE="$SORTIE/bo-hello.exe"
fi

echecs=0
total=0
for source in $(find tools -name 'test_*.rs' | sort); do
    nom=$(basename "$source" .rs)
    total=$((total + 1))
    printf '%-38s ' "$nom"

    if ! rustc --edition 2021 --test -o "$SORTIE/$nom" "$source" \
            >"$SORTIE/$nom.compile" 2>&1; then
        echo "COMPILATION"
        sed 's/^/    /' "$SORTIE/$nom.compile" | head -12
        echecs=$((echecs + 1))
        continue
    fi

    if resultat=$("$SORTIE/$nom" --test-threads=1 2>&1); then
        echo "$(echo "$resultat" | grep -m1 'test result' || echo ok)"
    else
        echo "ECHEC"
        echo "$resultat" | grep -A8 '^failures:' | head -24 | sed 's/^/    /'
        echecs=$((echecs + 1))
    fi
done

echo
echo "suites Rust hote : $((total - echecs))/$total"

echo
echo "suites C++ hote :"

# Certaines pieces du portage Ladybird sont de l'arithmetique pure : quels
# pixels une capture doit reecrire, quel rectangle annoncer. Elles ne dependent
# ni d'AK, ni de LibGfx, ni d'une surface projetee -- et elles ne s'executent
# pourtant que dans QEMU, apres vingt minutes de construction, dans un scenario
# ou une erreur d'un pixel laisse une trainee sans faire echouer quoi que ce
# soit. Les sortir dans un en-tete sans dependance permet de les exercer ici.
COMPILATEUR=""
for candidat in g++ c++ clang++; do
    if command -v "$candidat" >/dev/null 2>&1; then
        COMPILATEUR="$candidat"
        break
    fi
done

cpp_total=0
for source in $(find tools -name 'test_*.cpp' | sort); do
    nom=$(basename "$source" .cpp)
    cpp_total=$((cpp_total + 1))
    printf '%-38s ' "$nom"

    if [ -z "$COMPILATEUR" ]; then
        echo "PAS DE COMPILATEUR C++"
        echecs=$((echecs + 1))
        continue
    fi

    if ! "$COMPILATEUR" -std=c++20 -Wall -Wextra -Werror \
            -o "$SORTIE/$nom" "$source" >"$SORTIE/$nom.compile" 2>&1; then
        echo "COMPILATION"
        sed 's/^/    /' "$SORTIE/$nom.compile" | head -12
        echecs=$((echecs + 1))
        continue
    fi

    if resultat=$("$SORTIE/$nom" 2>&1); then
        echo "$(echo "$resultat" | tail -1)"
    else
        echo "ECHEC"
        echo "$resultat" | grep -A2 'ECHEC' | head -24 | sed 's/^/    /'
        echecs=$((echecs + 1))
    fi
done

if [ "$cpp_total" -eq 0 ]; then
    echo "aucune suite C++ decouverte"
fi

echo
echo "suites Python fiabilite :"
if ! PYTHONPATH=tools/ci/reliability \
     python3 -m unittest discover -s tools/ci/reliability -p 'test_*.py' -v; then
    echecs=$((echecs + 1))
fi

[ "$echecs" -eq 0 ] || exit 1
