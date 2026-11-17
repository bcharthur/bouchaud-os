#!/usr/bin/env bash
# TEMPETE D'EXECVE (BOUCHAUD_TEMPETE_EXEC_V1) -- fuite de processus et gel.
#
#   tools/ci/run_tempete_exec.sh BOOTIMAGE [SECONDES_COMPLET]
#
# Un demarrage, trois passages de tempete-exec-probe (tools/userland), qui
# rejoue le motif de la batterie de workers de Ladybird : un processus
# multi-fils enchaine fork (+ execve), chaque enfant faute, protege
# (mprotect), projette et cree un fil, puis sort :
#
#   fork     15 s : fork + _exit, sans execve
#   vide     15 s : fork + execve + sortie immediate
#   complet  SECONDES_COMPLET (40) : fork + execve + fil + memoire
#
# Exige :
#   1. chaque passage imprime TEMPETE_EXEC_FIN ... echecs=0 ;
#   2. aucune panique, aucune ligne [DIAG-EN-SOUFFRANCE] (noyau fige) ;
#   3. AUCUNE FUITE DE PROCESSUS : au dernier [PROC-STAT], le nombre de
#      processus encore en memoire (`processus_crees - processus_detruits`)
#      ne depasse pas 64 apres plusieurs milliers de cycles
#      (BOUCHAUD_PROCESSUS_DETRUITS_V1). Avant BOUCHAUD_SORTIE_SANS_REFERENCE_V1,
#      un processus sorti par exit_group, ou ayant fait execve, n'etait JAMAIS
#      detruit : 7 700 crees, 3 544 detruits, RAM pleine en ~2 000 cycles,
#      puis fautes de page `erreur_es` et fork en echec ;
#   4. la memoire physique (prefixe des lignes) n'a pas monte de plus de 10
#      points entre le debut et la fin.
#
# Sur un gel, le rapport automatique est imprime (tools/ci/reliability/
# rapport_gel.py), symbolise avec le noyau ELF du meme build s'il est present.
#
# SILENCE TOTAL (BOUCHAUD_TEMPETE_SILENCE_V1). Un noyau fige interruptions
# masquees sur le coeur 0 n'emet meme plus la ligne [DIAG-EN-SOUFFRANCE] :
# la sortie serie s'arrete au milieu d'une ligne et QEMU tourne a 170 % (vu
# localement le 17/11, main@65f1e65 et cbf2d0a, apres ~1 min de tempete). Le
# banc le declare apres SILENCE_S secondes hote sans un octet (90), releve
# alors l'etat de chaque vCPU par le moniteur QEMU (tools/ci/
# releve_cpus_qemu.py, s'il est present) AVANT d'arreter QEMU, et imprime la
# fin du journal serie dans la sortie du job : un echec porte ses preuves,
# meme quand l'artefact n'est pas lisible.
#
# BO_SMP : nombre de coeurs (4). BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_tempete_exec.sh BOOTIMAGE [SECONDES_COMPLET]}
COMPLET=${2:-40}
SMP=${BO_SMP:-4}
OUT=${OUT:-tempete-exec}
rm -rf "$OUT" tools/userland/out-tempete-exec
mkdir -p "$OUT/scenario/bin"
OUT=$(cd "$OUT" && pwd)
SCENARIO="$OUT/scenario"
IMAGE="$OUT/disque.img"
LOG="$OUT/serie.log"
MONITEUR="$OUT/moniteur.sock"
# Un chemin de socket UNIX tient en 107 octets.
if [ "${#MONITEUR}" -ge 100 ]; then MONITEUR=$(mktemp -u /tmp/bo-tempete-XXXXXX.sock); fi
SILENCE_S=${SILENCE_S:-90}
(cd tools/userland && OUT=out-tempete-exec ./build.sh musl >/dev/null)
cp tools/userland/out-tempete-exec/tempete-exec-probe "$SCENARIO/bin/"
cat > "$SCENARIO/autorun" <<EOF
echo TEMPETE_BANC_DEBUT
/bin/tempete-exec-probe 15 3 fork
/bin/tempete-exec-probe 15 3 vide
/bin/tempete-exec-probe ${COMPLET} 3 complet
echo TEMPETE_BANC_FIN
EOF
(cd tools/userland && IMAGE="$IMAGE" ./mkdisk.sh "$SCENARIO" >/dev/null)

ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "TEMPETE_EXEC_BANC accel='${ACCEL}' smp=${SMP} complet=${COMPLET}s"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT",snapshot=on \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp "$SMP" -display none -no-reboot \
  -monitor unix:"$MONITEUR",server,nowait \
  -serial file:"$LOG" &
