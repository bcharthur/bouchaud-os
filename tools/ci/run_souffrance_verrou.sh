#!/usr/bin/env bash
#
# BOUCHAUD_SOUFFRANCE_PROPRIETAIRE_V1 (#283) : la ligne d'un noyau FIGE doit
# nommer le verrou, son coeur proprietaire et la ligne de code qui l'attend.
#
#   tools/ci/run_souffrance_verrou.sh BOOTIMAGE_TEMOIN BOOTIMAGE_NORMAL
#
# BOOTIMAGE_TEMOIN : noyau compile avec `banc-souffrance-verrou` (deux fils se
# prend un verrou sans le rendre, le contexte de demarrage l'attend : tous
# les coeurs figes).
# Exige, sur -smp 2 :
#   1. au moins une ligne `[DIAG-EN-SOUFFRANCE]` ;
#   2. exactement un coeur en attente, sur un verrou non nul, dont le
#      proprietaire est L'AUTRE coeur, et dont le site est
#      `banc_souffrance.rs:<ligne de l'appel lock()>`, et le site de prise
#      du detenteur `<banc_souffrance.rs:<ligne>` ;
#   3. le champ `ipi=[...]` present pour les quatre emplacements.
# BOOTIMAGE_NORMAL (temoin negatif) : le meme demarrage sans le drapeau
# n'emet AUCUNE ligne de souffrance en 60 s.
set -uo pipefail
cd "$(dirname "$0")/../.."
TEMOIN=${1:?usage: run_souffrance_verrou.sh BOOTIMAGE_TEMOIN BOOTIMAGE_NORMAL}
NORMAL=${2:?usage: run_souffrance_verrou.sh BOOTIMAGE_TEMOIN BOOTIMAGE_NORMAL}
OUT=${OUT:-souffrance-verrou}
mkdir -p "$OUT"

LIGNE=$(grep -n 'let (_ligne_attente, garde) = (line!(), VERROU.lock());' src/platform/pc/banc_souffrance.rs | cut -d: -f1)
LIGNE_DETENTEUR=$(grep -n 'let (_ligne_detenteur, mut garde) = (line!(), VERROU.lock());' src/platform/pc/banc_souffrance.rs | cut -d: -f1)
[ -n "$LIGNE" ] && [ -n "$LIGNE_DETENTEUR" ] || { echo "SOUFFRANCE_VERROU_ECHEC raison=ligne_du_temoin_introuvable"; exit 1; }

joue() {  # joue IMAGE JOURNAL SECONDES
  timeout "$3" qemu-system-x86_64 \
    -drive format=raw,file="$1",snapshot=on \
    -m 2048 -smp 2 -display none -no-reboot \
    -serial file:"$2" >/dev/null 2>&1
  return 0
}

echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }

joue "$TEMOIN" "$OUT/temoin.log" 150
sed -E 's/\x1b\[[0-9;]*m//g' "$OUT/temoin.log" > "$OUT/temoin.txt"
grep -aF "BOUCHAUD_BANC_SOUFFRANCE armement" "$OUT/temoin.txt" >/dev/null \
  && ok "temoin arme" || echec "le temoin n'a pas ete arme"
python3 - "$OUT/temoin.txt" "$LIGNE" "$LIGNE_DETENTEUR" <<'PY' || echecs=$((echecs + 1))
import re, sys
texte = open(sys.argv[1], errors="replace").read()
ligne = sys.argv[2]
ligne_detenteur = sys.argv[3]
lignes = [l for l in texte.splitlines() if "[DIAG-EN-SOUFFRANCE]" in l]
if not lignes:
    print("  ECHEC   aucune ligne [DIAG-EN-SOUFFRANCE]")
    sys.exit(1)
derniere = lignes[-1]
print("  ligne : " + derniere[derniere.index("[DIAG-EN-SOUFFRANCE]"):][:400])
m = re.search(r"spin=\[([^\]]*)\]", derniere)
i = re.search(r"ipi=\[([^\]]*)\]", derniere)
if not m or not i:
    print("  ECHEC   champs spin=/ipi= absents")
    sys.exit(1)
entrees = m.group(1).split()
attentes = []
for cpu, e in enumerate(entrees):
    v = re.fullmatch(r"(0x[0-9a-f]+)/(\d+)/p(-?\d+)@([^:]+):(\d+)<([^:]+):(\d+)", e)
    if not v:
        print(f"  ECHEC   entree spin illisible : {e}")
        sys.exit(1)
    if int(v.group(1), 16) != 0:
        attentes.append((cpu, v))
ipis = i.group(1).split()
if len(entrees) != 4 or len(ipis) != 4 or not all(re.fullmatch(r"\d+/0x[0-9a-f]+/u\d+", x) for x in ipis):
    print(f"  ECHEC   quatre emplacements attendus (spin={len(entrees)}, ipi={len(ipis)})")
    sys.exit(1)
if len(attentes) != 1:
    print(f"  ECHEC   un coeur en attente attendu, vu {len(attentes)}")
    sys.exit(1)
cpu, v = attentes[0]
proprio, fichier, no = int(v.group(3)), v.group(4), v.group(5)
fautes = []
if proprio not in (0, 1) or proprio == cpu:
    fautes.append(f"proprietaire {proprio} (attendu : l'autre coeur que {cpu})")
if fichier != "banc_souffrance.rs" or no != ligne:
    fautes.append(f"site {fichier}:{no} (attendu banc_souffrance.rs:{ligne})")
if v.group(6) != "banc_souffrance.rs" or v.group(7) != ligne_detenteur:
    fautes.append(f"site du detenteur {v.group(6)}:{v.group(7)} (attendu banc_souffrance.rs:{ligne_detenteur})")
if fautes:
    for f in fautes:
        print("  ECHEC   " + f)
    sys.exit(1)
print(f"  ok      coeur {cpu} attend le verrou {v.group(1)} tenu par le coeur {proprio}, site {fichier}:{no}, pris en {v.group(6)}:{v.group(7)}")
PY

joue "$NORMAL" "$OUT/normal.log" 60
if grep -aqF "[DIAG-EN-SOUFFRANCE]" "$OUT/normal.log"; then
  echec "temoin negatif : une ligne de souffrance sur le noyau normal"
else
  ok "temoin negatif : aucune ligne de souffrance sur le noyau normal (60 s)"
fi
grep -aq "SCHED-RESUME" "$OUT/normal.log" && ok "noyau normal vivant (resume periodique)" \
  || echec "noyau normal : aucun resume periodique en 60 s"

if (( echecs > 0 )); then
  echo "SOUFFRANCE_VERROU_ECHEC echecs=$echecs"
  exit 1
fi
echo SOUFFRANCE_VERROU_OK
