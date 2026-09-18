param(
    [string]$RepoRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [string]$Distribution = "",
    [ValidateRange(1,16)][int]$Jobs = 8,
    [switch]$BuildUsb
)
$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
if (-not (Test-Path (Join-Path $RepoRoot 'tools\ladybird\native-browser-final.sh'))) {
    throw 'Place ce script a la racine de bouchaud-os, ou indique -RepoRoot.'
}
if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) { throw 'WSL absent.' }
$WslArgs = @()
if ($Distribution) { $WslArgs = @('-d', $Distribution) }
$GitCommand = Get-Command git -ErrorAction Stop
$GitDirRaw = (& $GitCommand.Source -C $RepoRoot rev-parse --absolute-git-dir | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or -not $GitDirRaw) {
    throw 'Impossible de resoudre le vrai git-dir du checkout courant.'
}
$GitDir = (Resolve-Path -LiteralPath $GitDirRaw).Path

$GitCommonDirRaw = (& $GitCommand.Source -C $RepoRoot rev-parse --path-format=absolute --git-common-dir | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or -not $GitCommonDirRaw) {
    throw 'Impossible de resoudre le git common-dir du checkout courant.'
}
$GitCommonDir = (Resolve-Path -LiteralPath $GitCommonDirRaw).Path

$Repo64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($RepoRoot))
$GitDir64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($GitDir))
$GitCommonDir64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($GitCommonDir))
$Script = @'
#!/usr/bin/env bash
set -euo pipefail
WINDOWS=$(printf '%s' '__REPO64__' | base64 -d)
WINDOWS_GIT_DIR=$(printf '%s' '__GITDIR64__' | base64 -d)
WINDOWS_GIT_COMMON_DIR=$(printf '%s' '__GITCOMMON64__' | base64 -d)
SOURCE=$(wslpath -a -u "$WINDOWS")
SOURCE_GIT_DIR=$(wslpath -a -u "$WINDOWS_GIT_DIR")
SOURCE_COMMON_GIT_DIR=$(wslpath -a -u "$WINDOWS_GIT_COMMON_DIR")
[ -f "$SOURCE_GIT_DIR/HEAD" ] || { echo "git-dir WSL introuvable: $SOURCE_GIT_DIR"; exit 1; }
[ -f "$SOURCE_COMMON_GIT_DIR/HEAD" ] || { echo "git common-dir WSL introuvable: $SOURCE_COMMON_GIT_DIR"; exit 1; }
SOURCE_HEAD=$(git --git-dir="$SOURCE_GIT_DIR" --work-tree="$SOURCE" rev-parse --verify HEAD)
REQUESTED_JOBS=__JOBS__
[ "$(id -u)" -ne 0 ] || { echo 'Lancer WSL avec le compte arthur, pas root.'; exit 1; }
. /etc/os-release
[ "$ID" = ubuntu ] || { echo 'Ce script requiert une distribution Ubuntu WSL.'; exit 1; }
case "$VERSION_ID" in 22.04|24.04|26.04) ;; *) echo "Ubuntu $VERSION_ID non valide par ce script (22.04/24.04/26.04 requis)."; exit 1;; esac
KEY=$(printf '%s' "$SOURCE" | sha256sum | cut -c1-16)
BASE="$HOME/.cache/bouchaud-local-$KEY"
if [ -d "$BASE" ] && [ ! -f "$BASE/managed-v1" ]; then
    echo "Repertoire non gere refuse: $BASE"; exit 1
