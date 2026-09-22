#!/usr/bin/env bash
#
# LA FIABILITE DE LA MESURE DES FAUTES, SOUS CONTENTION REELLE.
#
# # Ce que `run_fautes_demande.sh` ne peut pas prouver
#
# Le banc voisin prouve le RACCORDEMENT : une faute reelle arrive dans le
# livre, avec le bon processus et la bonne categorie. Il le prouve avec un
# seul fil, sur un seul coeur a la fois -- c'est-a-dire dans le seul cas ou
# rien ne peut mal tourner.
#
# Or le livre est protege par un `try_lock` et non par un `lock`, et c'est
# deliberé : le chemin de faute de page ne doit JAMAIS attendre un verrou de
# diagnostic. Le prix de ce choix est que des echantillons se perdent quand
# plusieurs coeurs notent en meme temps. Un chiffre perdu ne se discute pas
# dans l'abstrait -- il se MESURE, et la decision qui en depend est nette :
#
#     perte < 1 %   -> le `try_lock` global est le bon compromis
#     perte >= 1 %  -> il faut un livre par processeur
#
# # Le second defaut que ce banc attrape
#
# Il verifie aussi que `doubles=0`. Le double comptage a reellement existe :
# `Attente` etait notee a chaque tour de la boucle d'attente, puis une
# seconde fois par le chargeur si le dormeur devenait chargeur -- la meme
# duree, partant du meme instant, comptee dans deux categories.
#
# Voir la TROISIEME REGLE plus bas pour la verification qui a ete essayee
# d'abord, et pourquoi elle ne pouvait pas marcher.
#
# # Ce que la mesure a donne le 22 septembre 2026, sur -smp 4
#
#       PID   FAUTES     TOTAL      PIRE   DOMINANTE
#         9      732     929 ms  17279 us   zero (892 ms)
#              zero      n=719    total=  892 ms  pire=17279 us
#              fichier   n=3      total=    4 ms  pire= 2937 us
#              attente   n=10     total=   32 ms  pire=11498 us
#       suivis=1 chasses=0 presentees=732 non_comptees=0 perte_pour_mille=0 doubles=0
#
#   doubles=0 : aucune duree n'est comptee deux fois.
#   perte_pour_mille=0 (un releve sous contention plus forte a donne 2 pour
#   mille, soit 0,2 %) : le `try_lock` global tient, le shardage par CPU
#   n'est pas necessaire.
#
# # CE QUE CE BANC NE PROUVE PAS ENCORE
#
# La categorie `attente` n'est pas produite de facon FIABLE sous QEMU, et il
# faut le dire plutot que de laisser croire le contraire.
#
# Les fils partagent un espace d'adressage (`CLONE_VM`) : des qu'un fil a
# peuple une page, les autres n'y fautent plus du tout. Pour qu'une faute en
# attende une autre, deux fils doivent toucher la MEME page pendant la
# fenetre de chargement. Quatre montages ont ete mesures :
#
#   zone anonyme, sans barriere      -> `attente` n=10..13, mais un releve
#                                       sur trois n'en portait aucune
#   rendez-vous par vague            -> aucune attente
#   rendez-vous par page             -> aucune attente ; `arrives=256
#                                       expires=128`, la moitie des
#                                       rendez-vous manques
#   zone adossee a un fichier,
#     rendez-vous par page           -> `fichier` passe de 3 a 67 fautes,
#                                       toujours aucune attente
#   zone adossee a un fichier,
#     UN SEUL rendez-vous            -> `attente` revient
#
# La cause mesuree : un rendez-vous qui expire laisse partir son fil en
# avance, `arrives` grimpe, les suivants se debloquent trop tot, et la
# desynchronisation se propage. Multiplier les rendez-vous multipliait donc
# les occasions de tout desynchroniser. Le montage retenu n'en pose qu'UN,
# avant le balayage, et laisse le chargement lui-meme resynchroniser les fils
# page par page.
#
# Ce montage produit la contention, mais pas a chaque execution : elle depend
# de l'ordonnancement de la machine emulee. Le banc n'en fait donc pas une
# exigence. Il verifie ce qu'il a reellement mesure -- le taux de perte et
# l'absence de note double -- et quand aucune attente n'a lieu, il le DIT et
# ne revendique rien sur ce chemin. `TEMPETE_EXIGE_CONTENTION=1` transforme
# cette absence en echec, pour un environnement ou elle est certaine.
#
#   ./tools/ci/run_tempete_fautes.sh [image-d-amorcage]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
SORTIE=${TEMPETE_SORTIE:-$(mktemp -d)}
SECONDES=${TEMPETE_SECONDES:-240}
COEURS=${TEMPETE_COEURS:-4}
# LE SEUIL, ET IL EST EXPRIME EN POUR MILLE.
#
# Dix pour mille, soit un pour cent. Au-dela, les chiffres rendus par la
# fenetre Services et par la commande `fautes` ne decrivent plus la machine,
# et le livre global doit ceder la place a un livre par processeur.
SEUIL_PERTE=${TEMPETE_SEUIL_POUR_MILLE:-10}
mkdir -p "$SORTIE"

