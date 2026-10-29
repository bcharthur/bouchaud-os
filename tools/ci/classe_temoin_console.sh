#!/usr/bin/env bash
# P13_TEMOIN_CONSOLE_FIL_SEUL_V1 -- classement du bras temoin console (cible
# fil, sans secours), source par run_serie_entrelacement.sh et teste a part
# (tools/ci/reliability/test_classe_temoin_console.py).
#
#   classe_temoin_console BILAN AIDES  ->  imprime OK | NON_QUALIFIANT | ECHEC
#
# OK : le repli est revenu (le defaut du temoin). Sans repli : NON_QUALIFIANT
# si d'autres que le fil ont emis pendant la suspension (AIDES > 0 : la
# console a progresse sans lui, la precondition n'a pas tenu), ECHEC sinon.
# AIDES vide (non mesure) : ECHEC -- jamais de verdict sans la mesure.
classe_temoin_console() {
  local bilan=$1 aides=$2
  if [ -z "$aides" ] || ! [[ "$aides" =~ ^[0-9]+$ ]]; then
    echo ECHEC
  elif printf '%s\n' "$bilan" | grep -qE 'BOUCHAUD_CONSOLE_BILAN active=0 repli_securite=1 '; then
    echo OK
  elif [ "$aides" -gt 0 ]; then
    echo NON_QUALIFIANT
  else
    echo ECHEC
  fi
}
