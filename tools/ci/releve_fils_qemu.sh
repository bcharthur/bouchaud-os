#!/usr/bin/env bash
#
# BOUCHAUD_SYSRQ_RELEVE_FILS_V1 (#283) : demande a une VM QEMU VIVANTE l'etat
# de chaque fil, et attend qu'il soit publie sur sa console serie.
#
#   tools/ci/releve_fils_qemu.sh MONITEUR_UNIX JOURNAL_SERIE [ATTENTE_S]
#
# `sendkey alt-sysrq` sur le moniteur fait entrer Alt+SysRq par l'IRQ1 ; le
# noyau (`drivers::sysrq`, `demande_releve_fils`) imprime alors
# `[FIL-RELEVE] debut`, une ligne `[FIL-FORENSIQUE]` par fil vivant, puis
# `[FIL-RELEVE] fin n=K fils=N`.
#
# Outil de DIAGNOSTIC : il n'entre dans aucun verdict et sort toujours 0. Il
# dit seulement ce qu'il a obtenu (`FIL_RELEVE_OBTENU` / `FIL_RELEVE_ABSENT`),
# pour que l'absence de releve -- fil diagnostique fige lui aussi -- soit elle
# aussi une information.
set -uo pipefail

MONITEUR=${1:?moniteur QEMU (socket unix) attendu}
JOURNAL=${2:?journal serie attendu}
ATTENTE=${3:-20}

avant=$(grep -ac '\[FIL-RELEVE\] fin' "$JOURNAL" 2>/dev/null || true)
avant=${avant:-0}

if ! python3 - "$MONITEUR" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(sys.argv[1])
try:
    s.recv(4096)  # banniere du moniteur
except socket.timeout:
    pass
s.sendall(b"sendkey alt-sysrq\n")
time.sleep(0.5)
s.close()
PY
then
  echo "FIL_RELEVE_ABSENT raison=moniteur_injoignable"
  exit 0
fi

for _ in $(seq 1 "$ATTENTE"); do
  sleep 1
  apres=$(grep -ac '\[FIL-RELEVE\] fin' "$JOURNAL" 2>/dev/null || true)
  if (( ${apres:-0} > avant )); then
    echo "FIL_RELEVE_OBTENU $(grep -a '\[FIL-RELEVE\] fin' "$JOURNAL" | tail -n 1 | sed 's/.*\[FIL-RELEVE\] //')"
    exit 0
  fi
done
echo "FIL_RELEVE_ABSENT raison=aucune_publication_en_${ATTENTE}s"
exit 0
