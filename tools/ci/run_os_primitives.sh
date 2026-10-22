#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_os_primitives.sh BOOTIMAGE}

rm -rf scenario-primitives primitives.img serie-primitives.log tools/userland/out-sondes
(cd tools/userland && OUT=out-sondes ./build.sh musl)
cd tools/userland/out-sondes
./verrous-probe
./exec-fd-probe
./wal-probe
./nom-long-probe
./sendfile-probe
./renommage-probe
./faute-noncanonique-probe
./faute-multifil-probe 1
./noeuds-churn-probe
./sigchld-multifil-probe
./sigmasque-fil-probe
./compta-stress-probe 5
./mtime-probe
./eof-fermeture-probe 1
./tube-atomicite-probe 300
./statut-sortie-probe
cd ../../..

SCENARIO=scenario-primitives
mkdir -p "$SCENARIO/bin"
for f in verrous-probe exec-fd-probe wal-probe disque-probe nom-long-probe session-probe sendfile-probe renommage-probe faute-noncanonique-probe faute-multifil-probe noeuds-churn-probe sigchld-multifil-probe sigmasque-fil-probe compta-stress-probe mtime-probe persist-course-probe eof-fermeture-probe tube-atomicite-probe statut-sortie-probe; do
  cp "tools/userland/out-sondes/$f" "$SCENARIO/bin/"
done
python3 - <<'PY'
bloc = (b"B" * 63 + b"\n") * 16384
with open("scenario-primitives/bin/gros.bin", "wb") as f:
    for _ in range(96):
        f.write(bloc)
PY
cat > "$SCENARIO/autorun" <<'AUTORUN'
strace echecs
/bin/verrous-probe
/bin/exec-fd-probe
/bin/wal-probe
/bin/disque-probe /bin/gros.bin
/bin/nom-long-probe
/bin/sendfile-probe
/bin/renommage-probe
/bin/faute-noncanonique-probe
/bin/faute-multifil-probe 2
/bin/noeuds-churn-probe
/bin/sigchld-multifil-probe
/bin/sigmasque-fil-probe
/bin/compta-stress-probe 30
/bin/mtime-probe
/bin/persist-course-probe
/bin/eof-fermeture-probe 2
/bin/tube-atomicite-probe 300
/bin/statut-sortie-probe
/bin/session-probe 4
echo SESSION_INVITE_REVENUE
strace off
echo PRIMITIVES_FIN
AUTORUN
(cd tools/userland && IMAGE="$PWD/../../primitives.img" ./mkdisk.sh "$PWD/../../scenario-primitives")

LOG=serie-primitives.log
: > "$LOG"
# BOUCHAUD_SYMBOLISE_NOYAU_V1 : en cas d'echec, ce que faisaient les coeurs,
# symbolise contre le noyau EXACT qui a tourne (les artefacts ne sont pas
# toujours lisibles ; le journal du job, si). Imprime EN DERNIER : l'API ne
# rend que la fin d'un journal de job.
ELF="$(dirname "$BOOT")/bouchaud-os"
# BOUCHAUD_PRIMITIVES_ECHEC_LISIBLE_V1 : les sondes mortes en echec et LEURS
# fautes, lues sur le journal ENTIER. Run 39455898970 (ca968270) :
# COMPTA_STRESS_OK absent, AUTORUN statut=139, et la mort de la sonde tombait
# avant les 256 Kio publies -- rien pour la classer. Les fautes volontaires
# des autres sondes (fils tues expres) ne sont pas recopiees : seulement
# celles des racines (ppid=0) mortes en code non nul.
racines_en_echec() {
  python3 - "$LOG" <<'PY'
import re, sys
lignes = re.sub(r"\x1b\[[0-9;]*m", "", open(sys.argv[1], errors="replace").read()).splitlines()
morts = [l for l in lignes if re.search(r"PROCESS_DEATH .*\bppid=0 ", l) and not re.search(r" code=0 ", l)]
pids = {m.group(1) for l in morts for m in [re.search(r"\bpid=(\d+)", l)] if m}
sortie = [l for l in lignes
          if l in morts
          or (lambda m: m is not None and m.group(1) in pids)(re.search(r"PROCESS_FAULT(?:_PILE|_DETAIL)? pid=(\d+)", l))
          or re.search(r"\b[A-Z_]+_(ECHEC|FAIL)\b|COMPTA_(RECUL|DEPASSE|STRESS) |AUTORUN FIN", l)]
print("== racines mortes en echec et leurs fautes (journal entier) ==")
for l in sortie[:40]:
    print("  " + l[:400])
PY
}
trap 'rc=$?; if [ "$rc" -ne 0 ]; then python3 tools/ci/symbolise_noyau.py "$LOG" "$ELF" || true; racines_en_echec || true; fi; exit "$rc"' EXIT
# BO_QEMU_KVM=1 : memes sondes sous KVM (-cpu host). La course que
# compta-stress-probe fabrique depend du rythme reel des vCPU : sous TCG les
# coeurs avancent par tranches emulees, sous KVM ils tournent en parallele
# et l'hote peut suspendre un vCPU au milieu d'une section (run 37589903681).
ACCEL=""
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "PRIMITIVES_ACCEL ${ACCEL:-tcg}"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file=primitives.img \
  -m 4096 -smp 4 -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" &
PID=$!
DEADLINE=$((SECONDS + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= DEADLINE )); then echo "ECHEANCE atteinte" >&2; break; fi
  sleep 0.5
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
tail -c 262144 "$LOG"

# BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : marqueurs, temoin de faute detaillee,
# statut de l'autorun, invariants de compta, panique, ATA -- un script a part,
# rejoue sur des journaux canoniques par les tests hote.
tools/ci/verdict_os_primitives.sh "$LOG"
# BOUCHAUD_PILE_DE_FAUTE_V1 : le fils `piege` de faute-noncanonique-probe
# meurt sur ud2 au bout de deux appels ; la pile publiee par le noyau a la
# faute, symbolisee contre la sonde, doit nommer son appelant. C'est la meme
# chaine qui nomme l'appelant d'un `ak_trap` de Ladybird. (Il faut les
# binaires du scenario : ce controle reste ici.)
python3 tools/ci/symbolise_fautes.py "$LOG" "$SCENARIO/bin" > pile-faute.log
grep -am1 -B2 -A8 'FAUTE_SYMBOLE .*faute-noncanonique' pile-faute.log || true
if ! grep -aF 'piege_niveau1' pile-faute.log; then
  # Ce que le noyau a publie pour le fils `piege`, et ce que l'outil en a
  # fait : de quoi trancher entre pile vide, image inconnue, ligne abimee.
  echo "PILE_DE_FAUTE_ECHEC piege_niveau1 absent de la pile symbolisee" >&2
  sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | awk '/PROCESS_FAULT(_PILE|_DETAIL)? pid=/ && n++ < 8 { print "  " substr($0, 1, 400) }' >&2
  awk '/FAUTE_SYMBOLE .*instruction illegale/ { on = 1 } on && n++ < 24 { print "  " $0 }' pile-faute.log >&2
  exit 1
fi
echo OS_PRIMITIVES_OK
