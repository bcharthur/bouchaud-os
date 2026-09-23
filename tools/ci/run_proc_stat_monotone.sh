#!/usr/bin/env bash
#
# `/proc/stat` NE DOIT JAMAIS RECULER, NI DEPASSER LA MACHINE.
#
# # Les trois proprietes, et pourquoi elles ne vont pas de soi
#
# `user` et `system` sont des compteurs CUMULATIFS. Un moniteur de charge --
# celui de Ladybird en est un -- calcule une occupation en soustrayant deux
# relevés successifs. Trois choses peuvent alors casser, et chacune se voit
# dans ce banc :
#
#   MONOTONIE      un compteur qui recule rend une difference negative, et le
#                  moniteur affiche n'importe quoi. C'est ce qui arrivait :
#                  le total etait la somme des taches VIVANTES, si bien qu'une
#                  tache qui meurt emportait son temps.
#
#   PLAFOND        `user + system` ne peut pas depasser le temps ecoule
#                  multiplie par le nombre de processeurs. Un depassement est
#                  la signature d'un DOUBLE COMPTAGE -- exactement le defaut
#                  qu'avait le livre des fautes, ou la meme duree etait
#                  imputee deux fois.
#
#   SATURATION     `idle = capacite - (user + system)` se calcule par
#                  `saturating_sub`. C'est le bon calcul, mais il est MUET :
#                  un depassement rendrait `idle=0` et `/proc/stat` aurait
#                  l'air sain. Le noyau compte donc les depassements
#                  separement, et ce banc lit ce compteur.
#
# # Ce que le banc ne peut pas faire, et qu'il ne pretend pas faire
#
# Il ne prouve pas que les valeurs soient JUSTES -- seulement qu'elles sont
# coherentes. Un noyau qui compterait la moitie du temps passerait ce banc.
# C'est la comparaison `cumulatif` / `somme_vivants` publiee a cote qui
# repond a cette question-la.
#
#   ./tools/ci/run_proc_stat_monotone.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${PROCSTAT_SORTIE:-$(mktemp -d)}
# Voir `run_cout_fork.sh` : un coureur de CI emule, et il est plus lent.
SECONDES=${PROCSTAT_SECONDES:-300}
# LE REPERTOIRE DE SORTIE EST RENDU ABSOLU, ET CE N'EST PAS DU CONFORT.
#
# `mkdisk.sh` est lance depuis `tools/userland` (il y cherche ses outils). Un
# chemin de sortie RELATIF y est donc resolu depuis `tools/userland` et non
# depuis la racine du depot -- l'image de scenario atterrit a cote, et le banc
# rend « mkdisk a echoue ».
#
# Le defaut ne se voyait pas en local, ou `mktemp -d` rend un chemin absolu. Il
# est apparu au premier passage en CI, qui passe `proc-stat-ci`.
mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    command -v "$candidat" >/dev/null 2>&1 && { CC=$candidat; break; }
done
# FERME PAR DEFAUT.
#
# « verification passee » sur machine sans compilateur, c'etait un VERT sans
# mesure. Un banc qui rend le meme code de sortie qu'il ait mesure ou non
# n'apprend rien a celui qui le lit, et le premier environnement mal outille
# fait disparaitre la propriete sans que personne ne le remarque.
if [ -z "$CC" ]; then
    echo "proc-stat : aucun compilateur C -- la charge d'epreuve ne peut pas etre construite" >&2
    echo "            ce banc ne sait pas conclure sans elle ; il echoue plutot que de passer" >&2
    exit 1
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"; mkdir -p "$SCENARIO"
if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/cumul" tools/userland/cumul-cpu.c 2>"$SORTIE/cc.log"; then
    echo "proc-stat : la charge d'epreuve ne se compile pas ici" >&2
    sed 's/^/    /' "$SORTIE/cc.log" | head -5 >&2
    exit 1
fi

# LA CHARGE EST CELLE QUI FAIT MOURIR DES PROCESSUS.
#
# C'est la mort d'une tache qui faisait reculer le compteur. Un scenario au
# repos ne pourrait pas le montrer.
printf 'exec /cumul\necho "=== PROCSTAT FIN ==="\n' > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp 4 -cpu max -display none -no-reboot \
  -serial file:"$LOG" >/dev/null 2>&1

sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$SORTIE/propre"
if ! grep -q "PROCSTAT FIN" "$SORTIE/propre"; then
    echo "proc-stat : le scenario n'est pas alle au bout" >&2
    tail -15 "$SORTIE/propre" >&2
    exit 1
fi

python3 - "$SORTIE/propre" <<'VERIF' || exit 1
import re, sys

lignes = open(sys.argv[1], errors="replace").read().splitlines()

# Les relevés vus par l'ANNEAU 3, c'est-a-dire ce que Ladybird lirait.
anneau3 = []
for l in lignes:
    m = re.search(r"CUMUL (\w+) cpu\s+(\d+) (\d+) (\d+) (\d+)", l)
    if m:
        anneau3.append((m.group(1), int(m.group(2)), int(m.group(4))))

# Les relevés internes du noyau, qui portent aussi les depassements.
depassements = 0
pire = 0
paires = []
decompositions = []
for l in lignes:
    m = re.search(r"publie_user_ms=(\d+).*?cumulatif_user_ms=(\d+).*?somme_vivants_user_ms=(\d+)", l)
    if m:
        paires.append((int(m.group(1)), int(m.group(2)), int(m.group(3))))
    d = re.search(r"zombies_ms=(\d+) temps_recycle_ms=(\d+) ecart_ms=(\d+) residu_ms=(\d+)", l)
    if d:
        decompositions.append(tuple(int(d.group(i)) for i in (1, 2, 3, 4)))
for l in lignes:
    m = re.search(r"depassements=(\d+) pire_depassement_ms=(\d+)", l)
    if m:
        depassements = max(depassements, int(m.group(1)))
        pire = max(pire, int(m.group(2)))

if len(anneau3) < 5:
    print(f"proc-stat : {len(anneau3)} releve(s) en anneau 3, cinq au moins attendus",
          file=sys.stderr)
    sys.exit(1)

# Divergence minimale, en millisecondes, sous laquelle la regle de provenance
# ne discrimine pas. La mesure de reference donnait 1113 ms ; 200 ms est un
# cinquieme de cela, assez bas pour ne pas dependre de la vitesse de la
# machine, assez haut pour que la comparaison ait un sens.
DIVERGENCE_MINIMALE = 200
# Part de l'ecart que ni la mort ni le recyclage n'expliquent, en pour cent.
# Voir BOUCHAUD_C33 : l'ecart mesure etait de 1113 ms et `temps_mort` n'en
# expliquait que 18. Tant que le residu est gros, une troisieme sortie de
# l'ensemble des vivants existe et n'a pas ete trouvee.
RESIDU_MAX_POUR_CENT = 10

echecs = []

# MONOTONIE, sur ce que voit l'anneau 3.
precedent = None
for nom, user, systeme in anneau3:
    if precedent is not None:
        pnom, puser, psys = precedent
        if user < puser:
            echecs.append(f"user recule : {pnom}={puser} puis {nom}={user}")
        if systeme < psys:
            echecs.append(f"system recule : {pnom}={psys} puis {nom}={systeme}")
    precedent = (nom, user, systeme)

