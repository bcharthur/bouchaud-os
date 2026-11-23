# Windows PowerShell 5.1. Detection only describes the host.
function Get-BouchaudPerformanceHost {
    param([Parameter(Mandatory=$true)][string]$QemuExe)
    $machine = Get-CimInstance Win32_ComputerSystem -ErrorAction SilentlyContinue
    $os = Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue
    $cpu = @(Get-CimInstance Win32_Processor -ErrorAction SilentlyContinue | ForEach-Object Name)
    $gpu = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | ForEach-Object Name)
    $help = & $QemuExe -accel help 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Performance : impossible de lire les accelerateurs QEMU.' }
    [pscustomobject]@{
        CpuName = $cpu -join ', '
        GpuName = $gpu -join ', '
        HypervisorPresent = $machine.HypervisorPresent
        TotalMiB = [long]($machine.TotalPhysicalMemory / 1MB)
        AvailableMiB = if ($null -ne $os.FreePhysicalMemory) { [long][math]::Floor($os.FreePhysicalMemory / 1024) } else { $null }
        QemuWhpx = [bool]($help -match '^\s*whpx\s*$')
        QemuTcg = [bool]($help -match '^\s*tcg\s*$')
    }
}

function Resolve-BouchaudPerformance {
    param(
        [Parameter(Mandatory=$true)]$Capabilities,
        [System.Collections.IDictionary]$Explicit = @{}
    )

    # Profile backed by the WHPX SMP1 reference runs. The presence of a
    # graphics adapter or an accelerator in QEMU help is not a benchmark.
    $profile = [ordered]@{
        Accel = 'whpx'
        CpuCount = 1
        RamMiB = 2048
        WhpxIrqchip = 'off'
        Surface = 'simple'
        P10GPU = $false
        Diagnostic = $false
    }
    $reason = 'WHPX SMP1 de reference. Initialisation a confirmer par QEMU.'
    if (-not ($Capabilities.QemuWhpx -and $Capabilities.HypervisorPresent)) {
        if (-not $Capabilities.QemuTcg) { throw 'Performance : aucun accelerateur pris en charge disponible.' }
        $profile.Accel = 'tcg'
        $reason = 'WHPX non detecte. Degradation explicite vers TCG (emulation logicielle).'
    }

    $available = $Capabilities.AvailableMiB
    if ($null -ne $available -and $available -lt 4096) {
        throw 'Performance : moins de 4096 MiB disponibles. Le profil minimal reserve 2048 MiB a Windows.'
    }
    foreach ($tier in @(12288,8192,4096)) {
        if ($available -ge ($tier + 3072)) {
            $profile.RamMiB = $tier
            break
        }
    }
    if ($null -eq $available) { $reason += ' RAM disponible inconnue, profil minimal retenu.' }

    foreach ($name in $profile.Keys) {
        if (($Explicit.Keys -contains $name) -and $Explicit[$name] -ne $profile[$name]) {
            throw "Performance : -$name contredit le profil ($($profile[$name])). Omettre -Performance pour fixer les parametres."
        }
    }
    foreach ($name in @('Legacy','LadybirdM8','LadybirdM9Test','LadybirdSansChrome','Gate0Autostart','Gate0SerialPort')) {
        if ($Explicit[$name]) { throw "Performance : -$name est incompatible avec le profil interactif." }
    }
    $profile['Reason'] = $reason
    $profile['AllowInitFallback'] = $profile.Accel -eq 'whpx' -and $Capabilities.QemuTcg -and -not ($Explicit.Keys -contains 'Accel')
    return [pscustomobject]$profile
}

function Test-BouchaudPerformanceFallback {
    param($Profile, $Verdict)
    return $Profile.AllowInitFallback -and $Verdict.whpx_indisponible -and -not $Verdict.noyau -and $null -eq $Verdict.vp_exit
}

function Write-BouchaudPerformance {
    param($Profile, $Capabilities, [string]$Accel, [string]$Reason)
    $irq = if ($Accel -eq 'whpx') { "WHPX irqchip $($Profile.WhpxIrqchip)" } else { 'N/A (TCG)' }
    Write-Host '=== BOUCHAUD PERFORMANCE ===' -ForegroundColor Cyan
    Write-Host "Mode           : PERFORMANCE"
    Write-Host "Accelerateur   : $($Accel.ToUpperInvariant())"
    Write-Host "CPU            : $($Profile.CpuCount) vCPU (profil de reference)"
    Write-Host "RAM            : $($Profile.RamMiB) MiB"
    Write-Host "Surface        : $($Profile.Surface)"
    Write-Host 'GPU            : CPU (aucun profil VIRGL/Vulkan qualifie automatiquement)'
    Write-Host "IRQ            : $irq"
    Write-Host 'Journalisation : production (evenements critiques conserves)'
    Write-Host 'Ladybird       : artefact verifie'
    Write-Host "Hote CPU       : $($Capabilities.CpuName)"
    Write-Host "Hote GPU       : $($Capabilities.GpuName)"
    $free = if ($null -eq $Capabilities.AvailableMiB) { 'N/A' } else { $Capabilities.AvailableMiB }
    Write-Host "Hote RAM       : $free/$($Capabilities.TotalMiB) MiB disponibles"
    Write-Host "Decision       : $Reason"
    if ($Accel -eq 'tcg') {
        Write-Warning 'TCG actif : le profil materiel de performance WHPX n''est pas atteint.'
    }
}

Export-ModuleMember -Function Get-BouchaudPerformanceHost,Resolve-BouchaudPerformance,Test-BouchaudPerformanceFallback,Write-BouchaudPerformance
