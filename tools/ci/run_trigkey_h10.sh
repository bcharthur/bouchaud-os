#!/usr/bin/env bash
#
# H10 -- l'outillage de preuve de la machine de reference, exerce sous QEMU.
#
# # Ce que ce scenario etablit, et ce qu'il n'etablit pas
#
# Il etablit que les sept commandes EXISTENT, s'executent, et rendent des
# verdicts coherents avec la machine sous elles. C'est une preuve de
# l'outillage, PAS une preuve du TRIGKEY : QEMU n'est pas la machine de
# reference, et aucun resultat produit ici ne vaut pour elle.
#
# Sa vraie fonction est d'empecher une regression silencieuse de l'outillage
# entre deux essais physiques. Un `hwtest` qui cesserait de rendre un verdict
# ferait passer un essai TRIGKEY pour concluant alors qu'il n'aurait rien dit.
#
# # Le point qui compte le plus
#
# `disktest --ecriture` doit REFUSER d'ecrire quand il n'existe pas de
# partition Bouchaud, et le scenario le verifie DANS LES DEUX SENS : avec
# partition, l'ecriture a lieu et le contenu d'origine est repose ; sans
# partition, elle est refusee. La seconde moitie est la plus importante -- sur
# le materiel, le disque brut porte le systeme de son proprietaire.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_trigkey_h10.sh BOOTIMAGE [LOG]}
LOG=${2:-target/trigkey-h10-ci/h10.log}
RACINE=$(dirname "$LOG")
DISQUE="$RACINE/disque.img"
SCENARIO="$RACINE/scenario.img"

mkdir -p "$RACINE"
rm -f "$LOG" "$LOG.sans" "$DISQUE" "$SCENARIO"

python3 tools/ci/fabrique-disque-gpt.py "$DISQUE" --mio 64 || exit 1

fabrique_scenario() {
    python3 - "$SCENARIO" "$1" <<'PY'
import sys, tarfile
from pathlib import Path
image, script = Path(sys.argv[1]), sys.argv[2]
autorun = image.parent / "autorun"
autorun.write_text(script, encoding="ascii")
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(autorun), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with autorun.open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY
}

lance() {
    local sortie=$1; shift
    timeout 240 qemu-system-x86_64 \
      -drive format=raw,file="$BOOT" \
      -drive format=raw,file="$SCENARIO" \
      -m 2048 -smp 4 -display none -no-reboot \
      "$@" \
      -serial file:"$sortie"
}

problemes=0
plainte() { printf 'ECHEC   %s\n' "$1"; problemes=$((problemes + 1)); }
exige() {
    if grep -aEq "$2" "$1"; then printf 'ok      %s\n' "$3"; else plainte "$3"; fi
}

# --- 1. Avec une partition Bouchaud -----------------------------------------
fabrique_scenario 'journal off
echo H10_BEGIN
hwinfo
hwtest
nvmetest
disktest
disktest --ecriture
persist-test --pose
persist-test --verifie
safe-mode
safe-mode --sortir
echo H10_FIN
poweroff
'
echo "=== QEMU H10, disque avec partition Bouchaud ==="
lance "$LOG" -drive if=none,id=nvm,format=raw,file="$DISQUE" -device nvme,serial=h10,drive=nvm

test -s "$LOG" || { echo "aucune sortie serie" >&2; exit 1; }
NET="$RACINE/propre.log"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$NET"
if grep -aiEq '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$NET"; then
    echo "panic pendant l'outillage H10" >&2
    exit 1
fi
echo "--- releve ---"; grep -aE 'H10_' "$NET" | sed 's/^.*\] //' | head -30; echo "--- fin ---"

exige "$NET" 'H10_HWINFO_FIN' 'hwinfo rend la main'
exige "$NET" 'H10_HWINFO_TIMER mode=' 'hwinfo nomme le mode de quantum local'
exige "$NET" 'H10_HWTEST_BILAN .*verdict=ok' 'hwtest rend un verdict global vert'
exige "$NET" 'H10_HWTEST point=smp-battement verdict=passe' 'tous les coeurs battent'
exige "$NET" 'H10_HWTEST point=partition-bouchaud verdict=passe' 'la partition Bouchaud est vue'
exige "$NET" 'H10_NVMETEST verdict=ok' 'nvmetest lit un bloc reel'
exige "$NET" 'H10_DISKTEST verdict=ok phase=ecriture .*relecture=1 restaure=1' \
      'disktest ecrit, relit et REPOSE le contenu d origine'
exige "$NET" 'H10_PERSIST verdict=non-concluant .*raison=meme-session' \
      'persist-test refuse de conclure sans redemarrage'
exige "$NET" 'H10_SAFEMODE etat=actif' 'safe-mode s active'
exige "$NET" 'H10_SAFEMODE etat=desactive' 'safe-mode se desactive'

# --- 2. SANS partition Bouchaud : l'ecriture doit etre REFUSEE --------------
#
# C'est la moitie qui protege un disque reel. Un disque vierge n'a pas de
# partition Bouchaud ; le disque interne d'une machine non installee non plus.
fabrique_scenario 'journal off
echo H10_BEGIN
disktest --ecriture
echo H10_FIN
poweroff
'
VIERGE="$RACINE/vierge.img"
rm -f "$VIERGE"; dd if=/dev/zero of="$VIERGE" bs=1M count=64 status=none
echo
echo "=== QEMU H10, disque SANS partition Bouchaud ==="
lance "$LOG.sans" -drive if=none,id=nvm,format=raw,file="$VIERGE" -device nvme,serial=h10,drive=nvm
NETS="$RACINE/propre-sans.log"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG.sans" > "$NETS"
echo "--- releve ---"; grep -aE 'H10_DISKTEST' "$NETS" | sed 's/^.*\] //' | head -5; echo "--- fin ---"

exige "$NETS" 'H10_DISKTEST verdict=refuse .*raison=pas-de-partition-bouchaud' \
      'sans partition Bouchaud, l ecriture est REFUSEE'
if grep -aEq 'H10_DISKTEST verdict=ok phase=ecriture' "$NETS"; then
    plainte 'une ecriture a eu lieu sur un disque sans partition Bouchaud'
fi

if [ "$problemes" -ne 0 ]; then
    echo "TRIGKEY_H10_ECHEC problemes=$problemes" >&2
    exit 1
fi
echo "TRIGKEY_H10_OK"
