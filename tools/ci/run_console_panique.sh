#!/usr/bin/env bash
# UNE PANIQUE N'EMPORTE PAS LES LIGNES QUI ATTENDAIENT (BOUCHAUD_CONSOLE_ASYNCHRONE_V1).
#
#   tools/ci/run_console_panique.sh BOOTIMAGE [LIGNES]
#
# Console asynchrone : une ligne attend dans un anneau que le fil `console`
# l'emette. Le banc remplit l'anneau (`console-async rafale N`, N lignes
# numerotees d'un trait) puis declenche une panique (`panic-test`) aussitot.
# Exige : la panique, et AVANT son message, les N lignes -- chacune entiere,
# dans l'ordre, sans doublon. BANC_CONSOLE=off : la meme chose console
# synchrone (temoin du chemin historique).
#
# L'autorun tourne dans le contexte de demarrage, qui ne cede pas le coeur : a
# SMP1, le fil `console` n'a pas encore tourne et la console est encore
# synchrone (`active=0`) -- le banc le dit, et exige la meme integrite. A
# SMP >= 2, `active=1` est exige en mode `on`.
#
# BO_SMP : coeurs (2 par defaut). BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_console_panique.sh BOOTIMAGE [LIGNES]}
LIGNES=${2:-600}
SMP=${BO_SMP:-2}
MODE=${BANC_CONSOLE:-on}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi

SCENARIO=scenario-console-panique
IMAGE=console-panique.img
LOG=console-panique.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG"
mkdir -p "$SCENARIO"
printf 'console-async %s\nconsole-async etat\nconsole-async rafale %s\npanic-test\n' "$MODE" "$LIGNES" > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

echo "CONSOLE_PANIQUE smp=${SMP} accel=${ACCEL} console=${MODE} lignes=${LIGNES}"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 1024 -smp "$SMP" -display none -no-reboot \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 300))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  if grep -aqE 'panicked at|KERNEL PANIC' "$LOG"; then sleep 3; break; fi
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

python3 - "$LOG" "$LIGNES" "$MODE" "$SMP" <<'EOF'
import re, sys
log, n, mode, smp = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4])
texte = re.sub(r"\x1b\[[0-9;]*m", "", open(log, encoding="utf-8", errors="replace").read()).replace("\r", "")
lignes = texte.split("\n")
fautes = []
panique = next((i for i, l in enumerate(lignes) if "panic-test demande par l'utilisateur root" in l), None)
if panique is None:
    fautes.append("message de panique absent")
    panique = len(lignes)
bilan = [l for l in lignes if "BOUCHAUD_CONSOLE_BILAN" in l]
attendu = "active=1" if mode == "on" else "active=0"
etat = (re.search(r"active=(\d)", bilan[0]).group(0) if bilan else "absent")
if not bilan or (attendu not in bilan[0] and not (mode == "on" and smp == 1)):
    fautes.append(f"console-async etat ne dit pas {attendu} : {bilan[:1]}")
motif = re.compile(r"\] CONSOLE_RAFALE (\d+)/" + str(n) + r" ={48}$")
vus, abimees, apres = [], 0, 0
for i, l in enumerate(lignes):
    if "CONSOLE_RAFALE" not in l:
        continue
    m = motif.search(l)
    if not m:
        abimees += 1
        continue
    if i > panique:
        apres += 1
    vus.append(int(m.group(1)))
if vus != list(range(1, n + 1)):
    manquants = sorted(set(range(1, n + 1)) - set(vus))
    fautes.append(f"rafale incomplete ou desordonnee : {len(vus)}/{n}, manquants {manquants[:10]}, "
                  f"doublons {len(vus) - len(set(vus))}")
if abimees:
    fautes.append(f"{abimees} ligne(s) de rafale abimee(s)")
if apres:
    fautes.append(f"{apres} ligne(s) de rafale APRES le message de panique")
if fautes:
    print("CONSOLE_PANIQUE_ECHEC " + " ; ".join(fautes))
    sys.exit(1)
print(f"CONSOLE_PANIQUE_OK console={mode} {etat} lignes={n}/{n} avant_la_panique=1 abimees=0")
EOF
