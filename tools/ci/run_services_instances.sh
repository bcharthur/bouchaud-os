#!/usr/bin/env bash
#
# L'ARBRE DES PROCESSUS DU NAVIGATEUR : UNE LIGNE PAR PID, PAS UN TOTAL.
#
# # La question a laquelle ce banc repond
#
# « Quel WebContent est en train de tuer les performances ? »
#
# Un total par role n'y repond pas. Trois WebContent dont un sature un coeur
# et deux dorment donnent EXACTEMENT le meme cumul que trois qui travaillent
# au tiers -- et le remede n'est pas le meme. Il faut une ligne par PID.
#
# # Ce que ce banc verifie, sans Ladybird
#
# L'artefact Ladybird demande deux heures et demie de construction. Pour
# eprouver la PUBLICATION il suffit de processus qui portent le bon nom :
# `tools/userland/faux-navigateur.c` en fournit quatre par binaire.
#
#   * les instances apparaissent, une par PID, sous le noeud de leur role ;
#   * elles sont DISTINCTES -- quatre PID, quatre lignes ;
#   * elles passent a l'arret quand le processus meurt, au lieu de rester
#     figees sur leur derniere mesure.
#
# Ce qui n'est PAS couvert ici : les valeurs mesurees d'un vrai processus
# Ladybird. Elles le seront par le banc navigateur, sur l'artefact.
#
# # Mesure du 22 septembre 2026
#
#     web_content              Arrete
#       9                        Arrete
#       10                       Arrete
#       11                       Arrete
#       12                       Arrete
#
#   ./tools/ci/run_services_instances.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${INSTANCES_SORTIE:-$(mktemp -d)}
SECONDES=${INSTANCES_SECONDES:-260}
ATTENDUES=${INSTANCES_ATTENDUES:-3}
mkdir -p "$SORTIE"

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    echo "  construire d'abord : tools/ci/build_kernel.sh" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
if [ -z "$CC" ]; then
    echo "instances : aucun compilateur C, verification passee"
    exit 0
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"
mkdir -p "$SCENARIO"
if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/WebContent" tools/userland/faux-navigateur.c 2>"$SORTIE/cc.log"; then
    echo "instances : la charge d'epreuve ne se compile pas ici, verification passee"
    sed 's/^/    /' "$SORTIE/cc.log" | head -5
    exit 0
fi

cat > "$SCENARIO/autorun" <<'AUTORUN'
exec /WebContent
echo "=== ARBRE ==="
services
echo "=== INSTANCES FIN ==="
AUTORUN

(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp 4 -cpu max -display none -no-reboot \
  -serial file:"$LOG" >/dev/null 2>&1

PROPRE="$SORTIE/serie.propre"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$PROPRE"

if ! grep -q "INSTANCES FIN" "$PROPRE"; then
    echo "instances : le scenario n'est pas alle au bout" >&2
    tail -20 "$PROPRE" >&2
    exit 1
fi

ARBRE="$SORTIE/arbre"
sed -n '/=== ARBRE ===/,/INSTANCES FIN/p' "$PROPRE" | sed -E 's/^(\[[^]]*\])+ //' > "$ARBRE"

echo "=== l'arbre du navigateur ==="
grep -A 10 -E "^  browser" "$ARBRE" | head -12

# LA REGLE : des instances DISTINCTES, sous leur role.
#
# Une ligne d'instance est un nombre seul, indente sous `web_content`. On
# compte les PID distincts : c'est ce que « une ligne par processus » veut
# dire, et c'est exactement ce que l'agregation d'avant ne pouvait pas donner.
COMBIEN=$(awk '
    /^    web_content/ { dans = 1; next }
    dans && /^      [0-9]+ / { print $1 }
    dans && /^    [a-z]/ { dans = 0 }
' "$ARBRE" | sort -u | wc -l)

if [ "$COMBIEN" -lt "$ATTENDUES" ]; then
    echo "instances : $COMBIEN instance(s) de web_content, $ATTENDUES attendues." >&2
    echo "            l'arbre agrege de nouveau ses processus, ou la publication" >&2
    echo "            par PID ne se fait plus." >&2
    exit 1
fi

printf '\033[32m%s\033[0m\n' "instances : $COMBIEN processus web_content publies un par un ; journal dans $SORTIE"
echo "SERVICES_INSTANCES_OK"
