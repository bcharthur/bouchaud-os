#!/usr/bin/env bash
# A/B : VARIANTE EXPERIMENTALE D'IPI AU REVEIL D'UNE INTERACTIVE
# (BOUCHAUD_REVEIL_IPI_INTERACTIF_EXPERIMENTAL), CONTRE LA POLITIQUE LIVREE.
#
#   tools/ci/run_ab_reveil_ipi.sh BOOTIMAGE [TOURS] [SMP]
#
# Un seul demarrage, TOURS tours (4 par defaut) ; chaque tour passe
# `ordonnanceur-probe 8 1` sous `ordo-variante defaut` PUIS sous
# `ordo-variante ipi-interactive` (une seule tentative : aucune remesure ne
# favorise un bras). Le demarrage finit sous `defaut`.
#
# CRITERES DECLARES AVANT LA MESURE (une politique par defaut ne change
# qu'avec un A/B KVM CONVAINCANT sur les cinq) :
#   latence     mediane des p99 interactifs PROPRES : variante <= 0,8 x defaut,
#               et mediane des pires interactifs propres : variante <= defaut ;
#   debit       mediane des tours de calcul pendant la phase interactive :
#               variante >= 0,95 x defaut ;
#   equite      mediane des tours pendant la phase NORMALE (ou la variante ne
#               doit rien changer) : ecart <= 5 % ;
#   surete      `ipi_variante` reste a zero sous `defaut`, avance sous la
#               variante ; aucune panique ;
#   robustesse  chaque passe rend un verdict explicite ; aucun QUALIFIE_ECHEC
#               sous la variante.
#
# Le banc ECHOUE (sortie 1) sur un defaut d'instrument ou de surete : passe
# sans verdict, IPI de variante sous `defaut`, variante jamais exercee,
# panique. Le resultat de l'experience -- CONVAINCANT ou NON_CONVAINCANT --
# est une MESURE, imprimee avec ses chiffres : il ne change rien seul.
#
# BO_QEMU_KVM=1 : sous KVM (le seul A/B qui compte pour la decision).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ab_reveil_ipi.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-4}
SMP=${3:-8}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${AB_SORTIE:-$PWD/ab-reveil-ipi-$NOM-smp$SMP}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  echo "echo AB_REVEIL_DEBUT"
  for t in $(seq "$TOURS"); do
    for v in defaut ipi-interactive; do
      echo "ordo-variante $v"
      echo "echo AB_BRAS tour=$t variante=$v"
      echo "/bin/ordonnanceur-probe 8 1"
      echo "ordo-variante"
    done
  done
  echo "ordo-variante defaut"
  echo "echo AB_REVEIL_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ab.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ab.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 150))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'AB_REVEIL_FIN|KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

python3 - "$LOG" "$TOURS" "$SMP" "$NOM" <<'PY'
import re, statistics, sys
log, tours, smp, nom = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
texte = open(log, encoding="utf-8", errors="replace").read()
texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l) for l in texte.splitlines()]
bras, courant = [], None
for l in lignes:
    # La sortie de `echo`, pas sa trace (`+ echo AB_BRAS ...`).
    m = re.match(r"AB_BRAS tour=(\d+) variante=(\S+)", l)
    if m:
        courant = {"tour": int(m.group(1)), "variante": m.group(2), "lignes": []}
        bras.append(courant)
        continue
    if courant is not None:
        courant["lignes"].append(l)
QUANT = re.compile(r"^\s+(sous charge \((?:normal|interactif)\)|au repos)\s+median\s+(\d+) us\s+p95\s+(\d+) us"
                   r"\s+p99\s+(\d+) us\s+pire\s+(\d+) us\s+\[propres")
fautes, mesures = [], {"defaut": [], "ipi-interactive": []}
for b in bras:
    r = {"verdict": None, "ipi_variante": None}
    tours_vus = []
    for l in b["lignes"]:
        q = QUANT.match(l)
        if q:
            r[q.group(1)] = tuple(int(x) for x in q.groups()[1:])
        m = re.match(r"^\s+(\d+) tours de calcul", l)
        if m:
            tours_vus.append(int(m.group(1)))
        m = re.match(r"^VERDICT_SONDE : (\S+)", l)
        if m:
            r["verdict"] = m.group(1)
        m = re.search(r"ORDO_VARIANTE reveil=(\S+) ipi_variante=(\d+)", l)
        if m and r["verdict"] is not None and r["ipi_variante"] is None:
            r["ipi_variante"] = int(m.group(2))
            r["reveil_affiche"] = m.group(1)
        m = re.match(r"^\s+ventilation sous charge \(interactif\) : .* election=(\d+):(\d+):(\d+) election_externe=(\d+):(\d+):(\d+)", l)
        if m:
            r["election"] = tuple(int(x) for x in m.groups())
    if len(tours_vus) >= 2:
        r["tours_normal"], r["tours_interactif"] = tours_vus[0], tours_vus[1]
    b["r"] = r
    if r["verdict"] is None or "sous charge (interactif)" not in r or "tours_interactif" not in r:
        fautes.append(f"tour {b['tour']} {b['variante']} : passe sans verdict ou sans mesure")
        continue
    mesures[b["variante"]].append(r)