fi
mkdir -p "$BASE"
touch "$BASE/managed-v1"
exec 9>"$BASE/build.lock"
flock -n 9 || { echo 'Une compilation locale est deja en cours.'; exit 1; }
exec > >(tee "$BASE/last-build.log") 2>&1
trap 'echo "Journal : $BASE/last-build.log"' EXIT
START=$SECONDS
if [ ! -f "$BASE/toolchain-v2-$VERSION_ID" ]; then
    echo 'Installation initiale des outils Ubuntu (mot de passe sudo demande).'
    sudo apt-get update
    sudo apt-get install -y --no-install-recommends \
        build-essential pkg-config python3 python3-dev python3-venv perl \
        ccache cmake ninja-build rsync git curl wget gnupg lsb-release zip unzip tar xz-utils file \
        autoconf autoconf-archive automake libtool libltdl-dev gettext m4 \
        nasm bison flex libssl-dev libedit-dev linux-libc-dev mesa-common-dev ca-certificates
    if ! command -v clang-20 >/dev/null || ! command -v clang++-20 >/dev/null || ! command -v llvm-ar-20 >/dev/null || ! command -v ld.lld-20 >/dev/null; then
        # Ubuntu 26.04 supplies LLVM 20 in universe. Prefer configured Ubuntu
        # repositories; do not invent an apt.llvm.org suite for a new release.
        llvm_available() {
            local package candidate
            for package in clang-20 lld-20 llvm-20; do
                candidate=$(LC_ALL=C apt-cache policy "$package" | awk '/Candidate:/ {print $2}')
                [ -n "$candidate" ] && [ "$candidate" != '(none)' ] || return 1
            done
        }
        if ! llvm_available; then
            sudo apt-get install -y --no-install-recommends software-properties-common
            sudo add-apt-repository -y universe
            sudo apt-get update
        fi
        if ! llvm_available; then
            case "$VERSION_CODENAME" in
                jammy|noble)
                    curl --fail --location https://apt.llvm.org/llvm-snapshot.gpg.key -o "$BASE/llvm-key"
                    sudo gpg --dearmor --yes -o /usr/share/keyrings/bouchaud-llvm.gpg "$BASE/llvm-key"
                    printf 'deb [signed-by=/usr/share/keyrings/bouchaud-llvm.gpg] https://apt.llvm.org/%s/ llvm-toolchain-%s-20 main\n' "$VERSION_CODENAME" "$VERSION_CODENAME" \
                        | sudo tee /etc/apt/sources.list.d/bouchaud-llvm20.list
                    sudo apt-get update
                    ;;
                *)
                    echo "LLVM 20 absent des depots Ubuntu $VERSION_ID apres activation de universe."
                    echo 'Verifier les sources APT et la sortie de apt-get update ci-dessus.'
                    exit 1
                    ;;
            esac
        fi
        sudo apt-get install -y --no-install-recommends clang-20 lld-20 llvm-20
    fi
    touch "$BASE/toolchain-v2-$VERSION_ID"
fi
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v rustup >/dev/null; then
    curl --proto '=https' --tlsv1.2 --fail --location https://sh.rustup.rs -o "$BASE/rustup-init.sh"
    sh "$BASE/rustup-init.sh" -y --profile minimal --default-toolchain stable
fi
for tool in git python3 rsync cmake ninja clang-20 clang++-20 llvm-ar-20 ld.lld-20 ccache rustup; do
    command -v "$tool" >/dev/null || { echo "Outil manquant: $tool"; exit 1; }
done
MEM_MB=$(awk '/MemTotal:/ {print int($2/1024)}' /proc/meminfo)
SAFE_JOBS=$(( (MEM_MB - 2048) / 2048 ))
[ "$SAFE_JOBS" -ge 1 ] || { echo 'Memoire WSL insuffisante (4 Go minimum).'; exit 1; }
export BO_JOBS=$REQUESTED_JOBS
[ "$BO_JOBS" -le "$SAFE_JOBS" ] || BO_JOBS=$SAFE_JOBS
export CMAKE_BUILD_PARALLEL_LEVEL=$BO_JOBS
export VCPKG_MAX_CONCURRENCY=$BO_JOBS
export BO_CCACHE_MAXSIZE=8G
BUILD="$BASE/repo"
FREE_KB=$(df -Pk "$BASE" | awk 'END {print $4}')
[ "$FREE_KB" -ge 31457280 ] || { echo 'Il faut au moins 30 Gio libres dans WSL.'; exit 1; }
echo "WSL : $MEM_MB Mio RAM, $BO_JOBS compilations simultanees, dossier $BUILD"
# This copy is managed exclusively by this script. Preserve build caches.
if [ ! -d "$BUILD/.git" ]; then
    mkdir -p "$BUILD"
    git -C "$BUILD" init --quiet
