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
# Si `tools/userland/out-python/` ou `tools/userland/out-qt/` existent (voir
# build-python.sh et build-qt.sh), la sonde Python et la demonstration Qt sont
# ajoutees au scenario. Sans eux, le scenario se limite aux trois sondes en C.
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
# Le temoin C++23 conditionne tout le portage Ladybird : il doit tourner a
# chaque scenario, pas seulement le jour ou l'on y pense.
if ! (cd tools/userland && OUT=../../$WORK/files ./build.sh cpp23 >/dev/null); then
    red "la construction C++23 a echoue (g++ >= 13 requis)"
    exit 1
fi

# --- 2. Scenario ------------------------------------------------------------

# Chaque sonde renvoie 0 quand toutes ses verifications passent, et le noyau
# retient le premier echec rencontre (voir shell::run_batch).
cat > "$WORK/files/autorun" <<'SCENARIO'
# Scenario joue au demarrage par le noyau (voir src/kernel/autorun.rs).
uname
df
ls /
exec /ring3-selftest
exec /posix-probe
exec /audio-probe
exec /net-probe 91.189.91.83
exec /persist-probe
exec /shm-probe
exec /ipc-probe
exec /ordonnanceur-probe
exec /qpa-probe
exec /cpp23-probe
SCENARIO

# AK, la base de Ladybird. Comme Python et Qt : si le portage a ete construit,
# on le joue ; sinon on s'en passe. C'est ce qui garde `test.sh` utilisable sans
# avoir recupere les 27 877 fichiers de l'upstream.
AK_PROBE=third_party/build-ak-bouchaud/ak-probe
if [ -x "$AK_PROBE" ]; then
    info "== AK (portage Ladybird) =="
    cp "$AK_PROBE" "$WORK/files/ak-probe"
    echo "exec /ak-probe" >> "$WORK/files/autorun"
    CORE_PROBE=third_party/build-libcore-bouchaud/libcore-probe
    if [ -x "$CORE_PROBE" ]; then
        cp "$CORE_PROBE" "$WORK/files/libcore-probe"
        echo "exec /libcore-probe" >> "$WORK/files/autorun"
    fi
else
    info "AK absent — pour l'ajouter au scenario :"
    info "  ./tools/ladybird/fetch.sh && ./tools/ladybird/build-deps.sh --cible \\"
    info "  && ./tools/ladybird/build-ak.sh --cible"
fi

