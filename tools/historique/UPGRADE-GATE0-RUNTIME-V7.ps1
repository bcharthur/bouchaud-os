$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

$ExpectedBranch = "gate0/final-20260826"
$HandoffMarker = "BOUCHAUD_GATE0_POST_SWITCH_HANDOFF_V2"
$TcpMarker = "BOUCHAUD_GATE0_TCP_SERIAL_V7"

function Replace-ExactlyOnce {
    param(
        [Parameter(Mandatory=$true)][string]$Text,
        [Parameter(Mandatory=$true)][string]$Old,
        [Parameter(Mandatory=$true)][string]$New,
        [Parameter(Mandatory=$true)][string]$Label
    )
    $first = $Text.IndexOf($Old, [StringComparison]::Ordinal)
    if ($first -lt 0) { throw "Bloc introuvable : $Label" }
    $second = $Text.IndexOf($Old, $first + $Old.Length, [StringComparison]::Ordinal)
    if ($second -ge 0) { throw "Bloc present plusieurs fois : $Label" }
    return $Text.Substring(0,$first) + $New + $Text.Substring($first + $Old.Length)
}

Write-Host "=== BOUCHAUD OS / GATE 0 RUNTIME V7 ===" -ForegroundColor Cyan

$branch = (& git branch --show-current).Trim()
if ($branch -ne $ExpectedBranch) {
    throw "Branche '$branch'. Attendue : '$ExpectedBranch'."
}

$dirty = @(git status --porcelain --untracked-files=no)
if ($dirty.Count -gt 0) {
    Write-Host ($dirty -join "`n") -ForegroundColor Yellow
    throw "Des fichiers suivis sont modifies. Aucun reset/clean automatique."
}

$thread = Get-Content ".\src\kernel\process\thread.rs" -Raw
if (-not $thread.Contains($HandoffMarker)) {
    throw "Le handoff Gate0 final n'est pas present."
}

$runPath = Join-Path $Root "run.ps1"
$run = [IO.File]::ReadAllText($runPath).Replace("`r`n","`n")

if (-not $run.Contains($TcpMarker)) {
    Write-Host "Ajout du canal serie TCP de validation..." -ForegroundColor Cyan

    $oldParam = @'
    [switch]$Gate0Autostart,

    # Memoire donnee a la machine.
'@
    $newParam = @'
    [switch]$Gate0Autostart,

    # BOUCHAUD_GATE0_TCP_SERIAL_V7
    # Canal serie dedie au runner Gate0. 0 conserve le comportement normal
    # `-serial stdio`. Une valeur >0 fait ecouter QEMU sur loopback et ATTEND
    # la connexion du runner avant de laisser demarrer la VM : aucun octet de
    # boot ne peut donc etre perdu.
    [ValidateRange(0, 65535)]
    [int]$Gate0SerialPort = 0,

    # Memoire donnee a la machine.
'@
    $run = Replace-ExactlyOnce $run $oldParam $newParam "parametre Gate0SerialPort"

    $oldSerial = @'
$qemuArgs += @(
    "-serial",
    "stdio"
)
'@
    $newSerial = @'
if ($Gate0SerialPort -gt 0) {
    $qemuArgs += @(
        "-serial",
        ("tcp:127.0.0.1:{0},server=on,wait=on,nodelay=on" -f $Gate0SerialPort)
    )
    Write-Host (
        "serie Gate0 : tcp://127.0.0.1:{0} (QEMU attend le collecteur)" -f `
            $Gate0SerialPort
    ) -ForegroundColor Yellow
}
else {
    $qemuArgs += @(
        "-serial",
        "stdio"
    )
}
'@
    $run = Replace-ExactlyOnce $run $oldSerial $newSerial "serie stdio -> TCP Gate0"

    [IO.File]::WriteAllText(
        $runPath,
        $run,
        [Text.UTF8Encoding]::new($false)
    )
}
else {
    Write-Host "Canal serie TCP V7 deja present." -ForegroundColor Green
}

Write-Host "`n=== Verification PowerShell ===" -ForegroundColor Cyan
foreach ($script in @("run.ps1","RUN-GATE0-FINAL.ps1")) {
    $tokens = $null
    $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $Root $script),
        [ref]$tokens,
        [ref]$errors
    )
    if ($errors.Count -gt 0) {
        $errors | Format-List
        throw "$script ne passe pas le parseur PowerShell."
    }
    Write-Host "OK $script" -ForegroundColor Green
}

git diff --check
if ($LASTEXITCODE -ne 0) { throw "git diff --check a echoue." }

$changed = @(git diff --name-only)
foreach ($path in $changed) {
    if ($path -ne "run.ps1") {
        throw "Fichier suivi inattendu dans le diff : $path"
    }
}

if ($changed.Count -gt 0) {
    git add -- run.ps1
    if ($LASTEXITCODE -ne 0) { throw "git add run.ps1 a echoue." }

    git commit -m "test(gate0): stream QEMU serial over loopback TCP"
    if ($LASTEXITCODE -ne 0) { throw "Commit du runner TCP Gate0 a echoue." }
}

Write-Host "`nGATE 0 RUNTIME V7 PRET" -ForegroundColor Green
Write-Host "commit : $((& git rev-parse --short HEAD).Trim())"
Write-Host ""
Write-Host "Lance :" -ForegroundColor Cyan
Write-Host "  .\RUN-GATE0-FINAL.ps1 -SkipStatic" -ForegroundColor White
