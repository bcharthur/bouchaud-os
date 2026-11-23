$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../windows/BouchaudPerformance.psm1') -Force

function Assert-Profile([bool]$Ok, [string]$Message) {
    if (-not $Ok) { throw $Message }
}

function Assert-Refused($Capabilities, [hashtable]$Explicit, [string]$Reason) {
    try { $null = Resolve-BouchaudPerformance $Capabilities $Explicit }
    catch {
        if ($_.Exception.Message -notlike "*$Reason*") { throw }
        return
    }
    throw "Refus attendu : $Reason"
}

$capabilities = @{ HypervisorPresent = $true; QemuWhpx = $true; QemuTcg = $true; AvailableMiB = 12000 }
$profile = Resolve-BouchaudPerformance $capabilities
Assert-Profile ($profile.Accel -eq 'whpx' -and $profile.CpuCount -eq 1 -and $profile.RamMiB -eq 8192) 'Profil WHPX incorrect'
Assert-Profile ($profile.Surface -eq 'simple' -and $profile.WhpxIrqchip -eq 'off' -and -not $profile.P10GPU -and -not $profile.Diagnostic) 'Configuration de reference modifiee'
$verdict = @{ whpx_indisponible = $true; noyau = $false; vp_exit = $null }
Assert-Profile (Test-BouchaudPerformanceFallback $profile $verdict) 'Repli initial WHPX absent'
$verdict.noyau = $true
Assert-Profile (-not (Test-BouchaudPerformanceFallback $profile $verdict)) 'Repli apres boot interdit'
$verdict.noyau = $false
$verdict.vp_exit = 4
Assert-Profile (-not (Test-BouchaudPerformanceFallback $profile $verdict)) 'Repli apres VP exit interdit'
$explicit = Resolve-BouchaudPerformance $capabilities @{ Accel = 'whpx'; CpuCount = 1; P10GPU = $false; RamMiB = 8192 }
Assert-Profile (-not $explicit.AllowInitFallback) 'WHPX explicite ne doit pas replier'
Assert-Refused $capabilities @{ CpuCount = 4 } '-CpuCount'
Assert-Refused $capabilities @{ P10GPU = $true } '-P10GPU'
Assert-Refused $capabilities @{ Diagnostic = $true } '-Diagnostic'
Assert-Refused $capabilities @{ Surface = 'double' } '-Surface'
Assert-Refused $capabilities @{ WhpxIrqchip = 'on' } '-WhpxIrqchip'
Assert-Refused $capabilities @{ Accel = 'tcg' } '-Accel'
Assert-Refused $capabilities @{ Legacy = $true } '-Legacy'
Assert-Refused $capabilities @{ Gate0Autostart = $true } '-Gate0Autostart'

$capabilities.HypervisorPresent = $false
$tcg = Resolve-BouchaudPerformance $capabilities
Assert-Profile ($tcg.Accel -eq 'tcg' -and -not $tcg.AllowInitFallback -and $tcg.Reason -like '*emulation logicielle*') 'TCG doit etre annonce'
$capabilities.HypervisorPresent = $true
$capabilities.QemuWhpx = $false
Assert-Profile ((Resolve-BouchaudPerformance $capabilities).Accel -eq 'tcg') 'QEMU sans WHPX'
$capabilities.QemuTcg = $false
Assert-Refused $capabilities @{} 'aucun accelerateur'
$capabilities.QemuTcg = $true
$message = Write-BouchaudPerformance -Profile $tcg -Capabilities $capabilities -Accel 'tcg' -Reason $tcg.Reason 3>&1 6>&1 | Out-String
Assert-Profile ($message -like "*profil materiel de performance WHPX n'est pas atteint*") 'Avertissement TCG absent'

foreach ($case in @(
    @{ Free=$null; Ram=2048 }, @{ Free=4096; Ram=2048 }, @{ Free=7167; Ram=2048 },
    @{ Free=7168; Ram=4096 }, @{ Free=11263; Ram=4096 }, @{ Free=11264; Ram=8192 },
    @{ Free=15359; Ram=8192 }, @{ Free=15360; Ram=12288 }
)) {
    $capabilities.AvailableMiB = $case.Free
    Assert-Profile ((Resolve-BouchaudPerformance $capabilities).RamMiB -eq $case.Ram) "Palier RAM incorrect ($($case.Free))"
}
foreach ($free in @(0, 3072, 4095)) {
    $capabilities.AvailableMiB = $free
    Assert-Refused $capabilities @{} 'moins de 4096 MiB'
}
Write-Output 'BOUCHAUD_PERFORMANCE_TEST_OK'
