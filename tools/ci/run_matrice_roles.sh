#!/usr/bin/env bash
# Matrice de securite des roles du navigateur, par de vrais appels systeme
# (BOUCHAUD_MATRICE_ROLES_V1).
#
#   tools/ci/run_matrice_roles.sh BOOTIMAGE
#
# La meme sonde, copiee sous le nom de chaque service Ladybird, recoit du
# noyau le role de ce service (`security/profile.rs`) et tente pour de bon
# chaque operation : base SQL et reglages du profil (persistant et
# ephemere), cache HTTP, telechargements, historique, /usr, /persist,
# /dev/dsp, socket, exec. Tout ecart avec la politique attendue est un echec ;
# un refus doit etre EACCES/EPERM, jamais une simple absence (ENOENT).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_matrice_roles.sh BOOTIMAGE}
ROLES="WebContent WebWorker ImageDecoder Compositor RequestServer"

SCENARIO=scenario-matrice
IMAGE=matrice.img
LOG=serie-matrice.log
OUT=out-matrice
rm -rf "$SCENARIO" "$IMAGE" "$LOG" "tools/userland/$OUT"
(cd tools/userland && OUT=$OUT ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin" "$SCENARIO/usr/libexec/ladybird"
cp "tools/userland/$OUT/matrice-roles-probe" "$SCENARIO/bin/"
for r in $ROLES; do
  cp "tools/userland/$OUT/matrice-roles-probe" "$SCENARIO/usr/libexec/ladybird/$r"
done
{
  echo "/bin/matrice-roles-probe prepare"
  for r in $ROLES; do echo "/usr/libexec/ladybird/$r"; done
  echo "echo MATRICE_FIN"
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

: > "$LOG"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp 2 -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -audiodev none,id=muet -device AC97,audiodev=muet \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 300))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aq 'MATRICE_FIN\|KERNEL PANIC' "$LOG" && { sleep 1; break; }
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

# Nettoye UNE fois : sous `pipefail`, `sed | grep -q` echoue des que grep
# ferme le tube avant la fin (SIGPIPE), meme quand la ligne est trouvee.
sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG" | tr -d '\r' > "$LOG.propre"
propre() { cat "$LOG.propre"; }
propre | grep -aE '^matrice-roles-probe|^MATRICE' || true
echo "== refus de securite journalises par le noyau (attendus) =="
propre | grep -aoE '\[SECURITY-DENY\].*' | sed -E 's/seq=[0-9]+ //' | sort | uniq -c | sort -rn | head -30 || true

if grep -aq 'KERNEL PANIC' "$LOG"; then echo "panique noyau" >&2; exit 1; fi
ok=0
for r in $ROLES; do
  if grep -aq "MATRICE_ROLE_OK role=$r " "$LOG.propre"; then ok=$((ok + 1)); else echo "role $r : matrice en echec ou absente" >&2; fi
done
if [ "$ok" -ne 5 ]; then
  echo "MATRICE_ROLES_ECHEC roles_ok=$ok/5"
  exit 1
fi
echo "MATRICE_ROLES_OK roles=5 verifications=$(grep -ac "MATRICE role=" "$LOG.propre")"
