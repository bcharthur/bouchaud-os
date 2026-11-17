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
#   3. le champ `ipi=[...]` present pour les quatre emplacements ;
#   4. le champ `nmi=[...]` : le coeur detenteur a repondu a la NMI, en mode
#      noyau, avec un RIP dans la fonction `detenteur` (BOUCHAUD_RELEVE_NMI_V1) ;
#   5. vu de QEMU (releve_cpus_qemu.py, six echantillons a une seconde
#      d'intervalle), un coeur est en mode noyau dans `detenteur` sur au moins
#      trois d'entre eux (BOUCHAUD_RELEVE_CPUS_QEMU_V1). Le detenteur boucle
#      IF=1 et sert ses IRQ : deux echantillons seulement, tous deux exiges
#      dans la boucle, rendaient le banc rouge au hasard (integration
#      44089209830 : coeur 1 en `detenteur`, puis dans une IRQ).
# BOOTIMAGE_NORMAL (temoin negatif) : le meme demarrage sans le drapeau
# n'emet AUCUNE ligne de souffrance en 60 s.
set -uo pipefail
cd "$(dirname "$0")/../.."
TEMOIN=${1:?usage: run_souffrance_verrou.sh BOOTIMAGE_TEMOIN BOOTIMAGE_NORMAL}
NORMAL=${2:?usage: run_souffrance_verrou.sh BOOTIMAGE_TEMOIN BOOTIMAGE_NORMAL}
OUT=${OUT:-souffrance-verrou}
mkdir -p "$OUT"

# Les deux images doivent etre ce qu'on croit qu'elles sont. Le 14/11, une
# image « normale » construite par `cargo bootimage` juste apres le temoin
# contenait encore le temoin : son « blocage au demarrage » a ete pris, a tort,
# pour une regression. On le verifie donc sur le binaire lui-meme.
grep -aqF 'BOUCHAUD_BANC_SOUFFRANCE armement' "$TEMOIN" \
  || { echo "SOUFFRANCE_VERROU_ECHEC raison=image_temoin_sans_temoin"; exit 1; }
if grep -aqF 'BOUCHAUD_BANC_SOUFFRANCE armement' "$NORMAL"; then
  echo "SOUFFRANCE_VERROU_ECHEC raison=image_normale_contient_le_temoin"; exit 1
fi

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
echecs_qemu=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }

# Le temoin tourne avec un moniteur : une fois la ligne de souffrance sortie,
# releve_cpus_qemu.py lit ce que QEMU voit de chaque vCPU
# (BOUCHAUD_RELEVE_CPUS_QEMU_V1), comme le font les bancs Ladybird sur gel.
MONITEUR="$OUT/moniteur.sock"
# Rien d'un run precedent : releve_cpus_qemu.py AJOUTE a son fichier, et la
# boucle d'attente ci-dessous lirait la ligne de souffrance de l'ancien
# journal avant que QEMU ne le recree (releve pris dans le BIOS, temoin
# « non arme »).
rm -f "$MONITEUR" "$OUT/temoin.log.cpus" "$OUT/temoin.txt.cpus"
: > "$OUT/temoin.log"
qemu-system-x86_64 \
  -drive format=raw,file="$TEMOIN",snapshot=on \
  -m 2048 -smp 2 -display none -no-reboot \
  -monitor "unix:$MONITEUR,server,nowait" \
  -serial file:"$OUT/temoin.log" >/dev/null 2>&1 &
QPID=$!
for _ in $(seq 1 150); do
  kill -0 "$QPID" 2>/dev/null || break
  if grep -aqF '[DIAG-EN-SOUFFRANCE]' "$OUT/temoin.log" 2>/dev/null; then break; fi
  sleep 1
done
RELEVE_ECHANTILLONS=6 python3 tools/ci/releve_cpus_qemu.py "$MONITEUR" "$OUT/temoin.log.cpus" >/dev/null || true
kill -TERM "$QPID" 2>/dev/null || true
sleep 1
kill -KILL "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true
sed -E 's/\x1b\[[0-9;]*m//g' "$OUT/temoin.log" > "$OUT/temoin.txt"
# rapport_gel.py lit les registres a cote du journal qu'on lui donne.
cp "$OUT/temoin.log.cpus" "$OUT/temoin.txt.cpus" 2>/dev/null || true
# Vu de QEMU, le detenteur fige doit etre vu en mode noyau dans `detenteur`
# sur au moins trois des six echantillons (les autres peuvent tomber dans une
# IRQ qu'il sert, IF=1).
python3 - "$OUT/temoin.log.cpus" "$(sed -nE 's/.*BOUCHAUD_BANC_SOUFFRANCE detenteur cpu=[0-9]+ verrou=0x[0-9a-f]+ fonction=(0x[0-9a-f]+).*/\1/p' "$OUT/temoin.txt" | sed -n 1p)" <<'PY' || echecs_qemu=1
import re, sys
try:
    lignes = open(sys.argv[1]).read()
