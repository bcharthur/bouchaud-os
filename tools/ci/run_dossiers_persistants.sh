#!/usr/bin/env bash
# Dossiers personnels et ecriture differee, sous coupure (BOUCHAUD_DOSSIERS_PERSISTANTS_V1).
#
#   tools/ci/run_dossiers_persistants.sh BOOTIMAGE [SORTIE]
#
# UN disque, DEUX demarrages.
#
# Demarrage 1 : le shell cree, SANS fsync ni sync, un fichier, un dossier vide
# et un fichier vide aux droits 0600 sous /persist, puis lance le bureau, qui
# prepare le dossier personnel (/persist/home/root et ses trois sous-dossiers).
# Apres la premiere trame et un delai superieur au plafond d'ecriture differee,
# QEMU est TUE (SIGKILL : une coupure, pas une extinction -- ni sync, ni
# demontage).
#
# Demarrage 2 : le shell relit. Invariants (fail closed) :
#   D1  le fichier ecrit sans fsync est la, avec son contenu ;
#   D2  le dossier vide et le fichier vide sont la (BOUCHAUD_PERSIST_META_V1) ;
#   D3  les droits 0600 du fichier vide ont survecu ;
#   D4  le dossier personnel et Documents, Images, Telechargements sont la ;
#   D5  au moins une transaction d'ecriture differee a eu lieu au demarrage 1.
set -uo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_dossiers_persistants.sh BOOTIMAGE [SORTIE]}
OUT=${2:-target/dossiers-persistants}
SMP=${BO_SMP:-2}
rm -rf "$OUT"; mkdir -p "$OUT/s1" "$OUT/s2"
OUT=$(cd "$OUT" && pwd)
cat > "$OUT/s1/autorun" <<'EOS'
touch /persist/essai-differe.txt
write /persist/essai-differe.txt bonjour-sans-fsync
mkdir /persist/vide
touch /persist/vide/fichier-vide
chmod 600 /persist/vide/fichier-vide
desktop
EOS
IMAGE="$OUT/disque.img" bash tools/userland/mkdisk.sh "$OUT/s1" >/dev/null 2>&1 \
  || { echo "mkdisk a echoue" >&2; exit 1; }

boot() {
  local log=$1
  : > "$log"
  qemu-system-x86_64 -cpu max -smp "$SMP" -m 2048 -display none -no-reboot \
    -drive "format=raw,file=$BOOT" -drive "format=raw,file=$OUT/disque.img" \
    -serial "file:$log" >/dev/null 2>&1 &
  echo $!
}
propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$1" | tr -d '\r'; }

# --- Demarrage 1 : ecrire sans fsync, puis couper ---------------------------
log1="$OUT/boot-1.log"
pid=$(boot "$log1")
fin=$((SECONDS + 240))
until [ "$(propre "$log1" | grep -ac "BOUCHAUD_BOOT_POINT bureau-premiere-trame")" -gt 0 ]; do
  (( SECONDS < fin )) && kill -0 "$pid" 2>/dev/null || { echo "demarrage 1 : pas de bureau" >&2; kill -9 "$pid" 2>/dev/null; exit 1; }
  sleep 0.5
done
# Plafond d'ecriture differee (8 s) + marge pour la transaction sous TCG.
sleep 14
kill -9 "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
transactions=$(propre "$log1" | grep -ac "BOUCHAUD_ECRITURE_DIFFEREE n=" || true)
propre "$log1" | grep -aE "BOUCHAUD_ECRITURE_DIFFEREE|BOUCHAUD_DOSSIERS_PERSONNELS" | sed 's/.*\] //' | sed -n '1,8p'

# --- Demarrage 2 : relire ------------------------------------------------------
# Le disque garde sa zone ; seul l'autorun de l'archive change. mkdisk reecrit
# l'archive en tete de l'image SANS toucher la zone de fin (meme taille).
cat > "$OUT/s2/autorun" <<'EOS'
echo RELU_DEBUT
cat /persist/essai-differe.txt
ls /persist/vide
stat /persist/vide/fichier-vide
ls /persist/home/root
echo RELU_FIN
EOS
zone=$(mktemp)
taille=$(stat -c %s "$OUT/disque.img")
# La zone de persistance occupe les 128 Mio de fin : on la garde de cote.
dd if="$OUT/disque.img" of="$zone" bs=512 skip=$(( taille / 512 - 262144 )) status=none
IMAGE="$OUT/disque2.img" bash tools/userland/mkdisk.sh "$OUT/s2" >/dev/null 2>&1 \
  || { echo "mkdisk 2 a echoue" >&2; exit 1; }
taille2=$(stat -c %s "$OUT/disque2.img")
dd if="$zone" of="$OUT/disque2.img" bs=512 seek=$(( taille2 / 512 - 262144 )) conv=notrunc status=none
rm -f "$zone"; mv "$OUT/disque2.img" "$OUT/disque.img"
log2="$OUT/boot-2.log"
pid=$(boot "$log2")
fin=$((SECONDS + 240))
until [ "$(propre "$log2" | grep -ac "RELU_FIN")" -gt 0 ]; do
  (( SECONDS < fin )) && kill -0 "$pid" 2>/dev/null || break
  sleep 0.5
done
kill -9 "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
releve=$(propre "$log2" | sed -n '/RELU_DEBUT/,/RELU_FIN/p' | sed -E 's/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //')
echo "$releve"

echec=0
grep -q "bonjour-sans-fsync" <<<"$releve" || { echo "D1 ECHEC : fichier ecrit sans fsync perdu"; echec=1; }
grep -q "fichier-vide" <<<"$releve" || { echo "D2 ECHEC : dossier vide ou fichier vide perdu"; echec=1; }
grep -Eq "600|rw-------" <<<"$releve" || { echo "D3 ECHEC : droits 0600 perdus"; echec=1; }
for d in Documents Images "chargements"; do
  grep -Eq "$d" <<<"$releve" || { echo "D4 ECHEC : $d absent du dossier personnel"; echec=1; }
done
(( transactions >= 1 )) || { echo "D5 ECHEC : aucune ecriture differee au demarrage 1"; echec=1; }
if propre "$log1" "$log2" 2>/dev/null | grep -aE "KERNEL PANIC|panicked at" >/dev/null; then echo "PANIQUE"; echec=1; fi
if (( echec == 0 )); then
  echo "DOSSIERS_PERSISTANTS_OK transactions_demarrage1=$transactions"
fi
exit $echec
