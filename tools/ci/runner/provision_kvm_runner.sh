#!/usr/bin/env bash
# Prepare a DEDICATED Linux x64 host as a self-hosted GitHub Actions runner
# for the P13 KVM campaigns (BOUCHAUD_RUNNER_KVM_V1).
#
#   sudo tools/ci/runner/provision_kvm_runner.sh kvm4|kvm8 [--install]
#
# Without --install: checks only, changes nothing, exit 0 iff the host is fit.
# With --install: installs the listed apt packages and the pinned Rust
# toolchain for the runner user, then checks again.
#
# What this script will NOT do -- these stay human actions:
#   * fetch a registration token (Settings > Actions > Runners > New
#     self-hosted runner, or `gh api -X POST repos/OWNER/REPO/actions/runners/registration-token`);
#   * register the runner (the exact command is printed at the end);
#   * decide that a machine is dedicated: BOUCHAUD_DEDICATED_HOST=YES is set
#     in the runner's .env by the operator, never by a workflow.
#
# Fit = all of:
#   * >= N physical cores in the affinity mask (N = 4 for kvm4, 8 for kvm8),
#     counted as CORES, not SMT threads: SMP8 on 4 cores x 2 threads is the
#     overallocation P13 must not measure;
#   * no CPU quota below N (cgroup v2 cpu.max);
#   * /dev/kvm readable and writable by the runner user;
#   * qemu-system-x86_64 >= 8.0, OVMF, musl-gcc, mtools, python3 >= 3.10;
#   * >= 60 GiB free under the runner work directory (kernel + Ladybird
#     builds, trial images are deleted per trial but peaks are large);
#   * >= 16 GiB RAM (kvm8 guests use 4 GiB; QEMU + builds);
#   * no swap pressure, load average below the core count.
set -uo pipefail
cd "$(dirname "$0")/../../.."

LABEL=${1:?usage: provision_kvm_runner.sh kvm4|kvm8 [--install]}
INSTALL=${2:-}
case "$LABEL" in kvm4) N=4 ;; kvm8) N=8 ;; *) echo "label inconnu: $LABEL" >&2; exit 2 ;; esac
RUNNER_USER=${RUNNER_USER:-${SUDO_USER:-$(id -un)}}
WORK=${RUNNER_WORK:-/home/$RUNNER_USER/actions-runner/_work}
TOOLCHAIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
PAQUETS=(qemu-system-x86 qemu-utils ovmf musl-tools mtools build-essential binutils file python3 git curl jq dosfstools)

ko=0
verdict() { if [ "$2" = 0 ]; then echo "RUNNER_CHECK ok   $1"; else echo "RUNNER_CHECK FAIL $1"; ko=1; fi; }

if [ "$INSTALL" = --install ]; then
  [ "$(id -u)" = 0 ] || { echo "--install exige root" >&2; exit 2; }
  apt-get update && apt-get install -y --no-install-recommends "${PAQUETS[@]}" || exit 1
  usermod -aG kvm "$RUNNER_USER" || true
  sudo -u "$RUNNER_USER" bash -lc "command -v rustup >/dev/null || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    . \$HOME/.cargo/env && rustup toolchain install $TOOLCHAIN --component rust-src,llvm-tools-preview,clippy \
    && rustup target add x86_64-unknown-uefi --toolchain $TOOLCHAIN"
fi

# --- CPU: cores, not threads -------------------------------------------------
mapfile -t cpus < <(python3 -c 'import os; print("\n".join(map(str, sorted(os.sched_getaffinity(0)))))')
coeurs=$(for c in "${cpus[@]}"; do
  p=/sys/devices/system/cpu/cpu$c/topology
  echo "$(cat $p/physical_package_id 2>/dev/null || echo 0):$(cat $p/core_id 2>/dev/null || echo $c)"
done | sort -u | wc -l)
verdict "cores_physiques=$coeurs threads=${#cpus[@]} requis=$N (sans surallocation SMT)" $(( coeurs >= N ? 0 : 1 ))
if [ -r /sys/fs/cgroup/cpu.max ]; then
  read -r q per < /sys/fs/cgroup/cpu.max
  if [ "$q" = max ]; then verdict "quota_cpu=illimite" 0
  else verdict "quota_cpu=$((q / per)) requis=$N" $(( q / per >= N ? 0 : 1 )); fi