except OSError:
    print("  ECHEC   aucun registre QEMU releve"); sys.exit(1)
fonction = int(sys.argv[2] or "0", 16)
par = {}
for m in re.finditer(r"ech=(\d) cpu=(\d+) rip=(0x[0-9a-f]+) cpl=(\d)", lignes):
    par.setdefault(int(m.group(2)), {})[int(m.group(1))] = (int(m.group(3), 16), int(m.group(4)))
for cpu, e in sorted(par.items()):
    dedans = sorted(n for n, (rip, cpl) in e.items() if cpl == 0 and fonction <= rip < fonction + 0x400)
    if len(dedans) >= 3:
        rips = sorted({e[n][0] for n in dedans})
        print(f"  ok      QEMU : le coeur {cpu} est dans detenteur en mode noyau sur {len(dedans)}/{len(e)} "
              f"echantillons (rip {', '.join(hex(r) for r in rips)})")
        sys.exit(0)
print(f"  ECHEC   QEMU : aucun coeur dans detenteur ({fonction:#x}) sur au moins 3 echantillons : {par}")
sys.exit(1)
PY
grep -aF "BOUCHAUD_BANC_SOUFFRANCE armement" "$OUT/temoin.txt" >/dev/null \
  && ok "temoin arme" || echec "le temoin n'a pas ete arme"
FONCTION=$(sed -nE 's/.*BOUCHAUD_BANC_SOUFFRANCE detenteur cpu=[0-9]+ verrou=0x[0-9a-f]+ fonction=(0x[0-9a-f]+).*/\1/p' "$OUT/temoin.txt" | sed -n 1p)
python3 - "$OUT/temoin.txt" "$LIGNE" "$LIGNE_DETENTEUR" "${FONCTION:-0x0}" <<'PY' || echecs=$((echecs + 1))
import re, sys
texte = open(sys.argv[1], errors="replace").read()
ligne = sys.argv[2]
ligne_detenteur = sys.argv[3]
fonction = int(sys.argv[4], 16)
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
# BOUCHAUD_RELEVE_NMI_V1 : le detenteur fige doit avoir repondu a la NMI, en
# mode noyau, avec un RIP DANS sa boucle (`detenteur`, quelques centaines
# d'octets au plus) -- pas un RIP perime du dernier IPI.
n = re.search(r"nmi=\[([^\]]*)\]", derniere)
if not n:
    print("  ECHEC   champ nmi= absent")
    sys.exit(1)
releves = dict(x.split(":", 1) for x in n.group(1).split())
r = releves.get(f"c{proprio}", "-")
m = re.match(r"(0x[0-9a-f]+)/([ku])/if([01])", r)
if not m:
    print(f"  ECHEC   le coeur detenteur {proprio} n'a pas repondu a la NMI ({r})")
    sys.exit(1)
rip = int(m.group(1), 16)
if fonction == 0 or not (fonction <= rip < fonction + 0x400) or m.group(2) != "k":
    print(f"  ECHEC   NMI du detenteur : rip={rip:#x} mode={m.group(2)}, attendu dans detenteur@{fonction:#x} en mode noyau")
    sys.exit(1)
print(f"  ok      NMI : le coeur {proprio} s'execute en {rip:#x}, dans detenteur ({fonction:#x}), mode noyau, if{m.group(3)}")
PY

joue "$NORMAL" "$OUT/normal.log" 60
if grep -aqF "[DIAG-EN-SOUFFRANCE]" "$OUT/normal.log"; then
  echec "temoin negatif : une ligne de souffrance sur le noyau normal"
else
  ok "temoin negatif : aucune ligne de souffrance sur le noyau normal (60 s)"
fi
grep -aq "SCHED-RESUME" "$OUT/normal.log" && ok "noyau normal vivant (resume periodique)" \
  || echec "noyau normal : aucun resume periodique en 60 s"

echecs=$((echecs + echecs_qemu))
if (( echecs > 0 )); then
  echo "SOUFFRANCE_VERROU_ECHEC echecs=$echecs"
  exit 1
fi
echo SOUFFRANCE_VERROU_OK
