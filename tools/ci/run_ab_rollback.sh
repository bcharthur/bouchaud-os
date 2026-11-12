#!/usr/bin/env bash
# A/B rollback, end to end, under QEMU + OVMF (BOUCHAUD_LOADER_AB_V1).
#
#   tools/ci/run_ab_rollback.sh SCENARIO [OUT_DIR]
#
# SCENARIO:
#   panique    slot B = candidate kernel that panics on every boot
#   gel        slot B = candidate kernel that freezes on every boot
#   meta-0     copy 0 of the metadata corrupted -> copy 1 used
#   meta-1     copy 1 corrupted -> copy 0 used
#   meta-2     both copies corrupted -> RECOVERY_REQUIRED, nothing booted
#   sante      slot B = good system (serviced + healthd in its ramdisk):
#              serviced declares health -> the candidate is COMMITTED
#   sans-vidange  with BO_AB_SUPPORT=usb: slot B = good system whose USB
#              key refuses SYNCHRONIZE CACHE -> no commit ever, rollback to A
#   seconde-cle   with BO_AB_SUPPORT=usb-deux: the A/B key flushes, a second
#              key refuses -> the second key changes nothing, B commits
#   sante-ko   slot B = good kernel, but its critical service never becomes
#              ready: serviced -> RecoveryRequired, never commits -> rollback
#   mise-a-jour  A runs, B is zeroed and Empty. updated (ring 3, install
#              capability only) checks its refusals, writes B from the image
#              in A's ramdisk, the kernel re-reads it and marks it Candidate;
#              then B boots, is confirmed by serviced, and stays.
#
# BO_AB_SUPPORT=nvme|usb|usb-deux selects the disk transport (default nvme);
# usb-deux adds a second, blank USB key on port 2.
#
# Each "boot" is a fresh QEMU process on the SAME disk image (state persists
# between boots exactly as it would on a disk). A candidate failure ends by
# KILLING QEMU (SIGKILL): a power cut, never a clean shutdown. The loader
# (UEFI preboot shim) prints `BOUCHAUD_AB_BOOT slot=X raison=...` before the
# handoff; the bench checks the slot sequence:
#   panique / gel : B B B A A   (3 attempts, then automatic rollback)
#   meta-0/meta-1 : B B B A     (the surviving copy carries the candidate)
#   meta-2        : RECOVERY_REQUIRED, no kernel ever starts
#   sante         : B+ B=        (committed, then B is the good active slot)
#   sante-ko      : B- B- B- A= A=
#   mise-a-jour   : A* B+ B=     (A*: install + every refusal observed)
#   sans-cle      : A0 A=        a RELEASE build (no feature banc-cle-dev, no
#                                key): the kernel trusts 0 keys, begin is
#                                refused aucune_cle_de_confiance even for a
#                                validly signed manifest; B stays Empty; the
#                                bench key is absent from the kernel ELF
#   recovery-requise  : R        both copies invalid, RECOVERY partition present:
#                                recovery boots (no supervisor, no desktop),
#                                says META is unreadable, refuses a rollback
#   recovery-rollback : R B=     'r' at the boot prompt -> recovery; verifie A/B
#                                against META, rollback refused without
#                                --confirme, done with it; then B boots
#   echecs-repetes    : A- A- A- R  the ACTIVE slot never confirms health:
#                                3 boots, then recovery (counter = 3)
# Recovery scenarios talk to the machine over the serial line (OVMF ConIn
# for the key, then the recovery console).
# (B+ commit done, B=/A= already the good slot, B- RecoveryRequired.)
# The disk is an NVMe namespace: the kernel finds META there by the unique
# GUID the loader handed over.
# Fail-closed: a missing marker, an unexpected slot, a good kernel that does
# not start, or a kernel started under RECOVERY_REQUIRED is red.
set -euo pipefail
cd "$(dirname "$0")/../.."
SCENARIO=${1:?usage: run_ab_rollback.sh panique|gel|meta-0|meta-1|meta-2|sante|sante-ko|sans-vidange|seconde-cle|mise-a-jour|mise-a-jour-morts|sans-cle|ota-verifie|ota-complet|ota-rollback|ota-reseau|recovery-requise|recovery-rollback|echecs-repetes [OUT]}
OTA_COMPLET=0; case "$1" in ota-complet|ota-rollback|ota-reseau) OTA_COMPLET=1 ;; esac
# Demarrage ou l'agent installe et redemarre vers B (ota-reseau : le second).
BOOT_INSTALLE=1; [ "$1" = ota-reseau ] && BOOT_INSTALLE=2
RECOVERY=0
# mise-a-jour-morts = mise-a-jour + updated dies on purpose twice (during the
# writes, during the read-back); the third instance must inherit nothing.
VARIANTE=""
# BOUCHAUD_ANTI_RETOUR_V1 (mise-a-jour): release of system A = floor at start;
# the update is release PLANCHER_INITIAL + 1.
PLANCHER_INITIAL=1000
RELEASE_SEQ=1001
if [ "$SCENARIO" = mise-a-jour-morts ]; then SCENARIO=mise-a-jour; VARIANTE=morts; fi
OUT=${2:-target/ab-rollback-$SCENARIO}
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
FEATURES=uefi-boot,reference-bringup,reference-desktop
BOOT_DELAI_S=${BO_AB_BOOT_S:-150}
rm -rf "$OUT"; mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

case "$SCENARIO" in
  panique|gel) MAUVAIS=$SCENARIO; META_SPEC=b-candidat:3; ATTENDU="B B B A A" ;;
  meta-0) MAUVAIS=panique; META_SPEC=corrompu:0; ATTENDU="B B B A" ;;
  meta-1) MAUVAIS=panique; META_SPEC=corrompu:1; ATTENDU="B B B A" ;;
  meta-2) MAUVAIS=panique; META_SPEC=corrompu:2; ATTENDU="RECOVERY" ;;
  sante) MAUVAIS=bon; META_SPEC=b-candidat:3; ATTENDU="B+ B="; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  sante-ko) MAUVAIS=bon; META_SPEC=b-candidat:3; ATTENDU="B- B- B- A= A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  # Temoin negatif de BOUCHAUD_USB_VIDANGE_REELLE_V1 (a jouer avec
  # BO_AB_SUPPORT=usb) : B est un bon systeme, mais son support refuse
  # SYNCHRONIZE CACHE. Aucun commit ne doit avoir lieu : B- trois fois, puis A.
  sans-vidange) MAUVAIS=sans-vidange; META_SPEC=b-candidat:3; ATTENDU="B- B- B- A= A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  # BOUCHAUD_USB_LIAISON_SUPPORT_V1 (a jouer avec BO_AB_SUPPORT=usb-deux) : la
  # cle de demarrage vide, une SECONDE cle refuse SYNCHRONIZE CACHE. La sonde
  # de la seconde ne doit pas retirer la promesse de la premiere : B commit.
  seconde-cle) MAUVAIS=seconde-cle-sans-vidange; META_SPEC=b-candidat:3; ATTENDU="B+ B="; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  mise-a-jour) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A* B+ B="; BOOT_DELAI_S=${BO_AB_BOOT_S:-600} ;;
  sans-cle) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A0 A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-600} ;;
  # BOUCHAUD_OTA_V1 : l'agent OTA verifie six sources HTTP servies par l'hote
  # (une disponible, une a jour, quatre a refuser). A demarre sain et RIEN
  # n'est installe : la verification ne telecharge pas d'image.
  ota-verifie) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-600} ;;
  # BOUCHAUD_OTA_V1, demonstration COMPLETE : A (release 1000) telecharge en
  # HTTPS (CA de banc) la release 1001 -- un AUTRE noyau (BOUCHAUD_BUILD
  # commit=ota-v2-...) --, updated l'installe dans B, redemarrage consenti, B
  # confirme sa sante, le plancher passe a 1001, STATE (compteur) traverse tout.
  ota-complet) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A* B+ B="; BOOT_DELAI_S=${BO_AB_BOOT_S:-1500}; export BO_AB_STATE_MIO=256 ;;
  # La meme chaine, mais la release 1001 a un service critique defaillant :
  # trois demarrages B refuses, retour automatique a A, STATE intacte, et la
  # release en echec n'est PLUS jamais telechargee.
  ota-rollback) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A* B- B- B- A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-1500}; export BO_AB_STATE_MIO=256 ;;
  # BOUCHAUD_OTA_RESEAU_V1 (campagne D) : la meme release 1001, servie par un
  # reseau hostile. Demarrage 1 (A) : une source au certificat d'une CA
  # INCONNUE et une source en erreur HTTP 500 sont refusees ; la bonne source
  # sert d'abord une partie ALTEREE (refus, relance bornee a 60 s), puis une
  # partie TRONQUEE (reprise), puis le lien tombe 20 s (set_link, reprise),
  # puis l'ALIMENTATION est coupee en plein telechargement (A~). Demarrage 2
  # (A, META intact) : le telechargement interrompu est vu dans STATE, tout
  # recommence sous signature, installe, redemarre (A*). Demarrage 3 : B+.
  ota-reseau) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A~ A* B+"; BOOT_DELAI_S=${BO_AB_BOOT_S:-1500}; export BO_AB_STATE_MIO=256 ;;
  recovery-requise) MAUVAIS=bon; META_SPEC=corrompu:2; ATTENDU="R"; RECOVERY=1 ;;
  recovery-rollback) MAUVAIS=bon; META_SPEC=b-bon; ATTENDU="R B="; RECOVERY=1; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  echecs-repetes) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A- A- A- R"; RECOVERY=1; BOOT_DELAI_S=${BO_AB_BOOT_S:-300} ;;
  *) echo "scenario inconnu: $SCENARIO" >&2; exit 2 ;;
