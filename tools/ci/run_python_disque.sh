#!/usr/bin/env bash
# BOUCHAUD_PYTHON_DISQUE_V1 -- l'interpreteur Python est lu sur le DISQUE.
#
#   run_python_disque.sh BOOTIMAGE
#
# Depuis le 13/11/2026, rustpython.wasm (16,4 Mo) n'est plus embarque dans le
# noyau : il doublait le temps de chargement BIOS (10,0-10,7 s contre
# 4,8-5,2 s sans, QEMU TCG). Ce banc prouve les deux faces du changement, sur
# le vrai chemin (disque de donnees -> index tar -> backing -> wasmi) :
#
#   avec    : /usr/lib/python/rustpython.wasm present -> Python calcule
#             vraiment (sum(x*x) -> 30), lu UNE fois puis garde en cache ;
#   sans    : disque sans interpreteur -> refus NOMME (« interpreteur absent »),
#             aucune panique, la session continue (marqueur apres l'echec).
#
# Et l'image de boot ne doit plus contenir l'interpreteur : on le verifie sur
# sa taille (le binaire seul pese 16,4 Mo).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_python_disque.sh BOOTIMAGE}
WASM=src/assets/python/rustpython.wasm
OUT=${OUT:-python-disque}
rm -rf "$OUT"
mkdir -p "$OUT"

taille_boot=$(stat -c %s "$BOOT")
taille_wasm=$(stat -c %s "$WASM")
echo "bootimage : $taille_boot octets ; interpreteur : $taille_wasm octets"
if [ "$taille_boot" -ge $((20 * 1024 * 1024)) ]; then
  echo "PYTHON_DISQUE_ECHEC bootimage de $taille_boot octets : l'interpreteur semble encore embarque" >&2
  exit 1
fi

fabrique() {  # fabrique IMAGE AVEC_WASM(0|1)
  python3 - "$1" "$2" "$WASM" <<'PY'
import io, sys, tarfile
image, avec, wasm = sys.argv[1], sys.argv[2] == "1", sys.argv[3]
script = (
    "echo PYTHON_DISQUE_DEBUT\n"
    "python-selftest\n"
    "echo PYTHON_DISQUE_SESSION_VIVANTE\n"
    # Second appel : l'interpreteur deja lu ne doit PAS etre relu sur le
    # disque (une seule ligne BOUCHAUD_PYTHON_INTERPRETE_CHARGE).
    # NB : le code de sortie de `python -c` n'atteint pas `statut=` -- deja
    # vrai avec l'interpreteur embarque (observe le 13/11/2026 sur l'ancien
    # noyau) ; ce banc ne s'en sert donc pas comme preuve.
    "python -c \"raise SystemExit(6*7)\"\n"
).encode()
def ajoute(tar, nom, octets, mode=0o644):
    info = tarfile.TarInfo(nom)
    info.size = len(octets); info.mode = mode
    info.uid = info.gid = 0; info.uname = info.gname = "root"
    tar.addfile(info, io.BytesIO(octets))
def dossier(tar, nom):
    info = tarfile.TarInfo(nom); info.type = tarfile.DIRTYPE; info.mode = 0o755
    tar.addfile(info)
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    ajoute(tar, "./autorun", script)
    if avec:
        for d in ("./usr", "./usr/lib", "./usr/lib/python"):
            dossier(tar, d)
        ajoute(tar, "./usr/lib/python/rustpython.wasm", open(wasm, "rb").read())
with open(image, "ab") as f:
    taille = f.tell()
    f.write(b"\0" * ((-taille) % 512 + 8 * 1024 * 1024))
PY
}

joue() {  # joue NOM
  local nom=$1 code
  set +e
  timeout 600 qemu-system-x86_64 \
    -drive format=raw,file="$BOOT",snapshot=on \
    -drive format=raw,file="$OUT/$nom.img" \
    -m 2048 -display none -no-reboot \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -serial file:"$OUT/$nom.log"
  code=$?
  set -e
  echo "[$nom] qemu exit=$code"
  if grep -aqE 'KERNEL PANIC|panicked at' "$OUT/$nom.log"; then
    echo "PYTHON_DISQUE_ECHEC panique noyau ($nom)" >&2
    grep -aE 'KERNEL PANIC|panicked at' "$OUT/$nom.log" >&2 || true
    exit 1
  fi
  grep -aF "PYTHON_DISQUE_SESSION_VIVANTE" "$OUT/$nom.log" >/dev/null || {
    echo "PYTHON_DISQUE_ECHEC ($nom) : le script n'est pas alle au bout" >&2
    tail -40 "$OUT/$nom.log" >&2
    exit 1
  }
}

fabrique "$OUT/avec.img" 1
fabrique "$OUT/sans.img" 0
joue avec
joue sans

grep -aF "python-selftest: OK (sum(x*x)=30)" "$OUT/avec.log"
n=$(grep -acF "BOUCHAUD_PYTHON_INTERPRETE_CHARGE" "$OUT/avec.log")
if [ "$n" -ne 1 ]; then
  echo "PYTHON_DISQUE_ECHEC interpreteur lu $n fois (attendu : 1, puis cache)" >&2
  exit 1
fi
grep -aE "BOUCHAUD_PYTHON_INTERPRETE_CHARGE octets=$taille_wasm lecture_ms=[0-9]+" "$OUT/avec.log"
grep -aF "python-selftest: ECHEC (interpreteur absent : /usr/lib/python/rustpython.wasm" "$OUT/sans.log"
grep -aF "python: interpreteur absent : /usr/lib/python/rustpython.wasm" "$OUT/sans.log"
if grep -aF "python-selftest: OK" "$OUT/sans.log"; then
  echo "PYTHON_DISQUE_ECHEC : Python a calcule sans interpreteur sur le disque ?" >&2
  exit 1
fi
echo PYTHON_DISQUE_OK
