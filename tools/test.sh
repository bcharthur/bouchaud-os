#!/bin/bash
# Lanceur de sondes de Bouchaud OS : construit, boote, verifie, et rend un code.
#
# Les sondes userland (ring3-selftest, qpa-probe, posix-probe) verifient plus de
# cent points de l'ABI et ont trouve la plupart des defauts corriges jusqu'ici.
# Elles ne servaient a rien tant qu'il fallait penser a les lancer a la main, en
# enchainant six commandes : ce script en fait un `make test`.
#
#   ./tools/test.sh            construit tout et joue le scenario complet
#   ./tools/test.sh --quick    reutilise l'image de boot deja construite
#   ./tools/test.sh --keep     laisse les artefacts dans tools/userland/test-out
#
# Comment ca marche : les binaires et un script `/autorun` sont deposes sur le
# disque de donnees. Au demarrage, le noyau voit `/autorun`, le joue au lieu
# d'ouvrir une session, recopie tout sur COM1, puis eteint la machine par le
# peripherique `isa-debug-exit` de QEMU. L'hote n'a plus qu'a lire le code de
# sortie et le journal.
#
# Code de retour : 0 si tout passe, 1 sinon.

set -u
cd "$(dirname "$0")/.."
ROOT=$(pwd)

QUICK=0
KEEP=0
for arg in "$@"; do
    case "$arg" in
        --quick) QUICK=1 ;;
        --keep) KEEP=1 ;;
        -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
        *) echo "option inconnue : $arg" >&2; exit 2 ;;
    esac
done

BOOTIMG=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
WORK=tools/userland/test-out
DISK=$WORK/test-disk.img
LOG=$WORK/serial.log
# Le scenario complet demande environ cinq minutes sur une machine sans KVM
# (tout est emule instruction par instruction) et une poignee de secondes avec.
# Le delai est donc large a dessein : le depasser doit vouloir dire « bloque »,
# pas « un peu lent aujourd'hui ».
TIMEOUT=${TIMEOUT:-900}

red()  { printf '\033[31m%s\033[0m\n' "$*"; }
green(){ printf '\033[32m%s\033[0m\n' "$*"; }
info() { printf '\033[36m%s\033[0m\n' "$*"; }

# --- 1. Construction -------------------------------------------------------

if [ "$QUICK" = 0 ]; then
    info "== construction du noyau =="
    if ! cargo bootimage 2>&1 | tail -3; then
        red "bootimage a echoue"
        exit 1
    fi
elif [ ! -f "$BOOTIMG" ]; then
    red "--quick demande, mais $BOOTIMG n'existe pas"
    exit 1
fi

info "== construction des sondes userland =="
if ! (cd tools/userland && OUT=../../$WORK/files ./build.sh musl >/dev/null); then
    red "la construction musl a echoue (paquet musl-tools installe ?)"
    exit 1
fi
# ring3-selftest n'utilise pas de libc : il se construit dans l'autre chaine.
if ! (cd tools/userland && OUT=../../$WORK/files ./build.sh freestanding >/dev/null); then
    red "la construction freestanding a echoue"
    exit 1
fi

# --- 2. Scenario ------------------------------------------------------------

# Chaque sonde renvoie 0 quand toutes ses verifications passent. `&&` suffit
# donc a propager l'echec, et `$?` en fin de script devient le verdict.
cat > "$WORK/files/autorun" <<'SCENARIO'
# Scenario joue au demarrage par le noyau (voir src/kernel/autorun.rs).
uname
df
ls /
exec /ring3-selftest
exec /posix-probe
exec /qpa-probe
SCENARIO

info "== fabrication du disque de test =="
if ! (cd tools/userland && IMAGE=../../$DISK ./mkdisk.sh ../../$WORK/files >/dev/null); then
    red "mkdisk a echoue"
    exit 1
fi

# --- 3. Execution -----------------------------------------------------------

info "== boot QEMU (sans affichage, delai max ${TIMEOUT}s) =="
# -device isa-debug-exit : c'est par lui que le noyau rend son verdict.
#   QEMU sort avec (code << 1) | 1, soit 33 pour un succes et 35 pour un echec.
# -no-reboot : sans lui, une triple faute rebooterait en boucle au lieu de
#   terminer, et le scenario tournerait jusqu'au delai.
timeout "$TIMEOUT" qemu-system-x86_64 \
    -drive "format=raw,file=$BOOTIMG" \
    -drive "format=raw,file=$DISK" \
    -m 2048 \
    -display none \
    -no-reboot \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -netdev user,id=net0 \
    -device e1000,netdev=net0 \
    -serial "file:$LOG"
QEMU_STATUS=$?

# --- 4. Verdict -------------------------------------------------------------

echo ""
info "== journal serie : $LOG =="

FAILED=0
report() { # report <succes 0|1> <libelle>
    if [ "$1" = 0 ]; then green "  ok   $2"; else red "  ECHEC $2"; FAILED=1; fi
}

if [ "$QEMU_STATUS" = 124 ]; then
    red "QEMU n'a pas rendu la main en ${TIMEOUT}s — le noyau est probablement bloque."
    tail -30 "$LOG" 2>/dev/null
    exit 1
fi

grep -q "=== AUTORUN DEBUT ===" "$LOG" 2>/dev/null
report $? "le noyau a demarre et joue le scenario"

grep -q "=== AUTORUN FIN ===" "$LOG" 2>/dev/null
report $? "le scenario est alle jusqu'au bout"

# ring3-selftest n'a pas de libc : il ne sait pas compter ses echecs et se
# contente d'aller au bout. Sa derniere ligne fait donc office de bilan.
grep -q "\[main\] termine" "$LOG" 2>/dev/null
report $? "ring3-selftest est alle jusqu'a sa derniere etape"

# Les deux autres sondes impriment leur propre bilan ; on les compte plutot que
# de faire confiance au seul code de sortie, qui ne dit pas laquelle a lache.
if grep -q "RESULTAT" "$LOG" 2>/dev/null; then
    while IFS= read -r line; do
        case "$line" in
            *"0 verification(s) en echec"*) green "  ok   $line" ;;
            *) red "  ECHEC $line"; FAILED=1 ;;
        esac
    done < <(grep "RESULTAT" "$LOG")
fi

# Une panique noyau peut survenir apres un scenario par ailleurs vert.
if grep -qi "panic\|DOUBLE FAULT" "$LOG" 2>/dev/null; then
    red "  ECHEC le noyau a panique :"
    grep -i -A3 "panic\|DOUBLE FAULT" "$LOG" | head -12
    FAILED=1
fi

# 33 = EXIT_OK cote noyau, 35 = EXIT_FAIL (voir src/kernel/power.rs).
case "$QEMU_STATUS" in
    33) green "  ok   le noyau rend un statut de succes" ;;
    35) red   "  ECHEC le noyau rend un statut d'echec"; FAILED=1 ;;
    *)  red   "  ECHEC sortie QEMU inattendue ($QEMU_STATUS)"; FAILED=1 ;;
esac

[ "$KEEP" = 1 ] || rm -rf "$WORK/files"

echo ""
if [ "$FAILED" = 0 ]; then
    green "TOUT PASSE"
    exit 0
fi
red "DES VERIFICATIONS ONT ECHOUE — journal complet dans $LOG"
exit 1
