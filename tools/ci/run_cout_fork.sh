#!/usr/bin/env bash
#
# CE QUE COUTE UN `fork`, ET POURQUOI LE NAVIGATEUR LE PAIE LE PLUS CHER.
#
# `AddressSpace::duplicate` recopie immediatement chaque page possedee par le
# pere. Le noyau l'assume en commentaire ; personne ne l'avait chiffre.
#
# Or `Core::Process::spawn` de Ladybird fait `fork` puis `execve` : la copie
# entiere est jetee a la ligne suivante. Et le pere est le navigateur, c'est-a-
# dire le plus gros processus de la machine.
#
# Le banc mesure le cout du `fork` a quatre tailles residentes et verifie que
# ce cout reste BORNE. Ce qu'il doit attraper est une croissance lineaire avec
# la taille du pere -- la signature de la recopie.
#
#   ./tools/ci/run_cout_fork.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${FORK_SORTIE:-$(mktemp -d)}
# LE PLAFOND EST CELUI D'UNE MACHINE DE CI, PAS DE CELLE-CI.
#
# La charge dure environ quatre minutes ici. Un coureur GitHub n'a pas de KVM
# et emule : le meme scenario y prend plusieurs fois plus longtemps, et un
# plafond taille sur la machine de developpement rendrait « le scenario n'est
# pas alle au bout » -- un faux rouge qui ressemble a une panne.
SECONDES=${FORK_SECONDES:-600}
# En microsecondes, pour le palier de 256 Mio. Voir l'en-tete du .c.
BUDGET=${FORK_BUDGET_US:-2000000}
# LE REPERTOIRE DE SORTIE EST RENDU ABSOLU, ET CE N'EST PAS DU CONFORT.
#
# `mkdisk.sh` est lance depuis `tools/userland` (il y cherche ses outils). Un
# chemin de sortie RELATIF y est donc resolu depuis `tools/userland` et non
# depuis la racine du depot -- l'image de scenario atterrit a cote, et le banc
# rend « mkdisk a echoue ».
#
# Le defaut ne se voyait pas en local, ou `mktemp -d` rend un chemin absolu. Il
# est apparu au premier passage en CI, qui passe `proc-stat-ci`.
mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
# UN GARDE QUI N'A PAS PU MESURER N'EST PAS VERT.
#
# Ces trois sorties rendaient 0 quand le banc etait incapable de mesurer quoi
# que ce soit. Tant que le script tournait a la main, c'etait un confort. Depuis
# qu'il est dans la CI, c'est un mensonge : un coureur sans compilateur
# affichait un vert qui ne protegeait rien, et une regression du fork serait
# passee sans que personne ne le voie.
if [ -z "$CC" ]; then
    echo "cout fork : aucun compilateur C -- rien n'a pu etre mesure" >&2
    echo "            installer gcc/cc/clang, ou retirer ce banc de la CI" >&2
    exit 1
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"
if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/coutfork" tools/userland/cout-fork.c 2>"$SORTIE/cc.log"; then
    echo "cout fork : la charge d'epreuve ne se compile pas ici" >&2
    sed -n '1,20p' "$SORTIE/cc.log" >&2
    exit 1
fi

if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/sortie" tools/userland/sortie-immediate.c 2>>"$SORTIE/cc.log"; then
    echo "cout fork : la cible d'execve ne se compile pas ici" >&2
    sed -n '1,20p' "$SORTIE/cc.log" >&2
    exit 1
fi

cat > "$SCENARIO/autorun" <<'AUTORUN'
exec /coutfork
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

if ! grep -q "COUT FORK FIN" "$PROPRE"; then
    echo "cout fork : le scenario n'est pas alle au bout" >&2
    tail -30 "$PROPRE" >&2
    exit 1
fi

LIGNES=$(grep 'COUT_FORK rss_mio=' "$PROPRE")
echo "$LIGNES"

echecs=0
declare -A MEILLEUR
while IFS= read -r ligne; do
    MIO=$(echo "$ligne" | grep -o 'rss_mio=[0-9]*' | cut -d= -f2)
    US=$(echo "$ligne" | grep -o 'fork_us=-\?[0-9]*' | cut -d= -f2)
    [ -z "${MIO:-}" ] && continue
    if [ -z "${US:-}" ] || [ "$US" -lt 0 ]; then
        echo "cout fork : fork refuse a ${MIO} Mio" >&2
        echecs=$((echecs + 1))
        continue
    fi
    # Le MEILLEUR des essais, pas la moyenne : un banc qui echoue sur le bruit
    # d'une machine chargee apprend a etre ignore, et ce qu'on surveille est un
    # cout STRUCTUREL, present des le meilleur cas.
    if [ -z "${MEILLEUR[$MIO]:-}" ] || [ "$US" -lt "${MEILLEUR[$MIO]}" ]; then
        MEILLEUR[$MIO]=$US
    fi
