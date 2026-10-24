#!/usr/bin/env bash
# Qualification dédiée P13 : tests identiques, criteres inchanges.
# Usage : tools/ci/run_p13_dedicated_kvm.sh BOOTIMAGE [baseline|smp8]
set -Eeuo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_p13_dedicated_kvm.sh BOOTIMAGE [baseline|smp8]}
MODE=${2:-baseline}
[[ "$MODE" = baseline || "$MODE" = smp8 ]] || { echo 'MODE_INCONNU' >&2; exit 2; }
[[ -f "$BOOT" ]] || { echo "IMAGE_ABSENTE $BOOT" >&2; exit 2; }
[[ -n "$(git rev-parse HEAD)" ]] || exit 2
OUT=${P13_QUALIFICATION_OUT:-"$PWD/p13-dedicated-${MODE}"}
[[ ! -e "$OUT" ]] || { echo "Dossier sortie existe deja $OUT (conserver les preuves)" >&2; exit 2; }
mkdir -p "$OUT"
# La garde est AVANT les tests et enregistre le SHA et le type d'hote.
MIN_CPUS=4
[[ "$MODE" != smp8 ]] || MIN_CPUS=8
python3 tools/ci/p13_kvm_host_preflight.py --min-cpus "$MIN_CPUS" --output "$OUT/host.json"
sha256sum "$BOOT" > "$OUT/bootimage.sha256"
git rev-parse HEAD > "$OUT/git-head.txt"
cp /proc/stat "$OUT/host-proc-stat-avant.txt"
cat /proc/loadavg > "$OUT/host-loadavg-avant.txt"
result=0
run_attr() {
  local smp=$1 dest="$OUT/ordonnanceur-attribution-kvm-smp$1"
  echo "P13_DEDICATED_ATTRIBUTION smp=$smp"
  ORDO_SORTIE="$dest" BO_QEMU_KVM=1 tools/ci/run_ordonnanceur_attribution.sh "$BOOT" 2 "$smp" > "$OUT/attribution-smp${smp}.stdout.log" 2>&1 || result=1
  # La CI peut produire des logs et retourner 0 pour un témoin d'instrument :
  # la qualification explicite est toujours obligatoire.
  if [[ $smp -eq 8 ]]; then
    python3 tools/ci/p13_smp8_gate.py "$dest/serie.log" \
      --stdout "$OUT/attribution-smp${smp}.stdout.log" > "$OUT/attribution-smp8.strict.log" 2>&1 || result=1
  else
    python3 tools/ci/qualifie_p13.py attribution "$dest/serie.log" --smp "$smp" --passes 2 --mode strict > "$OUT/attribution-smp${smp}.strict.log" 2>&1 || result=1
  fi
}
run_ab() {
  local smp=$1 dest="$OUT/ab-reveil-precis-kvm-smp$1"
  echo "P13_DEDICATED_AB smp=$smp"
  AB_SORTIE="$dest" BO_QEMU_KVM=1 tools/ci/run_ab_reveil_precis.sh "$BOOT" 4 "$smp" > "$OUT/ab-smp${smp}.stdout.log" 2>&1 || result=1
  python3 tools/ci/qualifie_p13.py ab "$dest/serie.log" --smp "$smp" --passes 4 --mode strict > "$OUT/ab-smp${smp}.strict.log" 2>&1 || result=1
}
if [[ "$MODE" = baseline ]]; then
  for smp in 1 2 4; do run_attr "$smp"; done
  for smp in 1 4; do run_ab "$smp"; done
else
  run_attr 8
fi
cp /proc/stat "$OUT/host-proc-stat-apres.txt"
cat /proc/loadavg > "$OUT/host-loadavg-apres.txt"
python3 tools/ci/p13_evidence_report.py "$OUT" --sha "$(cat "$OUT/git-head.txt")" \
    --json "$OUT/diagnostic.json" --markdown "$OUT/diagnostic.md" || result=1
printf 'P13_DEDICATED_RESULT mode=%s sha=%s verdict=%s\n' "$MODE" "$(cat "$OUT/git-head.txt")" \
    "$(if [[ $result -eq 0 ]]; then echo QUALIFIED; else echo NOT_ESTABLISHED; fi)"
exit "$result"