fi
charge=$(cut -d' ' -f1 /proc/loadavg)
verdict "charge=$charge < coeurs=$coeurs" "$(python3 -c "print(0 if $charge < $coeurs else 1)")"

# --- KVM ---------------------------------------------------------------------
if sudo -u "$RUNNER_USER" test -r /dev/kvm -a -w /dev/kvm 2>/dev/null || { [ "$(id -un)" = "$RUNNER_USER" ] && test -r /dev/kvm -a -w /dev/kvm; }; then
  verdict "/dev/kvm rw pour $RUNNER_USER" 0
else
  verdict "/dev/kvm rw pour $RUNNER_USER (groupe kvm ?)" 1
fi
grep -qE 'vmx|svm' /proc/cpuinfo; verdict "virtualisation materielle (vmx/svm)" $?

# --- tools -------------------------------------------------------------------
v=$(qemu-system-x86_64 --version 2>/dev/null | awk 'NR == 1 { for (i = 1; i < NF; i++) if ($i == "version") { split($(i + 1), a, "."); print a[1] } }')
verdict "qemu-system-x86_64 majeur=${v:-absent} >= 8" $(( ${v:-0} >= 8 ? 0 : 1 ))
[ -f /usr/share/OVMF/OVMF_CODE_4M.fd ]; verdict "OVMF_CODE_4M.fd" $?
for t in musl-gcc mcopy mkfs.fat python3 git jq; do command -v "$t" >/dev/null; verdict "outil $t" $?; done
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)'; verdict "python3 >= 3.10" $?
sudo -u "$RUNNER_USER" bash -lc "rustup run $TOOLCHAIN rustc --version" >/dev/null 2>&1 || rustup run "$TOOLCHAIN" rustc --version >/dev/null 2>&1
verdict "toolchain $TOOLCHAIN pour $RUNNER_USER" $?

# --- disk and memory -----------------------------------------------------------
# Checks never create anything: measure the nearest existing ancestor.
d=$WORK; while [ ! -d "$d" ]; do d=$(dirname "$d"); done
libre=$(df -BG --output=avail "$d" 2>/dev/null | tail -1 | tr -dc 0-9)
verdict "disque_libre=${libre:-0}G sous $WORK >= 60G" $(( ${libre:-0} >= 60 ? 0 : 1 ))
ram=$(awk '/MemTotal/ { print int($2 / 1048576) }' /proc/meminfo)
verdict "ram=${ram}G >= 16G" $(( ram >= 16 ? 0 : 1 ))

if [ "$ko" != 0 ]; then
  echo "RUNNER_PROVISION_FAIL label=bouchaud-$LABEL : hote non apte, rien a enregistrer"
  exit 1
fi
cat <<EOF
RUNNER_PROVISION_OK label=bouchaud-$LABEL
Action humaine restante (jeton a usage unique, jamais commite) :
  sudo -u $RUNNER_USER bash -c 'cd ~/actions-runner && ./config.sh \\
     --url https://github.com/<OWNER>/<REPO> --token <REGISTRATION_TOKEN> \\
     --labels bouchaud-$LABEL --name bouchaud-$LABEL-\$(hostname -s) --work _work --unattended'
  echo BOUCHAUD_DEDICATED_HOST=YES | sudo -u $RUNNER_USER tee -a ~$RUNNER_USER/actions-runner/.env
  sudo ~$RUNNER_USER/actions-runner/svc.sh install $RUNNER_USER && sudo ~$RUNNER_USER/actions-runner/svc.sh start
Puis : workflow_dispatch de p13-dedicated-kvm.yml (campaign=$([ "$LABEL" = kvm4 ] && echo baseline || echo smp8)) ;
le preflight tools/ci/p13_kvm_host_preflight.py re-verifie l'hote au debut de chaque job.
EOF
