#!/usr/bin/env bash
#
# BANC DE CHARGE ENTREE-SORTIE DE LA TRIGKEY
#
# # Ce que ce banc mesure, et qu'aucun autre ne mesurait
#
# Les trois archives physiques s'arretent toutes a l'instant precis ou le
# navigateur commence a lire ses binaires sur la cle d'amorcage. Le banc
# d'endurance existant ne pouvait pas le voir : personne n'y lisait le volume
# USB pendant que l'enregistreur ecrivait. Les deux consommateurs du pilote
# xHCI n'y etaient donc jamais en concurrence -- et c'est la concurrence qui
# faisait la panne.
#
# Ici, pendant toute la session :
#
#   * un fil lit le volume USB en continu, par le meme chemin bloc que le
#     chargeur du navigateur ;
#   * la scrutation HID tourne, et son PIRE ecart entre deux tours servis est
#     mesure ;
#   * trois pannes artificielles sont armees en cours de route -- echeance de
#     donnees, echeance de statut, verrou deja tenu ;
#   * l'enregistreur travaille en RAM SEULE, et le support n'est touche qu'a
#     l'extinction volontaire, qui a lieu a la fin du banc.
#
# Criteres verifies (A, B, C, D du lot) :
#
#   A  l'enregistreur RAM n'a pas cesse de produire pendant toute la duree ;
#   B  les echeances injectees n'ont tue ni la scrutation HID ni
#      l'ordonnanceur, et le transport BOT s'est repris ;
#   C  le verrou du pilote est revenu a `aucun` ;
#   D  l'extinction a vraiment draine le tambour, et l'extracteur officiel
#      retrouve le debut, le milieu, la fin et la marque de FIN.
#
#     tools/ci/run_trigkey_ladybird_io_stress.sh [secondes]
set -uo pipefail
cd "$(dirname "$0")/../.."

SECONDES=${1:-120}
ECART_HID_MAX_MS=${BANC_ECART_HID_MAX_MS:-1000}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
# `BANC_TRAVAIL=/chemin` garde les artefacts : image, cle, journal serie et
# archive extraite. C'est ce qu'on veut quand le banc echoue, et c'est
# exactement ce qui manquait aux trois sessions physiques.
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

echo "=== noyau UEFI stage2 + banc de charge (${SECONDES}s) ==="
BOUCHAUD_BANC_SECONDES="$SECONDES" \
cargo +nightly-2026-06-01 build --target targets/x86_64-bouchaud_os_uefi.json \
  --no-default-features --features uefi-boot,reference-bringup,reference-desktop,banc-io || exit 1
NOYAU=target/x86_64-bouchaud_os_uefi/debug/bouchaud-os

echo "=== image amorcable ==="
# HORS DU DEPOT : `.cargo/config.toml` force la cible bare-metal, et le
# constructeur est un outil HOTE.
cp -r tools/reference/uefi-image-builder "$TRAVAIL/builder"
rm -rf "$TRAVAIL/builder/target"
RAMDISK=${BOUCHAUD_RAMDISK:-}
if [ -n "$RAMDISK" ] && [ -f "$RAMDISK" ]; then
    echo "    donnees Ladybird reelles : $RAMDISK"
    ( cd "$TRAVAIL/builder" && cargo +nightly-2026-06-01 run --release -- \
        "$OLDPWD/$NOYAU" "$TRAVAIL/stage2.img" 1920 1080 "$RAMDISK" ) || exit 1
else
    # SANS L'ARBRE LADYBIRD, LA CHARGE RESTE CELLE QU'ON VEUT MESURER.
    #
    # Ce banc ne mesure pas le rendu du navigateur : il mesure ce que son
    # CHARGEMENT fait au pilote USB. Le fil de charge produit ce trafic-la par
    # le meme chemin bloc, avec ou sans les binaires reels -- et sans eux, il
    # le produit meme en integration continue.
    echo "    pas de ramdisk Ladybird : la charge vient du fil de banc"
    ( cd "$TRAVAIL/builder" && cargo +nightly-2026-06-01 run --release -- \
        "$OLDPWD/$NOYAU" "$TRAVAIL/stage2.img" 1920 1080 ) || exit 1
