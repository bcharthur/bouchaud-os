#!/usr/bin/env bash
#
# Le chemin de STOCKAGE, exerce de bout en bout dans QEMU.
#
# # Ce qu'aucune campagne ne faisait
#
# `bouchaud_nvme_args` existait dans `plateforme.sh` et n'avait AUCUN appelant.
# Aucune campagne n'attachait donc de disque NVMe, et le pilote -- son
# initialisation, ses files d'entree-sortie, sa premiere lecture reelle --
# n'etait execute nulle part. La seule machine qui l'exercait etait la machine
# physique, ou il a double-faute.
#
# Un pilote qui n'est execute que sur le materiel qu'on ne possede pas n'a pas
# de preuve : il a des temoignages.
#
# # Ce que ce scenario etablit
#
#   1. le controleur est trouve, remis a zero et demarre ;
#   2. ses files d'entree-sortie sont creees ;
#   3. la PREMIERE LECTURE REELLE part et revient -- c'est celle qui a
#      double-faute sur la machine de reference, et le releve `NVME_IO_*` la
#      suit pas a pas ;
#   4. la table GPT ecrite par un outil INDEPENDANT est relue correctement ;
#   5. la partition systeme Bouchaud est montee.
#
# Le disque est fabrique par `fabrique-disque-gpt.py`, qui ne partage aucune
# ligne avec le noyau : ce que le noyau relit est donc une table produite par
# quelqu'un d'autre.
#
# # Sur quelle machine
#
# `BOUCHAUD_MACHINE` decide, et le defaut est celle qui DEMARRE.
#
# Le premier essai forcait q35, plateforme de reference du chantier 10. Il a
# rendu un journal serie entierement VIDE apres trois minutes : le noyau n'y
# demarre pas encore. C'est une information utile -- elle est rapportee telle
# quelle -- et ce n'est pas une raison pour que le pilote NVMe reste sans
# aucune preuve d'execution en attendant.
#
# Le peripherique `nvme` de QEMU s'attache aussi au bus d'i440fx : il y perd
# ses fonctions PCIe, pas sa capacite a servir des blocs. Le pilote y est donc
# exerce entierement.
set -uo pipefail
cd "$(dirname "$0")/../.."
. tools/ci/plateforme.sh

BOOT=${1:?usage: run_nvme_gpt.sh BOOTIMAGE [LOG]}
LOG=${2:-target/nvme-gpt-ci/nvme-gpt.log}
DISQUE=${DISQUE:-target/nvme-gpt-ci/disque-gpt.img}

mkdir -p "$(dirname "$LOG")"
: > "$LOG"

python3 tools/ci/fabrique-disque-gpt.py "$DISQUE" --mio 64 || exit 1

export BOUCHAUD_MACHINE=${BOUCHAUD_MACHINE:-pc}
bouchaud_profil_resume

echo "=== QEMU NVMe + GPT ==="
qemu-system-x86_64 \
  $(bouchaud_machine_args) \
  -drive format=raw,file="$BOOT" \
  $(bouchaud_nvme_args "$DISQUE") \
  -m 4096 -smp 4 -cpu max -display none -no-reboot \
  -audiodev none,id=muet \
  -serial file:"$LOG" 2> "$LOG.qemu" &
PID=$!

FATAL='\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|TRIPLE FAULT|panicked at|SpinLock recursive acquisition'
DEADLINE=$((SECONDS + 180))
while kill -0 "$PID" 2>/dev/null; do
  if grep -aEq "$FATAL" "$LOG"; then
    echo "fatal detecte" >&2
    break
  fi
  if grep -aFq "BOUCHAUD_INSTALL_SYSTEME_MONTE" "$LOG" \
     || grep -aFq "BOUCHAUD_NVME_PERSISTENCE_ABSENTE" "$LOG"; then
    break
  fi
  if (( SECONDS >= DEADLINE )); then
    echo "ECHEANCE : le montage differe n'a rien conclu" >&2
    break
  fi
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

octets=$(stat -c '%s' "$LOG" 2>/dev/null || echo 0)
echo "journal serie : $octets octets"

# UN JOURNAL VIDE N'EST PAS UN PILOTE MUET : C'EST UNE MACHINE QUI N'A PAS
# DEMARRE.
#
# Les deux se lisent pareil dans une liste de marqueurs absents -- huit lignes
# « marqueur obligatoire absent » qui accusent le pilote NVMe alors que le
# noyau n'a jamais rien ecrit. Les distinguer est ce qui evite de chercher un
# defaut de stockage la ou il n'y a pas de demarrage.
if [ "$octets" -eq 0 ]; then
  echo "ECHEC : AUCUNE sortie serie. Le noyau n'a pas demarre sur cette machine." >&2
  echo "        machine=$BOUCHAUD_MACHINE" >&2
  echo "--- ce que QEMU a dit ---" >&2
  cat "$LOG.qemu" >&2 2>/dev/null || true
  echo "--- fin ---" >&2
  exit 1
fi

echo "--- premieres lignes ---"
head -c 2000 "$LOG" || true
echo
echo "--- releve NVMe ---"
grep -aE 'BOUCHAUD_NVME|NVME_IO_|BOUCHAUD_INSTALL|\[NVME\]' "$LOG" | tail -n 60 || true
echo "--- fin du releve ---"
if [ -s "$LOG.qemu" ]; then
  echo "--- QEMU (stderr) ---"
  head -c 2000 "$LOG.qemu"
  echo
fi

echecs=0

if grep -aEq "$FATAL" "$LOG"; then
  echo "ECHEC : panique ou faute pendant le chemin de stockage" >&2
  grep -aE -B 20 "$FATAL" "$LOG" | tail -n 40 >&2
  echecs=$((echecs + 1))
fi

# Ce que le scenario EXIGE, dans l'ordre du chemin.
OBLIGATOIRES=(
  'BOUCHAUD_NVME_GREEN'
  'NVME_IO_READ_ENTER'
  'NVME_IO_PRP_READY'
  'NVME_IO_SQE_READY'
  'NVME_IO_DOORBELL'
  'NVME_IO_CQE_OK'
  'NVME_IO_COPY_END'
  'BOUCHAUD_INSTALL_SYSTEME_MONTE'
)
for marqueur in "${OBLIGATOIRES[@]}"; do
  if ! grep -aFq "$marqueur" "$LOG"; then
    echo "marqueur obligatoire absent : $marqueur" >&2
    echecs=$((echecs + 1))
  fi
done

# Ce que le scenario INTERDIT.
INTERDITS=(
  'BOUCHAUD_NVME_HORS_SERVICE'
  'NVME_IO_DELAI'
  'BOUCHAUD_HEAP_LISTE_LIBRE_CORROMPUE'
  'BOUCHAUD_PILE_NOYAU_DEBORDEE'
)
for marqueur in "${INTERDITS[@]}"; do
  if grep -aFq "$marqueur" "$LOG"; then
    echo "marqueur interdit present : $marqueur" >&2
    grep -aF "$marqueur" "$LOG" | head -n 3 >&2
    echecs=$((echecs + 1))
  fi
done

if [ "$echecs" -ne 0 ]; then
  echo "NVME_GPT_ECHEC problemes=$echecs" >&2
  exit 1
fi
echo "NVME_GPT_OK"
