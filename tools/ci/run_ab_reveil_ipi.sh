#!/usr/bin/env bash
# A/B : VARIANTE EXPERIMENTALE D'IPI AU REVEIL D'UNE INTERACTIVE
# (BOUCHAUD_REVEIL_IPI_INTERACTIF_EXPERIMENTAL), CONTRE LA POLITIQUE LIVREE.
#
#   tools/ci/run_ab_reveil_ipi.sh BOOTIMAGE [TOURS] [SMP]
#
# Un seul demarrage, TOURS tours (6 par defaut). Chaque tour passe
# `ordonnanceur-probe 8 1` sous les deux politiques, dans un ORDRE ALTERNE
# (BOUCHAUD_AB_REVEIL_IPI_V2) : tour impair defaut puis variante (AB), tour
# pair variante puis defaut (BA) -- la position dans le tour (calcul deja
# chaud, journal deja rempli) ne favorise aucun bras. Une seule tentative par
# passe : aucune remesure ne favorise un bras. Le demarrage finit sous
# `defaut`.
#
# QUELLES PASSES COMPTENT. Seule une passe QUALIFIEE (QUALIFIE_OK ou
# QUALIFIE_ECHEC : ses phases sont jugeables, preuve de l'hote a l'appui sous
# KVM) entre dans les chiffres. Une passe NON_QUALIFIANT n'est JAMAIS comptee
# pour aucun bras. Les criteres se calculent sur les TOURS APPARIES -- les deux
# bras qualifies dans le meme tour --, et il en faut au moins QUORUM
# (max(3, 2/3 des tours)) ; sinon le resultat est NON_CONCLUANT, pas un
# verdict.
#
# CRITERES DECLARES AVANT LA MESURE (une politique par defaut ne change
# qu'avec un A/B KVM CONVAINCANT sur les cinq, quorum atteint) :
#   latence     mediane des p99 interactifs PROPRES : variante <= 0,8 x defaut,
#               et mediane des pires interactifs propres : variante <= defaut ;
#   debit       mediane des tours de calcul pendant la phase interactive :
#               variante >= 0,95 x defaut ;
#   equite      mediane des tours pendant la phase NORMALE (ou la variante ne
#               doit rien changer) : ecart <= 5 % ;
#   surete      `ipi_variante` reste a zero sous `defaut`, avance sous la
#               variante ; aucune panique ;
#   robustesse  chaque passe rend un verdict explicite ; aucun QUALIFIE_ECHEC
#               sous la variante.
#
# Le banc ECHOUE (sortie 1) sur un defaut d'instrument ou de surete : passe
# sans verdict, IPI de variante sous `defaut`, variante jamais exercee,
# panique, et sous KVM une passe sans preuve de l'hote (`preuve_hote` doit
# valoir kvm-steal). Le resultat de l'experience -- CONVAINCANT,
# NON_CONVAINCANT ou NON_CONCLUANT -- est une MESURE, imprimee avec ses
# chiffres : il ne change rien seul.
#
# BO_QEMU_KVM=1 : sous KVM (le seul A/B qui compte pour la decision).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ab_reveil_ipi.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-6}
SMP=${3:-8}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  # BOUCHAUD_PREUVE_HOTE_V1 : le temps vole publie par l'hote.
  ACCEL="-enable-kvm -cpu host,kvm-steal-time=on"
  NOM=kvm
fi
SORTIE=${AB_SORTIE:-$PWD/ab-reveil-ipi-$NOM-smp$SMP}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  echo "echo AB_REVEIL_DEBUT"
  for t in $(seq "$TOURS"); do
    # AB aux tours impairs, BA aux tours pairs.
    if [ $((t % 2)) = 1 ]; then ordre="defaut ipi-interactive"; else ordre="ipi-interactive defaut"; fi
    for v in $ordre; do
      echo "ordo-variante $v"
      echo "echo AB_BRAS tour=$t variante=$v"
      echo "/bin/ordonnanceur-probe 8 1"
      echo "ordo-variante"
    done
  done
  echo "ordo-variante defaut"
  echo "echo AB_REVEIL_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ab.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ab.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 150))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'AB_REVEIL_FIN|KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

python3 tools/ci/analyse_ab_reveil_ipi.py "$LOG" "$TOURS" "$SMP" "$NOM"