fi

echo "=== cle USB portant la partition BLACKBOX ==="
python3 tools/reference/fabrique-disque-blackbox.py "$TRAVAIL/cle.img" --mio 128 --partition-mio 32 || exit 1

# DEUX SUPPORTS, ET C EST LE POINT DU BANC.
#
# Une interface Mass Storage dont le disque porte la partition
# BOUCHAUD-BLACKBOX est CEDEE a l enregistreur : elle ne devient pas un volume
# bloc. Avec une seule cle, le fil de charge n avait donc rien a lire -- et le
# banc mesurait un pilote sans concurrence, ce qui est exactement ce qu il est
# cense ne pas faire.
#
# Le second support devient Volume(3) et porte la charge. Les deux partagent le
# meme controleur, le meme anneau d evenements et le meme verrou : c est cette
# concurrence-la qui a coute le clavier et l archive le 16 septembre.
echo "=== support de charge ==="
python3 - "$TRAVAIL/charge.img" <<'CHARGE'
import os, sys
# Soixante-quatre mebioctets de motif non nul : un disque de zeros se
# compresse dans le cache de l hote et ne couterait aucun transfert reel.
chemin = sys.argv[1]
bloc = bytes((i * 7 + 13) % 251 for i in range(65536))
with open(chemin, "wb") as f:
    for _ in range(64 * 1024 * 1024 // len(bloc)):
        f.write(bloc)
print("CHARGE_DISQUE {} octets={}".format(chemin, os.path.getsize(chemin)))
CHARGE

echo "=== QEMU, 16 coeurs, xHCI, clavier et souris USB ==="
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$TRAVAIL/vars.fd"
# `-serial file:` et non `-serial none` : le banc rend son verdict sur COM1.
# La cadence des sondes suit `presence_com1()`, et le port existe donc ici --
# c'est une difference assumee avec la TRIGKEY, dont le verdict vient de
# l'archive et non de la console.
timeout $((SECONDES + 120)) qemu-system-x86_64 \
  -machine q35 -m 4096 -smp 16 -display none -no-reboot \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file="$TRAVAIL/vars.fd" \
  -drive format=raw,file="$TRAVAIL/stage2.img" \
  -device qemu-xhci,id=xhci \
  -device usb-kbd,bus=xhci.0 \
  -device usb-tablet,bus=xhci.0 \
  -drive if=none,id=cleusb,format=raw,file="$TRAVAIL/cle.img" \
  -device usb-storage,bus=xhci.0,port=3,drive=cleusb \
  -drive if=none,id=clecharge,format=raw,file="$TRAVAIL/charge.img" \
  -device usb-storage,bus=xhci.0,port=4,drive=clecharge \
  -serial file:"$TRAVAIL/serie.log"

if ! grep -aq 'BOUCHAUD_BLACKBOX_USB_READY' "$TRAVAIL/serie.log"; then
    echo "l'enregistreur n'a jamais trouve sa partition : le banc ne teste rien" >&2
    grep -a 'BOUCHAUD_BLACKBOX' "$TRAVAIL/serie.log" | tail -5 >&2
    exit 1
fi

VERDICT=$(grep -a 'BOUCHAUD_BANC_IO_VERDICT' "$TRAVAIL/serie.log" | tail -1)
if [ -z "$VERDICT" ]; then
    echo "le banc n'a pas rendu son verdict : la session ne s'est pas terminee" >&2
    tail -20 "$TRAVAIL/serie.log" >&2
    exit 1
fi
echo "$VERDICT"

echo "=== extraction par le parser OFFICIEL du depot ==="
python3 tools/reference/extract-blackbox.py --image "$TRAVAIL/cle.img" \
    --output "$TRAVAIL/archive" || exit 1

python3 - "$TRAVAIL/archive" "$SECONDES" "$ECART_HID_MAX_MS" "$VERDICT" <<'PY'
import json, pathlib, re, sys

racine = pathlib.Path(sys.argv[1])
duree = int(sys.argv[2])
ecart_max_permis = int(sys.argv[3])
verdict = dict(
    partie.split("=", 1)
    for partie in sys.argv[4].split()
    if "=" in partie
)

echecs = []


def exige(condition, message):
    if not condition:
        echecs.append(message)


def entier(cle):
    try:
        return int(verdict.get(cle, "0"))
    except ValueError:
        return 0


manifeste = json.loads((racine / "manifest.json").read_text())
sessions = manifeste["sessions"]
if not sessions:
    sys.exit("aucune session dans l'archive")
sdir = racine / sessions[-1]["directory"]
echantillons = (sdir / "samples.log").read_text(errors="replace")
marques = (sdir / "markers.log").read_text(errors="replace")

horodatages = [int(m) for m in re.findall(r"^sample ts_ns=(\d+)", echantillons, re.M)]
if len(horodatages) < 2:
    sys.exit("moins de deux echantillons : rien a mesurer")
couverture = (horodatages[-1] - horodatages[0]) / 1e9

print(f"BANC_ECHANTILLONS={len(horodatages)}")
print(f"BANC_PREMIER_S={horodatages[0]/1e9:.2f}")
print(f"BANC_DERNIER_S={horodatages[-1]/1e9:.2f}")
print(f"BANC_COUVERTURE_S={couverture:.1f}")

# --- A : l'enregistreur RAM n'a pas cesse de produire -----------------------
#
# La couverture est exigee sur QUATRE-VINGTS POUR CENT de la duree, et non sur
# la totalite : la session commence avant le premier echantillon -- il faut
# que le materiel soit la -- et se termine par un vidage qui prend du temps.
# Exiger cent pour cent ferait echouer le banc sur son propre demarrage.
minimum = duree * 0.8
exige(
    couverture >= minimum,
    f"A : couverture {couverture:.1f}s < {minimum:.1f}s -- l'enregistreur a cesse de produire",
)
# ET LE MILIEU, PAS SEULEMENT LES DEUX BOUTS.
#
# Une archive qui garde le debut et la fin mais rien entre les deux est
# exactement le symptome qu'on corrige. On exige donc un echantillon dans
# chaque tiers de la session.
debut, fin = horodatages[0], horodatages[-1]
tiers = (fin - debut) / 3
for numero, (bas, haut) in enumerate(
    [(debut, debut + tiers), (debut + tiers, debut + 2 * tiers), (debut + 2 * tiers, fin)], 1
):
    exige(
        any(bas <= t <= haut for t in horodatages),
        f"A : aucun echantillon dans le tiers {numero} de la session",
    )

poses = entier("blackbox_ram_records")
ecrases = entier("blackbox_ram_overwrites")
perdus = entier("blackbox_ram_lost")
print(f"BANC_TAMBOUR_POSES={poses}")
print(f"BANC_TAMBOUR_ECRASES={ecrases}")
print(f"BANC_TAMBOUR_PERDUS={perdus}")
exige(poses > 0, "A : le tambour RAM n'a rien pose")
# `ecrases` compte les descripteurs RECYCLES, dont ceux d'enregistrements
# deja sortis vers le support -- le vidage final en recycle par construction.
# `perdus` compte ceux qui n'etaient pas encore sortis, et c'est le seul des
# deux qui designe quelque chose qui manque.
exige(perdus == 0, f"A : {perdus} enregistrement(s) perdu(s) avant d'atteindre le support")

# --- B : les echeances injectees n'ont tue ni le HID ni l'ordonnanceur ------
consommees = entier("injections_consommees")
exige(consommees >= 3, f"B : {consommees} injection(s) consommee(s) sur 3 armees")
ecart = entier("hid_poll_gap_max_ms")
print(f"BANC_HID_ECART_MAX_MS={ecart}")
exige(
    ecart <= ecart_max_permis,
    f"B : pire ecart de scrutation HID {ecart} ms > {ecart_max_permis} ms",
)
exige(
    verdict.get("bot_etat") == "pret",
    f"B : le transport BOT a fini en « {verdict.get('bot_etat')} » au lieu de « pret »",
)
reprises = entier("bot_recoveries")
reussies = entier("bot_recovery_success")
print(f"BANC_BOT_ECHEANCES={entier('bot_timeouts')}")
print(f"BANC_BOT_REPRISES={reprises} REUSSIES={reussies}")
exige(reprises > 0, "B : aucune reprise BOT -- les echeances injectees n'ont pas porte")
exige(reussies > 0, "B : aucune reprise BOT n'a abouti")
# L'ORDONNANCEUR A CONTINUE : le compositeur a compose des trames APRES les
# injections, et le coeur zero a recu des quantums. Sans ces deux-la, un
# transport « pret » ne dirait rien -- une machine figee l'est aussi.
exige(entier("wm_heartbeat") > 0, "B : le gestionnaire de fenetres n'a pas battu")
exige(entier("scheduler_heartbeat") > 0, "B : le coeur zero n'a recu aucun quantum")

# --- C : le verrou est revenu a personne ------------------------------------
proprietaire = verdict.get("runtime_owner")
print(f"BANC_VERROU_FINAL={proprietaire}")
print(f"BANC_VERROU_TENUE_MAX_NS={entier('runtime_max_hold_ns')}")
exige(
    proprietaire == "aucun",
    f"C : le verrou du pilote est reste tenu par « {proprietaire} »",
)

# --- D : l'extinction a vraiment draine -------------------------------------
exige("FIN raison=" in marques, "D : aucune marque de FIN -- l'extinction n'a pas draine")
exige("START boot_id=" in marques, "D : aucune marque de DEBUT")
fin_marque = re.search(
    r"FIN .*?tambour_ecrases=(\d+) tambour_perdus=(\d+) tambour_refuses=(\d+)", marques
)
if fin_marque:
    print(
        f"BANC_FIN_ECRASES={fin_marque.group(1)} "
        f"PERDUS={fin_marque.group(2)} REFUSES={fin_marque.group(3)}"
    )
    exige(
        int(fin_marque.group(2)) == 0,
        f"D : la marque de FIN compte {fin_marque.group(2)} enregistrement(s) perdu(s)",
    )
exige(
    "draine=1" in marques,
    "D : la marque de FIN dit que le drainage n'est pas alle au bout",
)
fin_vidage = re.search(r"FIN .*?vidage_poses=(\d+) vidage_manquants=(\d+)", marques)
if fin_vidage:
    print(f"BANC_VIDAGE_POSES={fin_vidage.group(1)} MANQUANTS={fin_vidage.group(2)}")
    exige(
        int(fin_vidage.group(2)) == 0,
        f"D : {fin_vidage.group(2)} enregistrement(s) introuvable(s) au vidage",
    )
# LES QUATRE FLUX, PAS SEULEMENT LES ECHANTILLONS.
#
# Une archive qui ne porte que les echantillons a perdu le journal serie --
# c'est-a-dire tout le demarrage -- et les evenements de vol, qui sont les
# seuls a dater un IRQ qui ne revient pas.
for fichier, pourquoi in (
    ("serial.log", "le journal serie, donc tout le demarrage"),
    ("flight.csv", "les evenements de vol, seuls a dater un IRQ qui ne revient pas"),
    ("memory.log", "les releves memoire"),
):
    chemin = sdir / fichier
    exige(
        chemin.exists() and chemin.stat().st_size > 0,
        f"D : {fichier} est vide -- l'extinction n'a pas converti {pourquoi}",
    )

if echecs:
    for echec in echecs:
        print(f"ECHEC {echec}")
    sys.exit(f"{len(echecs)} critere(s) non tenu(s)")
print("BOUCHAUD_TRIGKEY_IO_STRESS_OK")
PY
