#!/usr/bin/env bash
#
# LA PREUVE VISUELLE DE LA FENETRE SERVICES.
#
# # Pourquoi ce banc existe
#
# La passe precedente a livre un registre de services, des tests verts et une
# ligne de commande -- et la fenetre Services affichait toujours ses six lignes
# Ladybird codees en dur. La photo physique du 17 septembre le montre sans
# ambiguite. « Les tests passent » ne prouve pas qu'un pixel a change.
#
# Ce banc demarre le bureau, ouvre la fenetre Services PAR LE MEME CHEMIN que
# le clic sur l'icone -- `make_app(KIND_SERVICES)` --, capture le tampon video
# par le moniteur QEMU, et convertit la capture en PNG regardable.
#
# Il ne remplace pas le regard : il le rend possible.
set -uo pipefail
cd "$(dirname "$0")/../.."

SORTIE=${SERVICES_VUE_SORTIE:-$(mktemp -d)}
mkdir -p "$SORTIE"
BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SECONDES=${SERVICES_VUE_SECONDES:-45}

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    echo "  construire d'abord : tools/ci/build_kernel.sh" >&2
    exit 1
fi

echo "=== disque de scenario : autorun -> desktop avec Services ouvert ==="
SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"
mkdir -p "$SCENARIO"
# COMMENT LA FENETRE S'OUVRE, ET CE QUE CELA PROUVE
#
# Par defaut, le banc ouvre Services par `make_app(KIND_SERVICES)` -- la
# fonction EXACTE que le double-clic sur l'icone appelle, atteinte par le
# meme gestionnaire de fenetres. Ce qui n'est donc pas couvert se resume a
# une chose : la table `ICONS[5] = ("Services", KIND_SERVICES)` et le
# test de survol qui la consulte. `verifie-fenetre-services.py` defend cette
# arete ; tout le reste du chemin est exerce ici.
#
# `SERVICES_VUE_SOURIS=1` tente l'ouverture par une VRAIE souris injectee au
# moniteur QEMU. Le code est garde parce qu'il documente trois pieges reels
# (axe Y inverse, octets signes, terminal d'accueil qui recouvre l'icone),
# mais il n'est pas fiable : le double-clic synthetique n'ouvre pas la
# fenetre a tous les coups. Ce n'est donc pas le defaut -- un banc de preuve
# qui echoue une fois sur deux ne prouve rien.
if [ "${SERVICES_VUE_SOURIS:-0}" != "1" ]; then
    cat > "$SCENARIO/autorun" <<'AUTORUN'
export BO_AUTOSTART_SERVICES=1
desktop
AUTORUN
else
    cat > "$SCENARIO/autorun" <<'AUTORUN'
desktop
AUTORUN
fi
(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

MONITEUR="$SORTIE/moniteur.sock"
LOG="$SORTIE/serie.log"
: > "$LOG"

echo "=== QEMU : bureau, capture du tampon video ==="
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp 4 -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -monitor "unix:$MONITEUR,server,nowait" \
  -serial file:"$LOG" &
PID=$!
trap 'kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null' EXIT

# ATTENDRE LE BUREAU, PAS UNE DUREE. Un delai fixe capture un ecran noir le
# jour ou la machine est lente, et personne ne sait dire pourquoi.
DEBUT=$SECONDS
pret=0
while [ $((SECONDS - DEBUT)) -lt "$SECONDES" ]; do
    if grep -aq "BOUCHAUD_DESKTOP_FIRST_FRAME\|BOUCHAUD_WM_PREMIERE_TRAME\|desktop:" "$LOG" 2>/dev/null; then
        pret=1
        break
    fi
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "QEMU s'est arrete avant le bureau" >&2
        tail -20 "$LOG" >&2
        exit 1
    fi
    sleep 1
done
# Laisser quelques trames au compositeur : la premiere peint le fond, la
# fenetre arrive ensuite.
sleep 6

# ── OUVRIR SERVICES PAR L'ICONE ──────────────────────────────────────────────
#
# `ICON_POSITIONS[5]` vaut (106, 42) et la cellule fait 78x80 : le centre de
# l'image est a (145, 70). La souris PS/2 est RELATIVE -- on la ramene d'abord
# dans le coin par un grand deplacement negatif, que le pilote borne, puis on
# avance de la position voulue.
#
# Les quatre commandes partent dans UNE session du moniteur : ouvrir une
# connexion par commande mettait facilement plus d'une demi-seconde entre les
# deux appuis, et le bureau n'y voyait plus un double-clic.
# ── OUVRIR SERVICES PAR L'ICONE, A LA SOURIS ────────────────────────────────
#
# Trois pieges, tous constates sur une capture et non devines :
#
# 1. L'AXE Y DE LA PS/2 POINTE VERS LE HAUT. `paquet.rs` calcule
#    `new_y = old_y.saturating_sub(dy)` : un `mouse_move x 70` REMONTE le
#    pointeur. Pour descendre, dy doit etre NEGATIF.
# 2. dx et dy sont des octets signes (`PKT[1] as i8`) : au-dela de 127 le
#    deplacement se replie. On avance par pas de cent.
# 3. LE BUREAU OUVRE UN TERMINAL AU DEMARRAGE, et sa fenetre RECOUVRE l'icone
#    Services, qui est en (106,42). Cliquer a cet endroit visait sa barre de
#    titre. Il faut donc fermer ce terminal AVANT de viser l'icone.

# Ramene le pointeur dans le coin haut-gauche : x diminue (dx<0), y diminue
# (dy>0, cf. piege 1).
range() { for _ in $(seq 1 40); do echo "mouse_move -100 100"; done; }
# Deplace de (0,0) vers (x,y) par pas bornes.
vise() {
    local x=$1 y=$2
    while [ "$x" -gt 0 ]; do
        local pas=$(( x > 100 ? 100 : x ))
        echo "mouse_move $pas 0"
        x=$(( x - pas ))
    done
    while [ "$y" -gt 0 ]; do
        local pas=$(( y > 100 ? 100 : y ))
        echo "mouse_move 0 -$pas"
        y=$(( y - pas ))
    done
}

if [ "${SERVICES_VUE_SOURIS:-0}" = "1" ]; then
    echo "=== fermeture du terminal d'accueil, qui recouvre l'icone ==="
    { range; vise 390 46; echo "mouse_button 1"; echo "mouse_button 0"; } \
        | socat - "unix-connect:$MONITEUR" >/dev/null 2>&1
    sleep 2

    echo "=== ouverture par double-clic sur l'icone Services ==="
    # Les deux appuis partent dans UNE session : une connexion par commande
    # mettait plus d'une demi-seconde entre eux, et le bureau n'y voyait plus
    # un double-clic.
    { range; vise 145 70; echo "mouse_button 1"; echo "mouse_button 0"; \
      echo "mouse_button 1"; echo "mouse_button 0"; } \
        | socat - "unix-connect:$MONITEUR" >/dev/null 2>&1
    sleep 4
fi

CAPTURE="$SORTIE/services.ppm"
echo "screendump $CAPTURE" | socat - "unix-connect:$MONITEUR" >/dev/null 2>&1
sleep 2
if [ ! -s "$CAPTURE" ]; then
    echo "aucune capture produite (bureau pret=$pret)" >&2
    tail -20 "$LOG" >&2
    exit 1
fi

python3 tools/ci/ppm-vers-png.py "$CAPTURE" "$SORTIE/services.png" || exit 1
echo "SERVICES_VUE_CAPTURE=$SORTIE/services.png"
echo "SERVICES_VUE_SERIE=$LOG"
echo "SERVICES_VUE_OK"
