#!/usr/bin/env bash
#
# TOUT CE QUE LA BARRIERE GITHUB EXECUTE, ET QUI TOURNE ICI.
#
# # Pourquoi ce script existe
#
# Un lot a ete pousse avec « barriere locale verte » pour seule preuve, et la
# barriere GitHub l'a refuse : `dns-probe` CAS 3 mesurait qu'une attente
# bloquante de cinq secondes consommait cinq secondes de processeur. Le defaut
# etait reproductible ici depuis le debut -- simplement, le balayage que je
# faisais a la main ne contenait pas `verifie-dns.sh`.
#
# Une liste tenue de tete n'est pas une liste. Celle-ci est dans le depot, a
# cote des scenarios, et elle DIT ce qu'elle ne peut pas jouer plutot que de
# l'omettre en silence.
#
# # Ce qu'il ne fait pas
#
# Il ne remplace pas la barriere GitHub. Deux scenarios exigent l'image du
# navigateur (`BouchaudBrowserHost`), qui ne vit pas dans le depot : ils sont
# NOMMES et sautes, jamais comptes comme verts.
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:-target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin}

if [ ! -s "$BOOT" ]; then
    echo "image absente : $BOOT" >&2
    echo "construire d'abord : cargo bootimage" >&2
    exit 1
fi

echecs=0
sautes=0
resultat() { printf '  %-34s %s\n' "$1" "$2"; }

execute() {
    local nom=$1; shift
    local journal="target/barriere-locale/$nom.log"
    mkdir -p target/barriere-locale
    if timeout 900 "$@" > "$journal" 2>&1; then
        resultat "$nom" "ok"
    else
        resultat "$nom" "ECHEC  ($journal)"
        echecs=$((echecs + 1))
    fi
}

saute() {
    resultat "$1" "SAUTE  $2"
    sautes=$((sautes + 1))
}

echo "== garde-fous et suites hote =="
execute garde-fous       bash tools/ci/run_architecture_guards.sh
execute suites-hote      bash tools/ci/run_host_tests.sh
execute securite-source  python3 tools/security/verifie-security.py

echo
echo "== scenarios QEMU =="
for scenario in run_qemu_smoke run_system_health run_os_primitives \
                run_nvme_gpt run_nvme_parallele run_usb_arbre \
                run_usb_branchement run_usb_stockage run_trigkey_h10 \
                run_mm_ng6 run_native_ipc_probe run_security_runtime; do
    if [ -x "tools/ci/$scenario.sh" ] || [ -f "tools/ci/$scenario.sh" ]; then
        execute "$scenario" bash "tools/ci/$scenario.sh" "$BOOT"
    else
        saute "$scenario" "script absent"
    fi
done

# `verifie-dns.sh` construit son propre noyau et ne prend pas d'image en
# argument. C'est LUI qui a attrape le quantum impute a un coeur endormi : il
# compare deux horloges, ce qu'aucun autre scenario ne fait.
execute dns-udp-ring3 bash tools/net/verifie-dns.sh

echo
echo "== ce qui ne peut PAS tourner ici =="
if [ -f ladybird-browser.img ]; then
    execute run_platform_boot_note true
else
    saute run_platform_boot       "image scenario Ladybird absente (BouchaudBrowserHost)"
    saute run_ladybird_browser_host "image scenario Ladybird absente (BouchaudBrowserHost)"
fi

echo
if [ "$echecs" -ne 0 ]; then
    echo "BARRIERE_LOCALE_ECHEC echecs=$echecs sautes=$sautes" >&2
    exit 1
fi
echo "BARRIERE_LOCALE_OK echecs=0 sautes=$sautes"
echo "Un scenario SAUTE n'est pas un scenario vert : il reste a la barriere GitHub."