esac

# --- release key (mise-a-jour only): generated HERE, deleted at exit -------
# A development key: its public half is compiled into the kernel built below
# (BOUCHAUD_CLE_VERSION_DEV), its private half signs the manifest, then the
# whole directory goes. No key is ever stored in the repository.
CLES=$(mktemp -d); trap 'rm -rf "$CLES"' EXIT
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ] || [ "$SCENARIO" = ota-verifie ] || [ "$OTA_COMPLET" = 1 ]; then
  openssl genpkey -algorithm ed25519 -out "$CLES/version.pem"
  CLE_PUBLIQUE=$(openssl pkey -in "$CLES/version.pem" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
  echo "AB_CLE_DEV publique=$CLE_PUBLIQUE (generee pour ce banc, detruite a la fin)"
  # sans-cle: the manifest IS signed with it, but the kernel is a normal
  # build that does not trust it.
  if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = ota-verifie ] || [ "$OTA_COMPLET" = 1 ]; then export BOUCHAUD_CLE_VERSION_DEV=$CLE_PUBLIQUE; fi
  # ota-verifie : une SECONDE cle, que le noyau ne connait pas.
  [ "$SCENARIO" = ota-verifie ] && openssl genpkey -algorithm ed25519 -out "$CLES/autre.pem"
fi
# BOUCHAUD_CA_BANC_V1 (ota-complet, ota-rollback) : une CA de BANC (P-256),
# nee et morte ici, et le certificat du serveur HTTPS de l'hote (SAN
# DNS:10.0.2.2, l'hote vu de QEMU). Le noyau de banc lui fait confiance
# (feature banc-ca-dev + BOUCHAUD_CA_BANC_DEV) ; aucun noyau livrable.
if [ "$OTA_COMPLET" = 1 ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 \
    -subj "/CN=Bouchaud banc OTA CA" -keyout "$CLES/ca.key" -out "$CLES/ca.pem" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  openssl x509 -in "$CLES/ca.pem" -outform DER -out "$CLES/ca.der"
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj "/CN=10.0.2.2" \
    -keyout "$CLES/srv.key" -out "$CLES/srv.csr" 2>/dev/null
  printf 'subjectAltName=DNS:10.0.2.2\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > "$CLES/srv.ext"
  openssl x509 -req -in "$CLES/srv.csr" -CA "$CLES/ca.pem" -CAkey "$CLES/ca.key" -CAcreateserial -days 2 \
    -extfile "$CLES/srv.ext" -out "$CLES/srv.pem" 2>/dev/null
  export BOUCHAUD_CA_BANC_DEV="$CLES/ca.der"
  if [ "$SCENARIO" = ota-reseau ]; then
    # Une SECONDE CA, que le noyau ne connait pas, et un serveur qu'elle signe
    # (meme nom, meme adresse) : seul le certificat le distingue.
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 \
      -subj "/CN=Bouchaud banc CA INCONNUE" -keyout "$CLES/ca-inconnue.key" -out "$CLES/ca-inconnue.pem" \
      -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
    openssl x509 -req -in "$CLES/srv.csr" -CA "$CLES/ca-inconnue.pem" -CAkey "$CLES/ca-inconnue.key" -CAcreateserial -days 2 \
      -extfile "$CLES/srv.ext" -out "$CLES/srv-inconnu.pem" 2>/dev/null
  fi
fi

# --- build: shim, kernels, payloads, metadata, disk ------------------------
cargo +"$TOOLCHAIN" build --release --manifest-path tools/reference/uefi-preboot-probe/Cargo.toml \
  --target x86_64-unknown-uefi
SHIM=tools/reference/uefi-preboot-probe/target/x86_64-unknown-uefi/release/bouchaud-uefi-preboot-probe.efi
for k in $(printf '%s\n' bon "$MAUVAIS" | sort -u); do
  f=$FEATURES; [ "$k" != bon ] && f="$FEATURES,banc-ab-$k"
  # The bench key needs BOTH the variable and this feature (BOUCHAUD_CLE_DEV_GARDE_V1).
  if [ -n "${BOUCHAUD_CLE_VERSION_DEV:-}" ]; then f="$f,banc-cle-dev"; fi
  if [ -n "${BOUCHAUD_CA_BANC_DEV:-}" ]; then f="$f,banc-ca-dev"; fi
  cargo +"$TOOLCHAIN" build --target targets/x86_64-bouchaud_os_uefi.json --no-default-features --features "$f"
  # Debug info is for the host (symbolisation), not for the slot: stripping it
  # keeps a slot image small enough to be shipped and written by updated.
  strip --strip-debug -o "$OUT/kernel-$k.elf" target/x86_64-bouchaud_os_uefi/debug/bouchaud-os
done
if [ "$OTA_COMPLET" = 1 ]; then
  # Version 2 : un AUTRE noyau (identite de construction differente, octets
  # differents), meme politique. Le journal le dit : BOUCHAUD_BUILD commit=ota-v2-...
  V2="ota-v2-$(git rev-parse --short=12 HEAD)"
  BOUCHAUD_BUILD_COMMIT="$V2" cargo +"$TOOLCHAIN" build --target targets/x86_64-bouchaud_os_uefi.json \
    --no-default-features --features "$FEATURES,banc-cle-dev,banc-ca-dev"
  strip --strip-debug -o "$OUT/kernel-v2.elf" target/x86_64-bouchaud_os_uefi/debug/bouchaud-os
  if cmp -s "$OUT/kernel-bon.elf" "$OUT/kernel-v2.elf"; then
    echo "AB_ROLLBACK_FAIL raison=versions_identiques" >&2; exit 1
  fi
  echo "AB_OTA_VERSIONS v1=$(sha256sum "$OUT/kernel-bon.elf" | cut -c1-16) v2=$(sha256sum "$OUT/kernel-v2.elf" | cut -c1-16) build_v2=$V2"
fi
# BOUCHAUD_AB_DIAGNOSTIC_ECHEC_V1 : en échec, les lignes décisives de chaque
# démarrage vont dans le journal du job. Les artefacts boot-*.log ne sont pas
# toujours lisibles (rétention, accès), et un `B-` doit pouvoir s'attribuer
# sans eux : E/S et reprises USB, vidange, état et commit de serviced, refus
# du noyau, services. Diagnostic seulement : aucun verdict n'en dépend.
diagnostic_echec() {
  local f n propre
  for f in "$OUT"/boot-*.log; do
    [ -s "$f" ] || continue
    n=$(basename "$f" .log)
    propre=$(sed 's/\x1b\[[0-9;]*m//g' "$f" | tr -d '\r' \
      | sed -E 's/^(\[[0-9:]+\])\[[^]]*\](\[FPS:[^]]*\])? ?/\1 /' | awk '!vu[$0]++')
    echo "AB_DIAG ---- $n"
    # 1. Verdict : démarrage, sonde de vidange, état et commit, refus, services.
    grep -aE "BOUCHAUD_AB_(BOOT|PASSAGE)|BOUCHAUD_USB_STOCKAGE_VIDANGE|BOUCHAUD_USB_(REFUS_VIDANGE|VOLUME_ORPHELIN)|BOUCHAUD_UPDATE_SANTE|SERVICED_(UPDATE_STATE|HEALTH_COMMIT)|\[SERVICED\] (system|svc=[a-z]+ [A-Za-z]+->(Failed|Stopped))|KERNEL PANIC|panicked at" \
      <<< "$propre" | awk -v n="$n" 'c < 40 { print "AB_DIAG " n " " $0; c++ }' || true
    # 1b. BOUCHAUD_OTA_V1 : agent, updated, transport HTTPS.
    grep -aE "BOUCHAUD_OTA_|UPDATED_(OTA|INSTALL|SKIP|IDLE|PROGRESS)|nautile/(ERR|NET)\] (https|\[TLS)|BOUCHAUD_PERSIST_(MONTAGE|DEMARRAGES)" \
      <<< "$propre" | awk -v n="$n" 'c < 60 { print "AB_DIAG " n " " $0; c++ }' || true
    # 2. Transport USB : décompte, puis les premiers échecs d'E/S.
    echo "AB_DIAG $n usb io_echec=$(grep -ac 'BOUCHAUD_USB_STOCKAGE_IO_ECHEC' <<< "$propre" || true) reprise_ok=$(grep -ac 'BOUCHAUD_USB_BOT_REPRISE_OK' <<< "$propre" || true) reprise_echec=$(grep -ac 'BOUCHAUD_USB_BOT_REPRISE_ECHEC' <<< "$propre" || true) vidange_refusee=$(grep -ac 'BOUCHAUD_USB_STOCKAGE_VIDANGE_REFUSEE' <<< "$propre" || true)"
    # awk lit tout (pas de `head`) : sous pipefail, aucun SIGPIPE du producteur.
    # `|| true` : un grep sans correspondance (code 1) ne doit pas, sous set -e,
    # interrompre le diagnostic des démarrages suivants (run 41916742669).
    grep -a "BOUCHAUD_USB_STOCKAGE_IO_ECHEC" <<< "$propre" | awk -v n="$n" 'c < 8 { print "AB_DIAG " n " " $0; c++ }' || true
  done
}
TRAVAIL=$(mktemp -d)
trap 'rc=$?; [ "$rc" -ne 0 ] && diagnostic_echec; rm -rf "$TRAVAIL" "$CLES"' EXIT
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
( cd "$TRAVAIL/builder" && cargo +"$TOOLCHAIN" build --release -q )
BUILDER=$TRAVAIL/builder/target/release/bouchaud-reference-uefi-image-builder
"$BUILDER" --charge-installation "$OUT/kernel-bon.elf" "$OUT/slot-a"
if [ "$OTA_COMPLET" = 1 ]; then
  "$BUILDER" --charge-installation "$OUT/kernel-v2.elf" "$OUT/slot-b"
else
  "$BUILDER" --charge-installation "$OUT/kernel-$MAUVAIS.elf" "$OUT/slot-b"
fi
# A tar ramdisk: ramdisk OUT name=src [name=src ...]
ramdisk() {
  python3 - "$@" <<'PY'
import io, sys, tarfile
out, entries = sys.argv[1], sys.argv[2:]
with tarfile.open(out, "w", format=tarfile.USTAR_FORMAT) as t:
    for e in entries:
        name, src = e.split("=", 1)
        data = open(src, "rb").read()
        i = tarfile.TarInfo(name); i.size = len(data); i.mtime = 0
        i.mode = 0o755 if name.startswith("bin/") else 0o644
        t.addfile(i, io.BytesIO(data))
PY
}
case "$SCENARIO" in sante|sante-ko|sans-vidange|seconde-cle|mise-a-jour|sans-cle|ota-verifie|ota-complet|ota-rollback|ota-reseau|recovery-rollback|echecs-repetes)
  # The system userland of each slot: serviced (system supervisor) and its
  # critical services (health contract v3: healthd, updated, logd). In sante-ko the
  # CANDIDATE's critical service fails.
  SDK=userland/libs/bouchaud-sdk/build-program.sh
  for prog in services/serviced/serviced services/healthd/healthd services/updated/updated services/logd/logd tests/healthd-defaillant; do
    "$SDK" "$TRAVAIL/$(basename $prog)" "userland/$prog.c"
  done
  systeme="bin/serviced=$TRAVAIL/serviced bin/updated=$TRAVAIL/updated bin/logd=$TRAVAIL/logd"
  critique_b=$TRAVAIL/healthd; { [ "$SCENARIO" = sante-ko ] || [ "$SCENARIO" = ota-rollback ]; } && critique_b=$TRAVAIL/healthd-defaillant
  critique_a=$TRAVAIL/healthd; [ "$SCENARIO" = echecs-repetes ] && critique_a=$TRAVAIL/healthd-defaillant
  ramdisk "$OUT/slot-b/ramdisk" $systeme "bin/healthd=$critique_b"
  manifeste_v2() {  # SORTIE SEQUENCE MATERIEL [CLE]
    local r="$OUT/release-$(basename "$(dirname "$1")")-$(basename "$1")"
    python3 tools/release/fabrique_release.py --image "$OUT/system-b.img" --version "bench-$(git rev-parse --short HEAD)" \
      --sequence "$2" --materiel "$3" --canal testing --commit "$(git rev-parse HEAD)" --sortie "$r" \
      --cle-dev "${4:-$CLES/version.pem}" >/dev/null || { echo "AB_ROLLBACK_FAIL raison=fabrique_release" >&2; exit 1; }
    cp "$r/manifest" "$1"; cp "$r/manifest.sig" "$1.sig"
  }
  if [ "$OTA_COMPLET" = 1 ]; then
    # BOUCHAUD_OTA_V1 : la release 1001 telle que la chaine de release la
    # publie (fabrique_release.py, parties de 16 Mio), servie en HTTPS.
    PORT_OTA=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    printf 'source=https://10.0.2.2:%s/rel\ntelechargement=consenti\nredemarrage=consenti\n' "$PORT_OTA" > "$OUT/ota.conf"
    if [ "$SCENARIO" = ota-reseau ]; then
      PORT_CA=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
      printf 'source=https://10.0.2.2:%s/rel\nsource=https://10.0.2.2:%s/erreur\nsource=https://10.0.2.2:%s/rel\ntelechargement=consenti\nredemarrage=consenti\n' \
        "$PORT_CA" "$PORT_OTA" "$PORT_OTA" > "$OUT/ota.conf"
    fi
    ramdisk "$OUT/slot-b/ramdisk" $systeme "bin/healthd=$critique_b" "etc/bouchaud/ota.conf=$OUT/ota.conf"
    "$BUILDER" --image-slot "$OUT/slot-b" "$OUT/system-b.img"
    python3 tools/release/fabrique_release.py --image "$OUT/system-b.img" --version "ota-v2-$(git rev-parse --short HEAD)" \
      --sequence "$RELEASE_SEQ" --canal testing --commit "$(git rev-parse HEAD)" --sortie "$OUT/srv/rel" \
      --parties-mio 4 --cle-dev "$CLES/version.pem" || { echo "AB_ROLLBACK_FAIL raison=fabrique_release" >&2; exit 1; }
    NB_PARTIES=$(wc -l < "$OUT/srv/rel/system-slot.parts")
    # Temoin (revue #308) : dans ota-complet, la PREMIERE installation echoue
    # volontairement au finish ; l'agent doit la reproposer, updated doit
    # encore surveiller.
    temoin=(); if [ "$SCENARIO" = ota-complet ]; then : > "$OUT/temoin"; temoin=("update/temoin-echec-premier=$OUT/temoin"); fi
    ramdisk "$OUT/slot-a/ramdisk" $systeme "bin/healthd=$TRAVAIL/healthd" "etc/bouchaud/ota.conf=$OUT/ota.conf" "${temoin[@]}"
  elif [ "$SCENARIO" = ota-verifie ]; then
    # BOUCHAUD_OTA_V1 : ce que sert l'hote (http.server, 10.0.2.2 vu de QEMU).
    "$BUILDER" --image-slot "$OUT/slot-b" "$OUT/system-b.img"
    mkdir -p "$OUT/srv"/{ok,egal,vieux,arm,autre}
    manifeste_v2 "$OUT/srv/ok/manifest" "$RELEASE_SEQ" x86_64-uefi
    manifeste_v2 "$OUT/srv/egal/manifest" "$PLANCHER_INITIAL" x86_64-uefi
    manifeste_v2 "$OUT/srv/vieux/manifest" "$((PLANCHER_INITIAL - 1))" x86_64-uefi
    manifeste_v2 "$OUT/srv/arm/manifest" "$RELEASE_SEQ" arm64-uefi
    manifeste_v2 "$OUT/srv/autre/manifest" "$RELEASE_SEQ" x86_64-uefi "$CLES/autre.pem"
    PORT_OTA=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
    printf 'source=http://10.0.2.2:%s/%s\n' "$PORT_OTA" ok "$PORT_OTA" egal "$PORT_OTA" vieux \
      "$PORT_OTA" arm "$PORT_OTA" autre "$PORT_OTA" absent > "$OUT/ota.conf"
    ramdisk "$OUT/slot-a/ramdisk" $systeme "bin/healthd=$TRAVAIL/healthd" "etc/bouchaud/ota.conf=$OUT/ota.conf"
  elif [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; then
    # The release: slot B's image, built exactly as --disque-ab would lay it,
    # and its manifest. Shipped INSIDE the running system A; B on disk is
    # zeroed below, so only updated can make it bootable.
    "$BUILDER" --image-slot "$OUT/slot-b" "$OUT/system-b.img"
    octets=$(stat -c %s "$OUT/system-b.img")
    empreinte=$(sha256sum "$OUT/system-b.img" | cut -d' ' -f1)
    # The canonical manifest (system/update-core/src/manifeste.rs), signed.
    # BOUCHAUD_ANTI_RETOUR_V1: v2, release sequence RELEASE_SEQ, above the
    # anti-rollback floor seeded in META below (PLANCHER_INITIAL).
    # BOUCHAUD_RELEASE_V1: produced by the RELEASE tool itself
    # (tools/release/fabrique_release.py, bench key), so this bench also
    # proves that what the release pipeline emits is what the kernel installs.
    manifeste_v2 "$OUT/manifest" "$RELEASE_SEQ" x86_64-uefi
    # Two CORRECTLY SIGNED manifests the kernel must still refuse.
    manifeste_v2 "$OUT/manifest-ancien" "$((PLANCHER_INITIAL - 1))" x86_64-uefi
    manifeste_v2 "$OUT/manifest-materiel" "$RELEASE_SEQ" arm64-uefi
    : > "$OUT/selftest"
    morts=(); [ "$VARIANTE" = morts ] && morts=("update/selftest-mort=$OUT/selftest")
    ramdisk "$OUT/slot-a/ramdisk" $systeme "bin/healthd=$TRAVAIL/healthd" \
      "update/manifest=$OUT/manifest" "update/manifest.sig=$OUT/manifest.sig" \
      "update/selftest=$OUT/selftest" "update/system.img=$OUT/system-b.img" "${morts[@]}" \
      "update/manifest-ancien=$OUT/manifest-ancien" "update/manifest-ancien.sig=$OUT/manifest-ancien.sig" \
      "update/manifest-materiel=$OUT/manifest-materiel" "update/manifest-materiel.sig=$OUT/manifest-materiel.sig"
  else
    ramdisk "$OUT/slot-a/ramdisk" $systeme "bin/healthd=$critique_a"
  fi
  ;;
esac
rustc +"$TOOLCHAIN" --edition 2021 -O tools/update/meta.rs -o "$TRAVAIL/meta"
"$TRAVAIL/meta" "$META_SPEC" "$OUT/meta.img"
if [ "$RECOVERY" = 1 ]; then
  # RECOVERY: the good kernel, nothing else. It needs no userland.
  "$BUILDER" --charge-installation "$OUT/kernel-bon.elf" "$OUT/recovery"
  "$BUILDER" --disque-ab "$OUT/disque-ab.img" "$SHIM" "$OUT/meta.img" "$OUT/slot-a" "$OUT/slot-b" "$OUT/recovery" | tee "$OUT/disque.txt"
else
  "$BUILDER" --disque-ab "$OUT/disque-ab.img" "$SHIM" "$OUT/meta.img" "$OUT/slot-a" "$OUT/slot-b" | tee "$OUT/disque.txt"
fi
partition() { awk -v n="nom=$1" '$2 == n { for (i = 1; i <= NF; i++) { split($i, kv, "="); v[kv[1]] = kv[2] } print v["premier_lba"], v["octets"] }' "$OUT/disque.txt"; }
if [ "$SCENARIO" = recovery-rollback ]; then
  # META records the REAL digests of both slots, so `verifie` is meaningful.
  empreinte() { read -r l o < <(partition "$1"); dd if="$OUT/disque-ab.img" bs=512 skip="$l" count=$(( o / 512 )) status=none | sha256sum | cut -d' ' -f1; }
  "$TRAVAIL/meta" "$META_SPEC" "$OUT/meta.img" "$(empreinte BOUCHAUD-SYS-A)" "$(empreinte BOUCHAUD-SYS-B)"
  read -r l o < <(partition BOUCHAUD-META)
  dd if="$OUT/meta.img" of="$OUT/disque-ab.img" bs=512 seek="$l" conv=notrunc status=none
fi
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = ota-verifie ] || [ "$OTA_COMPLET" = 1 ]; then
  # BOUCHAUD_ANTI_RETOUR_V1: the installed system A is release
  # PLANCHER_INITIAL and the floor is already there (META blocks 3 and 4),
  # written by an encoder INDEPENDENT of update-core (struct + zlib.crc32).
  read -r l o < <(partition BOUCHAUD-META)
  python3 tools/update/pose_anti_retour.py "$OUT/disque-ab.img" "$l" "$PLANCHER_INITIAL"
