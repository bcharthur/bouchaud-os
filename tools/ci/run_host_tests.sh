#!/usr/bin/env bash
#
# Toutes les suites de tests hote, DECOUVERTES et non enumerees.
#
# # Pourquoi ces suites ne passent pas par `cargo test`
#
# La cible du noyau est bare-metal : `cargo test` y produit un second `core`
# et echoue sur un lang item duplique. Chaque suite est donc un binaire
# autonome, compile par `rustc --test`, qui `#[path]`-inclut le module de
# production qu'il verifie. Le test s'execute ainsi contre le VRAI code, pas
# contre une copie -- ce qui est tout l'interet, une copie divergeant en
# silence.
#
# # Pourquoi la decouverte
#
# La liste vit deja dans `tools/dev/validate-fast.ps1`, pour la machine
# d'Arthur. La dupliquer dans un workflow, c'est garantir qu'elles divergeront :
# une suite ajoutee d'un cote ne serait jamais lancee de l'autre, et une suite
# qui n'est jamais lancee ne protege rien.

set -uo pipefail
cd "$(dirname "$0")/../.."

SORTIE="${TMPDIR:-/tmp}/bouchaud-host-tests.$$"
mkdir -p "$SORTIE"
trap 'rm -rf "$SORTIE"' EXIT

# Deux suites lisent un executable PE32+ de reference. Le fabriquer ici plutot
# que de les ignorer : elles verifient le chargeur, qui est du code de
# production.
if python3 tools/exec/fabrique-hello-exe.py "$SORTIE/bo-hello.exe" >/dev/null 2>&1; then
    export BO_HELLO_EXE="$SORTIE/bo-hello.exe"
fi

echecs=0
total=0
for source in $(find tools -name 'test_*.rs' | sort); do
    nom=$(basename "$source" .rs)
    total=$((total + 1))
    printf '%-34s ' "$nom"

    if ! rustc --edition 2021 --test -o "$SORTIE/$nom" "$source" \
            >"$SORTIE/$nom.compile" 2>&1; then
        echo "COMPILATION"
        sed 's/^/    /' "$SORTIE/$nom.compile" | head -12
        echecs=$((echecs + 1))
        continue
    fi

    # `--test-threads=1` pour tout le monde : plusieurs suites manipulent un
    # etat global de production (compteurs, registres), et les laisser courir
    # en parallele fabriquerait des echecs qui ne disent rien du code.
    if resultat=$("$SORTIE/$nom" --test-threads=1 2>&1); then
        echo "$(echo "$resultat" | grep -m1 'test result' || echo ok)"
    else
        echo "ECHEC"
        echo "$resultat" | grep -A6 '^failures:' | head -20 | sed 's/^/    /'
        echecs=$((echecs + 1))
    fi
done

echo
echo "suites hote : $((total - echecs))/$total"
[ "$echecs" -eq 0 ] || exit 1
