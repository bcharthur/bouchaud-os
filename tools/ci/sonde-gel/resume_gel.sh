#!/usr/bin/env bash
# resume_gel.sh DOSSIER : une ligne par demarrage + agregat
d=$1; n=0; gels=0; pire=0; bornes=0; panic=0; att=0
for f in $d/smp*-*.log; do
  n=$((n+1))
  p=$(grep -aoE 'SONDE-GEL-RESUME\][^\[]*' $f | tail -1 | grep -oE 'pire_ms=[0-9]+' | cut -d= -f2); p=${p:-0}
  b=$(grep -aoE 'serie_bornes=[0-9]+' $f | tail -1 | cut -d= -f2); b=${b:-0}
  a=$(grep -aoE 'attente_lecteurs_pire_ms=[0-9]+' $f | tail -1 | cut -d= -f2); a=${a:-0}
  k=$(grep -c panicked $f)
  fin=$(grep -aoE 'SNG v=1 sec=FIN[^\[]*' $f | grep -oE 'echecs=[0-9]+ perdus=[0-9]+ ressuscites=[0-9]+')
  [ $p -ge 200 ] && gels=$((gels+1)); [ $p -gt $pire ] && pire=$p; [ $a -gt $att ] && att=$a
  bornes=$((bornes+b)); panic=$((panic+k))
  echo "$(basename $f) gel_pire_ms=$p attente_lecteurs_ms=$a serie_bornes=$b panic=$k $fin"
done
echo "TOTAL $d demarrages=$n gels>=200ms=$gels pire_gel_ms=$pire pire_attente_lecteurs_ms=$att serie_bornes=$bornes panics=$panic"
