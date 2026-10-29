#!/usr/bin/env bash
# C5 (P13_PAGE_REMPLIE_AVANT_PUBLICATION_V1) : banc canonique
# `run_faute_persistance.sh`, rouges comptes et JUGES contre l'attente.
#
#   tools/ci/run_c5_cible.sh IMAGE REPETITIONS rouge|vert SORTIE
#
# `rouge` (image `banc-c5-ordre-historique`, reproduction negative) : au moins
# un tour rouge, et chaque rouge porte la signature C5 (faute inattendue dans
# persist-course, PERSIST_COURSE_OK manquant). `vert` (image livree, ou
# `banc-c5-fenetre-corrigee`) : tous les tours verts. BO_QEMU_KVM=1 : sous KVM.
set -uo pipefail
cd "$(dirname "$0")/../.."
IMAGE=${1:?image} N=${2:?repetitions} ATTENDU=${3:?rouge|vert} SORTIE=${4:?sortie}
[[ $ATTENDU == rouge || $ATTENDU == vert ]] || exit 2
FAUTE_PERSIST_SORTIE="$SORTIE" bash tools/ci/run_faute_persistance.sh "$IMAGE" "$N" > "$SORTIE.txt" 2>&1 || true
verts=$(grep -c 'FAUTE_PERSISTANCE_TOUR_OK' "$SORTIE.txt" || true)
rouges=$(grep -c 'FAUTE_PERSISTANCE_TOUR_ECHEC' "$SORTIE.txt" || true)
signes=$(grep -c "manque=\['PERSIST_COURSE_OK'\] fatal=0 faute_inattendue=1" "$SORTIE.txt" || true)
accel=tcg; [[ ${BO_QEMU_KVM:-0} == 1 ]] && accel=kvm
echo "C5_CIBLE accel=$accel image=$(sha256sum "$IMAGE" | cut -c1-16) tours=$N verts=$verts rouges=$rouges rouges_signature_c5=$signes attendu=$ATTENDU"
# Chaque tour rouge et son verdict, dans le journal du job : un rouge sans
# la signature C5 doit pouvoir s'attribuer sans l'artefact (KVM cible
# 40822176701 : temoin 4 rouges, 3 signes, verdict du 4e invisible).
awk '/FAUTE_PERSISTANCE_TOUR_OK/{n=0} /FAUTE_PERSISTANCE_TOUR_ECHEC/{n=12} n>0{print "  rouge : " $0; n--}' "$SORTIE.txt" | head -60 || true
grep -rahE 'PROCESS_FAULT|KERNEL PANIC|panicked at' "$SORTIE" 2>/dev/null | sed -E 's/\x1b\[[0-9;]*m//g' | tr -d '\r' | sort | uniq -c | sort -rn | head -12 | sed 's/^/  faute : /' || true
if [[ $ATTENDU == rouge ]]; then
  [[ $rouges -ge 1 && $signes -eq $rouges && $((verts + rouges)) -eq $N ]] && { echo C5_CIBLE_OK; exit 0; }
else
  [[ $verts -eq $N && $rouges -eq 0 ]] && { echo C5_CIBLE_OK; exit 0; }
fi
echo C5_CIBLE_ECHEC
exit 1