done <<< "$LIGNES"

if [ "${#MEILLEUR[@]}" -eq 0 ]; then
    echo "cout fork : aucune mesure exploitable" >&2
    exit 1
fi

echo
echo "palier   meilleur fork   us par Mio"
for MIO in $(printf '%s\n' "${!MEILLEUR[@]}" | sort -n); do
    US=${MEILLEUR[$MIO]}
    printf '%4s Mio %10s us %10s\n' "$MIO" "$US" "$((US / MIO))"
done

# LES CHEMINS QUI NE REVIENNENT PAS RENDENT-ILS CE QU'ILS ONT PRIS ?
#
# Ce banc fait `fork` + `execve` + `exit` en boucle : c'est exactement la charge
# qui exerce les deux chemins no-return du noyau. Il serait absurde de la faire
# tourner sans en tirer cette verification-la.
echo
if ! python3 tools/ci/verifie_execve_bkl.py "$PROPRE"; then
    echecs=$((echecs + 1))
fi

# LA COPIE EST-ELLE COMPLETE ? La question n'est pas rhetorique depuis que
# `duplicate` prend ses frames sans les mettre a zero : une copie partielle
# donnerait a l'enfant la memoire d'un autre processus, et le programme
# continuerait de tourner sans rien dire.
LIGNE_COPIE=$(grep 'COUT_FORK_COPIE' "$PROPRE" | head -1 || true)
echo
if [ -z "$LIGNE_COPIE" ]; then
    echo "cout fork : aucune verification de copie ; le contrat de la frame non mise a zero n'est pas teste" >&2
    echecs=$((echecs + 1))
elif ! echo "$LIGNE_COPIE" | grep -q 'ok=1'; then
    echo "cout fork : la copie du fork n'est pas fidele -- $LIGNE_COPIE" >&2
    echo "            voir alloc_frame_a_recouvrir : la frame sort SALE de l'allocateur" >&2
    echecs=$((echecs + 1))
else
    echo "$LIGNE_COPIE"
fi

# LE GESTE COMPLET, ET CE QU'IL AJOUTE.
#
# `fork` seul ne dit pas tout : l'`execve` qui suit rend une a une les frames
# que la duplication venait d'allouer. Les deux lignes cote a cote chiffrent
# les DEUX passages sur la taille residente du pere.
LIGNES_EXEC=$(grep 'COUT_FORK_EXEC rss_mio=' "$PROPRE" || true)
if [ -n "$LIGNES_EXEC" ]; then
    echo
    echo "$LIGNES_EXEC"
    echo
    echo "== etapes de l'execve, vues du noyau =="
    grep 'PERF_EXECVE' "$PROPRE" | sed 's/^\[kernel\] //' | head -5
    if ! grep -q 'PERF_EXECVE' "$PROPRE"; then
        echo "cout fork : aucune ligne PERF_EXECVE ; le chemin sys_execve n'a pas ete pris" >&2
        echecs=$((echecs + 1))
    fi
else
    echo "cout fork : aucune mesure fork+execve" >&2
    echecs=$((echecs + 1))
fi

GROS=$(printf '%s\n' "${!MEILLEUR[@]}" | sort -n | tail -1)
US=${MEILLEUR[$GROS]}
if [ "$US" -gt "$BUDGET" ]; then
    echo "cout fork : ${GROS} Mio ont coute ${US} us, budget ${BUDGET} us" >&2
    echo "            regarder AddressSpace::duplicate : la copie est immediate" >&2
    echecs=$((echecs + 1))
fi

if [ "$echecs" -ne 0 ]; then
    echo "cout fork : $echecs depassement(s) ; journaux dans $SORTIE" >&2
    exit 1
fi
printf '\033[32m%s\033[0m\n' "cout fork : ${GROS} Mio forkes en ${US} us, sous le budget de ${BUDGET} us"
echo "COUT_FORK_OK"
