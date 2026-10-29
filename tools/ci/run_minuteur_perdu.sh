#!/usr/bin/env bash
# P13_MINUTEUR_PERIME_V1 (C23) -- un tir perdu du minuteur local ne le tue pas.
#
#   tools/ci/run_minuteur_perdu.sh BOOTIMAGE_DE_BANC [DUREE_MS]
#
# Image construite avec `--features banc-minuteur-perdu` (correctif) ou
# `--features banc-minuteur-historique` (temoin) : l'injection est la seule
# autre ecriture du MSR, jamais dans la livree.
#
# Confirmation stricte 40812095472 (2d850401, KVM SMP1, A/B reveil precis,
# tour 4 `on`) : `deadline_tsc=0 arme_tsc=286824508404` fige d'un releve a
# l'autre, `tirs=+0 refus_quantum_avant=+4656`. Le minuteur avait tire (ou
# perdu son tir) sans etre rearme ; tout armement precis se jugeait contre
# cette echeance morte, le quantum ne se rearme qu'au vecteur : le coeur ne
# vivait plus que du PIT.
#
# `serie-minuteur-perdu` met le minuteur du coeur zero dans CET etat (MSR a 0,
# echeance logicielle passee), verifie 20 ms plus tard, sans dormir, qu'il a
# tenu, puis dort un tick a la fois DUREE_MS ms (3000 par defaut). Pendant ce
# temps, le shell attend `/bin/ordonnanceur-probe 8 1`, la sonde de l'A/B ou
# la panne a ete vue : le coeur zero passe par l'inactivite de l'ordonnanceur
# (le shell qui dort hors ordonnanceur n'y passe pas -- premier essai KVM
# 40819500995 : `releves=0 tirs=0`, minuteur ranime par un autre chemin).
# Exige :
#   * l'etat injecte a tenu (`injection_tenue=1`, aucun quantum pendant) --
#     sinon NON_QUALIFIANT (code 2), jamais vert ;
#   * le minuteur est releve (`releves>=1`), le quantum bat de nouveau
#     (`quantums>0`), les tirs precis reprennent (`tirs>0`), il est vivant a
#     la fin.
# Image temoin `banc-minuteur-historique` : ces exigences DOIVENT echouer
# (code 1 : tirs=0, quantums=0, minuteur mort a la fin).
# Code 0 : OK ; 1 : ECHEC ; 2 : NON_QUALIFIANT. TSC-deadline requis : KVM.
# BO_SMP (1 par defaut), BO_QEMU_KVM=1. BANC_TENTATIVES=N (1..3) : seul un
# NON_QUALIFIANT est remesure ; chaque essai est garde (minuteur-perdu-tN.log).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_minuteur_perdu.sh BOOTIMAGE [DUREE_MS]}
DUREE=${2:-3000}
TENTATIVES=${BANC_TENTATIVES:-1}
case "$TENTATIVES" in 1|2|3) ;; *) echo "BANC_TENTATIVES=$TENTATIVES hors de 1..3" >&2; exit 1;; esac
if [ "$TENTATIVES" -gt 1 ]; then
  for t in $(seq 1 "$TENTATIVES"); do
    set +e
    BANC_TENTATIVES=1 "$PWD/tools/ci/run_minuteur_perdu.sh" "$BOOT" "$DUREE"
    code=$?
    set -e
    cp minuteur-perdu.log "minuteur-perdu-t$t.log" 2>/dev/null || true
    echo "MINUTEUR_PERDU_TENTATIVE t=$t/$TENTATIVES code=$code"
    if [ "$code" != 2 ]; then exit "$code"; fi
  done
  echo "MINUTEUR_PERDU_NON_QUALIFIANT tentatives=$TENTATIVES (injection jamais tenue)"
  exit 2
