#!/usr/bin/env bash
# C6a -- LE REARMEMENT APRES IDLE, FENETRE ELARGIE (BOUCHAUD_COMPTA_INJECTION_V1).
#
#   tools/ci/run_compta_injection.sh BOOTIMAGE [FENETRE_US] [DUREE_S]
#
# 3adcea3c (BOUCHAUD_COMPTA_REARME_EN_SECTION_V1) lit l'instant et le coeur du
# rearmement apres `hlt` DANS la section. Le defaut ne s'etait vu qu'une fois
# (os-primitives TCG, `chevauchements=1 pire_chevauchement_us=5859`). Ce banc
# le rend deterministe : `compta-injection FENETRE` elargit l'intervalle entre
# le retour du `hlt` et le rearmement (attente interruptions ouvertes puis
# cession), pendant que compta-stress-probe fait dormir, ceder et migrer ses
# fils. Deux demarrages, meme fenetre :
#   * `ancien`    -- l'ordre d'avant le correctif : il DOIT chevaucher ;
#   * `correctif` -- le code livre : 0 chevauchement exige ;
#   * `dechiree`  -- BOUCHAUD_INDICE_COURANT_ATOMIQUE_V1 (C14) : l'indice de la
#     tache courante lu en deux temps, la fenetre entre le coeur et sa case.
#     Il DOIT montrer le defaut : une panique `compta:` (ecrivain concurrent,
#     ouverture ou fermeture par un autre coeur) ou un chevauchement.
# Une fenetre qui n'a jamais cede ne prouve rien : NON_QUALIFIANT (code 2).
#
# BO_SMP : coeurs (2 par defaut). BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_compta_injection.sh BOOTIMAGE [FENETRE_US] [DUREE_S]}
FENETRE=${2:-2000}
DUREE=${3:-8}
SMP=${BO_SMP:-2}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi

(cd tools/userland && OUT=out-compta-injection ./build.sh musl >/dev/null)

