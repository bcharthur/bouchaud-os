#!/usr/bin/env bash
# ORDONNANCEUR-PROBE, ATTRIBUTION ET QUALIFICATION DES VERDICTS
# (BOUCHAUD_ATTRIBUTION_REVEIL_V1/V2, BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1).
#
#   tools/ci/run_ordonnanceur_attribution.sh BOOTIMAGE [PASSES] [SMP]
#
# Lance `ordonnanceur-probe 8` PASSES fois de suite (3 par defaut) dans un
# seul demarrage, a SMP coeurs (1 par defaut). Les criteres de la sonde sont
# INCHANGES. Chaque passe rend un verdict : QUALIFIE_OK, QUALIFIE_ECHEC ou
# NON_QUALIFIANT (raison), avec une ligne PREUVE chiffree par comparaison
# (jugee sur tous les reveils ET sur les seuls reveils propres) et la
# ventilation du noyau pour chaque phase.
#
# Exige toujours : les passes au bout, un verdict par passe, cinq preuves et
# trois ventilations par tentative, aucun QUALIFIE_ECHEC, aucune panique.
#
# Par defaut, un NON_QUALIFIANT n'est PAS une reussite : sortie 2,
# `ORDONNANCEUR_ATTRIBUTION_NON_QUALIFIANT`.
#
# BANC_TEMOIN_INSTRUMENT=1 : le banc juge l'INSTRUMENT, pas l'ordonnanceur.
# Un NON_QUALIFIANT explicite et prouve y est accepte (sortie 0), et le
# marqueur le dit : `ORDONNANCEUR_TEMOIN_INSTRUMENT ... non_qualifiant=N
# (ne qualifie pas P13)`. Seul `ORDONNANCEUR_ATTRIBUTION_OK` (toutes les
# passes QUALIFIE_OK) est une preuve sur l'ordonnanceur.
#
# BO_QEMU_KVM=1 : sous KVM, avec le temps vole publie par l'hote
# (BOUCHAUD_PREUVE_HOTE_V1) : chaque phase doit l'avoir (`preuve_hote=kvm-steal`),
# sans quoi rien n'est prouve externe et le banc echoue (defaut d'instrument).
# C'est la methode de preuve de P13 : sous KVM, un reveil n'est ecarte que si
# l'hote PROUVE l'avoir retarde ; tout le reste compte contre le noyau.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ordonnanceur_attribution.sh BOOTIMAGE [PASSES] [SMP]}
PASSES=${2:-3}
SMP=${3:-1}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host,kvm-steal-time=on"
  NOM=kvm
fi
SORTIE=${ORDO_SORTIE:-$PWD/ordonnanceur-attribution-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  echo "echo ORDO_ATTRIBUTION_DEBUT"
  # Diagnostic seulement : les compteurs du reveil (armements, echeances dues,
  # IPI vers une tache installee, emissions serie masquees) avant et apres les
  # passes. `reveil-precis` sans argument ne change aucune politique.
  echo "reveil-precis"
  for _ in $(seq "$PASSES"); do echo "/bin/ordonnanceur-probe 8"; done
  echo "reveil-precis"
  echo "echo ORDO_ATTRIBUTION_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ordo.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ordo.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
# Jusqu'a trois tentatives par passe (une NON_QUALIFIANTE est remesuree).
LIMITE=$((SECONDS + 120 + PASSES * 360))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  # La sortie de `echo`, pas sa trace (`+ echo ORDO_ATTRIBUTION_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'ORDO_ATTRIBUTION_FIN' | grep -av '+ echo ORDO_ATTRIBUTION_FIN' >/dev/null; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
propre() { sed -E 's/\x1b\[[0-9;]*m//g; s/^(\[[^]]*\]){2}\[FPS:[^]]*\] //' "$1"; }
propre "$LOG" | grep -aE '^  (au repos|sous charge)|^  attribution|^  ventilation|^  comptes|^  phase non jugeable|^  PREUVE|^  tentative|^  - |^VERDICT_SONDE|^RESULTAT|\[POINTS-COOP\]|REVEIL_PRECIS voulu=' | sed 's/^/  /' || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
compte() { propre "$LOG" | grep -acE "$1" || true; }
exige "les $PASSES passes sont allees au bout" grep -aq 'ORDO_ATTRIBUTION_FIN' "$LOG"
VERDICTS=$(compte '^VERDICT_SONDE : ')
OK=$(compte '^VERDICT_SONDE : QUALIFIE_OK')
ECHECS=$(compte '^VERDICT_SONDE : QUALIFIE_ECHEC')
NONQ=$(compte '^VERDICT_SONDE : NON_QUALIFIANT')
TENTATIVES=$(compte '^  tentative [0-9]+/[0-9]+ : ')
PREUVES=$(compte '^  PREUVE ')
VENTILEES=$(compte '^  ventilation [^:]+ : seuil_us=')
ATTRIBUEES=$(compte '^  attribution .* : reveils=')
exige "un verdict explicite par passe : $VERDICTS/$PASSES" test "$VERDICTS" = "$PASSES"
exige "cinq preuves chiffrees par tentative : $PREUVES/$((TENTATIVES * 5))" test "$PREUVES" = "$((TENTATIVES * 5))"
exige "le noyau a ventile chaque phase : $VENTILEES/$((TENTATIVES * 3))" test "$VENTILEES" = "$((TENTATIVES * 3))"
exige "le noyau a attribue chaque phase : $ATTRIBUEES/$((TENTATIVES * 3))" test "$ATTRIBUEES" = "$((TENTATIVES * 3))"
COMPTES=$(compte '^  comptes [^:]+ : preuve_hote=(kvm-steal|absente) ')
exige "les comptes du fil rapportes pour chaque phase (sommeils ranges a leur entree) : $COMPTES/$((TENTATIVES * 3))" test "$COMPTES" = "$((TENTATIVES * 3))"
PROUVEES=$(compte '^  comptes [^:]+ : preuve_hote=kvm-steal ')
if [ "$NOM" = kvm ]; then
  exige "sous KVM, la preuve de l'hote est publiee a chaque phase : $PROUVEES/$((TENTATIVES * 3))" test "$PROUVEES" = "$((TENTATIVES * 3))"
  PREUVE=kvm-steal
else
  PREUVE=absente
fi
exige "aucun echec qualifie : $ECHECS" test "$ECHECS" = 0
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
BILAN="passes=$PASSES smp=$SMP accel=$NOM preuve_hote=$PREUVE qualifie_ok=$OK non_qualifiant=$NONQ qualifie_echec=$ECHECS tentatives=$TENTATIVES"
if [ "$echecs" -ne 0 ]; then
  echo "ORDONNANCEUR_ATTRIBUTION_ECHEC n=$echecs $BILAN"
  exit 1
fi
if [ "$OK" = "$PASSES" ]; then
  echo "ORDONNANCEUR_ATTRIBUTION_OK $BILAN"
elif [ "${BANC_TEMOIN_INSTRUMENT:-0}" = 1 ]; then
  echo "ORDONNANCEUR_TEMOIN_INSTRUMENT $BILAN (verdicts explicites et prouves ; ne qualifie pas P13)"
else
  echo "ORDONNANCEUR_ATTRIBUTION_NON_QUALIFIANT $BILAN"
  exit 2
fi