fi
git -C "$BUILD" fetch --quiet "$SOURCE_COMMON_GIT_DIR" "$SOURCE_HEAD"
git -C "$BUILD" reset --mixed --quiet FETCH_HEAD
python3 - "$SOURCE" "$SOURCE_GIT_DIR" "$BUILD" <<'PY'
import filecmp, json, os, shutil, subprocess, sys
from pathlib import Path
source, source_git_dir, dest = map(Path, sys.argv[1:])
state = dest / '.git/bouchaud-synced.json'
previous = set(json.loads(state.read_text())) if state.exists() else set()
raw = subprocess.check_output([
    'git',
    f'--git-dir={source_git_dir}',
    f'--work-tree={source}',
    'ls-files', '-z', '--cached', '--others', '--exclude-standard',
    '--exclude=native-browser-m9.previous-*',
    '--exclude=.ladybird-local-export.*',
    '--exclude=.ladybird-local-export-path',
])
current = set()
for raw_name in raw.split(b'\0'):
    if not raw_name:
        continue
    name = os.fsdecode(raw_name)
    relative = Path(name)
    if relative.is_absolute() or '..' in relative.parts or '.git' in relative.parts:
        raise SystemExit('Unsafe source path')
    src, dst = source / relative, dest / relative
    if not src.exists():
        continue  # tracked deletion in the working tree
    if src.is_symlink():
        raise SystemExit('Source symlink unsupported: ' + name)
    if not src.is_file():
        raise SystemExit('Submodule/directory unsupported: ' + name)
    current.add(name)
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.is_symlink():
        dst.unlink()
    # Normalize build text on the Linux copy, before comparison. In particular,
    # CRLF in UPSTREAM.md otherwise appends CR to the SHA read by fetch.sh.
    text_suffixes = {'.md', '.txt', '.sh', '.bash', '.py', '.ps1', '.toml',
                     '.json', '.yml', '.yaml', '.cmake', '.rs', '.c', '.h',
                     '.cc', '.cpp', '.hpp', '.s', '.asm', '.ld', '.patch',
                     '.diff', '.lock', '.in', '.cfg', '.conf'}
    is_text = relative.suffix.lower() in text_suffixes or relative.name in {
        'CMakeLists.txt', 'Makefile', 'Dockerfile', '.gitattributes', '.gitignore'}
    if is_text:
        data = src.read_bytes()
        if b'\0' not in data:
            data = data.replace(b'\r\n', b'\n')
        if not dst.exists() or dst.read_bytes() != data:
            dst.write_bytes(data)
    elif not dst.exists() or not filecmp.cmp(src, dst, shallow=False):
        shutil.copyfile(src, dst)
    if name.endswith('.sh'):
        dst.chmod(0o755)
for name in previous - current:
    relative = Path(name)
    if relative.is_absolute() or '..' in relative.parts or '.git' in relative.parts:
        raise SystemExit('Unsafe previous source path')
    path = dest / relative
    if path.is_file() or path.is_symlink():
        path.unlink()
