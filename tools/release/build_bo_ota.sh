#!/usr/bin/env bash
# BOUCHAUD_OTA_ANNEAU3_V1 -- construit l'agent OTA de l'anneau 3 (/bin/bo-ota).
#
#   build_bo_ota.sh SORTIE [--banc]
#
# Binaire Linux statique (musl, static-pie), Rust std, avec la pile TLS du
# depot (system/tls-core : les fichiers du noyau). --banc ajoute la CA de
# BANC (feature banc-ca-dev, exige BOUCHAUD_CA_BANC_DEV) : jamais livre
# (tools/verifie-cle-dev.py, tools/release/assemble_systeme.sh refuse).
#
# Construit HORS de l'arbre : la configuration cargo du depot (.cargo/
# config.toml) impose build-std et la cible du noyau a tout ce qui s'y
# compile, et la decouverte de configuration suit le repertoire courant.
set -euo pipefail
SORTIE=${1:?usage: build_bo_ota.sh SORTIE [--banc]}
RACINE=$(cd "$(dirname "$0")/../.." && pwd)
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$RACINE/rust-toolchain.toml")
FEATURES=()
if [ "${2:-}" = --banc ]; then
  [ -n "${BOUCHAUD_CA_BANC_DEV:-}" ] || { echo "BO_OTA_BUILD_REFUS --banc sans BOUCHAUD_CA_BANC_DEV" >&2; exit 2; }
  FEATURES=(--features banc-ca-dev)
elif [ -n "${BOUCHAUD_CA_BANC_DEV:-}" ]; then
  # La variable seule ferait echouer roots.rs (garde de construction) ; on le
  # dit avant cargo.
  echo "BO_OTA_BUILD_REFUS BOUCHAUD_CA_BANC_DEV pose sans --banc" >&2; exit 2
fi
rustup target add --toolchain "$TOOLCHAIN" x86_64-unknown-linux-musl >/dev/null 2>&1 || true
CIBLE=${BO_OTA_TARGET_DIR:-$RACINE/target/bo-ota}
( cd / && CARGO_TARGET_DIR="$CIBLE" cargo +"$TOOLCHAIN" build --release -q \
    --manifest-path "$RACINE/userland/services/ota/Cargo.toml" \
    --target x86_64-unknown-linux-musl "${FEATURES[@]}" )
mkdir -p "$(dirname "$SORTIE")"
cp "$CIBLE/x86_64-unknown-linux-musl/release/bo-ota" "$SORTIE"
# Un binaire dynamique ne demarrerait pas (aucun chargeur musl dans le systeme).
# (grep -c lit tout : aucun SIGPIPE du producteur sous pipefail.)
if [ "$(readelf -lW "$SORTIE" | grep -c 'INTERP' || true)" != 0 ]; then
  echo "BO_OTA_BUILD_REFUS binaire_dynamique $SORTIE" >&2; exit 1
fi
echo "BO_OTA_BUILD_OK sortie=$SORTIE octets=$(stat -c %s "$SORTIE") banc=$([ "${2:-}" = --banc ] && echo 1 || echo 0) sha256=$(sha256sum "$SORTIE" | cut -c1-16)"