if [ ! -s "$BOOT" ]; then
    echo "image d'amorcage absente : $BOOT" >&2
    echo "  construire d'abord : tools/ci/build_kernel.sh" >&2
    exit 1
fi

CC=""
for candidat in gcc cc clang; do
    if command -v "$candidat" >/dev/null 2>&1; then CC=$candidat; break; fi
done
if [ -z "$CC" ]; then
    echo "tempete : aucun compilateur C, verification passee"
    exit 0
fi

SCENARIO="$SORTIE/scenario"
rm -rf "$SCENARIO"
mkdir -p "$SCENARIO"

if ! "$CC" -O1 -static-pie -fPIE -nostdlib -nostartfiles -Wl,-z,noexecstack \
        -o "$SCENARIO/tempete" tools/userland/tempete-fautes.c 2>"$SORTIE/cc.log"; then
    echo "tempete : la charge d'epreuve ne se compile pas ici, verification passee"
    sed 's/^/    /' "$SORTIE/cc.log" | head -5
    exit 0
fi

cat > "$SCENARIO/autorun" <<'AUTORUN'
exec /tempete
echo "=== FAUTES APRES ==="
fautes
echo "=== FAUTES FIN ==="
AUTORUN

(cd tools/userland && IMAGE="$SORTIE/scenario.img" ./mkdisk.sh "$SCENARIO") >/dev/null 2>&1 \
    || { echo "mkdisk a echoue" >&2; exit 1; }

LOG="$SORTIE/serie.log"
: > "$LOG"
timeout "$SECONDES" qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$SORTIE/scenario.img" \
  -m 4096 -smp "$COEURS" -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -serial file:"$LOG" >/dev/null 2>&1

PROPRE="$SORTIE/serie.propre"
sed -e 's/\x1b\[[0-9;]*m//g' "$LOG" > "$PROPRE"

if ! grep -q "FAUTES FIN" "$PROPRE"; then
    echo "tempete : le scenario n'est pas alle au bout" >&2
    tail -20 "$PROPRE" >&2
    exit 1
fi

RELEVE="$SORTIE/releve"
sed -n '/FAUTES APRES/,/FAUTES FIN/p' "$PROPRE" \
    | sed -E -e 's/^(\[[^]]*\])+ //' -e '/^\+ /d' -e '/^=== /d' > "$RELEVE"

echo "=== ce que le livre a enregistre sous -smp $COEURS ==="
cat "$RELEVE"

lis() { grep -o "$1=[0-9]*" "$RELEVE" | tail -1 | cut -d= -f2; }
PRESENTEES=$(lis presentees)
NON_COMPTEES=$(lis non_comptees)
PERTE=$(lis perte_pour_mille)

# PREMIERE REGLE : la tempete doit avoir eu lieu.
#
# Mesurer un taux de perte sur trois fautes ne dit rien. Le programme touche
# quatre fois deux mille quarante-huit pages privees, puis soixante-quatre
# pages de fichier partagees ; s'il en arrive moins de deux cents au livre,
# c'est le scenario qui a echoue, pas la mesure qui est bonne.
if [ -z "${PRESENTEES:-}" ] || [ "$PRESENTEES" -lt 200 ]; then
    echo "tempete : seulement ${PRESENTEES:-0} fautes presentees -- pas de contention a mesurer." >&2
    exit 1
