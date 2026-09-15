$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
Set-Location $Root

Write-Host "=== FIX GATE 0 RUNNER V8 ===" -ForegroundColor Cyan

$runner = Join-Path $Root "RUN-GATE0-FINAL.ps1"
if (-not (Test-Path $runner)) {
    throw "RUN-GATE0-FINAL.ps1 absent. Extrais tout le ZIP a la racine."
}

$source = Get-Content $runner -Raw
if (-not $source.Contains('$qemuProcessId')) {
    throw "Le RUN-GATE0-FINAL.ps1 V8 n'a pas ete correctement extrait."
}

$tokens = $null
$errors = $null
[void][Management.Automation.Language.Parser]::ParseFile(
    $runner, [ref]$tokens, [ref]$errors
)
if ($errors.Count -gt 0) {
    $errors | Format-List
    throw "RUN-GATE0-FINAL.ps1 V8 ne passe pas le parseur PowerShell."
}

Write-Host "Runner V8 : syntaxe OK" -ForegroundColor Green

$qemu = @(Get-Process -Name "qemu-system-x86_64" -ErrorAction SilentlyContinue)
if ($qemu.Count -gt 0) {
    Write-Host ""
    Write-Host "QEMU residuel detecte :" -ForegroundColor Yellow
    $qemu | Select-Object Id, StartTime, Path | Format-Table -AutoSize
    Write-Host ""
    Write-Host "Le runner ne le tuera pas automatiquement." -ForegroundColor Yellow
    Write-Host "Si c'est bien la VM Gate0 bloquee du run precedent, ferme sa fenetre" -ForegroundColor Yellow
    Write-Host "ou execute :" -ForegroundColor Yellow
    Write-Host "  Get-Process qemu-system-x86_64 | Stop-Process -Force" -ForegroundColor White
}
else {
    Write-Host "Aucun QEMU residuel." -ForegroundColor Green
}

Write-Host ""
Write-Host "Le noyau/commit 2ad9d39 n'est PAS modifie." -ForegroundColor DarkGray
Write-Host "Relance ensuite :" -ForegroundColor Cyan
Write-Host "  .\RUN-GATE0-FINAL.ps1 -SkipStatic" -ForegroundColor White