fi
SMP=${BO_SMP:-1}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host,kvm-steal-time=on"
fi
SCENARIO=scenario-minuteur-perdu
IMAGE=minuteur-perdu.img
LOG=minuteur-perdu.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG"
mkdir -p "$SCENARIO/bin"
(cd tools/userland && OUT=out-minuteur-perdu ./build.sh musl >/dev/null)
cp tools/userland/out-minuteur-perdu/ordonnanceur-probe "$SCENARIO/bin/"
{
  echo 'reveil-precis on'
  echo "serie-minuteur-perdu $DUREE"
  echo '/bin/ordonnanceur-probe 8 1'
  echo 'serie-banc-attend minuteur'
  echo 'echo MINUTEUR_PERDU_AUTORUN_FIN'
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)
grep -aq 'BOUCHAUD_IMAGE_TEMOIN feature=banc-minuteur-perdu' "$BOOT" \
  || { echo "MINUTEUR_PERDU : image sans l'option banc-minuteur-perdu" >&2; exit 2; }
echo "MINUTEUR_PERDU smp=$SMP accel=$ACCEL duree_ms=$DUREE"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$IMAGE" \
  -m 1024 -smp "$SMP" -display none -no-reboot ${BO_QEMU_EXTRA:-} -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 240))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  grep -aqE 'SERIE_BANC_ATTEND_(OK|ECHEC) type=minuteur|MINUTEUR_PERDU_AUTORUN_FIN' "$LOG" && { sleep 1; break; }
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true; sleep 1; kill -KILL "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true

propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r'; }
ligne=$(propre | grep -ao 'SERIE_MINUTEUR_PERDU coeur=.*' | sed -n 1p || true)
champ() { echo "$ligne" | grep -oE "(^| )$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
temoin=$(propre | grep -ao 'BOUCHAUD_IMAGE_TEMOIN feature=[a-z0-9-]*' | sed 's/.*feature=//' | sort -u | tr '\n' ',' || true)
echo "IMAGE sha256=$(sha256sum "$BOOT" | cut -c1-64) build=$(propre | grep -ao 'BOUCHAUD_BUILD commit=[^ ]*' | sed -n 1p | sed 's/.*commit=//') temoin=${temoin:-aucun}"
echo "${ligne:-SERIE_MINUTEUR_PERDU absent}"
propre | grep -aoE '(SERIE_BANC_ATTEND_[A-Z]+|SERIE_MINUTEUR_PERDU_ECHEC|ORDONNANCEUR_[A-Z_]+|MINUTEUR_PERDU_AUTORUN_FIN)( [^ ]*){0,4}' | sed 's/^/  serie : /' | head -8 || true

# 1. Conditions de reproduction : l'etat observe a ete injecte et a tenu.
nq=""
grep -aq 'SERIE_BANC_ATTEND_OK type=minuteur' "$LOG" || nq="$nq barriere-minuteur"
[ -n "$ligne" ] || nq="$nq ligne-absente"
[ "$(champ injection_tenue)" = 1 ] || nq="$nq injection-non-tenue"
[ "$(champ quantums_pendant_injection)" = 0 ] || nq="$nq quantum-pendant-injection"
if grep -aqE 'KERNEL PANIC|panicked at' "$LOG"; then
  nq=""  # une panique est un ECHEC, jamais un non-qualifiant
elif [ -n "$nq" ]; then
  echo "MINUTEUR_PERDU_NON_QUALIFIANT raisons=${nq# }"
  exit 2
fi

# 2. Exigences du correctif.
echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "le minuteur mort est releve (releves >= 1)" test "$(champ releves)" -ge 1
exige "le quantum bat de nouveau (quantums > 0)" test "$(champ quantums)" -gt 0
exige "les tirs precis reprennent (tirs > 0)" test "$(champ tirs)" -gt 0
exige "le minuteur est vivant a la fin" test "$(champ vivant_a_la_fin)" = 1
if [ "$echecs" -eq 0 ]; then
  echo "MINUTEUR_PERDU_OK tirs=$(champ tirs) quantums=$(champ quantums) releves=$(champ releves)"
else
  echo "MINUTEUR_PERDU_ECHEC n=$echecs tirs=$(champ tirs) quantums=$(champ quantums) releves=$(champ releves)"
  exit 1
fi