# LA REGLE QUI ATTRAPE LE RETOUR A LA SOMME DES VIVANTS.
#
# La monotonie NE SUFFIT PAS, et la mesure l'a montre : en reinjectant la
# somme des taches vivantes, ce banc restait vert. La croissance des autres
# taches masque la perte, si bien que vingt et un relevés consecutifs peuvent
# monter sans que le compteur soit juste pour autant.
#
# Ce qui discrimine est la comparaison des DEUX calculs, que le noyau publie
# cote a cote. `/proc/stat` doit suivre le cumulatif ; s'il suit la somme des
# vivants, c'est que quelqu'un est revenu au calcul d'avant.
if paires:
    publie_ms, dernier_cum, dernier_viv = paires[-1]
    ecart = dernier_cum - dernier_viv
    # Le banc n'a de valeur que si les deux calculs DIVERGENT : sans mort de
    # tache ils coincident, et la comparaison ne prouverait rien. On le dit
    # plutot que de rendre un vert vide de sens.
    if ecart < DIVERGENCE_MINIMALE:
        # UN BANC QUI NE PEUT PAS CONCLURE N'EST PAS UN BANC QUI PASSE.
        #
        # La regle de provenance compare `publie` a deux valeurs. Tant qu'elles
        # sont proches, elle ne discrimine RIEN : la regression reinjectee
        # passerait. L'ancienne version le disait puis rendait zero, ce qui est
        # la definition d'un faux vert.
        echecs.append(
            f"INCONCLUSIF : les deux calculs ne divergent que de {ecart} ms "
            f"(minimum {DIVERGENCE_MINIMALE} ms) ; la regle de provenance ne "
            f"discrimine pas, la charge n'a pas fait mourir assez de taches")
    else:
        d_cum = abs(publie_ms - dernier_cum)
        d_viv = abs(publie_ms - dernier_viv)
        print(f"proc-stat : publie={publie_ms} ms, cumulatif={dernier_cum} ms, "
              f"somme_vivants={dernier_viv} ms")
        if d_viv < d_cum:
            echecs.append(
                f"/proc/stat suit la SOMME DES VIVANTS ({dernier_viv} ms) et non le "
                f"cumulatif ({dernier_cum} ms) : le temps des taches mortes est perdu")

# L'ECART DOIT ETRE EXPLIQUE, PAS SEULEMENT CONSTATE.
#
# BOUCHAUD_C33_L_ECART_INEXPLIQUE
#
# Le releve precedent donnait un ecart de 1113 ms dont `temps_mort` expliquait
# 18. Constater l'ecart prouve que l'ancien calcul perdait du temps ; ne pas
# savoir OU il part laisse ouverte la possibilite que le CUMULATIF soit lui
# aussi faux -- par exemple s'il comptait du temps deux fois.
#
# Deux sorties de l'ensemble des vivants sont nommees et comptees : la mort
# (`marque_zombie`) et le RECYCLAGE d'emplacement (`*ancienne = *tache`, qui
# efface les compteurs de l'incarnation precedente). Leur somme doit couvrir
# l'ecart. Ce qui reste est le `residu` : s'il est gros, il y a une troisieme
# sortie, et P18 ne se ferme pas.
if decompositions:
    mort, recycle, ecart_k, residu = decompositions[-1]
    print(f"proc-stat : ecart={ecart_k} ms = zombies {mort} + recycle {recycle} "
          f"+ residu {residu}")
    if ecart_k >= DIVERGENCE_MINIMALE:
        part = (residu * 100) // max(ecart_k, 1)
        if part > RESIDU_MAX_POUR_CENT:
            echecs.append(
                f"{part} % de l'ecart ({residu} ms sur {ecart_k}) n'est explique "
                f"ni par les zombies ni par le recyclage : une troisieme sortie de "
                f"l'ensemble des vivants existe et n'a pas ete trouvee")
else:
    echecs.append("aucune decomposition de l'ecart dans le journal : "
                  "le noyau ne publie pas zombies_ms/temps_recycle_ms/residu_ms")

# PLAFOND : le noyau le verifie a chaque releve et compte les depassements.
if depassements:
    echecs.append(
        f"user+system a depasse la capacite {depassements} fois "
        f"(pire ecart {pire} ms) -- signature d'un double comptage")

print(f"proc-stat : {len(anneau3)} releves en anneau 3, "
      f"user {anneau3[0][1]} -> {anneau3[-1][1]}, "
      f"system {anneau3[0][2]} -> {anneau3[-1][2]}, depassements={depassements}")

if echecs:
    print("proc-stat : les compteurs ne tiennent pas leurs proprietes", file=sys.stderr)
    for e in echecs:
        print("   ", e, file=sys.stderr)
    sys.exit(1)
VERIF

printf '\033[32m%s\033[0m\n' "proc-stat : monotone, sous le plafond ; journal dans $SORTIE"
echo "PROC_STAT_MONOTONE_OK"
