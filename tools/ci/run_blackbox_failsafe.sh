#!/usr/bin/env bash
#
# BOUCHAUD_C72_SCENARIO_G -- LE CHECKPOINT SURVIT A UNE EXTINCTION RATEE
#
# # Ce que ce banc prouve, et pourquoi il existe
#
# Au test physique du Trigkey (image d131f2a), l'utilisateur demande
# l'extinction, la sauvegarde finale echoue, la machine s'eteint -- et TOUTE la
# session est perdue, y compris les journaux qui expliquaient le reseau.
#
# Le checkpoint existe pour qu'au pire on ne perde que la derniere fenetre.
# Encore faut-il le PROUVER, et le prouver veut dire faire echouer le vidage
# final pour de vrai, pas esperer qu'il echoue un jour.
#
# # Deux executions, et la seconde est ce qui rend la premiere credible
#
#   bras KO    checkpoint pose, PUIS les trois pannes USB armees juste avant
#              l'extinction. Attendu : pas de marque FIN, mais un CHECKPOINT
#              valide et les donnees lisibles jusqu'a lui.
#
#   bras OK    meme image, memes secondes, sans sabotage final.
#              Attendu : COMPLETE.
#
# Sans le bras OK, un `PARTIEL_CHECKPOINT` ne prouverait rien : il pourrait
# venir d'un banc qui n'arrive jamais a poser une marque de fin, sabotage ou
# pas. C'est la DIFFERENCE entre les deux bras qui dit que la condition a
# reellement ete exercee.
#
#     tools/ci/run_blackbox_failsafe.sh
set -uo pipefail
cd "$(dirname "$0")/../.."

SECONDES=${BLACKBOX_FAILSAFE_SECONDES:-40}
CHECKPOINT_S=${BLACKBOX_FAILSAFE_CHECKPOINT_S:-20}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

if [ "$CHECKPOINT_S" -ge "$SECONDES" ]; then
    echo "checkpoint ($CHECKPOINT_S s) au-dela de la duree ($SECONDES s) : le banc ne testerait rien" >&2
    exit 1
fi

construit_et_tourne() {
    local bras="$1" final_ko="$2" sortie="$3"
    echo "=== bras ${bras} : noyau (final_ko=${final_ko}, checkpoint a ${CHECKPOINT_S}s) ==="
    # INJECTIONS=0 : les trois pannes horodatees du banc d'endurance ne doivent
    # PAS tomber avant le checkpoint, sinon on mesurerait la reprise et non la
    # survie du checkpoint.
    BOUCHAUD_BANC_SECONDES="$SECONDES" \
    BOUCHAUD_BANC_INJECTIONS=0 \
    BOUCHAUD_BANC_CHECKPOINT_S="$CHECKPOINT_S" \
    BOUCHAUD_BANC_FINAL_KO="$final_ko" \
    cargo +nightly-2026-06-01 build --target targets/x86_64-bouchaud_os_uefi.json \
      --no-default-features --features uefi-boot,reference-bringup,reference-desktop,banc-io || return 1

    rm -rf "$sortie"; mkdir -p "$sortie"
    cp -r tools/reference/uefi-image-builder "$sortie/builder"
    rm -rf "$sortie/builder/target"
    ( cd "$sortie/builder" && cargo +nightly-2026-06-01 run --release -- \
        "$OLDPWD/target/x86_64-bouchaud_os_uefi/debug/bouchaud-os" \
        "$sortie/stage2.img" 1920 1080 ) || return 1

    python3 tools/reference/fabrique-disque-blackbox.py "$sortie/cle.img" \
        --mio 64 --partition-mio 32 || return 1

    cp /usr/share/OVMF/OVMF_VARS_4M.fd "$sortie/vars.fd"
    timeout $((SECONDES + 150)) qemu-system-x86_64 \
      -machine q35 -m 4096 -smp 4 -display none -no-reboot \
      -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
      -drive if=pflash,format=raw,unit=1,file="$sortie/vars.fd" \
      -drive format=raw,file="$sortie/stage2.img" \
      -device qemu-xhci,id=xhci \
      -drive if=none,id=cleusb,format=raw,file="$sortie/cle.img" \
      -device usb-storage,bus=xhci.0,port=1,drive=cleusb \
      -serial file:"$sortie/serie.log"

    if ! grep -aq 'BOUCHAUD_BLACKBOX_USB_READY' "$sortie/serie.log"; then
        echo "bras ${bras} : l'enregistreur n'a jamais trouve sa partition" >&2
        return 1
    fi
    python3 tools/reference/extract-blackbox.py --image "$sortie/cle.img" \
        --output "$sortie/archive" || return 1
}

construit_et_tourne KO 1 "$TRAVAIL/ko" || exit 1
construit_et_tourne OK 0 "$TRAVAIL/ok" || exit 1

python3 - "$TRAVAIL/ko" "$TRAVAIL/ok" <<'PY'
import json, pathlib, sys

def verdict(racine):
    m = json.loads((pathlib.Path(racine) / "archive/manifest.json").read_text())
    if not m["sessions"]:
        sys.exit(f"{racine} : aucune session dans l'archive")
    return m["sessions"][-1]

ko = verdict(sys.argv[1])
ok = verdict(sys.argv[2])
serie_ko = (pathlib.Path(sys.argv[1]) / "serie.log").read_text(errors="replace")

echecs = []

# Le sabotage a-t-il seulement eu lieu ? Un banc vert sans condition exercee
# ne vaut rien -- c'est la lecon des trois faux verts de la campagne Ladybird.
if "BOUCHAUD_BANC_IO_FINAL_KO arme=1" not in serie_ko:
    echecs.append("bras KO : le sabotage final n'a jamais ete arme")
if "BOUCHAUD_BANC_IO_CHECKPOINT" not in serie_ko:
    echecs.append("bras KO : aucun checkpoint n'a ete pose")

# Le bras KO : partiel, pas complet, pas coupe.
if ko["completude"] != "PARTIEL_CHECKPOINT":
    echecs.append(f"bras KO : completude={ko['completude']} attendu PARTIEL_CHECKPOINT")
if ko["fin_presente"]:
    echecs.append("bras KO : une marque FIN est presente alors que le vidage final a ete sabote")
if ko["checkpoints"] < 1:
    echecs.append("bras KO : aucun checkpoint dans l'archive")
if not ko["records"]:
    echecs.append("bras KO : archive vide -- le checkpoint n'a rien sauve")
if ko["dernier_checkpoint_confirme"] in (None, 0):
    echecs.append("bras KO : le checkpoint ne dit pas jusqu'ou les donnees sont confirmees")

# Le bras OK : la meme image sans sabotage DOIT aller au bout.
if ok["completude"] != "COMPLETE":
    echecs.append(f"bras OK : completude={ok['completude']} attendu COMPLETE — "
                  f"le bras KO ne prouve donc rien")

print(f"KO : completude={ko['completude']} fin={ko['fin_presente']} "
      f"checkpoints={ko['checkpoints']} seq={ko['dernier_checkpoint_seq']} "
      f"confirme={ko['dernier_checkpoint_confirme']} records={ko['records']}")
print(f"OK : completude={ok['completude']} fin={ok['fin_presente']} "
      f"checkpoints={ok['checkpoints']} records={ok['records']}")

if echecs:
    for e in echecs:
        print("BLACKBOX_FAILSAFE_FAIL", e)
    sys.exit(1)
print("BLACKBOX_FAILSAFE_SMOKE ok")
PY