fi
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ] || [ "$OTA_COMPLET" = 1 ]; then
  # Slot B starts EMPTY (zeroes) and META says so (a-bon): nothing bootable
  # there until updated writes it.
  read -r lba oct < <(partition BOUCHAUD-SYS-B)
  dd if=/dev/zero of="$OUT/disque-ab.img" bs=512 seek="$lba" count=$(( (oct + 511) / 512 )) conv=notrunc status=none
fi
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$OUT/vars.fd"

# --- talking to the machine (recovery scenarios) ----------------------------
envoie() { python3 -c 'import socket, sys
s = socket.socket(socket.AF_UNIX)
try:
    s.connect(sys.argv[1]); s.sendall(sys.argv[2].encode())
except OSError:
    pass' "$1" "$2" 2>/dev/null || true; }
# attend LOG MOTIF [S]: the pattern appears in the log within S seconds.
attend() { local f=$((SECONDS + ${3:-60})); while (( SECONDS < f )); do
  sed 's/\x1b\[[0-9;]*m//g' "$1" | tr -d '\r' | grep -aE "$2" >/dev/null && return 0; sleep 0.5; done; return 1; }
commande() { envoie "$1" "$3"$'\r'; attend "$2" "$4" "${5:-60}"; }
# One recovery session; prints R (every expectation met) or R!(what failed).
dialogue() {
  local sock=$1 log=$2 ko=""
  envoie "$sock" $'\r'   # flush anything typed before the console (stray keys)
  sleep 1
  case "$SCENARIO" in
    recovery-requise)
      attend "$log" "BOUCHAUD_RECOVERY_PRET raison=recovery_requise" 5 || ko+=raison,
      commande "$sock" "$log" etat "meta    : ILLISIBLE" || ko+=etat,
      commande "$sock" "$log" "rollback --confirme" "BOUCHAUD_RECOVERY_ROLLBACK_REFUS" || ko+=rollback_refus, ;;
    recovery-rollback)
      attend "$log" "BOUCHAUD_RECOVERY_PRET raison=recovery_demandee" 5 || ko+=raison,
      commande "$sock" "$log" etat "slot B  : bon" || ko+=etat,
      commande "$sock" "$log" "verifie A" "BOUCHAUD_RECOVERY_VERIFIE slot=A resultat=conforme" 240 || ko+=verifie_a,
      commande "$sock" "$log" "verifie B" "BOUCHAUD_RECOVERY_VERIFIE slot=B resultat=conforme" 240 || ko+=verifie_b,
      commande "$sock" "$log" rollback "Retaper avec --confirme" || ko+=lecture_seule,
      commande "$sock" "$log" "rollback --confirme" "BOUCHAUD_RECOVERY_ROLLBACK_OK actif=B" || ko+=rollback, ;;
    echecs-repetes)
      attend "$log" "BOUCHAUD_RECOVERY_PRET raison=recovery_echecs_repetes" 5 || ko+=raison,
      commande "$sock" "$log" etat "echecs  : 3 boot" || ko+=compteur, ;;
  esac
  # Recovery depends on nothing above the kernel.
  if sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -aE "SERVICED_SYSTEM_BEGIN|BOUCHAUD_SUPERVISEUR_LANCE" >/dev/null; then ko+=superviseur_lance,; fi
  if [ -z "$ko" ]; then echo R; else echo "R!($ko)"; fi
}

