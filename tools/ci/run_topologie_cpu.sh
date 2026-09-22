#!/usr/bin/env bash
#
# CE QUE LE MONDE UTILISATEUR APPREND DU NOMBRE DE PROCESSEURS.
#
# # Le defaut que ce banc fige
#
# `/sys/devices/system/cpu/online` annoncait « 0 ». Ce n'est pas un compte,
# c'est une PLAGE, et celle-la ne contient que le processeur zero. Tout ce qui
# dimensionne un pool de threads -- la glibc par `sysconf(_SC_NPROCESSORS_ONLN)`,
# AK, Skia, LibJS -- en concluait qu'il y avait UN processeur.
#
# Le releve physique le decrivait sans le nommer : « la charge WebContent peut
# etre tres elevee sur un seul coeur alors que le CPU global parait faible ».
#
# `/proc/cpuinfo` ne portait qu'un bloc, avec `siblings: 1` et `cpu cores: 1`,
# et `/proc/stat` n'existait pas du tout -- alors que c'est la seconde source
# que les compteurs de processeurs consultent.
#
# # Ce que le banc verifie
#
# Que la plage annoncee correspond au nombre de processeurs que QEMU donne.
# C'est la seule verification qui ne puisse pas etre satisfaite par une
# constante : on demarre la meme image avec deux tailles de machine, et la
# reponse doit changer.
#
#   ./tools/ci/run_topologie_cpu.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${TOPOLOGIE_SORTIE:-$(mktemp -d)}
SECONDES=${TOPOLOGIE_SECONDES:-150}
mkdir -p "$SORTIE"

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    echo "  construire d'abord : tools/ci/build_kernel.sh" >&2
    exit 1
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"

# BOUCHAUD_C26_CE_QUE_VOIT_L_ANNEAU_3
#
# Lire les fichiers depuis le shell du noyau ne prouve pas grand-chose : on y
# relit ce qu'on vient d'ecrire, sans passer par le bac a sable. La question
# qui compte est ce que voit un programme UTILISATEUR, avec les memes appels
# que Ladybird et ses bibliotheques -- un fichier juste mais REFUSE donne le
# meme resultat qu'un fichier faux.
#
# `voir-cpu` repond par la mesure, et il lit aussi CPUID : si le materiel et
# `/proc` divergent, c'est le noyau qui se trompe ; s'ils s'accordent et que
# Ladybird voit autre chose, c'est le bac a sable.
CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
SONDE=0
if [ -n "$CC" ] && "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles \
        -Wl,-z,noexecstack -o "$SCENARIO/voir-cpu" tools/userland/voir-cpu.c 2>/dev/null; then
    SONDE=1
else
    echo "topologie : sonde anneau 3 non compilable ici, seuls les fichiers sont lus"
fi

if [ "$SONDE" = 1 ]; then
    cat > "$SCENARIO/autorun" <<'AUTORUN'
echo "=== TOPOLOGIE DEBUT ==="
cat /sys/devices/system/cpu/online
cat /sys/devices/system/cpu/present
cat /sys/devices/system/cpu/possible
tail /proc/cpuinfo
cat /proc/stat
exec /voir-cpu
echo "=== TOPOLOGIE FIN ==="
AUTORUN
else
    cat > "$SCENARIO/autorun" <<'AUTORUN'
