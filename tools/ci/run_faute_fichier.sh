#!/usr/bin/env bash
# OU SONT LES HUIT SECONDES DU PREMIER WEBWORKER ?
#
# BOUCHAUD_C51_OU_SONT_LES_HUIT_SECONDES
#
# Ladybird donne le contraste :
#
#     WebWorker #1  2440 fautes fichier / 8 008 342 us
#     WebWorker #3  2440 fautes fichier /    54 398 us
#
# Mais `FichierPrive total_us` mesure la latence VECUE -- attente comprise. Il
# ne dit pas si le temps est parti dans l'ATA, dans le cache, dans un verrou,
# ou a attendre qu'un autre coeur charge la meme page. Les quatre appellent des
# remedes differents.
#
# Ce banc reproduit le mecanisme SANS Ladybird. La seule chose qui compte est
# la taille du binaire : au-dela de `INLINE_BOOT_FILE_SIZE` (4 Mio), `tar.rs`
# cesse de copier le contenu dans le noeud et l'enregistre comme etendue ATA.
# Les pages de `gros-elf` arrivent donc par le meme chemin que celles des ELF
# de Ladybird -- et le premier lancement est froid, les suivants chauds.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${FAUTE_SORTIE:-$(mktemp -d)}
mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)
SECONDES=${FAUTE_SECONDES:-300}
LANCEMENTS=${FAUTE_LANCEMENTS:-4}

if [ ! -f "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
if [ -z "$CC" ]; then
    echo "faute fichier : aucun compilateur C -- rien n'a pu etre mesure" >&2
    exit 1
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"

# Le bloc doit etre REPRODUCTIBLE : un contenu aleatoire ferait varier la
# taille du binaire d'un run a l'autre, et donc le nombre de pages.
python3 - "$SORTIE/gros.bin" <<'PY'
import sys
# Motif deterministe de 5 Mio. Ni compressible par le systeme de fichiers, ni
# aleatoire : deux executions doivent produire le meme binaire.
taille = 5 * 1024 * 1024
with open(sys.argv[1], "wb") as f:
    f.write(bytes((i * 37 + (i >> 8) * 11) & 0xFF for i in range(taille)))
PY

if ! (cd "$SORTIE" && "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles \
        -Wl,-z,noexecstack -o "$SCENARIO/gros-elf" \
        "$OLDPWD/tools/userland/gros-elf.c") 2>"$SORTIE/cc.log"; then
    echo "faute fichier : la charge d'epreuve ne se compile pas" >&2
    sed -n '1,20p' "$SORTIE/cc.log" >&2
    exit 1
fi

TAILLE=$(stat -c %s "$SCENARIO/gros-elf")
if [ "$TAILLE" -le 4194304 ]; then
    # Sous le seuil, le fichier serait INLINE et le banc ne mesurerait pas le
    # chemin qu'il pretend mesurer. Mieux vaut refuser que rendre un vert faux.
    echo "faute fichier : binaire de $TAILLE octets, sous INLINE_BOOT_FILE_SIZE" >&2
    echo "                il serait inline, pas adosse au disque : rien a mesurer" >&2
    exit 1
fi
echo "faute fichier : binaire de $TAILLE octets (> 4 Mio, donc ata-disk)"

{
    for i in $(seq 1 "$LANCEMENTS"); do
        echo "echo GROS_ELF_LANCEMENT_$i && /gros-elf"
    done
    echo "echo GROS_ELF_FIN"
} > "$SCENARIO/autorun"

(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 2048 -smp 4 -display none -serial file:"$LOG" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 >/dev/null 2>&1 || true

PROPRE="$SORTIE/propre.log"
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$PROPRE"

if ! grep -q "GROS_ELF_FIN" "$PROPRE"; then
    echo "faute fichier : le scenario n'est pas alle au bout" >&2
    tail -12 "$PROPRE" >&2
    exit 1
fi

echo
echo "== source du binaire, telle que le noyau la voit =="
grep -o 'BACKING_PROBE path=/gros-elf.*' "$PROPRE" | tail -1 || true
grep -o 'BACKING_PROBE.*' "$PROPRE" | tail -1 || true

echo
echo "== decomposition du chemin FichierPrive, dans le temps =="
grep -o 'FAULT_FILE_BREAKDOWN.*' "$PROPRE" || echo "  (aucune)"
echo
grep -o 'FAULT_WAIT.*' "$PROPRE" | tail -3 || true
grep -o 'BACKING_DISK.*' "$PROPRE" | tail -3 || true
grep -o 'BACKING_MEMORY.*' "$PROPRE" | tail -3 || true

echo
echo "== cout PAR LANCEMENT, par difference entre deux sorties =="
python3 tools/ci/analyse_faute_fichier.py "$PROPRE" || true

if ! grep -q 'FAULT_FILE_BREAKDOWN' "$PROPRE"; then
    echo "faute fichier : aucune decomposition emise -- rien n'a ete mesure" >&2
    exit 1
fi
echo
echo "FAUTE_FICHIER_OK ; journaux dans $SORTIE"
