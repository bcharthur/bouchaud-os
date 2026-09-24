#!/usr/bin/env bash
#
# BOUCHAUD_B4_LES_SCENARIOS_DHCP_DETERMINISTES
#
# # Ce que ce banc ajoute aux precedents
#
# `run_dhcp_wire.sh` prouve ce qui passe SUR LE FIL. `run_dhcp_recuperation.sh`
# prouve que la machine se rattrape seule. Ni l'un ni l'autre ne dit comment
# le CLIENT se comporte quand une reponse manque, arrive en retard, ou porte
# un mauvais identifiant -- parce qu'aucun des deux ne sait provoquer ces cas.
#
# Ici chaque panne est ARMEE et COMPTEE. La regle qui tient tout le banc :
#
#     un scenario negatif dont l'injection n'a pas ete APPLIQUEE est un echec,
#     jamais un succes.
#
# `DHCP_BANC_INJECTION` rapporte les deux nombres -- ce qui restait a injecter
# et ce qui l'a ete -- et le banc exige leur concordance.
#
#     tools/ci/run_dhcp_scenarios.sh [image]
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}
TRAVAIL=${BANC_TRAVAIL:-$(mktemp -d)}
mkdir -p "$TRAVAIL"
if [ -z "${BANC_TRAVAIL:-}" ]; then trap 'rm -rf "$TRAVAIL"' EXIT; fi

echo "=== image mesuree ==="
sha256sum "$BOOT"

echecs=0
declare -a RESUME=()

# joue <nom> <netdev|-> <commandes...>
joue() {
  local nom=$1 netdev=$2; shift 2
  local dossier="$TRAVAIL/$nom"
  mkdir -p "$dossier"
  : > "$dossier/autorun"
  for ligne in "$@"; do printf '%s\n' "$ligne" >> "$dossier/autorun"; done
  printf 'poweroff\n' >> "$dossier/autorun"

  python3 - "$dossier" <<'PY'
import sys, tarfile, pathlib
d = pathlib.Path(sys.argv[1])
image = d / "banc.img"
with tarfile.open(image, "w", format=tarfile.USTAR_FORMAT) as tar:
    info = tar.gettarinfo(str(d / "autorun"), arcname="./autorun")
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.mode = 0o644
    with (d / "autorun").open("rb") as src:
        tar.addfile(info, src)
with image.open("ab") as out:
    out.write(b"\0" * (4 * 1024 * 1024))
PY

  local nd="user,id=net0"
  [ "$netdev" != "-" ] && nd="$netdev"
  timeout -k 5 --signal=TERM 120 qemu-system-x86_64 \
    -drive format=raw,file="$BOOT" \
    -drive format=raw,file="$dossier/banc.img" \
    -m 2048 -display none -no-reboot \
    -netdev "$nd" -device e1000,netdev=net0 \
    -object filter-dump,id=cap0,netdev=net0,file="$dossier/fil.pcap" \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -serial file:"$dossier/serie.log" >/dev/null 2>&1

  SERIE=$(sed 's/\x1b\[[0-9;]*m//g' "$dossier/serie.log" 2>/dev/null || true)
  DOSSIER="$dossier"
}

# Des chaines ici, jamais `printf | grep -q` : sous `pipefail`, grep qui sort
# tot fait rendre 141 au tube. Voir l'entete de `run_rx_recuperation.sh`.
contient() { grep -aqE "$1" <<<"$SERIE"; }
compte()   { grep -acE "$1" <<<"$SERIE" || true; }

verdict() {
  local nom=$1 ok=$2 raison=$3
  if [ "$ok" = 1 ]; then
    RESUME+=("  $nom : OK")
  else
    RESUME+=("  $nom : ECHEC ($raison)")
    echecs=$((echecs + 1))
  fi
  echo "DHCP_SCENARIO nom=$nom verdict=$([ "$ok" = 1 ] && echo OK || echo ECHEC) raison=$raison"
}

# --- 1. DORA normal, des la premiere tentative ------------------------------
echo; echo "########## 1. DORA normal ##########"
joue normal - "netbanc-dhcp etat" "dhcp"
grep -aoE "DHCP_RX_TRACE[^\"]{0,300}|DHCP: [^\"]{0,60}" <<<"$SERIE" | tail -3
if contient "DHCP_RX_TRACE .*result=offre .*accepte=1" \
   && contient "DHCP_RX_TRACE .*result=accuse" \
   && contient "DHCP: bail obtenu"; then
  verdict dora-normal 1 offre-et-accuse-acceptes
else
  verdict dora-normal 0 pas-de-dora-complet
fi

# --- 5. la chaine complete des etages sur une OFFRE valide ------------------
echo; echo "########## 5. Ethernet -> IPv4 -> UDP:68 -> DHCP -> XID ##########"
# Meme passage que le precedent : la chaine se lit sur la trace acceptee.
if contient "DHCP_RX_TRACE .*eth=[1-9][0-9]* .*ipv4_ok=[1-9][0-9]* .*udp_ok=[1-9][0-9]* .*port68=[1-9][0-9]* .*depose=[1-9][0-9]* .*accepte=[1-9]"; then
  verdict chaine-etages 1 tous-les-etages-non-nuls
else
  verdict chaine-etages 0 un-etage-reste-a-zero
fi

