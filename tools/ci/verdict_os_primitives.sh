#!/usr/bin/env bash
# BOUCHAUD_VERDICT_OS_PRIMITIVES_V1 : le verdict d'os-primitives, lu sur un
# journal serie seul -- separe du demarrage QEMU pour etre rejoue sur des
# journaux canoniques (tools/ci/reliability/test_verdict_os_primitives.py).
#
#     verdict_os_primitives.sh JOURNAL
#
# Sortie 0 et `OS_PRIMITIVES_VERDICT_OK` seulement si : chaque marqueur de
# sonde est la (une trace `+ echo X` du shell n'en tient pas lieu), la ligne de
# detail de faute nomme l'ud2 du fils `piege`, l'autorun finit en `statut=0`,
# aucune panique, aucune commande ATA en echec. Aucun tube vers un lecteur
# qui s'arrete tot : sous `pipefail`, son producteur mourrait de SIGPIPE et le
# verdict serait faux (run 39503091649).
set -euo pipefail
LOG=${1:?usage: verdict_os_primitives.sh JOURNAL}
[ -s "$LOG" ] || { echo "VERDICT_ECHEC journal absent ou vide : $LOG" >&2; exit 1; }

# BOUCHAUD_TSC_SOURCE_V1 : l'horloge d'abord -- c'est quand un banc echoue
# qu'on en a besoin.
awk '{ gsub(/\x1b\[[0-9;]*m/, ""); gsub(/\r/, "") } match($0, /BOUCHAUD_TSC_(EARLY_CALIBRATION_OK|CONTROLE) .*/) { print substr($0, RSTART, RLENGTH) }' "$LOG"

echecs=0
for marker in VERROUS_POSIX_OK EXEC_FD_OK WAL_PROBE_OK DISQUE_PROBE_OK NOM_LONG_OK SENDFILE_OK RENOMMAGE_OK FAUTE_NONCANONIQUE_OK 'FAUTE_MULTIFIL_OK essais=16' 'NOEUDS_CHURN_OK cycles=8000' SIGCHLD_MULTIFIL_OK SIGMASQUE_FIL_OK COMPTA_STRESS_OK MTIME_STABLE_OK PERSIST_COURSE_OK 'EOF_FERMETURE_OK cas=6 tours=2' 'TUBE_ATOMICITE_OK tailles=3 ecrivains=4 tours=300 modes=2' 'STATUT_SORTIE_OK cas=11' \
              'SESSION_PERE_SORT fils=4' SESSION_INVITE_REVENUE PRIMITIVES_FIN; do
  # La ligne du marqueur, pas la trace de la commande qui l'imprime.
  ligne=$(awk -v m="$marker" 'index($0, m) && !index($0, "+ echo " m) { print; exit }' "$LOG")
  if [ -n "$ligne" ]; then
    printf '%s\n' "$ligne"
  else
    echo "MARQUEUR_ABSENT $marker" >&2
    echecs=$((echecs + 1))
  fi
done

# BOUCHAUD_FAUTE_DETAIL_V1 : le fils `piege` de faute-noncanonique-probe meurt
# sur ud2 (0f 0b) ; sa ligne de detail doit nommer CES octets. Lue sur le
# fichier, sans tube.
if ! grep -aqE 'PROCESS_FAULT_DETAIL pid=[0-9]+ tid=[0-9]+ cpu=[0-9]+ fs_base=0x[0-9a-f]+ fs_base_range=0x[0-9a-f]+ insn=0f0b' "$LOG"; then
  echo "FAUTE_DETAIL_ECHEC aucune ligne de detail ne nomme l'ud2 du fils piege" >&2
  awk '/PROCESS_FAULT_DETAIL/ && n++ < 8' "$LOG" >&2
  echecs=$((echecs + 1))
fi

# Un noyau qui termine l'autorun en erreur echoue, marqueurs presents ou non.
statut=$(awk '{ gsub(/\x1b\[[0-9;]*m/, "") } match($0, /=== AUTORUN FIN === statut=[0-9]+/) { v = substr($0, RSTART + 27, RLENGTH - 27) } END { print (v != "" ? v : "absent") }' "$LOG")
if [ "$statut" != 0 ]; then
  echo "AUTORUN_ECHEC statut=$statut" >&2
  echecs=$((echecs + 1))
fi

# BOUCHAUD_COMPTA_STRESS_V1 / BOUCHAUD_GS_NOYAU_EN_IRQ_V1 : releves, sans verdict.
awk 'match($0, /compta_relues=[0-9]+/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "compta_relues=absent") }' "$LOG"
awk 'match($0, /replis_apic=[0-9]+( ticks_ms=[0-9]+ mono_ms=[0-9]+)?/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "replis_apic=absent") }' "$LOG"
awk 'match($0, /tranches_gonflees=[0-9]+ pire_gonflement_us=[0-9]+ chevauchements=[0-9]+ pire_chevauchement_us=[0-9]+/) { v = substr($0, RSTART, RLENGTH) } END { print (v != "" ? v : "invariants_compta=absents") }' "$LOG"
# BOUCHAUD_COMPTA_INVARIANTS_TRANCHE_V1 : deux invariants d'imputation, nuls
# par construction (repli avant armement, horloge linearisee). Non nuls, c'est
# un defaut du noyau, pas une mesure.
if awk 'match($0, /tranches_gonflees=[0-9]+/) { g = substr($0, RSTART + 18, RLENGTH - 18) + 0 } match($0, /chevauchements=[0-9]+/) { c = substr($0, RSTART + 15, RLENGTH - 15) + 0 } END { exit !(g > 0 || c > 0) }' "$LOG"; then
  echo "INVARIANT_COMPTA_ECHEC tranche gonflee ou armement avant le dernier repli" >&2
  echecs=$((echecs + 1))
fi

if grep -aqE 'KERNEL PANIC|panicked at' "$LOG"; then
  echo "panique noyau" >&2
  echecs=$((echecs + 1))
fi
if grep -aqE "ata: (lecture|ecriture) " "$LOG"; then
  echo "Le pilote ATA a signale au moins une commande en echec" >&2
  grep -aE "ata: (lecture|ecriture) " "$LOG" >&2 || true
  echecs=$((echecs + 1))
fi

if [ "$echecs" -ne 0 ]; then
  echo "OS_PRIMITIVES_VERDICT_ECHEC echecs=$echecs" >&2
  exit 1
fi
echo OS_PRIMITIVES_VERDICT_OK
