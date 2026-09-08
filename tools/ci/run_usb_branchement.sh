#!/usr/bin/env bash
#
# Un clavier branche APRES le demarrage est-il vu ?
#
# # Pourquoi cette campagne existe
#
# C'est le premier geste de quiconque allume la machine : elle demarre, puis
# on branche le clavier. Si rien ne se passe alors, l'OS est inutilisable --
# et c'est un cas qu'AUCUNE campagne ne couvrait, parce que toutes branchent
# leurs peripheriques avant le demarrage.
#
# La topologie ici est vide au demarrage : le controleur xHCI existe, aucun
# peripherique n'est dessus. Le clavier et la souris sont ajoutes A CHAUD par
# le moniteur QMP pendant que la machine tourne.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_usb_branchement.sh BOOTIMAGE}
LOG=${2:-usb-branchement.log}

rm -rf usb-branchement-scenario usb-branchement.img "$LOG" qmp.sock
python3 - <<'PY'
from pathlib import Path
import tarfile

root = Path("usb-branchement-scenario")
root.mkdir(exist_ok=True)
autorun = root / "autorun"
# `--attends` rend la main des que le clavier arrive, et au plus tard au bout
# de vingt secondes. Sans lui il faudrait une commande `sleep`, et une attente
# fixe rendrait la campagne lente ou instable selon la charge du runner.
autorun.write_text(
    "journal off\n"
    "echo USB_BRANCHEMENT_AVANT\n"
    "lsusb\n"
    "echo USB_BRANCHEMENT_ATTENTE\n"
    "lsusb --attends 20000\n"
    "echo USB_BRANCHEMENT_APRES\n"
    "poweroff\n",
    encoding="ascii",
)
image = Path("usb-branchement.img")
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

set +e
timeout 180 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=usb-branchement.img \
  -m 2048 -display none -no-reboot \
  -device qemu-xhci,id=xhci \
  -qmp unix:qmp.sock,server=on,wait=off \
  -serial file:"$LOG" &
QEMU=$!
trap 'kill "$QEMU" 2>/dev/null || true' EXIT

# Attendre que la machine soit prete a recevoir le branchement. Borne : un
# noyau qui n'y arrive pas ne doit pas suspendre la CI.
for _ in $(seq 1 120); do
  grep -aFq USB_BRANCHEMENT_ATTENTE "$LOG" 2>/dev/null && break
  kill -0 "$QEMU" 2>/dev/null || break
  sleep 1
done

if ! grep -aFq USB_BRANCHEMENT_ATTENTE "$LOG" 2>/dev/null; then
  cat "$LOG" || true
  echo "la machine n'a jamais atteint le point de branchement" >&2
  exit 1
fi

python3 - <<'PY'
import json, socket, sys, time

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
for essai in range(50):
    try:
        sock.connect("qmp.sock")
        break
    except OSError:
        time.sleep(0.2)
else:
    print("QMP injoignable", file=sys.stderr)
    raise SystemExit(1)

fichier = sock.makefile("rwb")

def commande(payload):
    fichier.write((json.dumps(payload) + "\n").encode())
    fichier.flush()
    # La reponse peut etre precedee d'evenements asynchrones.
    for _ in range(200):
        ligne = fichier.readline()
        if not ligne:
            break
        reponse = json.loads(ligne)
        if "return" in reponse or "error" in reponse:
            return reponse
    return {"error": {"desc": "aucune reponse"}}

fichier.readline()  # banniere de bienvenue
commande({"execute": "qmp_capabilities"})
for identifiant, pilote in (("clavier", "usb-kbd"), ("souris", "usb-mouse")):
    reponse = commande({
        "execute": "device_add",
        "arguments": {"driver": pilote, "bus": "xhci.0", "id": identifiant},
    })
    if "error" in reponse:
        print(f"device_add {pilote} refuse : {reponse['error']}", file=sys.stderr)
        raise SystemExit(1)
    print(f"branche a chaud : {pilote}")
PY

wait "$QEMU"
code=$?
set -e
cat "$LOG" || true
printf 'qemu exit=%s\n' "$code"

test -s "$LOG" || { echo "aucune sortie serie" >&2; exit 1; }
if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$LOG"; then
  echo "panic/fault pendant le branchement a chaud" >&2
  exit 1
fi

echecs=0
exige() {
  if grep -aEq "$1" "$LOG"; then
    printf 'ok      %s\n' "$2"
  else
    printf 'ECHEC   %s\n' "$2"
    echecs=$((echecs + 1))
  fi
}
refuse() {
  if grep -aEq "$1" "$LOG"; then
    printf 'ECHEC   %s\n' "$2"
    echecs=$((echecs + 1))
  else
    printf 'ok      %s\n' "$2"
  fi
}

# AVANT : la machine doit vraiment avoir demarre SANS clavier, sans quoi la
# campagne prouverait l'enumeration au demarrage et non le branchement.
exige 'BOUCHAUD_LSUSB peripheriques=0 .*claviers=0 souris=0' \
  "la machine a demarre sans aucun peripherique USB"
exige 'BOUCHAUD_USB_BRANCHEMENT port=' \
  "un branchement a chaud a ete detecte"
exige 'BOUCHAUD_USB_BRANCHEMENT_OK ' \
  "le peripherique branche a chaud a ete enumere"
exige 'BOUCHAUD_LSUSB .*claviers=[1-9]' \
  "le clavier branche a chaud repond"
exige 'BOUCHAUD_LSUSB .*souris=[1-9]' \
  "la souris branchee a chaud repond"
refuse 'BOUCHAUD_LSUSB .*evenements_perdus=[1-9]' \
  "aucun rapport HID perdu pendant l'enumeration"

if [ "$echecs" -ne 0 ]; then
  echo "branchement a chaud : $echecs verification(s) en echec" >&2
  exit 1
fi
echo "USB_BRANCHEMENT_OK"
