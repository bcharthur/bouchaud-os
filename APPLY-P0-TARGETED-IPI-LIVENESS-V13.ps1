param(
    [switch]$Preview,
    [switch]$SkipCargoCheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$BklPath = "src\kernel\sync\bkl.rs"
$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath = "src\arch\x86_64\idt.rs"
$P0Marker = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"
$FixMarker = "BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13"

function Fail([string]$Message) {
    throw "[P0 v1.3 liveness] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Run this script from the bouchaud-os repository root."
}

foreach ($path in @($BklPath, $ThreadPath, $IdtPath)) {
    if (-not (Test-Path -LiteralPath $path)) {
        Fail "Missing file: $path"
    }
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Strict([string]$Path) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    $bytes = [System.IO.File]::ReadAllBytes($full)
    return $Utf8Strict.GetString($bytes)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    [System.IO.File]::WriteAllText($full, $Content, $Utf8NoBom)
}

$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath
$bkl = Read-Utf8Strict $BklPath

if (-not $thread.Contains($P0Marker) -or -not $idt.Contains($P0Marker)) {
    Fail "Targeted-IPI P0 v1.2 is not applied. Do not apply v1.3 on this tree."
}

if ($bkl.Contains($FixMarker)) {
    Write-Host "[OK] P0 v1.3 already applied." -ForegroundColor Green
    exit 0
}

$startToken = "pub fn resume_after_schedule(depth: usize) {"
$endToken = "/// Acquisitions par origine"

$start = $bkl.IndexOf($startToken)
if ($start -lt 0) {
    Fail "resume_after_schedule() not found."
}
$end = $bkl.IndexOf($endToken, $start)
if ($end -lt 0) {
    Fail "resume_after_schedule() end anchor not found."
}

$before = $bkl.Substring(0, $start)
$body = $bkl.Substring($start, $end - $start)
$after = $bkl.Substring($end)

if (-not $body.Contains("let mut active_spins = 0usize;")) {
    Fail "Expected adaptive resume wait state not found."
}
if (-not $body.Contains("wait_for_owner_change(&mut active_spins);")) {
    Fail "Expected HLT-capable resume wait path not found."
}

$newBody = $body.Replace(
    "    let mut active_spins = 0usize;`r`n",
    ""
).Replace(
    "    let mut active_spins = 0usize;`n",
    ""
)

$oldWaitCrlf = @'
        // Meme politique adaptative lors de la reprise d'une pile noyau.
        wait_for_owner_change(&mut active_spins);
'@
$oldWaitLf = $oldWaitCrlf

$newWait = @'
        // BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13
        //
        // Do NOT HLT while resuming a suspended scheduler/kernel continuation.
        // With targeted scheduler IPIs there is no longer a 4 ms broadcast
        // heartbeat guaranteed to wake this CPU after BKL release.
        //
        // The CPU was explicitly woken because it has useful work. Busy-wait
        // here until OWNER becomes free; ordinary enter() keeps the adaptive
        // HLT policy for unrelated BKL contention.
        spin_loop();
'@

if ($newBody.Contains($oldWaitCrlf)) {
    $newBody = $newBody.Replace($oldWaitCrlf, $newWait)
} elseif ($newBody.Contains($oldWaitLf)) {
    $newBody = $newBody.Replace($oldWaitLf, $newWait)
} else {
    # Line-ending-independent fallback.
    $newBody = [regex]::Replace(
        $newBody,
        '(?m)^\s*// Meme politique adaptative lors de la reprise d''une pile noyau\.\r?\n\s*wait_for_owner_change\(&mut active_spins\);',
        $newWait,
        1
    )
}

if (-not $newBody.Contains($FixMarker)) {
    Fail "Could not replace resume wait path."
}
if ($newBody.Contains("wait_for_owner_change(&mut active_spins);")) {
    Fail "HLT-capable resume wait path still present."
}

$newBkl = $before + $newBody + $after

if ($Preview) {
    Write-Host "[PREVIEW] $BklPath" -ForegroundColor Yellow
    Write-Host "  resume_after_schedule(): adaptive HLT -> active spin"
    Write-Host "  enter(): unchanged"
    Write-Host "  targeted IPI patch: unchanged"
    exit 0
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupRoot = ".bouchaud-history\backups\.bouchaud-p0-targeted-ipi-liveness-v13-$stamp"
New-Item -ItemType Directory -Force "$backupRoot\src\kernel\sync" | Out-Null
Copy-Item -LiteralPath $BklPath -Destination "$backupRoot\$BklPath" -Force

Write-Utf8NoBom $BklPath $newBkl

# Strict decode after write to catch source corruption before build.
$check = Read-Utf8Strict $BklPath
if (-not $check.Contains($FixMarker)) {
    Fail "Post-write marker check failed."
}

$funcStart = $check.IndexOf($startToken)
$funcEnd = $check.IndexOf($endToken, $funcStart)
$resumeBody = $check.Substring($funcStart, $funcEnd - $funcStart)

if ($resumeBody.Contains("wait_for_owner_change")) {
    Fail "resume_after_schedule() can still enter adaptive HLT."
}
if (-not $resumeBody.Contains("spin_loop();")) {
    Fail "resume_after_schedule() active spin missing."
}

Write-Host "[PATCH] $BklPath" -ForegroundColor Green
Write-Host "[BACKUP] $backupRoot" -ForegroundColor Yellow

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed."
}

if (-not $SkipCargoCheck) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check failed."
    }
}

Write-Host ""
Write-Host "P0 v1.3 liveness fix applied." -ForegroundColor Green
Write-Host "Next:" -ForegroundColor Cyan
Write-Host "  .\VERIFY-P0-TARGETED-IPI-LIVENESS-V13.ps1 -Build"
Write-Host "  .\run.ps1"
