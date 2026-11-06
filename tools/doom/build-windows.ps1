# Reproducible DOOM build: Windows + Zig 0.13, pinned Doomgeneric/Freedoom; no WSL.
# Source adapter is versioned; upstream, WAD, toolchain and ELF are generated (ignored).
param(
    [string]$Repo = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [switch]$NoLaunch,
    [string]$LadybirdRunId = '41920504209',
    [int]$RamMiB = 8192
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
function Note([string]$s) { Write-Host "[DOOM-WINDOWS] $s" -ForegroundColor Cyan }
function Assert-Ok([string]$action) {
    if ($LASTEXITCODE -ne 0) { throw "$action a echoue (code $LASTEXITCODE)." }
}
function Check-Elf([string]$path) {
    $fs = [IO.File]::OpenRead($path)
    try {
        $br = New-Object IO.BinaryReader($fs)
        if ($fs.Length -lt 64) { throw 'Executable ELF tronque.' }
        $m = $br.ReadBytes(16)
        if ($m[0] -ne 127 -or $m[1] -ne 69 -or $m[2] -ne 76 -or $m[3] -ne 70 -or $m[4] -ne 2 -or $m[5] -ne 1) {
            throw 'Le fichier produit nest pas un ELF x86-64 little-endian.'
        }
        $typ = $br.ReadUInt16()
        $machine = $br.ReadUInt16()
        if ($machine -ne 62 -or $typ -ne 3) { throw "ELF incorrect (machine=$machine, type=$typ; attendu x86-64 ET_DYN PIE)." }
        $fs.Position = 32
        $phoff = $br.ReadUInt64()
        $fs.Position = 54
        $phentsize = $br.ReadUInt16()
        $phnum = $br.ReadUInt16()
        if ($phoff -gt [uint64]$fs.Length -or $phentsize -lt 56 -or $phnum -gt 128) { throw 'Table de segments ELF invalide.' }
        $loadCount = 0
        for ($i = 0; $i -lt $phnum; $i++) {
            $at = [long]$phoff + $i * $phentsize
            $fs.Position = $at
            $kind = $br.ReadUInt32()
            if ($kind -eq 3) { throw 'PT_INTERP detecte : le binaire demande un chargeur dynamique.' }
            if ($kind -eq 1) {
                $loadCount++
                $fs.Position = $at + 16
                $vaddr = $br.ReadUInt64()
                $fs.Position = $at + 40
                $memsz = $br.ReadUInt64()
                if ($vaddr -gt 0x100000000 -or $memsz -gt 0x100000000) {
                    throw 'PT_LOAD PIE hors bornes (attendu adresses relatives).'
                }
            }
        }
        if ($loadCount -eq 0) { throw 'ELF sans PT_LOAD.' }
    } finally { $fs.Close() }
}

$Repo = [IO.Path]::GetFullPath($Repo)
if (!(Test-Path -LiteralPath (Join-Path $Repo 'run.ps1') -PathType Leaf)) { throw "Depot Bouchaud OS introuvable : $Repo" }
if (!(Get-Command git -ErrorAction SilentlyContinue)) { throw 'Git manquant : requis pour recuperer Doomgeneric.' }
$doomDir = Join-Path $Repo 'tools\doom'
$srcDir = Join-Path $doomDir 'upstream\doomgeneric'
$targetDir = Join-Path $Repo 'target\doom-bouchaud'
$wadDir = Join-Path $doomDir 'wads'
New-Item -ItemType Directory -Force -Path $doomDir, $targetDir, $wadDir | Out-Null
$adapt = Join-Path $doomDir 'doomgeneric_bouchaud.c'
if (!(Test-Path -LiteralPath $adapt -PathType Leaf)) { throw 'Adaptateur DOOM absent du depot Git (tools/doom/doomgeneric_bouchaud.c).' }

if (!(Test-Path -LiteralPath (Join-Path $srcDir 'Makefile') -PathType Leaf)) {
    $upstream = Join-Path $doomDir 'upstream'
    if (Test-Path -LiteralPath $upstream) { throw "Le clone Doomgeneric est incomplet : $upstream. Ne rien effacer sans examiner." }
    Note 'Recuperation du moteur Doomgeneric GPL...'
    & git clone 'https://github.com/ozkl/doomgeneric.git' $upstream
    Assert-Ok 'git clone Doomgeneric'
}
$upstream = Join-Path $doomDir 'upstream'
if (Test-Path -LiteralPath (Join-Path $upstream '.git')) {
    $commit = (& git -C $upstream rev-parse HEAD).Trim()
    Assert-Ok 'git rev-parse Doomgeneric'
    if ($commit -ne 'dcb7a8dbc7a16ce3dda29382ac9aae9d77d21284') {
        Note 'Alignement du moteur sur le commit verifie...'
        & git -C $upstream checkout --detach 'dcb7a8dbc7a16ce3dda29382ac9aae9d77d21284'
        Assert-Ok 'git checkout Doomgeneric'
    }
}
$wad = Join-Path $wadDir 'freedoom1.wad'
if (!(Test-Path -LiteralPath $wad -PathType Leaf)) {
    Note 'Recuperation du WAD officiel Freedoom 0.13.0...'
    $wadZip = Join-Path $targetDir 'freedoom-0.13.0.zip'
    $wadExtract = Join-Path $targetDir 'freedoom-extract'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri 'https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip' -OutFile $wadZip -UseBasicParsing
    $expectedWadHash = '3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59'
    if ((Get-FileHash -LiteralPath $wadZip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedWadHash) { throw 'Archive Freedoom corrompue ou non reconnue.' }
    Expand-Archive -LiteralPath $wadZip -DestinationPath $wadExtract -Force
    $found = Get-ChildItem -LiteralPath $wadExtract -Recurse -Filter 'freedoom1.wad' -File | Select-Object -First 1
    if ($null -eq $found) { throw 'freedoom1.wad manquant dans archive officielle.' }
    Copy-Item -LiteralPath $found.FullName -Destination $wad -Force
}
if ((Get-Item -LiteralPath $wad).Length -lt 10000000) { throw 'WAD trop petit; verifier sa validite.' }
Note 'Freedoom WAD disponible.'

$zigVersion = '0.13.0'
$zigRoot = Join-Path $targetDir 'toolchains'
$zigExe = Join-Path $zigRoot 'zig-windows-x86_64-0.13.0\zig.exe'
if (!(Test-Path -LiteralPath $zigExe -PathType Leaf)) {
    New-Item -ItemType Directory -Force -Path $zigRoot | Out-Null
    $zigZip = Join-Path $zigRoot 'zig-windows-x86_64-0.13.0.zip'
    $zigUrl = 'https://ziglang.org/download/0.13.0/zig-windows-x86_64-0.13.0.zip'
    $expected = 'd859994725ef9402381e557c60bb57497215682e355204d754ee3df75ee3c158'
    if (!(Test-Path -LiteralPath $zigZip -PathType Leaf) -or (Get-FileHash -LiteralPath $zigZip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
        Note 'Telechargement de Zig 0.13.0 PORTABLE Windows (79 Mo, aucun WSL)...'
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $zigUrl -OutFile $zigZip -UseBasicParsing
    }
    if ((Get-FileHash -LiteralPath $zigZip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Zig : SHA256 invalide. Ne pas executer cette archive.' }
    Note 'Extraction de Zig dans le dossier local du projet...'
    Expand-Archive -LiteralPath $zigZip -DestinationPath $zigRoot -Force
}
if (!(Test-Path -LiteralPath $zigExe -PathType Leaf)) { throw "zig.exe non trouve apres extraction : $zigExe" }
$actualVersion = (& $zigExe version).Trim()
Assert-Ok 'Execution de zig.exe'
if ($actualVersion -ne $zigVersion) { throw "Version Zig inattendue: $actualVersion" }
Note "Zig $actualVersion pret. Aucun WSL requis."

# The exact upstream list, excluding its X11 implementation, is the source of truth.
$makeText = [IO.File]::ReadAllText((Join-Path $srcDir 'Makefile'))
$match = [regex]::Match($makeText, '(?m)^SRC_DOOM\s*=\s*([^\r\n]+)')
if (!$match.Success) { throw 'Impossible de lire SRC_DOOM dans Makefile upstream.' }
$items = $match.Groups[1].Value.Trim() -split '\s+' | Where-Object { $_ -and $_ -ne 'doomgeneric_xlib.o' }
$srcFiles = @()
foreach ($item in $items) {
    if ($item -notmatch '^[a-zA-Z0-9_]+\.o$') { throw "Objet Doomgeneric inattendu : $item" }
    $source = Join-Path $srcDir ($item.Substring(0, $item.Length - 2) + '.c')
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Source manquante : $source" }
    $srcFiles += $source
}
if ($srcFiles.Count -lt 75) { throw "Sources moteur insuffisantes : $($srcFiles.Count)" }
$elf = Join-Path $targetDir 'doom-bouchaud'
$elfTemp = Join-Path $targetDir 'doom-bouchaud.new'
# Build into temporary file; keep the old executable until PIE passes validation.
if (Test-Path -LiteralPath $elfTemp) { Remove-Item -LiteralPath $elfTemp -Force }
Note "Compilation de $($srcFiles.Count) fichiers C en ELF statique x86-64 Linux, sur Windows..."
$argsZig = @('cc', '-target', 'x86_64-linux-musl', '-std=gnu11', '-O2', '-fPIE', '-static-pie', '-DNORMALUNIX', '-DLINUX', '-DSNDSERV', '-D_DEFAULT_SOURCE', "-I$srcDir", '-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections')
$argsZig += $srcFiles
$argsZig += @($adapt, '-lm', '-o', $elfTemp)
& $zigExe @argsZig
Assert-Ok 'Compilation PIE statique via Zig Windows vers Linux/musl'
if (!(Test-Path -LiteralPath $elfTemp -PathType Leaf)) { throw "Binaire manquant : $elfTemp" }
Check-Elf $elfTemp
Move-Item -LiteralPath $elfTemp -Destination $elf -Force
Note "ELF x86-64 PIE statique valide : $elf"
Note "WAD : $wad"
Note 'Preparation terminee sans WSL. Le noyau Bouchaud OS reste inchange.'
if (!$NoLaunch) {
    Push-Location $Repo
    try {
        & .\run.ps1 -Doom -LadybirdRunId $LadybirdRunId -RamMiB $RamMiB -Audio none
        Assert-Ok 'Demarrage QEMU Bouchaud OS'
    } finally { Pop-Location }
}
