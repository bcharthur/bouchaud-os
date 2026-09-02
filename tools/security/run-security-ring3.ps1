param(
    [string]$Bootimage = "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin",
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Out = Join-Path $Root "target\security-runtime"
$Probe = Join-Path $Root "tools\security\prebuilt\security-ring3-probe"
$Image = Join-Path $Out "security-probe.img"
$Log = Join-Path $Out "security-ring3.log"

if (-not [System.IO.Path]::IsPathRooted($Bootimage)) {
    $Bootimage = Join-Path $Root $Bootimage
}
if (-not (Test-Path $Bootimage -PathType Leaf)) {
    throw "Bootimage absent: $Bootimage. Lance cargo bootimage."
}
if (-not (Test-Path $Probe -PathType Leaf)) {
    throw "Probe precompile absent: $Probe"
}
if (-not (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue)) {
    throw "qemu-system-x86_64 introuvable dans PATH"
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
Remove-Item $Image,$Log -Force -ErrorAction SilentlyContinue

& python (Join-Path $Root "tools\security\make-security-probe-image.py") `
    --probe $Probe `
    --image $Image
if ($LASTEXITCODE -ne 0) { throw "Fabrication image security impossible" }

$qemu = (Get-Command qemu-system-x86_64).Source
$args = @(
    "-drive", "format=raw,file=$Bootimage",
    "-drive", "format=raw,file=$Image",
    "-m", "4096",
    "-smp", "4",
    "-cpu", "max",
    "-display", "none",
    "-no-reboot",
    "-netdev", "user,id=net0",
    "-device", "e1000,netdev=net0",
    "-audiodev", "none,id=muet",
    "-device", "AC97,audiodev=muet",
    "-serial", "file:$Log"
)

Write-Host "=== Security architecture / ring3 / SMP4 ==="
Write-Host "boot : $Bootimage"
Write-Host "disk : $Image"
Write-Host "log  : $Log"

$process = Start-Process -FilePath $qemu -ArgumentList $args -PassThru
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$success = $false
$fatal = $null

function Read-SecurityLog {
    if (-not (Test-Path $Log -PathType Leaf)) { return "" }
    return (Get-Content $Log -Tail 1400 -ErrorAction SilentlyContinue) -join "`n"
}

try {
    while ((Get-Date) -lt $deadline) {
        $tail = Read-SecurityLog

        if ($tail -match "\*\*\*\s*KERNEL PANIC\s*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|BKL(?:-FR)?.*VIOLATION") {
            $fatal = $Matches[0]
            break
        }
        if ($tail.Contains("[SECURITY-RING3] OK")) {
            $success = $true
            break
        }

        $process.Refresh()
        if ($process.HasExited) {
            Start-Sleep -Milliseconds 150
            $tail = Read-SecurityLog
            if ($tail.Contains("[SECURITY-RING3] OK")) {
                $success = $true
            }
            break
        }
        Start-Sleep -Milliseconds 200
    }
}
finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        $process.WaitForExit()
    }
}

if ($fatal) {
    Get-Content $Log -Tail 200
    throw "Fatal kernel pendant le test security: $fatal"
}
if (-not $success) {
    if (Test-Path $Log) { Get-Content $Log -Tail 220 }
    throw "Le marqueur [SECURITY-RING3] OK n'est pas apparu"
}

& python (Join-Path $Root "tools\ci\reliability\logscan.py") $Log
if ($LASTEXITCODE -ne 0) { throw "logscan a refuse le journal security" }

$required = @(
    "[SECURITY-RING3] WX_MMAP_DENIED",
    "[SECURITY-RING3] WX_MPROTECT_DENIED",
    "[SECURITY-RING3] SETUID_DROP_OK",
    "[SECURITY-RING3] PRIV_ESC_DENIED",
    "[SECURITY-RING3] DEVICE_DENIED",
    "[SECURITY-RING3] PATH_CANONICAL_OK",
    "[SECURITY-RING3] DIRFD_CANONICAL_OK",
    "[SECURITY-RING3] DIRFD_MUTATION_OK",
    "[SECURITY-RING3] STICKY_TMP_OK",
    "[SECURITY-RING3] MMAP_DAC_OK",
    "[SECURITY-RING3] RAW_SOCKET_DENIED",
    "[SECURITY-RING3] SIGNAL_DENIED",
    "[SECURITY-RING3] THREAD_SIGNAL_DENIED",
    "[SECURITY-RING3] NNP_OK",
    "[SECURITY-RING3] NATIVE_SHM_LIMIT_OK",
    "[SECURITY-RING3] JIT_DENIED",
    "[SECURITY-RING3] OK",
    "[SECURITY-DENY]"
)
$whole = Get-Content $Log -Raw
foreach ($marker in $required) {
    if (-not $whole.Contains($marker)) {
        throw "Marqueur security obligatoire absent: $marker"
    }
}

Select-String -Path $Log -Pattern "\[SECURITY-RING3\]|\[SECURITY-DENY\]" |
    ForEach-Object { $_.Line }

Write-Host ""
Write-Host "SECURITY_RING3_OK"