PID=$!
# Borne hote : le temps invite avance plus lentement sous TCG. Un gel est
# declare bien avant (deux lignes de souffrance, ~65 s apres le dernier
# service de diag-noyau).
LIMITE=$((SECONDS + (30 + COMPLET) * 8 + 300))
etat=echeance
propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$LOG"; }
# La SORTIE d'`echo TEMPETE_BANC_FIN`, pas sa trace (`+ echo ...`) ; awk lit
# tout son flux (pas de lecteur tronquant sous pipefail).
fin_vue() { awk '/TEMPETE_BANC_FIN/ && !/\+ echo/ { vu = 1 } END { exit !vu }' <<<"$texte"; }
taille_vue=-1
depuis=$SECONDS
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then break; fi
  taille=$(stat -c %s "$LOG" 2>/dev/null || echo 0)
  if [ "$taille" != "$taille_vue" ]; then
    taille_vue=$taille; depuis=$SECONDS
  elif (( SECONDS - depuis >= SILENCE_S )); then
    etat=silence; break
  fi
  texte=$(propre)
  if grep -aqE 'KERNEL PANIC|panicked at' <<<"$texte"; then etat=panique; sleep 2; break; fi
  if [ "$(grep -acF '[DIAG-EN-SOUFFRANCE]' <<<"$texte" || true)" -ge 2 ]; then etat=fige; break; fi
  if fin_vue; then
    # Laisser passer un releve [PROC-STAT] posterieur a la fin.
    sleep 12; etat=fini; break
  fi
  sleep 2
done
if [ "$etat" = silence ] || [ "$etat" = echeance ] || [ "$etat" = fige ]; then
  # QEMU tourne encore : l'etat des vCPU, AVANT de l'arreter.
  if [ -f tools/ci/releve_cpus_qemu.py ] && [ -S "$MONITEUR" ]; then
    python3 tools/ci/releve_cpus_qemu.py "$MONITEUR" "$LOG.cpus" || true
  else
    echo "QEMU_CPUS_ABSENT raison=outil_ou_moniteur_absent"
  fi
fi
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
texte=$(propre)
if [ "$etat" = echeance ] && fin_vue; then
  etat=fini   # QEMU s'est eteint de lui-meme apres l'autorun
elif [ "$etat" = echeance ] && [ ! -s "$LOG" ] && (( SECONDS < LIMITE )); then
  # QEMU mort sans un octet serie avant l'echeance : il n'a jamais demarre
  # (image verrouillee, option refusee...). Ni un gel, ni un budget depasse.
  etat=qemu_absent
fi

grep -aoE 'TEMPETE_EXEC_FIN mode=[a-z]+ cycles=[0-9]+ echecs=[0-9]+' <<<"$texte" || true
echo "TEMPETE_EXEC_ETAT etat=${etat}"
echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }

case "$etat" in
  fini) ok "le banc est alle au bout" ;;
  panique) echec "panique noyau"; grep -aE 'KERNEL PANIC|panicked at' <<<"$texte" | sed -n 1,3p ;;
  fige) echec "noyau fige ([DIAG-EN-SOUFFRANCE])" ;;
  silence) echec "silence total : aucun octet serie depuis ${SILENCE_S} s, QEMU vivant" ;;
  qemu_absent) echec "QEMU n'a jamais demarre (environnement, pas le noyau) : voir son message ci-dessus" ;;
  *) echec "echeance hote sans fin ni gel declare" ;;
