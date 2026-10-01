#!/usr/bin/env bash
# usage: campagne2.sh IMAGE ETIQUETTE [configs]  -- comme campagne.sh, avec la sonde hote
cd /home/user/bouchaud-os
S=/tmp/claude-0/-home-user-bouchaud-os/2ee20b70-3eb2-5e27-85c8-1e5956bc273b/scratchpad/sngb
IMG=$1; TAG=$2; CONFIGS=${3:-"4 8"}; BOOTS=${BOOTS:-3}
for c in $CONFIGS; do for i in $(seq 1 $BOOTS); do
  out=$S/$TAG/smp$c-$i; rm -rf $out; mkdir -p $S/$TAG
  python3 $S/hote_sonde.py "$out/" $S/$TAG/smp$c-$i.hote &
  SP=$!
  PYTHONPATH=tools/ci/reliability timeout 330 python3 tools/ci/reliability/soak.py $IMG --cpus $c --duration-seconds 260 --cycle-seconds 250 --memory-mb 2048 --out-dir $out --disque $S/sng.img --require-fichier $S/marq.txt > $out.txt 2>&1
  wait $SP
  L=$(ls $out/cycle-0001/*.log 2>/dev/null | head -1)
  sed -E 's/\x1b\[[0-9;]*m//g' "$L" > $S/$TAG/smp$c-$i.log 2>/dev/null
  echo "SMP$c #$i $(grep -aoE 'SNG v=1 sec=FIN.*' $S/$TAG/smp$c-$i.log | head -1) panique=$(grep -ac 'panicked at' $S/$TAG/smp$c-$i.log) gels=$(grep -aoE 'SONDE-GEL-RESUME[^\[]*' $S/$TAG/smp$c-$i.log | tail -1)"
done; done
echo CAMPAGNE_FIN