# Surete : les compteurs cumules d'IPI de variante.
cumul = [(b["variante"], b["r"].get("ipi_variante")) for b in bras]
precedent = 0
for v, c in cumul:
    if c is None:
        continue
    delta = c - precedent
    precedent = c
    if v == "defaut" and delta != 0:
        fautes.append(f"surete : {delta} IPI de variante sous defaut")
if precedent == 0:
    fautes.append("surete : la variante n'a jamais envoye d'IPI (pas exercee)")
if re.search(r"KERNEL PANIC|panicked at", texte):
    fautes.append("surete : panique")
if "AB_REVEIL_FIN" not in texte:
    fautes.append("le scenario n'est pas alle au bout")
if len(mesures["defaut"]) != tours or len(mesures["ipi-interactive"]) != tours:
    fautes.append(f"bras incomplets : defaut {len(mesures['defaut'])}/{tours}, variante {len(mesures['ipi-interactive'])}/{tours}")

print(f"  A/B reveil IPI, {nom} SMP{smp}, {tours} tours (valeurs PROPRES de la phase interactive)")
print(f"  {'tour':<5}{'bras':<17}{'verdict':<16}{'p99':>7}{'pire':>8}{'elect>1ms':>10}{'elect_us':>10}{'tours_int':>11}{'tours_norm':>11}")
for b in bras:
    r = b["r"]
    i = r.get("sous charge (interactif)", (0, 0, 0, 0))
    e = r.get("election", (0, 0, 0, 0, 0, 0))
    print(f"  {b['tour']:<5}{b['variante']:<17}{str(r['verdict']):<16}{i[2]:>7}{i[3]:>8}{e[0]:>10}{e[1]:>10}"
          f"{r.get('tours_interactif', 0):>11}{r.get('tours_normal', 0):>11}")

def med(v, cle, idx=None):
    vals = [(r[cle][idx] if idx is not None else r[cle]) for r in mesures[v] if cle in r]
    return statistics.median(vals) if vals else None

resultat = {}
if not fautes:
    d, x = "defaut", "ipi-interactive"
    p99 = (med(d, "sous charge (interactif)", 2), med(x, "sous charge (interactif)", 2))
    pire = (med(d, "sous charge (interactif)", 3), med(x, "sous charge (interactif)", 3))
    debit = (med(d, "tours_interactif"), med(x, "tours_interactif"))
    equite = (med(d, "tours_normal"), med(x, "tours_normal"))
    echecs_variante = sum(1 for r in mesures[x] if r["verdict"] == "QUALIFIE_ECHEC")
    criteres = [
        ("latence", p99[1] <= 0.8 * p99[0] and pire[1] <= pire[0],
         f"p99 median {p99[0]} -> {p99[1]} us (seuil {0.8 * p99[0]:.0f}), pire median {pire[0]} -> {pire[1]} us"),
        ("debit", debit[1] >= 0.95 * debit[0], f"tours (phase interactive) {debit[0]} -> {debit[1]}"),
        ("equite", abs(equite[1] - equite[0]) <= 0.05 * equite[0], f"tours (phase normale) {equite[0]} -> {equite[1]}"),
        ("surete", True, f"ipi_variante={precedent}, 0 sous defaut, aucune panique"),
        ("robustesse", echecs_variante == 0, f"QUALIFIE_ECHEC sous la variante : {echecs_variante}"),
    ]
    for nom_c, ok, detail in criteres:
        print(f"  {'ok     ' if ok else 'NON    '} {nom_c:<11} {detail}")
    convaincant = all(ok for _, ok, _ in criteres)
    print(("AB_REVEIL_IPI : CONVAINCANT" if convaincant else "AB_REVEIL_IPI : NON_CONVAINCANT")
          + f" accel={nom} smp={smp} tours={tours} (mesure ; la politique par defaut est inchangee)")
for f in fautes:
    print(f"  ECHEC   {f}")
if fautes:
    print(f"AB_REVEIL_IPI_ECHEC n={len(fautes)} accel={nom} smp={smp}")
    sys.exit(1)
PY
