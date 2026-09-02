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
echo "suites Python fiabilite :"
if ! PYTHONPATH=tools/ci/reliability \
     python3 -m unittest discover -s tools/ci/reliability -p 'test_*.py' -v; then
    echecs=$((echecs + 1))
fi

[ "$echecs" -eq 0 ] || exit 1
