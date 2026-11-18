#!/usr/bin/env bash
# L'exception W^X d'une JVM, et rien qu'elle (BOUCHAUD_JIT_RWX_V1).
#
# HotSpot ecrit son code compile dans des pages qu'il execute : sans page a la
# fois inscriptible et executable, `java` ne demarre pas, et aucun lanceur
# Minecraft non plus. Bouchaud l'accorde a UN cas : un lanceur Java
# `/usr/lib/jvm/<jdk>/bin/java` dont tout le chemin appartient a root et n'est
# inscriptible ni par le groupe ni par les autres -- sur de la memoire anonyme
# et privee seulement.
#
# La meme sonde est installee sous quatre noms ; le banc exige que SEUL le
# lanceur de confiance obtienne l'exception, et que meme lui n'obtienne jamais
# une page RWX partagee, adossee a un fichier ou mal decrite :
#
#   confiance  /usr/lib/jvm/sonde/bin/java    (root, 0755)  -> RWX anonyme
#   groupe     /usr/lib/jvm/groupe/bin/java   (root, 0775)  -> rien : un
#              membre du groupe pourrait remplacer le binaire
#   systeme    /jit-rwx-probe                 (root, 0755)  -> rien : pas un
#              lanceur Java, meme pour root
#   ouvert     /usr/lib/jvm/ouvert/bin/java   (root, 0777)  -> exec refuse
#   tmp        /tmp/usr/lib/jvm/x/bin/java                  -> exec refuse
#
# Et une seconde sonde verifie les appels systeme que HotSpot demandait encore
# (getrusage, fchdir, flock -- BOUCHAUD_GETRUSAGE_V1, BOUCHAUD_FLOCK_V1).
#
#   tools/ci/run_jit_rwx.sh BOOTIMAGE [SORTIE]
#   BO_JIT_RWX_JDK=/chemin/jdk tools/ci/run_jit_rwx.sh ...   # + une vraie JVM
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_jit_rwx.sh BOOTIMAGE [SORTIE]}
SORTIE=${2:-target/jit-rwx}
rm -rf "$SORTIE"; mkdir -p "$SORTIE/scen"
SORTIE=$(cd "$SORTIE" && pwd)
S="$SORTIE/scen"

command -v gcc >/dev/null || { echo "JIT_RWX_ECHEC gcc absent" >&2; exit 1; }
gcc -O2 -static-pie -fPIE -o "$S/jit-rwx-probe" tools/userland/jit-rwx-probe.c
chmod 0755 "$S/jit-rwx-probe"
gcc -O1 -static-pie -fPIE -o "$S/jvm-syscalls-probe" tools/userland/jvm-syscalls-probe.c
mkdir -p "$S/usr/lib/jvm/sonde/bin" "$S/usr/lib/jvm/groupe/bin" "$S/usr/lib/jvm/ouvert/bin" "$S/tmp/usr/lib/jvm/x/bin"
install -m 0755 "$S/jit-rwx-probe" "$S/usr/lib/jvm/sonde/bin/java"
install -m 0775 "$S/jit-rwx-probe" "$S/usr/lib/jvm/groupe/bin/java"
install -m 0777 "$S/jit-rwx-probe" "$S/usr/lib/jvm/ouvert/bin/java"
install -m 0755 "$S/jit-rwx-probe" "$S/tmp/usr/lib/jvm/x/bin/java"

{
  echo 'echo JIT_RWX_DEBUT'
  echo '/usr/lib/jvm/sonde/bin/java confiance /jit-rwx-probe'
  echo '/usr/lib/jvm/groupe/bin/java groupe /jit-rwx-probe'
  echo '/jit-rwx-probe systeme /jit-rwx-probe'
  echo '/usr/lib/jvm/ouvert/bin/java ouvert /jit-rwx-probe'
  echo '/tmp/usr/lib/jvm/x/bin/java tmp /jit-rwx-probe'
  echo '/jvm-syscalls-probe'
} > "$S/autorun"