# Python et Qt ne sont pas construits par ce script : ils demandent une
# vingtaine de minutes chacun et des sources telechargees. S'ils sont la, on les
# joue ; sinon on s'en passe. C'est ce qui permet de garder un `test.sh` rapide
# tout en couvrant la pile complete quand elle est disponible.
# La question qui decide de la suite : un binaire Linux **dynamique**, lie a la
# glibc et non recompile, s'execute-t-il ? Si oui, amener un moteur web revient
# a honorer les appels que son binaire emet, et non a le reecrire.
ABI_DIR=tools/userland/out-abi-linux
if [ -x "$ABI_DIR/glibc-probe" ]; then
    info "  + sysroot glibc detecte : la sonde d'ABI dynamique est ajoutee"
    cp -r "$ABI_DIR"/* "$WORK/files/"
    # La trace est active pour cette sonde et pour elle seule. Le chargeur
    # dynamique de la glibc ne dit rien quand il s'arrete : sans trace on ne
    # voit qu'un silence, avec elle on lit l'appel exact qui manque. C'est ce
    # qui a permis de trouver `rseq`, et c'est ce qui nommera le suivant.
    echo "strace on" >> "$WORK/files/autorun"
    echo "exec /glibc-probe" >> "$WORK/files/autorun"
    echo "strace off" >> "$WORK/files/autorun"
fi

PYTHON_DIR=tools/userland/out-python
QT_BIN=tools/userland/out-qt/qt-demo

if [ -x "$PYTHON_DIR/usr/bin/python3" ]; then
    info "  + CPython detecte : la sonde Python est ajoutee au scenario"
    cp -r "$PYTHON_DIR/usr" "$WORK/files/"
    cp tools/userland/python-probe.py "$WORK/files/"
    echo "exec /usr/bin/python3 /python-probe.py" >> "$WORK/files/autorun"
fi

if [ -x "$QT_BIN" ]; then
    info "  + Qt detecte : la demonstration graphique est ajoutee au scenario"
    cp "$QT_BIN" "$WORK/files/qt-demo"
    # Court : le scenario doit se terminer seul, on ne cherche qu'a prouver que
    # la boucle d'evenements tourne et que la fenetre s'affiche.
    echo "export QT_DEMO_DUREE_MS=4000" >> "$WORK/files/autorun"
    echo "exec /qt-demo" >> "$WORK/files/autorun"
fi

# Le moteur web se verifie deja sur la machine de developpement
# (`tools/userland/test-moteur.sh`), mais avec un bouchon a la place de l'hote
# Qt. Le jouer ici le confronte au vrai : metriques de fonte reelles, decodage
# d'image par libpng et libjpeg, QuickJS en ring 3.
NAVIGATEUR_DIR=tools/userland/out-navigateur

if [ -x "$NAVIGATEUR_DIR/bo-navigateur" ]; then
    info "  + navigateur detecte : les verifications du moteur web sont ajoutees"
    cp "$NAVIGATEUR_DIR/bo-navigateur" "$WORK/files/"
    mkdir -p "$WORK/files/usr/lib" "$WORK/files/usr/share"
    cp -r "$NAVIGATEUR_DIR/usr/share/bo-navigateur" "$WORK/files/usr/share/"
    cp "$NAVIGATEUR_DIR/usr/lib/python312.zip" "$WORK/files/usr/lib/"
    cp -r "$NAVIGATEUR_DIR/usr/lib/python3" "$WORK/files/usr/lib/"
    echo "export BO_PREFIXE=/usr" >> "$WORK/files/autorun"
    echo "exec /bo-navigateur /usr/share/bo-navigateur/test_moteur.py" \
        >> "$WORK/files/autorun"
    # Le decodage audio et video, avec le vrai libavcodec.
    echo "exec /bo-navigateur /usr/share/bo-navigateur/media-probe.py" \
        >> "$WORK/files/autorun"
    # Ce que le navigateur retient d'un demarrage a l'autre : temoins, cache,
    # stockage local. Verifiable seulement ici, la ou `/persist` existe.
    echo "exec /bo-navigateur /usr/share/bo-navigateur/persist-probe.py" \
        >> "$WORK/files/autorun"
fi

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
demarre() { # demarre <fichier de journal>
    timeout "$TIMEOUT" qemu-system-x86_64 \
        -drive "format=raw,file=$BOOTIMG" \
        -drive "format=raw,file=$DISK" \
        -m 2048 \
        -display none \
        -no-reboot \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
        -netdev user,id=net0 \
        -device e1000,netdev=net0 \
        -audiodev none,id=muet -device AC97,audiodev=muet \
        -serial "file:$1"
}

demarre "$LOG"
QEMU_STATUS=$?

# Second demarrage, sur la MEME image. C'est la seule facon de prouver la
# persistance : le premier passage ecrit dans `/persist`, le second doit y
# retrouver ce qu'il y a laisse. Refabriquer l'image entre les deux effacerait
# justement ce qu'on verifie — d'ou la sonde qui reconnait seule son passage.
LOG2="$WORK/serial-2.log"
info "== second boot (persistance : la machine doit se souvenir) =="
demarre "$LOG2"
QEMU_STATUS_2=$?

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

# Le temoin C++23 : sans lui, aucune brique Ladybird ne peut etre construite.
grep -q "temoin C++23" "$LOG" 2>/dev/null
report $? "le temoin C++23 s'est execute en ring 3"

# AK n'est verifie que s'il a ete construit : son absence n'est pas un echec.
if [ -x "$AK_PROBE" ]; then
    grep -q "temoin AK" "$LOG" 2>/dev/null
    report $? "AK s'est execute en ring 3"
    if [ -x "third_party/build-libcore-bouchaud/libcore-probe" ]; then
        grep -q "temoin LibSync" "$LOG" 2>/dev/null
        report $? "LibSync + LibCore se sont executes en ring 3"
    fi
fi

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

# Le second demarrage : c'est lui qui atteste la persistance.
if [ "$QEMU_STATUS_2" = 124 ]; then
    red "le second boot n'a pas rendu la main en ${TIMEOUT}s"
    FAILED=1
else
    grep -q "passage 2" "$LOG2" 2>/dev/null
    report $? "la sonde a reconnu un second demarrage (donc /persist a tenu)"
    if grep -q "RESULTAT" "$LOG2" 2>/dev/null; then
        while IFS= read -r line; do
            case "$line" in
                *"0 verification(s) en echec"*) green "  ok   [boot 2] $line" ;;
                *) red "  ECHEC [boot 2] $line"; FAILED=1 ;;
            esac
        done < <(grep "RESULTAT" "$LOG2")
    fi
fi

if [ -x "$QT_BIN" ]; then
    grep -q "plateforme : linuxfb" "$LOG" 2>/dev/null
    report $? "Qt demarre sur la plateforme linuxfb"
    grep -q "boucle d'evenements terminee (code 0)" "$LOG" 2>/dev/null
    report $? "la boucle d'evenements de Qt tourne et se termine"
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
