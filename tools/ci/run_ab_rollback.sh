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
#   sante-ko   slot B = good kernel, but its critical service never becomes
#              ready: serviced -> RecoveryRequired, never commits -> rollback
#   mise-a-jour  A runs, B is zeroed and Empty. updated (ring 3, install
#              capability only) checks its refusals, writes B from the image
#              in A's ramdisk, the kernel re-reads it and marks it Candidate;
#              then B boots, is confirmed by serviced, and stays.
#
# BO_AB_SUPPORT=nvme|usb selects the disk transport (default nvme).
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
SCENARIO=${1:?usage: run_ab_rollback.sh panique|gel|meta-0|meta-1|meta-2|sante|sante-ko|sans-vidange|mise-a-jour|mise-a-jour-morts|sans-cle|recovery-requise|recovery-rollback|echecs-repetes [OUT]}
RECOVERY=0
# mise-a-jour-morts = mise-a-jour + updated dies on purpose twice (during the
# writes, during the read-back); the third instance must inherit nothing.
VARIANTE=""
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
  mise-a-jour) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A* B+ B="; BOOT_DELAI_S=${BO_AB_BOOT_S:-600} ;;
  sans-cle) MAUVAIS=bon; META_SPEC=a-bon; ATTENDU="A0 A="; BOOT_DELAI_S=${BO_AB_BOOT_S:-600} ;;
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
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; then
  openssl genpkey -algorithm ed25519 -out "$CLES/version.pem"
  CLE_PUBLIQUE=$(openssl pkey -in "$CLES/version.pem" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
  echo "AB_CLE_DEV publique=$CLE_PUBLIQUE (generee pour ce banc, detruite a la fin)"
  # sans-cle: the manifest IS signed with it, but the kernel is a normal
  # build that does not trust it.
  if [ "$SCENARIO" = mise-a-jour ]; then export BOUCHAUD_CLE_VERSION_DEV=$CLE_PUBLIQUE; fi
fi

# --- build: shim, kernels, payloads, metadata, disk ------------------------
cargo +"$TOOLCHAIN" build --release --manifest-path tools/reference/uefi-preboot-probe/Cargo.toml \
  --target x86_64-unknown-uefi
SHIM=tools/reference/uefi-preboot-probe/target/x86_64-unknown-uefi/release/bouchaud-uefi-preboot-probe.efi
for k in $(printf '%s\n' bon "$MAUVAIS" | sort -u); do
  f=$FEATURES; [ "$k" != bon ] && f="$FEATURES,banc-ab-$k"
  # The bench key needs BOTH the variable and this feature (BOUCHAUD_CLE_DEV_GARDE_V1).
  if [ -n "${BOUCHAUD_CLE_VERSION_DEV:-}" ]; then f="$f,banc-cle-dev"; fi
  cargo +"$TOOLCHAIN" build --target targets/x86_64-bouchaud_os_uefi.json --no-default-features --features "$f"
  # Debug info is for the host (symbolisation), not for the slot: stripping it
  # keeps a slot image small enough to be shipped and written by updated.
  strip --strip-debug -o "$OUT/kernel-$k.elf" target/x86_64-bouchaud_os_uefi/debug/bouchaud-os
done
TRAVAIL=$(mktemp -d); trap 'rm -rf "$TRAVAIL" "$CLES"' EXIT
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
( cd "$TRAVAIL/builder" && cargo +"$TOOLCHAIN" build --release -q )
BUILDER=$TRAVAIL/builder/target/release/bouchaud-reference-uefi-image-builder
"$BUILDER" --charge-installation "$OUT/kernel-bon.elf" "$OUT/slot-a"
"$BUILDER" --charge-installation "$OUT/kernel-$MAUVAIS.elf" "$OUT/slot-b"
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
case "$SCENARIO" in sante|sante-ko|sans-vidange|mise-a-jour|sans-cle|recovery-rollback|echecs-repetes)
  # The system userland of each slot: serviced (system supervisor) and its
  # critical services (health contract v3: healthd, updated, logd). In sante-ko the
  # CANDIDATE's critical service fails.
  SDK=userland/libs/bouchaud-sdk/build-program.sh
  for prog in services/serviced/serviced services/healthd/healthd services/updated/updated services/logd/logd tests/healthd-defaillant; do
    "$SDK" "$TRAVAIL/$(basename $prog)" "userland/$prog.c"
  done
  systeme="bin/serviced=$TRAVAIL/serviced bin/updated=$TRAVAIL/updated bin/logd=$TRAVAIL/logd"
  critique_b=$TRAVAIL/healthd; [ "$SCENARIO" = sante-ko ] && critique_b=$TRAVAIL/healthd-defaillant
  critique_a=$TRAVAIL/healthd; [ "$SCENARIO" = echecs-repetes ] && critique_a=$TRAVAIL/healthd-defaillant
  ramdisk "$OUT/slot-b/ramdisk" $systeme "bin/healthd=$critique_b"
  if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; then
    # The release: slot B's image, built exactly as --disque-ab would lay it,
    # and its manifest. Shipped INSIDE the running system A; B on disk is
    # zeroed below, so only updated can make it bootable.
    "$BUILDER" --image-slot "$OUT/slot-b" "$OUT/system-b.img"
    octets=$(stat -c %s "$OUT/system-b.img")
    empreinte=$(sha256sum "$OUT/system-b.img" | cut -d' ' -f1)
    # The canonical manifest (system/update-core/src/manifeste.rs), signed.
    printf 'bouchaud-manifeste-v1\nversion=bench-%s\nimage_sha256=%s\nimage_octets=%s\nessais=3\n' \
      "$(git rev-parse --short HEAD)" "$empreinte" "$octets" > "$OUT/manifest"
    openssl pkeyutl -sign -rawin -inkey "$CLES/version.pem" -in "$OUT/manifest" -out "$OUT/manifest.sig"
    : > "$OUT/selftest"
    morts=(); [ "$VARIANTE" = morts ] && morts=("update/selftest-mort=$OUT/selftest")
    ramdisk "$OUT/slot-a/ramdisk" $systeme "bin/healthd=$TRAVAIL/healthd" \
      "update/manifest=$OUT/manifest" "update/manifest.sig=$OUT/manifest.sig" \
      "update/selftest=$OUT/selftest" "update/system.img=$OUT/system-b.img" "${morts[@]}"
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
if [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; then
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
  *) echo "BO_AB_SUPPORT inconnu : ${BO_AB_SUPPORT}" >&2; exit 2 ;;