esac
if [ "$etat" != fini ]; then
  echo "---- fin du journal serie (60 lignes) ----"
  awk -v n="$(wc -l <<<"$texte")" 'NR > n - 60 { print substr($0, 1, 240) }' <<<"$texte"
  echo "---- lignes [DIAG-EN-SOUFFRANCE] ----"
  awk '/\[DIAG-EN-SOUFFRANCE\]/ { print substr($0, 1, 600) }' <<<"$texte"
  echo "---- derniers [PROC-STAT] ----"
  awk '/\[PROC-STAT\]/ { l[NR % 8] = substr($0, 1, 240) } END { for (i in l) print l[i] }' <<<"$texte"
fi
if [ "$etat" = fige ] || [ "$etat" = echeance ] || [ "$etat" = silence ]; then
  if [ -f tools/ci/reliability/rapport_gel.py ]; then
    python3 tools/ci/reliability/rapport_gel.py "$LOG" target/x86_64-bouchaud_os/debug/bouchaud-os || true
  fi
fi

for mode in fork vide complet; do
  ligne=$(grep -aoE "TEMPETE_EXEC_FIN mode=${mode} cycles=[0-9]+ echecs=[0-9]+" <<<"$texte" | tail -1 || true)
  if [[ "$ligne" =~ cycles=([0-9]+)\ echecs=0$ ]] && (( BASH_REMATCH[1] > 0 )); then
    ok "mode ${mode} : ${ligne#*mode=}"
  else
    echec "mode ${mode} : ${ligne:-aucune fin}"
  fi
done

stat=$(grep -aoE 'vivantes=[0-9]+ releves_fautes_manques=[0-9]+ processus_crees=[0-9]+ processus_detruits=[0-9]+' <<<"$texte" | tail -1 || true)
if [[ "$stat" =~ vivantes=([0-9]+).*processus_crees=([0-9]+)\ processus_detruits=([0-9]+) ]]; then
  vivantes=${BASH_REMATCH[1]}; crees=${BASH_REMATCH[2]}; detruits=${BASH_REMATCH[3]}
  en_memoire=$((crees - detruits))
  # Borne absolue : `vivantes` compte des TACHES, pas des processus. Un
  # systeme sain garde ~20 processus en memoire apres le banc ; la fuite en
  # laissait un par cycle (plusieurs milliers ici).
  if (( en_memoire <= 64 )); then
    ok "aucune fuite de processus : ${crees} crees, ${detruits} detruits, ${en_memoire} en memoire pour ${vivantes} vivants"
  else
    echec "fuite de processus : ${crees} crees, ${detruits} detruits, ${en_memoire} en memoire pour ${vivantes} vivants"
  fi
else
  echec "aucun releve [PROC-STAT] avec processus_crees/detruits"
fi

memoire() { grep -aE "$1" <<<"$texte" | grep -av '+ echo' | sed -nE 's/^\[[0-9:]+\]\[ *[0-9]+%: *([0-9]+)%.*/\1/p' | sed -n "$2"; }
m0=$(memoire 'TEMPETE_BANC_DEBUT' 1p)
m1=$(memoire 'TEMPETE_BANC_FIN' '$p')
if [ -n "$m0" ] && [ -n "$m1" ]; then
  if (( m1 <= m0 + 10 )); then ok "memoire physique ${m0}% -> ${m1}%"
  else echec "memoire physique ${m0}% -> ${m1}% (+$((m1 - m0)) points)"; fi
else
  echec "memoire physique illisible (debut='${m0}' fin='${m1}')"
fi

if (( echecs > 0 )); then
  echo "TEMPETE_EXEC_ECHEC echecs=${echecs}"
  exit 1
fi
echo "TEMPETE_EXEC_OK smp=${SMP}"