echo "=== TOPOLOGIE DEBUT ==="
cat /sys/devices/system/cpu/online
cat /sys/devices/system/cpu/present
cat /sys/devices/system/cpu/possible
tail /proc/cpuinfo
cat /proc/stat
echo "=== TOPOLOGIE FIN ==="
AUTORUN
fi
(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

echecs=0

# DEUX TAILLES DE MACHINE, ET C'EST TOUT L'INTERET.
#
# Une seule taille se satisferait d'une constante bien choisie. Deux exigent
# que la valeur soit MESUREE.
for CPUS in 2 8; do
    LOG="$SORTIE/serie-$CPUS.log"
    : > "$LOG"
    timeout "$SECONDES" qemu-system-x86_64 \
      -drive format=raw,file="$BOOT" \
      -drive format=raw,file="$SORTIE/scenario.img" \
      -m 4096 -smp "$CPUS" -cpu max -display none -no-reboot \
      -netdev user,id=net0 -device e1000,netdev=net0 \
      -serial file:"$LOG" >/dev/null 2>&1

    PROPRE="$SORTIE/propre-$CPUS.log"
    # Le port serie rend des fins de ligne CRLF : sans le `\r`, toute
    # comparaison exacte echoue sur une valeur pourtant juste.
    sed -E -e 's/\x1b\[[0-9;]*m//g' -e 's/^(\[[^]]*\])+ //' -e 's/\r$//' "$LOG" > "$PROPRE"

    if ! grep -q "TOPOLOGIE FIN" "$PROPRE"; then
        echo "topologie : le scenario a $CPUS processeurs n'est pas alle au bout" >&2
        echecs=$((echecs + 1))
        continue
    fi

    ATTENDU="0-$((CPUS - 1))"
    BLOC=$(sed -n '/TOPOLOGIE DEBUT/,/TOPOLOGIE FIN/p' "$PROPRE")
    LU=$(echo "$BLOC" | grep -m1 -E '^0(-[0-9]+)?$')
    SIBLINGS=$(echo "$BLOC" | grep -m1 'siblings' | tr -d ' \t\r' | cut -d: -f2)

    printf '  -smp %-2s  online=%-6s siblings=%-4s ' "$CPUS" "${LU:-?}" "${SIBLINGS:-?}"
    if [ "${LU:-}" = "$ATTENDU" ]; then
        printf 'attendu %s : OK\n' "$ATTENDU"
    else
        printf 'attendu %s : ECHEC\n' "$ATTENDU"
        echecs=$((echecs + 1))
    fi
    if [ "${SIBLINGS:-0}" != "$CPUS" ]; then
        echo "        cpuinfo annonce siblings=$SIBLINGS pour $CPUS processeurs" >&2
        echecs=$((echecs + 1))
    fi
    if ! echo "$BLOC" | grep -q "^cpu$((CPUS - 1)) "; then
        echo "        /proc/stat n'a pas de ligne cpu$((CPUS - 1))" >&2
        echecs=$((echecs + 1))
    fi

    # CE QUE VOIT L'ANNEAU 3, et c'est la seule mesure qui engage Ladybird.
    if [ "$SONDE" = 1 ]; then
        VU=$(echo "$BLOC" | grep '^VOIR_CPU ' | sed 's/^VOIR_CPU //')
        echo "$VU" | sed 's/^/        anneau3 /'
        VERDICT=$(echo "$VU" | grep -m1 '^verdict=' | cut -d= -f2)
        if [ "${VERDICT:-}" != "coherent" ]; then
            echo "        l'anneau 3 rend verdict=${VERDICT:-absent}" >&2
            echecs=$((echecs + 1))
        fi
        VU_PROCS=$(echo "$VU" | grep -m1 '^cpuinfo_processors=' | cut -d= -f2)
        if [ "${VU_PROCS:-0}" != "$CPUS" ]; then
            echo "        l'anneau 3 voit ${VU_PROCS:-?} processeurs pour $CPUS" >&2
            echecs=$((echecs + 1))
        fi
        # SMT MESURE, ET NON SUPPOSE. QEMU lance `-smp N` en N paquets d'un
        # seul fil : `cpu cores` doit donc valoir N, et non N/2 comme le
        # rendait la constante « deux fils par coeur ».
        VU_CORES=$(echo "$VU" | grep -m1 '^cpuinfo_cores=' | cut -d= -f2)
        if [ "${VU_CORES:-0}" != "$CPUS" ]; then
            echo "        cpu cores=${VU_CORES:-?} pour $CPUS processeurs sans SMT" >&2
            echecs=$((echecs + 1))
        fi
    fi
done

if [ "$echecs" -ne 0 ]; then
    echo "topologie CPU : $echecs probleme(s) ; journaux dans $SORTIE" >&2
    exit 1
fi
printf '\033[32m%s\033[0m\n' "topologie CPU : la plage annoncee suit le materiel"
echo "TOPOLOGIE_CPU_OK"