esac
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
  qemu-system-x86_64 -machine q35 -m 2048 -smp 2 -cpu max \
    -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
    -drive if=pflash,format=raw,unit=1,file="$OUT/vars.fd" \
    -drive if=none,id=disque,format=raw,file="$OUT/disque-ab.img" \
    "${SUPPORT[@]}" \
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
    # Boot 1 of mise-a-jour is judged on updated alone: its own health line
    # can come before the install ends, and must not cut the install short.
    if { [ "$SCENARIO" = mise-a-jour ] || [ "$SCENARIO" = sans-cle ]; } && [ "$slot" = A ] && [ "$i" = 1 ]; then
      if ! kill -0 "$pid" 2>/dev/null; then break; fi
      sleep 1; continue
    fi
    case "$SCENARIO" in sante|sante-ko|sans-vidange|mise-a-jour|sans-cle|recovery-rollback|echecs-repetes)
      if [ -n "$slot" ] && grep -aq "result=committed" <<< "$propre"; then verdict="$slot+"; break; fi
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
           "UPDATED_INSTALL_OK" "signature=trusted hash=valid" "UPDATED_REFUSALS_OK"; do
    sed 's/\x1b\[[0-9;]*m//g' "$OUT/boot-1.log" | grep -aF "$m" >/dev/null \
      || { echo "AB_ROLLBACK_FAIL scenario=$SCENARIO marqueur_absent=$m" >&2; exit 1; }
  done
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
