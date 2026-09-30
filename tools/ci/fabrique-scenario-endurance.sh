#!/usr/bin/env bash
# Le disque de scenario de l'endurance : ce que chaque cycle EXECUTE.
#
# POURQUOI UN SCENARIO, ET PAS UN DEMARRAGE NU
#
# L'endurance bootait l'image et attendait. Un bureau au repos ne fait rien
# des chemins que les budgets surveillent -- gros verrou, latence des taches
# pretes, files d'execution, stockage -- et surtout n'emet jamais le releve
# periodique (`smpstat`) dont `check_budgets.py` a besoin. Treize budgets
# requis revenaient « absent », et l'etape tombait sans avoir rien mesure :
# une barriere toujours rouge ne protege pas plus qu'une barriere toujours
# verte.
#
# CE QUE CHAQUE CYCLE FAIT
#
# Un `smpstat` de depart, quatre familles de charge reelle en anneau 3, un
# `smpstat` d'arrivee. Rien n'est simule : ce sont les sondes que les autres
# bancs executent deja, et c'est le noyau qui produit chaque chiffre.
#
#   memoire/SMP   mmstress (travailleurs, courses unrelated/aba, churn)
#   ordonnanceur  ordonnanceur-probe : latence d'une tache interactive sous
#                 calcul -- la famine et la derive se voient ici
#   stockage      disque-probe sur un fichier de 6 Mio (au-dela du seuil
#                 inline : vraies lectures bloc), wal-probe
#   processus     session-probe (arret d'une session entiere), verrous-probe,
#                 poll-bkl-probe (attente de readiness hors gros verrou),
#                 exec-fd-probe (un VRAI execve : l'autorun lance ses
#                 programmes par exec_image, jamais par l'appel systeme, et
#                 la panique execve du lot B8 n'a ete vue que par le smoke
#                 Ladybird #377)
#
# Chaque famille ecrit un marqueur `_OK` seulement si toutes ses commandes ont
# reussi (`&&`), et `soak.py --require` exige tous les marqueurs : une famille
# qui se fige (interblocage, famine) ou echoue rend le cycle rouge, meme si le
# noyau n'a pas panique.
#
#     tools/ci/fabrique-scenario-endurance.sh TRAVAIL
#
# Produit TRAVAIL/endurance.img et TRAVAIL/marqueurs-endurance.txt (un
# marqueur par ligne, a passer a `soak.py --require`).
set -euo pipefail
cd "$(dirname "$0")/../.."

TRAVAIL=${1:?usage: fabrique-scenario-endurance.sh TRAVAIL}
mkdir -p "$TRAVAIL"
TRAVAIL=$(cd "$TRAVAIL" && pwd)
SCENARIO="$TRAVAIL/scenario"
rm -rf "$SCENARIO" "$TRAVAIL/sondes"
mkdir -p "$SCENARIO/bin"

# Les sondes, compilees HORS du depot : l'arbre reste propre.
(cd tools/userland && OUT="$TRAVAIL/sondes" ./build.sh musl >/dev/null)
CC=musl-gcc ELF_REPORT="$TRAVAIL/mmstress.readelf.txt" tools/userland/build-mmstress.sh >/dev/null
mv mmstress "$SCENARIO/bin/mmstress"
for sonde in ordonnanceur-probe disque-probe wal-probe session-probe verrous-probe poll-bkl-probe exec-fd-probe; do
    cp "$TRAVAIL/sondes/$sonde" "$SCENARIO/bin/$sonde"
done

# Au-dela de INLINE_BOOT_FILE_SIZE (4 Mio) : le contenu est une etendue du
# disque, et `disque-probe` passe par le vrai chemin de lecture bloc.
python3 - "$SCENARIO/bin/gros.bin" <<'PY'
import sys
bloc = (b"E" * 63 + b"\n") * 16384
with open(sys.argv[1], "wb") as f:
    for _ in range(6):
        f.write(bloc)
PY

cat > "$SCENARIO/autorun" <<'AUTORUN'
smpstat
echo ENDURANCE_MEMOIRE_DEBUT && /bin/mmstress 4 512 4 && /bin/mmstress unrelated && /bin/mmstress aba && /bin/mmstress churn && echo ENDURANCE_MEMOIRE_OK
echo ENDURANCE_ORDONNANCEUR_DEBUT && /bin/ordonnanceur-probe 8 && echo ENDURANCE_ORDONNANCEUR_OK
echo ENDURANCE_STOCKAGE_DEBUT && /bin/disque-probe /bin/gros.bin && /bin/wal-probe && echo ENDURANCE_STOCKAGE_OK
echo ENDURANCE_PROCESSUS_DEBUT && /bin/session-probe 4 && /bin/verrous-probe && /bin/poll-bkl-probe && /bin/exec-fd-probe && echo ENDURANCE_PROCESSUS_OK
smpstat
echo ENDURANCE_CYCLE_FIN
AUTORUN

# Les marqueurs EXIGES de chaque cycle. Les relevés sont exiges ici aussi :
# un cycle ou `smpstat` n'a rien emis ne peut pas nourrir les budgets, et doit
# le dire lui-meme plutot que de laisser `check_budgets.py` le decouvrir.
cat > "$TRAVAIL/marqueurs-endurance.txt" <<'MARQUEURS'
ENDURANCE_MEMOIRE_OK
ENDURANCE_ORDONNANCEUR_OK
ENDURANCE_STOCKAGE_OK
ENDURANCE_PROCESSUS_OK
ENDURANCE_CYCLE_FIN
=== AUTORUN FIN === statut=0
[BKL-DOMAINES]
[SCHED-NG-LAT]
[SCHED-NG-CENTILES]
[SCHED-NG-FILE]
MARQUEURS

(cd tools/userland && IMAGE="$TRAVAIL/endurance.img" ./mkdisk.sh "$SCENARIO" >/dev/null)
test -s "$TRAVAIL/endurance.img"
sha256sum "$TRAVAIL/endurance.img" "$SCENARIO/autorun"
echo "SCENARIO_ENDURANCE_OK image=$TRAVAIL/endurance.img"