bras() { # ordre -> journal compta-injection-ORDRE.log
  local ordre=$1 scenario=scenario-compta-injection image=compta-injection.img log=compta-injection-$1.log
  rm -rf "$scenario" "$image" "$log"
  mkdir -p "$scenario/bin"
  cp tools/userland/out-compta-injection/compta-stress-probe "$scenario/bin/"
  printf 'compta-injection %s %s\n/bin/compta-stress-probe %s\ncompta-injection etat\necho COMPTA_INJECTION_FIN\n' \
    "$FENETRE" "$ordre" "$DUREE" > "$scenario/autorun"
  (cd tools/userland && IMAGE="$PWD/../../$image" ./mkdisk.sh "$PWD/../../$scenario" >/dev/null)
  : > "$log"
  # shellcheck disable=SC2086
  qemu-system-x86_64 $ACCEL \
    -drive format=raw,file="$BOOT" \
    -drive format=raw,file="$image" \
    -m 1024 -smp "$SMP" -display none -no-reboot \
    -serial file:"$log" &
  local pid=$! limite=$((SECONDS + 300))
  while kill -0 "$pid" 2>/dev/null; do
    if (( SECONDS >= limite )); then echo "ECHEANCE atteinte ($ordre)" >&2; break; fi
    grep -aqE 'KERNEL PANIC|panicked at' "$log" && { sleep 2; break; }
    # Le reglage imprime deja un etat : on attend le second, apres la sonde.
    [ "$(grep -ac 'COMPTA_INJECTION_ETAT' "$log")" -ge 2 ] && { sleep 1; break; }
    sleep 1
  done
  kill -TERM "$pid" 2>/dev/null || true
  sleep 1
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

echo "COMPTA_INJECTION smp=${SMP} accel=${ACCEL} fenetre_us=${FENETRE} duree_s=${DUREE}"
bras ancien
bras correctif
# Le temoin depend d'une migration PENDANT la fenetre : jusqu'a trois
# demarrages, arret au premier qui montre le defaut (local : 5 sur 6 du premier
# coup). Trois temoins muets : ECHEC.
for t in 1 2 3; do
  bras dechiree
  echo "COMPTA_INJECTION_DECHIREE_TENTATIVE t=$t/3 $(grep -aoE -m1 'compta: [a-z ]+' compta-injection-dechiree.log || true)"
  grep -aqE 'compta: ' compta-injection-dechiree.log && break
done

python3 - compta-injection-ancien.log compta-injection-correctif.log compta-injection-dechiree.log <<'EOF'
import re, sys
def etat(chemin):
    t = re.sub(r"\x1b\[[0-9;]*m", "", open(chemin, encoding="utf-8", errors="replace").read())
    if re.search(r"KERNEL PANIC|panicked at", t):
        return None, "panique"
    m = re.findall(r"COMPTA_INJECTION_ETAT fenetres=(\d+) cessions=(\d+) chevauchements=(\d+) "
                   r"pire_chevauchement_us=(\d+) rearmes_deja_armes=(\d+)", t)
    if not m:
        return None, "bilan absent"
    # Le verdict de la sonde elle-meme : utime/stime relus par les fils.
    sonde = re.findall(r"COMPTA_STRESS_(OK|ECHEC)", t)
    return tuple(int(x) for x in m[-1]) + (sonde[-1] if sonde else "absent",), ""
(av, ra), (ap, rp) = etat(sys.argv[1]), etat(sys.argv[2])
for nom, e, r in (("ancien", av, ra), ("correctif", ap, rp)):
    print(f"  {nom:9} : " + (f"fenetres={e[0]} cessions={e[1]} chevauchements={e[2]} "
                             f"pire_chevauchement_us={e[3]} rearmes_deja_armes={e[4]} sonde={e[5]}" if e else r))
if av is None or ap is None:
    print("COMPTA_INJECTION_ECHEC demarrage incomplet")
    sys.exit(1)
if av[1] == 0 or ap[1] == 0:
    print("COMPTA_INJECTION_NON_QUALIFIANT la fenetre n'a jamais cede : rien n'est prouve")
    sys.exit(2)
fautes = []
if av[2] == 0:
    fautes.append("l'ordre d'avant ne chevauche pas : le banc ne reproduit pas le defaut")
if ap[2] != 0:
    fautes.append(f"le correctif chevauche ({ap[2]}, pire {ap[3]} us)")
# Les invariants existants de la sonde (temps CPU monotone, jamais au-dela
# du mur) tiennent sous le correctif ; sous l'ordre d'avant ils peuvent
# tomber (SMP2 : depassements=2), c'est le symptome, pas une exigence.
if ap[5] != "OK":
    fautes.append(f"compta-stress-probe sous le correctif : {ap[5]}")
if fautes:
    print("COMPTA_INJECTION_ECHEC " + " ; ".join(fautes))
    sys.exit(1)
# C14 : le temoin de la lecture dechiree. Une panique `compta:` EST le defaut.
td = re.sub(r"\x1b\[[0-9;]*m", "", open(sys.argv[3], encoding="utf-8", errors="replace").read())
pd = re.search(r"compta: [a-z ]+", td)
ed = re.findall(r"COMPTA_INJECTION_ETAT fenetres=(\d+) cessions=(\d+) chevauchements=(\d+)", td)
if pd:
    temoin = f"panique « {pd.group(0).strip()} »"
elif ed and int(ed[-1][2]) > 0:
    temoin = f"chevauchements={ed[-1][2]}"
elif ed and int(ed[-1][1]) == 0:
    print("COMPTA_INJECTION_NON_QUALIFIANT dechiree : la fenetre n'a jamais cede")
    sys.exit(2)
else:
    print("COMPTA_INJECTION_ECHEC dechiree : la lecture en deux temps n'a pas montre le defaut (temoin muet)")
    sys.exit(1)
print(f"  dechiree  : {temoin}")
print(f"COMPTA_INJECTION_OK dechiree={temoin} ancien={av[2]} chevauchement(s) (sonde {av[5]}) correctif=0 cessions={ap[1]} rearmes_evites={ap[4]} sonde=OK")
EOF
