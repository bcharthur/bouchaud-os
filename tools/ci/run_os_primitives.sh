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

# BOUCHAUD_TSC_SOURCE_V1 : la frequence du TSC retenue, sa source, et le
# controle par les ticks PIT (KVM : horloge monotone 4,5 fois trop rapide).
# AVANT les verdicts : un marqueur absent arrete le script (set -e), et
# c'est justement quand un banc echoue qu'on a besoin de l'horloge.
awk '{ gsub(/\x1b\[[0-9;]*m/, ""); gsub(/\r/, "") } match($0, /BOUCHAUD_TSC_(EARLY_CALIBRATION_OK|CONTROLE) .*/) { print substr($0, RSTART, RLENGTH) }' "$LOG"
for marker in VERROUS_POSIX_OK EXEC_FD_OK WAL_PROBE_OK DISQUE_PROBE_OK NOM_LONG_OK SENDFILE_OK RENOMMAGE_OK FAUTE_NONCANONIQUE_OK 'FAUTE_MULTIFIL_OK essais=16' 'NOEUDS_CHURN_OK cycles=8000' SIGCHLD_MULTIFIL_OK SIGMASQUE_FIL_OK COMPTA_STRESS_OK MTIME_STABLE_OK PERSIST_COURSE_OK 'EOF_FERMETURE_OK cas=6 tours=2' 'TUBE_ATOMICITE_OK tailles=3 ecrivains=4 tours=300' 'STATUT_SORTIE_OK cas=11' \
              'SESSION_PERE_SORT fils=4' SESSION_INVITE_REVENUE PRIMITIVES_FIN; do
  grep -aF "$marker" "$LOG"
done
# BOUCHAUD_PILE_DE_FAUTE_V1 : le fils `piege` de faute-noncanonique-probe
# meurt sur ud2 au bout de deux appels ; la pile publiee par le noyau a la
# faute, symbolisee contre la sonde, doit nommer son appelant. C'est la meme
# chaine qui nomme l'appelant d'un `ak_trap` de Ladybird.
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
# BOUCHAUD_FAUTE_DETAIL_V1 : le meme fils `piege` meurt sur ud2 (0f 0b) ; la
# ligne de detail doit nommer CES octets a son rip. Sans ce temoin, une ligne
# qui lirait la mauvaise adresse (ou rien) passerait inapercue jusqu'a la
# prochaine faute S1 -- celle qu'elle doit justement classer.
if ! sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aqE 'PROCESS_FAULT_DETAIL pid=[0-9]+ tid=[0-9]+ cpu=[0-9]+ fs_base=0x[0-9a-f]+ fs_base_range=0x[0-9a-f]+ insn=0f0b'; then
  echo "FAUTE_DETAIL_ECHEC aucune ligne de detail ne nomme l'ud2 du fils piege" >&2
  sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'PROCESS_FAULT_DETAIL' | head -8 >&2 || true
  exit 1
fi
# BOUCHAUD_COMPTA_STRESS_V1 : ce que la sequence a du refaire (preuve que la
# course a eu lieu), lu sur le dernier releve du noyau.
awk 'match($0, /compta_relues=[0-9]+/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "compta_relues=absent") }' "$LOG"
# BOUCHAUD_GS_NOYAU_EN_IRQ_V1 : recalculs de l'identite du coeur par CPUID.
awk 'match($0, /replis_apic=[0-9]+( ticks_ms=[0-9]+ mono_ms=[0-9]+)?/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "replis_apic=absent") }' "$LOG"
if grep -aq 'KERNEL PANIC' "$LOG"; then echo "panique noyau" >&2; exit 1; fi
if grep -aqE "ata: (lecture|ecriture) " "$LOG"; then
  echo "Le pilote ATA a signale au moins une commande en echec" >&2
  grep -aE "ata: (lecture|ecriture) " "$LOG" >&2
  exit 1
fi
echo OS_PRIMITIVES_OK