state.write_text(json.dumps(sorted(current)))
print(f'{len(current)} source files synchronized (including local edits).')
PY
cd "$BUILD"
echo 'Compilation Ladybird. Le premier passage reste long; conserver ce dossier.'
bash tools/ladybird/native-browser-final.sh --cible
OUT="$BUILD/third_party/native-browser-bouchaud"
printf 'Bouchaud V16 chrome=fontconfig+svg loading=1\n' > "$OUT/V16_UI_CAPABLE"
printf 'Bouchaud V19 onglets=1 recherche=1 presse-papiers=1 menu=1 telechargements=1 favoris=1\n' > "$OUT/V19_UI_CAPABLE"
for f in BouchaudBrowserHost WebContent RequestServer ImageDecoder Compositor WebWorker WebDriver webcontent-bootstrap; do
    test -s "$OUT/$f"
    readelf -l "$OUT/$f" > "$BASE/elf-check.txt"
    if grep -q INTERP "$BASE/elf-check.txt"; then echo "ELF non autonome: $f"; exit 1; fi
done
test -f "$OUT/M9_CAPABLE"
test -d "$OUT/resources"
python3 tools/reference/verify-ladybird-ramonly-artifact.py "$OUT/BouchaudBrowserHost"
# Do not disturb the existing Windows runtime until build and checks succeed.
STAGING=$(mktemp -d "$SOURCE/.ladybird-local-export.XXXXXX")
rsync -rL "$OUT/" "$STAGING/"
# Let Windows perform the final copy: a WSL directory rename can be denied
# even after all bytes were successfully copied to the mounted Windows disk.
printf '%s\n' "$(basename "$STAGING")" > "$SOURCE/.ladybird-local-export-path"
echo "LADYBIRD_LOCAL_OK : $((SECONDS-START)) secondes"
echo 'Compilation et verification terminees; export Windows en attente.'
'@
$Script = $Script.Replace('__REPO64__', $Repo64).Replace('__GITDIR64__', $GitDir64).Replace('__GITCOMMON64__', $GitCommonDir64).Replace('__JOBS__', [string]$Jobs)
$Temp = [IO.Path]::GetTempFileName()
try {
    [IO.File]::WriteAllText($Temp, $Script.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
    $LinuxTemp = & wsl.exe @WslArgs --exec wslpath -a -u $Temp
    if ($LASTEXITCODE -ne 0) { throw 'Impossible de convertir le chemin temporaire dans WSL.' }
    & wsl.exe @WslArgs --exec bash ($LinuxTemp.Trim())
    if ($LASTEXITCODE -ne 0) { throw 'Compilation WSL echouee. Voir le message Linux ci-dessus (journal si indique).' }
    $ExportMarker = Join-Path $RepoRoot '.ladybird-local-export-path'
    $ExportName = [IO.File]::ReadAllText($ExportMarker).Trim()
    if ($ExportName -notmatch '^\.ladybird-local-export\.[A-Za-z0-9]+$') { throw 'Chemin export invalide.' }
    $ExportRoot = Join-Path $RepoRoot $ExportName
    $NativeRoot = Join-Path $RepoRoot 'native-browser-m9'
    if (-not (Test-Path (Join-Path $ExportRoot 'BouchaudBrowserHost'))) { throw 'Export incomplet.' }
    New-Item -ItemType Directory -Path $NativeRoot -Force | Out-Null
    Copy-Item -Path (Join-Path $ExportRoot '*') -Destination $NativeRoot -Recurse -Force -ErrorAction Stop
    Write-Host 'Navigateur local copie dans native-browser-m9.' -ForegroundColor Green
    if ($BuildUsb) {
        & powershell.exe -ExecutionPolicy Bypass -File (Join-Path $RepoRoot 'tools\reference\build-trigkey-stage2-usb.ps1') -ForceLadybird
        if ($LASTEXITCODE -ne 0) { throw 'Ladybird compile, mais fabrication USB echouee.' }
    }
    Write-Host 'Termine. Ne pas utiliser -RefreshLadybird : le navigateur local est deja installe.' -ForegroundColor Green
} finally {
    Remove-Item -LiteralPath $Temp -Force -ErrorAction SilentlyContinue
}
