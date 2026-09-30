#!/usr/bin/env bash
# Banc « COM1 a debit borne » : un cycle QEMU dont le port serie se vide a
# DEBIT octets/s, comme un 16550 physique (115200 bauds ~ 11 520 o/s).
#
# QEMU ecrit COM1 dans un tube que `com1_lent_lecteur.py` vide lentement ; le
# tube plein, THRE reste a zero et le pilote attend comme sur le materiel.
# Sert a rendre mesurable ce que coute une impression interruptions masquees
# (BOUCHAUD_TLB_POINT_DE_SERVICE_V1) : a 2 400 o/s, le releve complet de
# l'ordonnanceur (~6 Ko) dure 2-3 s.
#
#   tools/ci/banc-com1-lent.sh NOYAU CPUS DEBIT SORTIE DISQUE [SECONDES]
#
# Resume sur une ligne : panique, echecs TLB, releves complets, pire duree
# masquee (SONDE-IRQ-DUREE) et phases d'endurance atteintes.
set -u
cd "$(dirname "$0")/../.."
k=${1:?noyau}; cpus=${2:?cpus}; debit=${3:?debit}; out=${4:?sortie}; disque=${5:?disque}; sec=${6:-300}
rm -rf "$out"; mkdir -p "$out"
cp "$disque" "$out/scenario.img"
mkfifo "$out/com1.in" "$out/com1.out"
python3 tools/ci/reliability/com1_lent_lecteur.py "$out/com1.out" "$out/serie.log" "$debit" &
lecteur=$!
exec 7<>"$out/com1.in"
timeout "$sec" qemu-system-x86_64 -drive "format=raw,file=$k" -m 4096 -smp "$cpus" -cpu max -display none \
  -chardev "pipe,id=s0,path=$out/com1" -serial chardev:s0 -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet \
  -drive "format=raw,file=$out/scenario.img"
rc=$?
exec 7>&-
wait "$lecteur"
propre=$(sed -E 's/\x1b\[[0-9;]*m//g' "$out/serie.log" | tr -d '\r')
echo "$(basename "$out") qemu_rc=$rc octets=$(stat -c %s "$out/serie.log") \
panique=$(grep -ac 'panicked at' <<<"$propre") \
tlb_echec=$(grep -ac 'TLB-SHOOTDOWN-ECHEC' <<<"$propre") \
releves_complets=$(grep -ac 'SONDE-IRQ-DUREE\] complet=1' <<<"$propre") \
pire_masque_ms=$(grep -aoE 'SONDE-IRQ-DUREE[^\\]*pire_ms=[0-9]+' <<<"$propre" | tail -1 | sed 's/.*=//') \
phases=$(grep -aoE 'ENDURANCE_[A-Z]+_OK' <<<"$propre" | sort -u | sed 's/ENDURANCE_//;s/_OK//' | paste -sd, -)"