# --- boot loop --------------------------------------------------------------
# BO_AB_SUPPORT : le disque A/B est un NVMe (defaut) ou une CLE USB (xHCI +
# usb-storage, BOUCHAUD_USB_VIDANGE_REELLE_V1). Sur cle, le commit de sante
# n'est accepte que si le support a prouve SYNCHRONIZE CACHE au montage.
case "${BO_AB_SUPPORT:-nvme}" in
  nvme) SUPPORT=(-device nvme,drive=disque,serial=bouchaud-ab) ;;
  usb)  SUPPORT=(-device qemu-xhci,id=xhci -device usb-storage,bus=xhci.0,drive=disque,removable=on,bootindex=0) ;;
  # La cle A/B sur le port 1, une seconde cle (GPT vierge) sur le port 2.
  usb-deux)
    python3 tools/ci/fabrique-disque-gpt.py "$OUT/seconde-cle.img" --mio 32 >/dev/null || exit 1
    SUPPORT=(-device qemu-xhci,id=xhci
      -device usb-storage,bus=xhci.0,port=1,drive=disque,removable=on,bootindex=0
      -drive if=none,id=seconde,format=raw,file="$OUT/seconde-cle.img"
      -device usb-storage,bus=xhci.0,port=2,drive=seconde,removable=on) ;;
  *) echo "BO_AB_SUPPORT inconnu : ${BO_AB_SUPPORT}" >&2; exit 2 ;;
