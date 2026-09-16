#!/usr/bin/env bash
#
# BANC D'ENDURANCE DE L'ENREGISTREUR DE VOL
#
# Le releve physique du 16 septembre couvrait 1,07 s a 8,15 s d'une session de
# vingt minutes. Aucune campagne ne pouvait le voir : sans partition
# BOUCHAUD-BLACKBOX presentee en USB, `blackbox_storage_ready()` est faux et
# tout le chemin d'ecriture sort immediatement. Le defaut n'existait donc que
# sur la machine, ce qui est la pire facon d'exister.
#
# Ce banc reproduit la cible : image UEFI stage2, xHCI, cle USB portant une
# vraie GPT nommee BOUCHAUD-BLACKBOX. Il exige ensuite que l'archive couvre au
# moins la duree demandee, DEBUT ET FIN relus par le parser officiel.
#
# # Pourquoi il eteint desormais la machine
#
# L'enregistreur ne touche plus le support pendant le runtime : produire une
# trace est purement memoire, et la persistance n'a lieu qu'a l'extinction
# VOLONTAIRE. Couper QEMU au bout de N secondes, comme le faisait ce banc,
# laisserait donc le tambour intact en RAM -- c'est-a-dire une archive vide.
#
# Le drapeau `banc-io` fournit l'extinction programmee. Les pannes
# artificielles, elles, sont DESARMEES ici : ce banc mesure une duree, pas une
# reprise, et il tourne sans volume de charge -- les pannes tomberaient sur le
# vidage final, ou elles mesureraient autre chose que ce qu'on demande.
# La reprise, c'est `run_trigkey_ladybird_io_stress.sh`.
#
#     tools/ci/run_blackbox_endurance.sh [secondes]
set -uo pipefail
cd "$(dirname "$0")/../.."

SECONDES=${1:-90}
MINIMUM=${BLACKBOX_COUVERTURE_MIN:-60}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
# `BANC_TRAVAIL=/chemin` garde les artefacts : image, cle, journal serie et
# archive extraite. C'est ce qu'on veut quand le banc echoue -- et c'est
# exactement ce qui manquait aux trois sessions physiques.
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

echo "=== noyau UEFI stage2 + extinction programmee (${SECONDES}s) ==="
BOUCHAUD_BANC_SECONDES="$SECONDES" BOUCHAUD_BANC_INJECTIONS=0 \
cargo +nightly-2026-06-01 build --target targets/x86_64-bouchaud_os_uefi.json \
  --no-default-features --features uefi-boot,reference-bringup,reference-desktop,banc-io || exit 1
NOYAU=target/x86_64-bouchaud_os_uefi/debug/bouchaud-os

echo "=== image amorcable ==="
# HORS DU DEPOT : `.cargo/config.toml` force la cible bare-metal, et le
# constructeur est un outil HOTE. Le script PowerShell fait le meme detour.
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
rm -rf "$TRAVAIL/builder/target"
( cd "$TRAVAIL/builder" && cargo +nightly-2026-06-01 run --release -- \
    "$OLDPWD/$NOYAU" "$TRAVAIL/stage2.img" 1920 1080 ) || exit 1

echo "=== cle USB portant la partition BLACKBOX ==="
python3 tools/reference/fabrique-disque-blackbox.py "$TRAVAIL/cle.img" --mio 64 --partition-mio 32 || exit 1

echo "=== QEMU ${SECONDES}s, 4 coeurs ==="
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$TRAVAIL/vars.fd"
timeout $((SECONDES + 120)) qemu-system-x86_64 \
  -machine q35 -m 4096 -smp 4 -display none -no-reboot \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file="$TRAVAIL/vars.fd" \
  -drive format=raw,file="$TRAVAIL/stage2.img" \
  -device qemu-xhci,id=xhci \
  -drive if=none,id=cleusb,format=raw,file="$TRAVAIL/cle.img" \
  -device usb-storage,bus=xhci.0,port=1,drive=cleusb \
  -serial file:"$TRAVAIL/serie.log"

if ! grep -aq 'BOUCHAUD_BLACKBOX_USB_READY' "$TRAVAIL/serie.log"; then
    echo "l'enregistreur n'a jamais trouve sa partition : le banc ne teste rien" >&2
    exit 1
fi

echo "=== extraction par le parser OFFICIEL du depot ==="
python3 tools/reference/extract-blackbox.py --image "$TRAVAIL/cle.img" \
    --output "$TRAVAIL/archive" || exit 1

python3 - "$TRAVAIL/archive" "$MINIMUM" <<'PY'
import re, sys, json, pathlib
racine = pathlib.Path(sys.argv[1]); minimum = int(sys.argv[2])
manifeste = json.loads((racine / "manifest.json").read_text())
sessions = manifeste["sessions"]
if not sessions:
    sys.exit("aucune session dans l'archive")
sdir = racine / sessions[-1]["directory"]
horodatages = [int(m) for m in
               re.findall(r'^sample ts_ns=(\d+)', (sdir / "samples.log").read_text(errors="replace"), re.M)]
if len(horodatages) < 2:
    sys.exit("moins de deux echantillons : rien a mesurer")
couverture = (horodatages[-1] - horodatages[0]) / 1e9
print(f"BLACKBOX_ECHANTILLONS={len(horodatages)}")
print(f"BLACKBOX_PREMIER_S={horodatages[0]/1e9:.2f}")
print(f"BLACKBOX_DERNIER_S={horodatages[-1]/1e9:.2f}")
print(f"BLACKBOX_COUVERTURE_S={couverture:.1f}")
# DEBUT **ET** FIN. Une archive qui ne garde que la fin perdrait l'amorcage,
# une qui ne garde que le debut perdrait le blocage : les deux sont exiges.
marques = (sdir / "markers.log").read_text(errors="replace")
if "START boot_id=" not in marques:
    sys.exit("marque de DEBUT absente : le debut de session n'a pas ete relu")
if "FIN raison=" not in marques:
    sys.exit("marque de FIN absente : le silence ne se distingue pas d'une coupure")
if "draine=1" not in marques:
    sys.exit("la marque de FIN dit que le drainage n'est pas alle au bout")
if couverture < minimum:
    sys.exit(f"couverture {couverture:.1f}s < {minimum}s exiges")
print("BOUCHAUD_BLACKBOX_ENDURANCE_OK")
PY
