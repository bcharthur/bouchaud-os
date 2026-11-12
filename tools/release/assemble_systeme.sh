#!/usr/bin/env bash
# BOUCHAUD_LIVRAISON_V1 -- le systeme complet d'un slot A/B (le contenu de son ramdisk).
#
#   tools/release/assemble_systeme.sh SORTIE [NATIF_LADYBIRD]
#
# Produit dans SORTIE l'arborescence que le noyau deplie dans son RAMFS au
# demarrage d'un slot, la MEME que la cle de reference Stage 2 (Ladybird,
# certificats, polices, page d'accueil -- tools/reference/prepare-reference-ladybird.ps1)
# plus ce qu'un systeme INSTALLE exige :
#   bin/serviced healthd updated logd   superviseur et services critiques (sante, mise a jour, journal)
#   bin/composited                      compositeur ring 3 (disponible ; le bureau par defaut reste l'Atelier)
#   doom, freedoom1.wad                 DOOM (doomgeneric epingle, Freedoom 0.13.0 verifie)
#   bo-navigateur, usr/libexec/ladybird, usr/share/ladybird, etc/ssl/certs
#                                       Ladybird et ses processus (artefact natif CI)
#   etc/bouchaud/version                version, commit, sequence, canal
#   etc/bouchaud/ota.conf               agent OTA : canal stable (GitHub latest/download),
#                                       telechargement automatique, redemarrage SUR CONSENTEMENT
#   etc/bouchaud/composants             ce qui est embarque, et ce qui MANQUE (fail closed)
#
# NATIF_LADYBIRD : le repertoire de l'artefact `bouchaud-ladybird-native-browser`.
# Absent : le systeme est assemble SANS Ladybird et `composants` le dit
# (livrable=non) -- jamais un faux complet.
#
# Variables : BO_VERSION (defaut : dev-<sha12>), BO_SEQUENCE (defaut 0 : build
# de developpement, que l'anti-retour ne confondra jamais avec une release),
# BO_OTA_SOURCE (defaut : releases/latest/download du depot).
set -euo pipefail
cd "$(dirname "$0")/../.."
SORTIE=${1:?usage: assemble_systeme.sh SORTIE [NATIF_LADYBIRD]}
NATIF=${2:-}
[ ! -e "$SORTIE" ] || [ -z "$(ls -A "$SORTIE")" ] || { echo "SYSTEME_REFUSE sortie_non_vide $SORTIE" >&2; exit 2; }
mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)
COMMIT=$(git rev-parse HEAD)
VERSION=${BO_VERSION:-dev-${COMMIT:0:12}}
SEQUENCE=${BO_SEQUENCE:-0}
SOURCE=${BO_OTA_SOURCE:-https://github.com/bcharthur/bouchaud-os/releases/latest/download}
[[ "$SEQUENCE" =~ ^[0-9]{1,19}$ ]] || { echo "SYSTEME_REFUSE sequence_invalide" >&2; exit 2; }
TRAVAIL=$(mktemp -d)
trap 'rm -rf -- "$TRAVAIL"' EXIT
mkdir -p "$SORTIE/bin" "$SORTIE/etc/bouchaud"

# --- services (BOGU, SDK natif) -----------------------------------------------
SDK=userland/libs/bouchaud-sdk/build-program.sh
for s in serviced healthd updated logd; do
  "$SDK" "$SORTIE/bin/$s" "userland/services/$s/$s.c"
done

# --- compositeur ring 3 ---------------------------------------------------------
(cd userland/services/composited && OUT="$TRAVAIL/composited" bash ./build.sh >/dev/null)
cp "$TRAVAIL/composited/composited-slice" "$SORTIE/bin/composited"

# --- DOOM ------------------------------------------------------------------------
[ -x target/doom-bouchaud/doom-bouchaud ] || tools/doom/build-linux.sh >/dev/null
cp target/doom-bouchaud/doom-bouchaud "$SORTIE/doom"
cp tools/doom/wads/freedoom1.wad "$SORTIE/freedoom1.wad"

# --- Ladybird ----------------------------------------------------------------------
LADYBIRD=absent
if [ -n "$NATIF" ]; then
  for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder Compositor WebWorker; do
    [ -f "$NATIF/$f" ] || { echo "SYSTEME_REFUSE artefact_ladybird_incomplet $f" >&2; exit 2; }
  done
  [ -d "$NATIF/resources" ] || { echo "SYSTEME_REFUSE artefact_ladybird_sans_resources" >&2; exit 2; }
  mkdir -p "$SORTIE/usr/libexec/ladybird" "$SORTIE/usr/share/ladybird/fontconfig" "$SORTIE/etc/ssl/certs"
  for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder Compositor WebWorker WebDriver webcontent-bootstrap; do
    [ -f "$NATIF/$f" ] && cp "$NATIF/$f" "$SORTIE/usr/libexec/ladybird/$f"
  done
  cp "$NATIF/BouchaudBrowserHost" "$SORTIE/bo-navigateur"
  cp -a "$NATIF/resources/." "$SORTIE/usr/share/ladybird/"
  cp tools/ladybird/fontconfig/fonts.conf "$SORTIE/usr/share/ladybird/fontconfig/fonts.conf"
  cp tools/ladybird/start.html "$SORTIE/usr/share/ladybird/bouchaud-start.html"
  # Le bundle CA de Ladybird (son TLS a lui, en ring 3) ; celui du noyau est compile.
  if [ -f tools/ladybird/certs/cacert.pem ]; then CA=tools/ladybird/certs/cacert.pem; else CA=/etc/ssl/certs/ca-certificates.crt; fi
  [ "$(stat -c %s "$CA")" -ge 1024 ] || { echo "SYSTEME_REFUSE bundle_ca_anormal" >&2; exit 2; }
  cp "$CA" "$SORTIE/etc/ssl/certs/ca-certificates.crt"
  printf 'Stage 2 FINAL V2 Ladybird\n' > "$SORTIE/etc/bouchaud-stage2-ladybird"
  chmod 755 "$SORTIE/bo-navigateur" "$SORTIE/usr/libexec/ladybird/"*
  LADYBIRD=present
fi
chmod 755 "$SORTIE/bin/"* "$SORTIE/doom"

# --- identite et OTA -----------------------------------------------------------------
printf 'version=%s\ncommit=%s\nsequence=%s\ncanal=stable\nmateriel=x86_64-uefi\n' \
  "$VERSION" "$COMMIT" "$SEQUENCE" > "$SORTIE/etc/bouchaud/version"
cat > "$SORTIE/etc/bouchaud/ota.conf" <<EOF
# BOUCHAUD_OTA_V1 -- agent de mise a jour (src/kernel/update/ota.rs).
# Canal stable : la derniere release NON prerelease du depot, c'est-a-dire une
# release promue testing -> stable (.github/workflows/release-trigkey.yml).
canal=stable
source=$SOURCE
# Telecharger et preparer le slot inactif sans demander.
telechargement=consenti
# Redemarrer : JAMAIS sans consentement (bo-update consentir, ou le bureau).
redemarrage=demande
EOF

LIVRABLE=oui; [ "$LADYBIRD" = present ] || LIVRABLE=non
{
  printf 'services=serviced,healthd,updated,logd\ncomposited=present\ndoom=present\nladybird=%s\n' "$LADYBIRD"
  printf 'ota=stable\nlivrable=%s\n' "$LIVRABLE"
} > "$SORTIE/etc/bouchaud/composants"
echo "SYSTEME_ASSEMBLE version=$VERSION commit=${COMMIT:0:12} sequence=$SEQUENCE ladybird=$LADYBIRD livrable=$LIVRABLE fichiers=$(find "$SORTIE" -type f | wc -l) octets=$(du -sb "$SORTIE" | cut -f1)"