esac
# BOUCHAUD_OTA_V1 : une carte que le pilote connait (e1000) et le reseau
# utilisateur de QEMU ; l'hote sert les sources sur 127.0.0.1.
RESEAU=()
if [ "$OTA_COMPLET" = 1 ]; then
  # L'horloge de l'invite a +1 h : le certificat de banc vient d'etre emis
  # (notBefore = maintenant) et le noyau VERIFIE sa periode de validite.
  RESEAU=(-nic user,model=e1000 -rtc "base=$(date -u -d '+1 hour' +%Y-%m-%dT%H:%M:%S)")
  # Le serveur HTTPS de banc (tools/ci/serveur_ota_banc.py) : coupure
  # volontaire de la partie 5 ; en ota-reseau, erreur 500, partie alteree,
  # partie tronquee, et un SECOND serveur au certificat d'une CA inconnue.
  modes=(); [ "$SCENARIO" = ota-reseau ] && modes=(--reseau)
  python3 tools/ci/serveur_ota_banc.py "$PORT_OTA" "$OUT/srv" "$CLES/srv.pem" "$CLES/srv.key" "${modes[@]}" > "$OUT/https.log" 2>&1 &
  PID_HTTP=$!
  if [ "$SCENARIO" = ota-reseau ]; then
    python3 tools/ci/serveur_ota_banc.py "$PORT_CA" "$OUT/srv" "$CLES/srv-inconnu.pem" "$CLES/srv.key" > "$OUT/https-ca-inconnue.log" 2>&1 &
    PID_HTTP="$PID_HTTP $!"
    # Le lien reseau est coupe puis retabli par le moniteur (set_link) :
    # une carte nommee.
    RESEAU=(-netdev user,id=n0 -device e1000,netdev=n0,id=e0 -rtc "base=$(date -u -d '+1 hour' +%Y-%m-%dT%H:%M:%S)")
  fi
  # Ajoute l'arret du serveur a la trap du banc, SANS la remplacer : le
  # diagnostic d'echec et le nettoyage doivent toujours avoir lieu.
  trap 'rc=$?; kill $PID_HTTP 2>/dev/null; [ "$rc" -ne 0 ] && diagnostic_echec; rm -rf "$TRAVAIL" "$CLES"' EXIT
elif [ "$SCENARIO" = ota-verifie ]; then
  RESEAU=(-nic user,model=e1000)
  python3 -m http.server "$PORT_OTA" --bind 127.0.0.1 --directory "$OUT/srv" > "$OUT/http.log" 2>&1 &
  PID_HTTP=$!
  # Ajoute l'arret du serveur a la trap du banc, SANS la remplacer : le
  # diagnostic d'echec et le nettoyage doivent toujours avoir lieu.
  trap 'rc=$?; kill "$PID_HTTP" 2>/dev/null; [ "$rc" -ne 0 ] && diagnostic_echec; rm -rf "$TRAVAIL" "$CLES"' EXIT
