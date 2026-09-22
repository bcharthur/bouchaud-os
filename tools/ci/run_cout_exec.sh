#!/usr/bin/env bash
#
# CE QUE COUTE UN `exec`, ET OU CE COUT PASSE.
#
# # Le defaut que ce banc fige
#
# `elf::build_stack` allouait, projetait et mettait a zero les HUIT
# MEBIOCTETS de la pile utilisateur, d'avance, a chaque `exec` :
#
#     SONDE_PILE alloc_us=52878 taille_kio=8192
#     PERF_EXEC_PRET image=/toucher pid=9 duree_us=71769
#                    espace_us=114 image_us=2852 pile_us=68801
#
# Cinquante-trois millisecondes sur les cinquante-quatre que coutait l'exec
# d'un binaire de treize kibioctets -- quatre-vingt-dix-huit pour cent. Le
# portage lance six processus par navigateur : plus de trois cents
# millisecondes de demarrage, entierement passees a mettre a zero de la
# memoire que presque aucun programme ne touche.
#
# Deux mille quarante-huit pages allouees d'un coup ne produisent ni erreur,
# ni faute, ni ligne de journal. La seule trace etait un demarrage lent, que
# l'on attribuait au navigateur.
#
# # La mesure, apres correction
#
#     PERF_EXEC_PRET duree_us=1326  espace_us=64  image_us=112  pile_us=1149
#     PERF_EXEC_PRET duree_us=943   espace_us=64  image_us=89   pile_us=788
#
# # Ce que le banc verifie
#
# Que l'exec d'un petit binaire reste sous un budget. Le budget est LARGE --
# dix millisecondes la ou la mesure donne une -- parce qu'un banc qui echoue
# sur le bruit d'une machine chargee apprend a etre ignore. Ce qu'il doit
# attraper est le retour du defaut, qui valait cinquante fois le budget.
#
#   ./tools/ci/run_cout_exec.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${EXEC_SORTIE:-$(mktemp -d)}
SECONDES=${EXEC_SECONDES:-150}
# En microsecondes. Voir l'en-tete : large a dessein.
BUDGET=${EXEC_BUDGET_US:-10000}
mkdir -p "$SORTIE"

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
if [ -z "$CC" ]; then
    echo "cout exec : aucun compilateur C, verification passee"
    exit 0
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"
if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/toucher" tools/userland/toucher-pages.c 2>"$SORTIE/cc.log"; then
    echo "cout exec : la charge d'epreuve ne se compile pas ici, verification passee"
    exit 0
fi

# QUATRE EXECS, ET LA PREMIERE NE COMPTE PAS.
#
# La premiere paie le cache de pages froid -- la mesure donne vingt
# millisecondes contre une pour les suivantes. La compter ferait echouer le
# banc sur un cout qui n'est pas celui qu'on surveille.
cat > "$SCENARIO/autorun" <<'AUTORUN'
echo "=== COUT EXEC DEBUT ==="
exec /toucher
exec /toucher
exec /toucher
exec /toucher
echo "=== COUT EXEC FIN ==="
AUTORUN
(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp 8 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" >/dev/null 2>&1

PROPRE="$SORTIE/propre.log"
sed -E -e 's/\x1b\[[0-9;]*m//g' -e 's/^(\[[^]]*\])+ //' -e 's/\r$//' "$LOG" > "$PROPRE"

if ! grep -q "COUT EXEC FIN" "$PROPRE"; then
    echo "cout exec : le scenario n'est pas alle au bout" >&2
    tail -20 "$PROPRE" >&2
    exit 1
fi

LIGNES=$(grep 'PERF_EXEC_PRET image=/toucher' "$PROPRE")
echo "$LIGNES" | sed 's/^\[kernel\] //'

echecs=0
RANG=0
while IFS= read -r ligne; do
    RANG=$((RANG + 1))
    [ "$RANG" -eq 1 ] && continue
    US=$(echo "$ligne" | grep -o 'duree_us=[0-9]*' | cut -d= -f2)
    if [ -z "${US:-}" ]; then
        echo "cout exec : duree illisible dans : $ligne" >&2
        echecs=$((echecs + 1))
        continue
    fi
    if [ "$US" -gt "$BUDGET" ]; then
        echo "cout exec : exec #$RANG a coute ${US} us, budget ${BUDGET} us" >&2
        echo "            regarder pile_us : c'est la que le defaut vivait" >&2
        echecs=$((echecs + 1))
    fi
done <<< "$LIGNES"

if [ "$RANG" -lt 2 ]; then
    echo "cout exec : aucune mesure exploitable" >&2
    exit 1
fi
if [ "$echecs" -ne 0 ]; then
    echo "cout exec : $echecs depassement(s) ; journaux dans $SORTIE" >&2
    exit 1
fi
printf '\033[32m%s\033[0m\n' "cout exec : $((RANG - 1)) exec(s) sous le budget de ${BUDGET} us"
echo "COUT_EXEC_OK"
