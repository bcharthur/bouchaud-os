#!/usr/bin/env bash
#
# LA PREUVE QUE LE LIVRE DES FAUTES SE REMPLIT, ET CE QU'IL MESURE.
#
# # Pourquoi ce banc existe
#
# `src/kernel/process/fautes.rs` est verifie sur l'hote -- son arithmetique,
# sa chasse, ses categories. Rien ne prouvait que le RACCORDEMENT marche :
# qu'une faute reelle arrive bien dans le livre, avec le bon processus et la
# bonne categorie.
#
# Et ce raccordement ne se prouve pas dans un scenario QEMU ordinaire. Le
# bureau, le shell et les sondes du noyau sont des taches NOYAU : le releve
# `[SMP-PF] c0=0/0/0/0/0` montre qu'aucune faute UTILISATEUR n'a lieu. Il faut
# donc une charge d'epreuve en anneau 3, et c'est `tools/userland/toucher-pages.c`.
#
# # Ce que la mesure a donne la premiere fois, le 22 septembre 2026
#
#     PID   FAUTES     TOTAL      PIRE   DOMINANTE
#       9        1       4 ms   4066 us   fichier (4 ms)
#
# Deux faits, et le second est le plus interessant :
#
#   * le livre se remplit, avec le bon PID et la bonne categorie ;
#   * UNE seule faute pour quatre mille pages touchees. Le grand tableau BSS
#     n'est donc pas peuple a la demande : le chargeur le projette d'avance.
#     La seule faute observee est celle du fichier, et elle coute QUATRE
#     MILLISECONDES a elle seule.
#
# Ce dernier chiffre est la raison d'etre du banc. Les binaires Ladybird sont
# des static-pie de plusieurs dizaines de mebioctets ; si une faute de fichier
# coute quelques millisecondes, leur demarrage se compte en secondes, et c'est
# precisement ce que la machine physique montre.
#
#   ./tools/ci/run_fautes_demande.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${FAUTES_SORTIE:-$(mktemp -d)}
SECONDES=${FAUTES_SECONDES:-150}
mkdir -p "$SORTIE"

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    echo "  construire d'abord : tools/ci/build_kernel.sh" >&2
    exit 1
fi

# Le compilateur n'est pas une exigence du depot : sans lui, on le DIT et on
# passe. Faire echouer une barriere sur l'outillage de la machine apprend a
# l'ignorer.
CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
if [ -z "$CC" ]; then
    echo "fautes : aucun compilateur C, verification passee"
    exit 0
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"
mkdir -p "$SCENARIO"

if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/toucher" tools/userland/toucher-pages.c 2>"$SORTIE/cc.log"; then
    echo "fautes : la charge d'epreuve ne se compile pas ici, verification passee"
    sed 's/^/    /' "$SORTIE/cc.log" | head -5
    exit 0
fi

cat > "$SCENARIO/autorun" <<'AUTORUN'
echo "=== FAUTES AVANT ==="
fautes
exec /toucher
echo "=== FAUTES APRES ==="
fautes
echo "=== FAUTES FIN ==="
AUTORUN

(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" >/dev/null 2>&1

PROPRE="$SORTIE/serie.propre"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$PROPRE"

if ! grep -q "FAUTES FIN" "$PROPRE"; then
    echo "fautes : le scenario n'est pas alle au bout" >&2
    tail -20 "$PROPRE" >&2
    exit 1
fi

echo "=== ce que le livre a enregistre ==="
sed -n '/FAUTES APRES/,/FAUTES FIN/p' "$PROPRE" \
    | sed -E -e 's/^(\[[^]]*\])+ //' -e '/^\+ /d' -e '/FAUTES /d'

# LA REGLE : le livre doit avoir vu QUELQUE CHOSE.
#
# `suivis=0` apres avoir execute un programme en anneau 3 veut dire que le
# raccordement est casse -- et c'est exactement l'etat dans lequel une
# instrumentation finit quand personne ne la fait monter.
SUIVIS=$(sed -n '/FAUTES APRES/,/FAUTES FIN/p' "$PROPRE" \
    | grep -o 'suivis=[0-9]*' | tail -1 | cut -d= -f2)
if [ -z "${SUIVIS:-}" ] || [ "$SUIVIS" -lt 1 ]; then
    echo "fautes : le livre est reste vide apres une execution en anneau 3." >&2
    echo "         le raccordement de note_faute() ne fonctionne plus." >&2
    exit 1
fi

printf '\033[32m%s\033[0m\n' "fautes : le livre a suivi $SUIVIS processus ; journal dans $SORTIE"
echo "FAUTES_DEMANDE_OK"