fi
# BOUCHAUD_AB_ETAT_APRES_COMMIT_ATTENDU_V1 : après `result=committed`,
# serviced relit l'état par la capacité et écrit `SERVICED_UPDATE_STATE`.
# Cette seconde ligne est la preuve « état visible après commit » que
# l'assertion `etat_apres_commit` exige ; on l'attend (borné) avant de couper.
attend_etat_apres_commit() {
  local borne=$((SECONDS + ${BO_AB_ETAT_APRES_COMMIT_S:-20}))
  while (( SECONDS < borne )); do
    if sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r' \
         | awk '/result=committed/ { c = 1; next } c && /SERVICED_UPDATE_STATE/ { t = 1 } END { exit !t }'; then
      return 0
    fi
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 0.2
  done
  return 1
}
sequence=()
nb=$(wc -w <<< "$ATTENDU")
for i in $(seq 1 "$nb"); do
  log="$OUT/boot-$i.log"
  : > "$log"
  sock="$OUT/serie-$i.sock"
  if [ "$RECOVERY" = 1 ]; then
    # A socket the bench can WRITE to (keys, commands); output still logged.
    SERIE=(-chardev "socket,id=s0,path=$sock,server=on,wait=off,logfile=$log" -serial chardev:s0)
  else
    SERIE=(-serial file:"$log")
  fi
  # ota-reseau : un moniteur QEMU par demarrage, pour couper le LIEN (set_link).
  MONITEUR=(); moniteur="$OUT/moniteur-$i.sock"
  [ "$SCENARIO" = ota-reseau ] && MONITEUR=(-monitor "unix:$moniteur,server=on,wait=off")
  lien=actif; t_coupe=0
  qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
    -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
    -drive if=pflash,format=raw,unit=1,file="$OUT/vars.fd" \
    -drive if=none,id=disque,format=raw,file="$OUT/disque-ab.img" \
    "${SUPPORT[@]}" "${RESEAU[@]}" "${MONITEUR[@]}" \
    -display none -no-reboot "${SERIE[@]}" >/dev/null 2>&1 &
  pid=$!
  verdict=""
  fin=$((SECONDS + BOOT_DELAI_S))
  while (( SECONDS < fin )); do
    propre=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r')
    if [ "$SCENARIO" = recovery-rollback ] && [ "$i" = 1 ] && ! grep -aq "BOUCHAUD_AB_" <<< "$propre"; then
      envoie "$sock" r   # held 'r' at the boot prompt (OVMF reads COM1 too)
    fi
    if grep -aq "BOUCHAUD_RECOVERY_PRET" <<< "$propre"; then
      verdict=$(dialogue "$sock" "$log"); break
    fi
    if grep -aq "BOUCHAUD_AB_RECOVERY_REQUIRED" <<< "$propre"; then
      # Give a kernel the chance to (wrongly) start, then judge.
      sleep 15
      propre=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r')
      if grep -aqE "BOUCHAUD_HWPROBE_BEGIN|BOUCHAUD_AB_CANDIDAT" <<< "$propre"; then verdict=RECOVERY_MAIS_NOYAU; else verdict=RECOVERY; fi
      break
    fi
    slot=$(grep -aoE "BOUCHAUD_AB_BOOT slot=[AB]" <<< "$propre" | tail -1 | sed 's/.*slot=//' || true)
    if [ "$SCENARIO" = mise-a-jour ] && [ "$slot" = A ] && grep -aqE "UPDATED_REFUSALS_(OK|FAIL)|UPDATED_INSTALL_FAIL|UPDATED_SKIP" <<< "$propre"; then
      if grep -aq "UPDATED_INSTALL_OK" <<< "$propre" && grep -aq "UPDATED_REFUSALS_OK" <<< "$propre"; then verdict="A*"; else verdict="A!"; fi
      break
    fi
    # sans-cle: A0 = the release kernel refused the signed manifest because it
    # trusts no key, and nothing was installed.
    if [ "$SCENARIO" = sans-cle ] && [ "$slot" = A ] && [ "$i" = 1 ] && grep -aqE "UPDATED_REFUSALS_(OK|FAIL)" <<< "$propre"; then
      if grep -aq "BOUCHAUD_UPDATE_INSTALL_REFUS etape=begin raison=aucune_cle_de_confiance" <<< "$propre" \
         && ! grep -aqE "BOUCHAUD_UPDATE_INSTALL_CANDIDAT|BOUCHAUD_UPDATE_MANIFESTE signature=fiable|UPDATED_INSTALL_OK" <<< "$propre"; then
        verdict="A0"
      else
        verdict="A!"
      fi
      break
    fi
    # BOUCHAUD_OTA_RESEAU_V1, demarrage 1 : refus, relance, troncature, puis,
    # dans le SECOND telechargement (apres BOUCHAUD_OTA_RELANCE) : lien coupe
    # 20 s apres la partie 3, ALIMENTATION coupee apres la partie 5 (A~).
    if [ "$SCENARIO" = ota-reseau ] && [ "$i" = 1 ]; then
      apres=$(awk '/BOUCHAUD_OTA_RELANCE/ { f = 1 } f' <<< "$propre")
      if [ "$lien" = actif ] && grep -aq "BOUCHAUD_OTA_PARTIE n=3 " <<< "$apres"; then
        envoie "$moniteur" $'set_link e0 off\n'; lien=coupe; t_coupe=$SECONDS
        echo "AB_RESEAU lien=coupe apres_partie=3"
      fi
      if [ "$lien" = coupe ] && (( SECONDS - t_coupe >= 20 )); then
        envoie "$moniteur" $'set_link e0 on\n'; lien=retabli
        echo "AB_RESEAU lien=retabli duree_s=$(( SECONDS - t_coupe ))"
      fi
      if grep -aqE "UPDATED_INSTALL|BOUCHAUD_UPDATE_INSTALL_CANDIDAT|BOUCHAUD_OTA_TELECHARGE " <<< "$propre"; then
        verdict="A!"; break
      fi
      if [ "$lien" = retabli ] && grep -aq "BOUCHAUD_OTA_PARTIE n=5 " <<< "$apres"; then
        echo "AB_RESEAU alimentation=coupee pendant=telechargement apres_partie=5"
        verdict="A~"; break
      fi
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    # ota-complet / ota-rollback, boot 1 (A) -- ota-reseau, boot 2: the agent
    # downloads, updated installs, the agent reboots into the candidate (QEMU
    # -no-reboot exits).
    if [ "$OTA_COMPLET" = 1 ] && [ "$slot" = A ] && [ "$i" = "$BOOT_INSTALLE" ]; then
      if grep -aq "BOUCHAUD_OTA_REDEMARRAGE consentement=configuration" <<< "$propre"; then
        for _ in $(seq 1 60); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
        if grep -aq "UPDATED_INSTALL_OK" <<< "$propre"; then verdict="A*"; else verdict="A!"; fi
        break
      fi
      # (l'echec VOLONTAIRE du temoin n'est pas un echec du demarrage)
      echecs_reels=$(grep -a "UPDATED_INSTALL_FAIL" <<< "$propre" | grep -cv "temoin=echec_voulu" || true)
      if grep -aqE "BOUCHAUD_OTA_TELECHARGEMENT_REFUSE|BOUCHAUD_OTA_INSTALLATION_ABSENTE" <<< "$propre" || [ "${echecs_reels:-0}" -gt 0 ]; then
        verdict="A!"; break
      fi
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    # ota-* : a steady boot is judged only once the agent's cycle is logged.
    if [ "$OTA_COMPLET" = 1 ] && grep -aq "result=already_good" <<< "$propre" && ! grep -aq "BOUCHAUD_OTA_CYCLE" <<< "$propre"; then
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    # ota-verifie: A healthy AND the agent's first cycle logged.
    if [ "$SCENARIO" = ota-verifie ] && [ "$slot" = A ]; then
      if grep -aq "result=already_good" <<< "$propre" && grep -aq "BOUCHAUD_OTA_CYCLE" <<< "$propre"; then
        verdict="A="; break
      fi
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    # Boot 1 of mise-a-jour is judged on updated alone: its own health line
    # can come before the install ends, and must not cut the install short.
    if { [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; } && [ "$slot" = A ] && [ "$i" = 1 ]; then
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    case "$SCENARIO" in sante|sante-ko|sans-vidange|seconde-cle|mise-a-jour|sans-cle|ota-complet|ota-rollback|ota-reseau|recovery-rollback|echecs-repetes)
      if [ -n "$slot" ] && grep -aq "result=committed" <<< "$propre"; then
        # Couper à la première ligne détruisait la seconde (run 41898808040,
        # #248 : `B+ B=` puis etat_apres_commit en échec). Si elle ne vient
        # pas dans la borne, l'assertion échoue comme avant : rien n'est relâché.
        attend_etat_apres_commit || true
        verdict="$slot+"; break
      fi
      if [ -n "$slot" ] && grep -aq "result=already_good" <<< "$propre"; then verdict="$slot="; break; fi
      if [ -n "$slot" ] && grep -aqE "result=refused|\[SERVICED\] system [A-Za-z]+->RecoveryRequired" <<< "$propre"; then
        verdict="$slot-"; break
      fi
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue ;;
    esac
    if [ "$slot" = B ] && grep -aqE "BOUCHAUD_AB_CANDIDAT_(PANIQUE|GEL)_VOLONTAIRE" <<< "$propre"; then
      verdict=B; break
    fi
    if [ "$slot" = A ] && grep -aq "BOUCHAUD_HWPROBE_BEGIN" <<< "$propre"; then verdict=A; break; fi
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 1
  done
  # POWER CUT: SIGKILL, no shutdown path.
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  [ -n "$verdict" ] || verdict="AUCUN(slot=${slot:-?})"
  raison=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | tr -d '\r' | grep -aoE "BOUCHAUD_AB_(BOOT slot=[AB] raison=[a-z_]+|RECOVERY_REQUIRED raison=[a-z_]+)" | tail -1 || true)
  echo "AB_BOOT n=$i verdict=$verdict ${raison}"
  sequence+=("$verdict")
done

obtenu="${sequence[*]}"
echo "AB_SEQUENCE scenario=$SCENARIO attendu=\"$ATTENDU\" obtenu=\"$obtenu\""
# The commit must be durable AND visible: right after it, serviced re-reads
# the state through the capability; B must be the active, Good slot.
if [ "$SCENARIO" = sante ] && ! sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | tr -d '\r' \
     | grep -aE "SERVICED_UPDATE_STATE.*active_slot=1.*state_b=2" >/dev/null; then
  echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=etat_apres_commit" >&2
  exit 1
fi
if [ "$SCENARIO" = mise-a-jour ]; then
  for m in "BOUCHAUD_UPDATE_MANIFESTE signature=fiable cle=dev-banc developpement=1" \
           "BOUCHAUD_UPDATE_INSTALL_CANDIDAT cible=B" "signature=fiable hash=valide" \
           "UPDATED_INSTALL_OK" "signature=trusted hash=valid" "UPDATED_REFUSALS_OK" \
           "BOUCHAUD_UPDATE_ANTI_RETOUR evenement=begin etat=valide plancher=$PLANCHER_INITIAL" \
           "BOUCHAUD_UPDATE_INSTALL_REFUS etape=begin raison=version_retrograde" \
           "BOUCHAUD_UPDATE_INSTALL_REFUS etape=begin raison=materiel_incompatible" \
           "begin with an OLDER signed release is refused (anti-rollback)" \
           "begin with a signed release for OTHER hardware is refused"; do
    sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aF "$m" >/dev/null \
      || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO marqueur_absent=$m" >&2; exit 1; }
  done
  # BOUCHAUD_ANTI_RETOUR_V1: the health commit of B (boot 2) raises the
  # floor to B's release; nothing lowers it afterwards.
  sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-2.log" \
    | grep -aF "BOUCHAUD_UPDATE_ANTI_RETOUR evenement=plancher_avance etat=valide plancher=$RELEASE_SEQ" >/dev/null \
    || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=plancher_non_avance_au_commit" >&2; exit 1; }
  if sed 's/\x1b\[[0-9;]*m//g' "$OUT"/boot-*.log | grep -aE 'BOUCHAUD_UPDATE_ANTI_RETOUR .*plancher=([0-9]+)' -o \
       | awk -F'plancher=' -v p="$PLANCHER_INITIAL" '$2 + 0 < p { bad = 1 } END { exit !bad }'; then
    echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=plancher_descendu" >&2; exit 1
  fi
  # The agent must not be killed by its own supervisor mid-install.
  if sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aE 'svc=updated Running->Degraded' >/dev/null; then
    echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=updated_battements_manques" >&2; exit 1
  fi
  sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aE '\[UPDATED\] (ok|FAIL)' | awk '!s[$0]++' | sed 's/^/  /'
  # BOUCHAUD_UPDATED_MESURES_V1: published, not thresholded.
  sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aoE 'UPDATED_MESURE .*' | sed 's/^/  /'
  if [ "$VARIANTE" = morts ]; then
    propre1=$(sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | tr -d '\r')
    for m in "UPDATED_MORT_VOLONTAIRE pendant=write" "UPDATED_MORT_VOLONTAIRE pendant=verify" \
             "BOUCHAUD_UPDATE_INSTALL_REFUS etape=write raison=session_d_un_autre_processus" \
             "UPDATED_SUCCESSEUR_APRES_DEUX_MORTS"; do
      grep -aF "$m" <<< "$propre1" >/dev/null || { echo "AB_ROLLBACK_FAIL variante=morts marqueur_absent=$m" >&2; exit 1; }
    done
    for m in "successor: the dead owner's session is not usable" "successor: no premature Candidate in META"; do
      n=$(grep -aF "[UPDATED] ok   $m" <<< "$propre1" | awk '!s[$0]++' | wc -l)
      [ "$n" -ge 1 ] && ! grep -aF "[UPDATED] FAIL $m" <<< "$propre1" >/dev/null \
        || { echo "AB_ROLLBACK_FAIL variante=morts controle=$m" >&2; exit 1; }
    done
    echo "AB_VARIANTE_MORTS_OK deux morts volontaires (write, verify), successeur sans heritage"
  fi