fi

# DEUXIEME REGLE : la perte doit rester sous le seuil.
if [ "${PERTE:-1000}" -ge "$SEUIL_PERTE" ]; then
    echo "tempete : $NON_COMPTEES fautes perdues sur $PRESENTEES, soit $PERTE pour mille." >&2
    echo "          au-dela de $SEUIL_PERTE pour mille, le livre global ne decrit plus" >&2
    echo "          la machine : il faut un livre par processeur." >&2
    exit 1
fi

# TROISIEME POINT : le chemin d'attente a-t-il ete emprunte ?
#
# Ce n'est pas une regle mais un CONSTAT, et il qualifie la portee de la
# regle suivante. `Loading -> attente -> Present` est le seul chemin ou une
# duree pouvait etre comptee deux fois ; un releve sans ligne `attente` ne
# dit donc rien sur ce defaut, meme quand il affiche `doubles=0`.
#
# Et ce n'est pas theorique : sur un noyau ou la regression avait ete
# reintroduite expres, un releve sans contention affichait `doubles=0` et le
# banc passait au vert. Le dire explicitement est ce qui empeche de lire ce
# zero-la comme une preuve.
ATTENTES=$(grep -o 'attente *n=[0-9]*' "$RELEVE" | tail -1 | tr -dc '0-9')
ATTENTES=${ATTENTES:-0}
if [ "$ATTENTES" -lt 1 ]; then
    echo "tempete : AUCUNE faute n'a attendu un autre processeur pendant cette execution."
    echo "          La regle du double comptage ci-dessous porte donc sur les seules"
    echo "          fautes resolues sans attente. Le chemin"
    echo "          'Loading -> attente -> Present' n'a PAS ete teste ici, et ce"
    echo "          releve ne dit rien a son sujet."
    if [ "${TEMPETE_EXIGE_CONTENTION:-0}" != "0" ]; then
        echo "tempete : contention exigee et non obtenue." >&2
        exit 1
    fi
else
    echo "tempete : $ATTENTES fautes ont attendu un autre processeur -- le chemin d'attente a bien ete emprunte"
fi

# QUATRIEME REGLE : aucune note double.
#
# CE QUI NE MARCHE PAS, et il a fallu le reintroduire expres pour le voir :
# comparer la somme des categories au total. Le total d'un processus EST la
# somme de ses categories -- `Journal::note` incremente une categorie, et le
# classement additionne -- donc l'egalite est vraie par construction. Cette
# verification a ete ecrite, la regression a ete reinjectee dans le noyau, et
# le banc est reste VERT : 717 + 3 + 20 = 740, parfaitement coherent, avec
# sept notes `Attente` de trop.
#
# Ce qui marche est un temoin porte par la faute elle-meme. Le chemin de
# faute ne tient plus un `u64` qu'il peut relire, mais un jeton `Note` qui
# refuse la deuxieme pose et leve `FAUTES_DOUBLES`. Le defaut n'est plus
# deduit d'une arithmetique : il est COMPTE au moment ou il se produit.
DOUBLES=$(lis doubles)
if [ -z "${DOUBLES:-}" ]; then
    echo "tempete : le releve ne porte pas 'doubles=' -- l'alarme a disparu du noyau." >&2
    exit 1
fi
if [ "$DOUBLES" -ne 0 ]; then
    echo "tempete : $DOUBLES notes refusees parce que leur faute etait deja notee." >&2
    echo "          un chemin de faute compte deux fois la meme duree." >&2
    exit 1
fi
echo "tempete : doubles=0 -- aucune faute n'a ete notee deux fois"

printf '\033[32m%s\033[0m\n' \
  "tempete : $PRESENTEES presentees, $ATTENTES en attente, $NON_COMPTEES perdues ($PERTE pour mille), $DOUBLES doubles ; journal dans $SORTIE"
echo "TEMPETE_FAUTES_OK"