# --- 2. la premiere OFFRE est perdue ----------------------------------------
echo; echo "########## 2. premiere OFFRE perdue ##########"
joue offre-perdue - "netbanc-dhcp perte 1" "dhcp" "netbanc-dhcp etat" "dhcp"
grep -aoE "DHCP_BANC_INJECTION[^\"]{0,140}|DHCP: [^\"]{0,60}|dhcp: pas d.OFFER[^\"]{0,20}" <<<"$SERIE" | tail -5
# LA PERTE DOIT SE VOIR, PAS SEULEMENT S'ETRE PRODUITE. Une tentative doit
# echouer faute d'offre, et la suivante reussir : sans ce couple, le scenario
# passerait aussi bien avec une injection sans effet.
if ! contient "DHCP_BANC_INJECTION scenario=etat offres_restantes=0 offres_jetees=1"; then
  verdict offre-perdue 0 injection-non-appliquee-ou-restee-en-attente
elif ! contient "dhcp: pas d.OFFER"; then
  verdict offre-perdue 0 la-perte-n-a-fait-echouer-aucune-tentative
elif [ "$(compte 'DHCP: bail obtenu')" -lt 1 ]; then
  verdict offre-perdue 0 aucune-reprise-apres-la-perte
else
  verdict offre-perdue 1 tentative-perdue-puis-bail-obtenu
fi

# --- 3. plusieurs OFFRES perdues : l'attente doit croitre -------------------
echo; echo "########## 3. plusieurs OFFRES perdues ##########"
joue offres-perdues - "netbanc-dhcp perte 3" "dhcp" "dhcp" "dhcp" "netbanc-dhcp etat" "dhcp"
grep -aoE "DHCP_BANC_INJECTION[^\"]{0,140}" <<<"$SERIE" | tail -2
if ! contient "DHCP_BANC_INJECTION scenario=etat offres_restantes=0 offres_jetees=3"; then
  verdict offres-perdues 0 les-trois-pertes-n-ont-pas-eu-lieu
elif [ "$(compte 'DHCP: bail obtenu')" -lt 1 ]; then
  verdict offres-perdues 0 jamais-de-reprise
else
  verdict offres-perdues 1 trois-pertes-puis-obtenue
fi

# --- 4. OFFRE au mauvais XID : rejet explicite ------------------------------
echo; echo "########## 4. OFFRE au mauvais XID ##########"
joue xid-faux - "netbanc-dhcp xid 1" "dhcp" "netbanc-dhcp etat" "dhcp"
grep -aoE "DHCP_BANC_INJECTION[^\"]{0,140}|dhcp: pas d.OFFER[^\"]{0,20}|DHCP: bail obtenu[^\"]{0,40}" <<<"$SERIE" | tail -4
# La falsification ne touche QUE ce qui aurait ete accepte : elle transforme
# donc une acceptation en rejet, et la tentative doit echouer pour cette
# raison precise. Sans ce couple, le scenario serait indiscernable du cas
# normal, ou une OFFRE perimee produit deja `xid_ko=1`.
if ! contient "DHCP_BANC_INJECTION scenario=etat .*xid_restants=0 xid_fausses=1"; then
  verdict xid-faux 0 falsification-non-appliquee
elif ! contient "dhcp: pas d.OFFER"; then
  verdict xid-faux 0 le-xid-faux-n-a-fait-echouer-aucune-tentative
elif ! contient "DHCP_RX_TRACE .*pris=[1-9][0-9]* .*xid_ko=[1-9]"; then
  verdict xid-faux 0 paquet-non-rejete-sur-le-xid
elif [ "$(compte 'DHCP: bail obtenu')" -lt 1 ]; then
  verdict xid-faux 0 aucune-reprise-apres-le-rejet
else
  verdict xid-faux 1 recu-rejete-sur-xid-puis-bail-obtenu
fi

# --- 8. une OFFRE perimee arrive pendant la tentative suivante --------------
echo; echo "########## 8. OFFRE retardee, xid perime ##########"
# Le modele e1000 de QEMU retient les trames une seconde apres l'ecriture de
# RCTL puis les relache en bloc (voir B2) : l'OFFRE de la tentative de
# demarrage arrive donc PENDANT la tentative suivante, avec son ancien xid.
# La panne est fournie par l'emulateur, pas injectee -- mais elle est
# reproductible, et c'est exactement le cas 8.
joue offre-retardee - "netbanc-dhcp etat" "dhcp"
grep -aoE "DHCP_RX_TRACE[^\"]{0,300}" <<<"$SERIE" | tail -2
if contient "DHCP_RX_TRACE .*xid_ko=[1-9][0-9]* .*accepte=[1-9]" \
   || contient "DHCP_RX_TRACE .*xid_ko=[1-9] .*accepte=[1-9]"; then
  verdict offre-retardee 1 perimee-rejetee-et-bail-obtenu
else
  verdict offre-retardee 0 pas-de-perimee-observee
fi

echo
echo "===================== RESUME B4 ====================="
printf '%s\n' "${RESUME[@]}"
echo
if [ "$echecs" -eq 0 ]; then
  echo "DHCP_SCENARIOS verdict=OK"
  exit 0
fi
echo "DHCP_SCENARIOS verdict=ECHEC scenarios_en_echec=$echecs"
exit 1