fi
if [ "$OTA_COMPLET" = 1 ]; then
  P() { sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-$1.log" | tr -d '\r'; }
  echec() { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO $*" >&2; exit 1; }
  b1=$(P 1); bi=$(P "$BOOT_INSTALLE"); n=$BOOT_INSTALLE
  # Boot n (A, release 1000) : HTTPS, parties, empreinte SIGNEE, installation, redemarrage consenti.
  # Deux versions DISTINCTES : le noyau de A n'est pas celui de la release.
  grep -aq "BOUCHAUD_BUILD commit=" <<< "$bi" || echec "boot=$n raison=identite_de_construction_absente"
  grep -aqF "BOUCHAUD_BUILD commit=$V2" <<< "$bi" && echec "boot=$n raison=A_porte_deja_la_version_2"
  for m in "BOUCHAUD_AB_BOOT slot=A raison=actif" \
           "BOUCHAUD_OTA_VERIFIE source=https://10.0.2.2:$PORT_OTA/rel resultat=disponible" \
           "BOUCHAUD_OTA_TELECHARGE source=https://10.0.2.2:$PORT_OTA/rel sequence=$RELEASE_SEQ" "empreinte=signee_verifiee" \
           "UPDATED_OTA_RELEASE_STAGED" "UPDATED_INSTALL_OK" "BOUCHAUD_UPDATE_INSTALL_CANDIDAT cible=B" \
           "BOUCHAUD_OTA_CANDIDAT_PRET sequence=$RELEASE_SEQ" "BOUCHAUD_OTA_REDEMARRAGE consentement=configuration" \
           "BOUCHAUD_PERSIST_DEMARRAGES n=$n slot=A"; do
    grep -aF "$m" <<< "$bi" >/dev/null || echec "boot=$n marqueur_absent=$m"
  done
  [ "$(grep -ac 'BOUCHAUD_OTA_PARTIE n=' <<< "$bi")" = "$NB_PARTIES" ] || echec "boot=$n raison=nombre_de_parties"
  # La coupure volontaire a eu lieu, et la partie 5 a ete REPRISE.
  grep -q "COUPURE_VOLONTAIRE /rel/system-slot.img.part005" "$OUT/https.log" || echec "raison=temoin_de_coupure_absent"
  grep -aE "BOUCHAUD_OTA_REPRISE url=https://10.0.2.2:$PORT_OTA/rel/system-slot.img.part005 essai=[2-4] resultat=ok" <<< "$b1" >/dev/null \
    || echec "boot=1 raison=partie_5_non_reprise"
  grep -q "HTTPS TLSv1.3 .*GET /rel/system-slot.img.part000" "$OUT/https.log" || echec "raison=telechargement_non_https"
  if [ "$SCENARIO" = ota-complet ]; then
    # Revue #308 : premier essai refuse par le noyau, puis REPROPOSE et reussi.
    for m in "UPDATED_OTA_ECHEC_VOLONTAIRE etape=finish" "BOUCHAUD_UPDATE_INSTALL_REFUS etape=finish raison=differe_du_manifeste_signe" \
             "BOUCHAUD_OTA_INSTALLATION_ECHOUEE essai=1 consequence=nouvel_essai" \
             "BOUCHAUD_OTA_CANDIDAT_PRET sequence=$RELEASE_SEQ essai=2"; do
      grep -aF "$m" <<< "$b1" >/dev/null || echec "boot=1 marqueur_absent=$m"
    done
    for g in 1 2; do
      grep -aF "UPDATED_OTA_RELEASE_STAGED pret generation=$g" <<< "$b1" >/dev/null \
        || echec "boot=1 raison=generation_${g}_non_traitee_par_updated"
    done
    b2=$(P 2); b3=$(P 3)
    # Boot 2 (B, release 1001, AUTRE noyau) : commit de sante, plancher 1001, STATE intacte.
    for m in "BOUCHAUD_AB_BOOT slot=B" "BOUCHAUD_BUILD commit=$V2" "result=committed" \
             "BOUCHAUD_UPDATE_ANTI_RETOUR evenement=plancher_avance etat=valide plancher=$RELEASE_SEQ" \
             "BOUCHAUD_PERSIST_MONTAGE etat=restaure" "BOUCHAUD_PERSIST_DEMARRAGES n=2 slot=B"; do
      grep -aF "$m" <<< "$b2" >/dev/null || echec "boot=2 marqueur_absent=$m"
    done
    # Boot 3 (B) : l'agent se voit a jour ; rien n'est retelecharge.
    for m in "BOUCHAUD_BUILD commit=$V2" "result=already_good" "BOUCHAUD_PERSIST_DEMARRAGES n=3 slot=B" \
             "BOUCHAUD_OTA_VERIFIE source=https://10.0.2.2:$PORT_OTA/rel resultat=a_jour sequence=$RELEASE_SEQ"; do
      grep -aF "$m" <<< "$b3" >/dev/null || echec "boot=3 marqueur_absent=$m"
    done
    grep -aq "BOUCHAUD_OTA_TELECHARGE" <<< "$b3" && echec "boot=3 raison=retelechargement"
    echo "AB_OTA_COMPLET_OK https=TLSv1.3 parties=$NB_PARTIES v1=1000 v2=$RELEASE_SEQ build_v2=$V2 sante=committe plancher=$RELEASE_SEQ state=1,2,3"
  elif [ "$SCENARIO" = ota-reseau ]; then
    # BOUCHAUD_OTA_RESEAU_V1 -- demarrage 1 : chaque defaut du reseau a SA
    # consequence, et rien n'est installe ni ecrit dans un slot.
    u="https://10.0.2.2:$PORT_OTA"
    for m in "BOUCHAUD_OTA_VERIFIE source=https://10.0.2.2:$PORT_CA/rel resultat=refuse raison=manifeste_injoignable" \
             "BOUCHAUD_OTA_VERIFIE source=$u/erreur resultat=refuse raison=manifeste_injoignable" \
             "BOUCHAUD_OTA_VERIFIE source=$u/rel resultat=disponible" \
             "BOUCHAUD_OTA_TELECHARGEMENT_REFUSE source=$u/rel raison=partie_empreinte_differente" \
             "BOUCHAUD_OTA_HISTORIQUE telechargement_refuse sequence=$RELEASE_SEQ raison=partie_empreinte_differente" \
             "BOUCHAUD_OTA_RELANCE dans_s=60 echecs_consecutifs=1 raison=partie_empreinte_differente"; do
      grep -aF "$m" <<< "$b1" >/dev/null || echec "boot=1 marqueur_absent=$m"
    done
    # La partie alteree n'est PAS reprise dans le meme telechargement (le
    # contenu, pas le transport) ; la partie tronquee l'est.
    grep -aE "BOUCHAUD_OTA_REPRISE url=$u/rel/system-slot.img.part001 " <<< "$b1" >/dev/null \
      && echec "boot=1 raison=partie_alteree_reprise_au_lieu_d_etre_refusee"
    tronq=$(grep -aoE "BOUCHAUD_OTA_REPRISE url=$u/rel/system-slot.img.part002 essai=1 raison=[a-z_]+" <<< "$b1" | sed 's/.*raison=//' | head -1 || true)
    [ -n "$tronq" ] || echec "boot=1 raison=troncature_non_vue"
    grep -aE "BOUCHAUD_OTA_REPRISE url=$u/rel/system-slot.img.part002 essai=[2-4] resultat=ok" <<< "$b1" >/dev/null \
      || echec "boot=1 raison=partie_tronquee_non_reprise"
    # Lien coupe 20 s apres la partie 3 du second telechargement : une
    # partie suivante echoue PUIS est reprise.
    apres1=$(awk '/BOUCHAUD_OTA_RELANCE/ { f = 1 } f' <<< "$b1")
    apres_coupe=$(awk '/BOUCHAUD_OTA_PARTIE n=3 / { f = 1 } f' <<< "$apres1")
    grep -aE "BOUCHAUD_OTA_REPRISE url=$u/rel/system-slot.img.part004 essai=[2-4] resultat=ok" <<< "$apres_coupe" >/dev/null \
      || echec "boot=1 raison=pas_de_reprise_apres_la_perte_du_lien"
    if grep -aqE "UPDATED_INSTALL|BOUCHAUD_UPDATE_INSTALL_(BEGIN|CANDIDAT)|BOUCHAUD_OTA_TELECHARGE " <<< "$b1"; then
      echec "boot=1 raison=installation_pendant_un_telechargement_incomplet"
    fi
    # Le serveur l'atteste (le transport n'est pas simule).
    for m in "TLS_REFUS" ; do grep -q "$m" "$OUT/https-ca-inconnue.log" || echec "raison=ca_inconnue_sans_refus_tls"; done
    grep -q '"GET ' "$OUT/https-ca-inconnue.log" && echec "raison=ca_inconnue_a_servi_une_requete"
    for m in "ERREUR_VOLONTAIRE /erreur/manifest 500" "ALTERATION_VOLONTAIRE /rel/system-slot.img.part001" \
             "TRONCATURE_VOLONTAIRE /rel/system-slot.img.part002"; do
      grep -qF "$m" "$OUT/https.log" || echec "raison=temoin_serveur_absent=$m"
    done
    # Demarrage 2 : A, META intact (le slot actif, pas un candidat) ; la
    # coupure est VUE dans STATE, tout recommence depuis la partie 0.
    b2=$(P 2); b3=$(P 3)
    for m in "BOUCHAUD_OTA_TELECHARGEMENT_INTERROMPU sequence=$RELEASE_SEQ" "consequence=reprise_depuis_la_premiere_partie" \
             "BOUCHAUD_OTA_HISTORIQUE telechargement_interrompu sequence=$RELEASE_SEQ" "BOUCHAUD_PERSIST_MONTAGE etat=restaure" \
             "BOUCHAUD_OTA_PARTIE n=0 "; do
      grep -aF "$m" <<< "$b2" >/dev/null || echec "boot=2 marqueur_absent=$m"
    done
    # Demarrage 3 : B, l'AUTRE noyau, commit de sante, plancher 1001.
    for m in "BOUCHAUD_AB_BOOT slot=B" "BOUCHAUD_BUILD commit=$V2" "result=committed" \
             "BOUCHAUD_UPDATE_ANTI_RETOUR evenement=plancher_avance etat=valide plancher=$RELEASE_SEQ" \
             "BOUCHAUD_PERSIST_MONTAGE etat=restaure" "BOUCHAUD_PERSIST_DEMARRAGES n=3 slot=B"; do
      grep -aF "$m" <<< "$b3" >/dev/null || echec "boot=3 marqueur_absent=$m"
    done
    echo "AB_OTA_RESEAU_OK ca_inconnue=refusee http500=refusee partie_alteree=refusee+relance_60s troncature=reprise($tronq) lien_coupe=reprise alimentation_coupee=telechargement_repris state=1,2,3 v2=$RELEASE_SEQ build_v2=$V2"
  else
    # ota-rollback : B (release 1001) echoue sa sante, retour a A ; STATE
    # traverse tout ; la release en echec n'est PLUS jamais telechargee.
    dernier=$(P 5)
    grep -aqF "BOUCHAUD_BUILD commit=$V2" <<< "$dernier" && echec "boot=5 raison=toujours_sur_la_version_2"
    for m in "BOUCHAUD_AB_BOOT slot=A" \
             "BOUCHAUD_OTA_VERIFIE source=https://10.0.2.2:$PORT_OTA/rel resultat=refuse raison=release_deja_en_echec_ici"; do
      grep -aF "$m" <<< "$dernier" >/dev/null || echec "boot=5 marqueur_absent=$m"
    done
    grep -aq "BOUCHAUD_OTA_TELECHARGE" <<< "$dernier" && echec "boot=5 raison=release_en_echec_retelechargee"
    grep -aq "plancher_avance" "$OUT"/boot-*.log && echec "raison=plancher_avance_par_une_release_en_echec"
    compteurs=$(for k in 1 2 3 4 5; do P $k | grep -aoE "BOUCHAUD_PERSIST_DEMARRAGES n=[0-9]+" | sed 's/.*n=//'; done | tr '\n' ' ')
    python3 -c 'import sys; c=[int(x) for x in sys.argv[1:]]; sys.exit(0 if len(c) >= 2 and c[0] == 1 and all(b > a for a, b in zip(c, c[1:])) else 1)' $compteurs \
      || echec "raison=state_non_preservee compteurs=$compteurs"
    echo "AB_OTA_ROLLBACK_OK https=TLSv1.3 sequence=\"A* B- B- B- A=\" state=$compteurs release_en_echec=jamais_reproposee"
  fi
fi
if [ "$SCENARIO" = ota-verifie ]; then
  propre1=$(sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | tr -d '\r')
  b="http://10.0.2.2:$PORT_OTA"
  for m in "BOUCHAUD_OTA_VERIFIE source=$b/ok resultat=disponible version=bench-$(git rev-parse --short HEAD) sequence=$RELEASE_SEQ" \
           "BOUCHAUD_OTA_VERIFIE source=$b/egal resultat=a_jour sequence=$PLANCHER_INITIAL" \
           "BOUCHAUD_OTA_VERIFIE source=$b/vieux resultat=refuse raison=version_retrograde" \
           "BOUCHAUD_OTA_VERIFIE source=$b/arm resultat=refuse raison=materiel_incompatible" \
           "BOUCHAUD_OTA_VERIFIE source=$b/autre resultat=refuse raison=signature_non_fiable" \
           "BOUCHAUD_OTA_VERIFIE source=$b/absent resultat=refuse raison=manifeste_injoignable" \
           "BOUCHAUD_OTA_CYCLE sources=6"; do
    grep -aF "$m" <<< "$propre1" >/dev/null || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO marqueur_absent=$m" >&2; exit 1; }
  done
  # Exactement une ligne par source, et la verification n'installe RIEN.
  [ "$(grep -ac "BOUCHAUD_OTA_VERIFIE " <<< "$propre1")" = 6 ] \
    || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=nombre_de_verdicts" >&2; exit 1; }
  if grep -aqE "BOUCHAUD_UPDATE_INSTALL_(BEGIN|CANDIDAT)" <<< "$propre1"; then
    echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=installation_sans_consentement" >&2; exit 1
  fi
  # L'hote a bien servi les fichiers demandes (le transport n'est pas simule).
  grep -q '"GET /ok/manifest.sig HTTP' "$OUT/http.log" || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=serveur_non_sollicite" >&2; exit 1; }
  grep -aF "BOUCHAUD_OTA_VERIFIE" <<< "$propre1" | sed 's/^.*BOUCHAUD_OTA/  BOUCHAUD_OTA/'
fi
if [ "$SCENARIO" = sans-cle ]; then
  sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aF "BOUCHAUD_UPDATE_CLES confiance=0 production=0 developpement=0" >/dev/null \
    || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=cles_de_confiance_non_nulles" >&2; exit 1; }
  # The bench public key must not be inside the release kernel, stripped
  # (what the slot carries) or not (what the build produced).
  python3 - "$CLE_PUBLIQUE" "$OUT/kernel-bon.elf" target/x86_64-bouchaud_os_uefi/debug/bouchaud-os <<'PY' \
    || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=cle_de_banc_dans_le_noyau" >&2; exit 1; }
import sys
cle = bytes.fromhex(sys.argv[1])
for f in sys.argv[2:]:
    if cle in open(f, "rb").read():
        print("cle de banc trouvee dans", f); sys.exit(1)
print("AB_CLE_ABSENTE_DU_NOYAU", " ".join(sys.argv[2:]))
PY
fi
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; then
  # No private key in anything this bench leaves behind (images, ramdisks,
  # logs, manifests).
  if grep -rlaE -- "-----BEGIN [A-Z ]*PRIVATE KEY-----" "$OUT" >/dev/null 2>&1; then
    echo "AB_ROLLBACK_FAIL scenario=$SCENARIO raison=cle_privee_dans_les_sorties" >&2; exit 1
  fi
  echo "AB_AUCUNE_CLE_PRIVEE_DANS_LES_SORTIES"
fi
if [ "$obtenu" != "$ATTENDU" ]; then
  echo "AB_ROLLBACK_FAIL scenario=$SCENARIO" >&2
  exit 1
fi
echo "AB_ROLLBACK_OK scenario=$SCENARIO"
