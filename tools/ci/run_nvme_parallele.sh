#!/usr/bin/env bash
#
# La profondeur de file NVMe, MESUREE au lieu d'etre annoncee.
#
# # Ce que ce scenario existe pour empecher
#
# Partitionner les ressources DMA par emplacement rend une profondeur
# superieure a un POSSIBLE et SURE. Elle ne la rend pas ATTEINTE.
#
# Le journal du sondage GPT le montrait sans ambiguite : quatre emplacements
# alloues, et huit commandes d'affilee toutes sur `emplacement=0`. Le seul
# appelant du pilote etait sequentiel. Une profondeur qu'aucun appelant
# n'exerce est une profondeur imaginaire, et l'annoncer dans le descripteur de
# la couche bloc serait un mensonge que rien ne contredirait.
#
# `nvme-parallele` lance quatre lecteurs, sur des blocs distincts, et publie le
# maximum d'emplacements reellement occupes ensemble.
#
# # Ce qui est verifie
#
#   1. les quatre emplacements sont alloues, avec leurs ressources propres ;
#   2. toutes les lectures reussissent -- une profondeur obtenue en perdant des
#      donnees ne vaut rien ;
#   3. la profondeur atteinte est SUPERIEURE A UN ;
#   4. les emplacements sont RENDUS : la sonde tourne trois fois, et les
#      passages suivants doivent repartir d'un pot plein. Une fuite d'un
#      emplacement par passage finirait par bloquer le pilote, et ne se verrait
#      pas sur un seul essai.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_nvme_parallele.sh BOOTIMAGE [LOG]}
LOG=${2:-target/nvme-parallele-ci/nvme-parallele.log}
RACINE=$(dirname "$LOG")
DISQUE="$RACINE/disque.img"
SCENARIO="$RACINE/scenario.img"

mkdir -p "$RACINE"
rm -f "$LOG" "$DISQUE" "$SCENARIO"

python3 tools/ci/fabrique-disque-gpt.py "$DISQUE" --mio 64 || exit 1

python3 - "$SCENARIO" <<'PY'
import sys, tarfile
from pathlib import Path
image = Path(sys.argv[1])
autorun = image.parent / "autorun"
autorun.write_text(
    "journal off\n"
    "echo NVME_PAR_BEGIN\n"
    # Trois passages : le premier mesure, les suivants prouvent que les
    # emplacements sont rendus.
    "nvme-parallele\n"
    "nvme-parallele\n"
    "nvme-parallele\n"
    "echo NVME_PAR_FIN\n"
    "poweroff\n",
    encoding="ascii",
)
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(autorun), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with autorun.open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY

echo "=== QEMU NVMe : profondeur de file ==="
timeout 240 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SCENARIO" \
  -m 2048 -smp 4 -display none -no-reboot \
  -drive if=none,id=nvm,format=raw,file="$DISQUE" \
  -device nvme,serial=bouchaud0,drive=nvm \
  -serial file:"$LOG"

if [ ! -s "$LOG" ]; then
    echo "aucune sortie serie : le noyau n'a rien emis" >&2
    exit 1
fi
echo "journal serie : $(wc -c < "$LOG") octets"

if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$LOG"; then
    echo "panic/faute pendant la sonde de concurrence" >&2
    exit 1
fi

echo "--- releve ---"
NETTOYE="$RACINE/propre.log"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$NETTOYE"
grep -aE 'BOUCHAUD_NVME_EMPLACEMENTS|NVME_PARALLELE' "$NETTOYE" || true
echo "--- fin du releve ---"

problemes=0
plainte() { printf 'ECHEC   %s\n' "$1"; problemes=$((problemes + 1)); }

grep -aq 'BOUCHAUD_NVME_EMPLACEMENTS nombre=4' "$NETTOYE" \
    || plainte 'les quatre emplacements ne sont pas alloues'

# Trois passages, tous conclusifs.
passages=$(grep -ac '^.*NVME_PARALLELE_OK' "$NETTOYE" || true)
if [ "$passages" -ne 3 ]; then
    plainte "trois passages attendus, $passages verdicts OK"
fi
if grep -aq 'NVME_PARALLELE_SEQUENTIEL' "$NETTOYE"; then
    plainte 'profondeur atteinte = 1 : les ressources sont partitionnees mais rien ne les exerce ensemble'
fi
if grep -aq 'NVME_PARALLELE_ECHEC' "$NETTOYE"; then
    plainte "$(grep -am1 'NVME_PARALLELE_ECHEC' "$NETTOYE")"
fi
if grep -aq 'NVME_PARALLELE .*echecs=[1-9]' "$NETTOYE"; then
    plainte 'des lectures ont echoue : une profondeur obtenue en perdant des donnees ne vaut rien'
fi

# LES EMPLACEMENTS SONT-ILS RENDUS ?
#
# Le premier passage peut legitimement voir un emplacement occupe : le fil de
# montage tourne encore. Les passages SUIVANTS doivent repartir d'un pot plein.
# Une fuite d'un emplacement par passage ne se verrait pas sur un seul essai,
# et bloquerait le pilote au quatrieme.
suivants=$(grep -aE 'NVME_PARALLELE lecteurs=' "$NETTOYE" | tail -n +2)
if [ -z "$suivants" ]; then
    plainte 'aucun passage de controle apres le premier'
else
    while IFS= read -r ligne; do
        case "$ligne" in
            *libres=4*) ;;
            *) plainte "un emplacement n'a pas ete rendu : $ligne" ;;
        esac
    done <<< "$suivants"
fi

# La profondeur atteinte, extraite du meilleur passage.
meilleure=$(grep -aoE 'profondeur_max=[0-9]+' "$NETTOYE" | cut -d= -f2 | sort -n | tail -1)
echo "profondeur maximale atteinte : ${meilleure:-0} sur 4 emplacements"
if [ "${meilleure:-0}" -lt 2 ]; then
    plainte 'profondeur maximale inferieure a deux'
fi

if [ "$problemes" -ne 0 ]; then
    echo "NVME_PARALLELE_SCENARIO_ECHEC problemes=$problemes" >&2
    exit 1
fi
echo "NVME_PARALLELE_SCENARIO_OK profondeur_max=$meilleure"