# Une vraie JVM, si on en fournit une (jdk glibc deplie, avec ses bibliotheques
# dans lib/ et lib64/ du scenario) : `java -version` puis un jar.
if [ -n "${BO_JIT_RWX_JDK:-}" ]; then
  cp -a "$BO_JIT_RWX_JDK"/. "$S/"
  echo 'echo JVM_DEBUT' >> "$S/autorun"
  printf '%s\n' "${BO_JIT_RWX_JVM:-/usr/lib/jvm/java21/bin/java -version}" >> "$S/autorun"
  echo 'echo JVM_FIN' >> "$S/autorun"
fi
echo 'echo JIT_RWX_BANC_FIN' >> "$S/autorun"
(cd tools/userland && IMAGE="$SORTIE/disque.img" ./mkdisk.sh "$S" >/dev/null)

LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/disque.img" \
  -m 4096 -smp 2 -display none -no-reboot -serial file:"$LOG" &
QEMU=$!
LIMITE=$((SECONDS + ${BO_JIT_RWX_S:-600}))
while kill -0 "$QEMU" 2>/dev/null; do
  grep -aq 'JIT_RWX_BANC_FIN\|KERNEL PANIC' "$LOG" && { sleep 1; break; }
  (( SECONDS >= LIMITE )) && break
  sleep 2
done
kill "$QEMU" 2>/dev/null || true
wait "$QEMU" 2>/dev/null || true

P="$SORTIE/serie.propre"
sed 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r' > "$P"
grep -a 'JIT_RWX_\|JVM_SYSCALLS\|SECURITY-JIT-RWX\|BOUCHAUD_WX_REFUSE\|KERNEL PANIC\|panicked' "$P" \
  | sed 's/^\[[^]]*\]\[[^]]*\]\[[^]]*\] //' | awk 'NR <= 40' || true

echecs=()
grep -aq 'KERNEL PANIC' "$P" && echecs+=(panique_noyau)
attendu() {
  local role=$1 valeurs=$2
  grep -aqE "JIT_RWX_SONDE role=$role $valeurs\$" "$P" || echecs+=("sonde_$role")
}
attendu confiance 'anon=1 execute=1 partage=0 fichier=0 mprotect=1 mpfich=0 trou=0 fils=1'
for role in groupe systeme; do
  attendu "$role" 'anon=0 execute=0 partage=0 fichier=0 mprotect=0 mpfich=0 trou=0 fils=0'
done
# Ces deux-la ne tournent meme pas : la politique d'exec les arrete avant.
refuse() {
  local role=$1 chemin=$2 raison=$3
  if grep -aq "JIT_RWX_SONDE role=$role " "$P"; then
    echecs+=("${role}_a_tourne")
  elif ! grep -aqF "$chemin : $raison" "$P"; then
    echecs+=("${role}_refus_absent")
  fi
}
refuse ouvert /usr/lib/jvm/ouvert/bin/java 'executable modifiable par tous interdit'
refuse tmp /tmp/usr/lib/jvm/x/bin/java 'execution depuis un emplacement non fiable interdite'
grep -aq 'JVM_SYSCALLS_FIN rusage=1 fchdir=1 flock=1' "$P" || echecs+=(syscalls_jvm)
# Le journal de securite nomme le seul lanceur qui a recu l'exception.
accordes=$(grep -aoE '\[SECURITY-JIT-RWX\] pid=[0-9]+ image=[^ ]+ accorde=1' "$P" | grep -oE 'image=[^ ]+' | sort -u || true)
attendus="image=/usr/lib/jvm/sonde/bin/java"
[ -n "${BO_JIT_RWX_JDK:-}" ] && attendus=$(printf '%s\n%s' "$attendus" "image=/usr/lib/jvm/java21/bin/java" | sort -u)
[ "$accordes" = "$attendus" ] || echecs+=("accords_inattendus:$(echo $accordes | tr ' ' ,)")
if [ -n "${BO_JIT_RWX_JDK:-}" ]; then
  grep -aq 'JVM_FIN' "$P" || echecs+=(jvm_inachevee)
  grep -aqE '(openjdk|java) version' "$P" || echecs+=(jvm_sans_version)
fi
if [ ${#echecs[@]} -ne 0 ]; then
  echo "JIT_RWX_ECHEC ${echecs[*]}"
  exit 1
fi
echo JIT_RWX_OK
