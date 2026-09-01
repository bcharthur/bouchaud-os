param(
    [string]$Bootimage = "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin",
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Out = Join-Path $Root "target\native-ipc-runtime"
$Probe = Join-Path $Root "tools\userland\prebuilt\native-ipc-ring3-probe"
$Image = Join-Path $Out "native-ipc-probe.img"
$Log = Join-Path $Out "native-ipc-ring3.log"
$Builder = Join-Path $Root "tools\native\make_native_ipc_probe_image.py"
$Scanner = Join-Path $Root "tools\ci\reliability\logscan.py"

if (-not [System.IO.Path]::IsPathRooted($Bootimage)) {
    $Bootimage = Join-Path $Root $Bootimage
}
if (-not (Test-Path $Bootimage -PathType Leaf)) {
    throw "Bootimage absent: $Bootimage. Lance d'abord cargo bootimage."
}
if (-not (Test-Path $Probe -PathType Leaf)) {
    throw "Probe precompile absent: $Probe"
}
if (-not (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue)) {
    throw "qemu-system-x86_64 introuvable dans PATH"
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
Remove-Item $Image,$Log -Force -ErrorAction SilentlyContinue

& python $Builder `
    --ring3-probe $Probe `
    --image $Image
if ($LASTEXITCODE -ne 0) {
    throw "Echec de fabrication de l'image native IPC"
}

$qemu = (Get-Command qemu-system-x86_64).Source
$qemuArgs = @(
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

Write-Host "=== Native IPC ring3 / SMP4 ==="
Write-Host "boot : $Bootimage"
Write-Host "disk : $Image"
Write-Host "log  : $Log"

$process = Start-Process -FilePath $qemu -ArgumentList $qemuArgs -PassThru
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$success = $false
$fatal = $null

function Read-ProbeLog {
    if (-not (Test-Path $Log -PathType Leaf)) {
        return ""
    }
    return (Get-Content $Log -Tail 1200 -ErrorAction SilentlyContinue) -join "`n"
}

try {
    while ((Get-Date) -lt $deadline) {
        $tail = Read-ProbeLog

        if ($tail -match "\*\*\*\s*KERNEL PANIC\s*\*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|BKL(?:-FR)?.*VIOLATION") {
            $fatal = $Matches[0]
            break
        }

        if ($tail.Contains("[NATIVE-IPC-RING3] OK")) {
            $success = $true
            break
        }

        $process.Refresh()
        if ($process.HasExited) {
            # QEMU normally exits immediately after the autorun succeeds and
            # Bouchaud requests shutdown. One final log read is mandatory:
            # otherwise a successful probe can race the parent polling loop.
            Start-Sleep -Milliseconds 150
            $tail = Read-ProbeLog

            if ($tail -match "\*\*\*\s*KERNEL PANIC\s*\*\*\*|DOUBLE FAULT|TRIPLE FAULT|SpinLock recursive acquisition|BKL(?:-FR)?.*VIOLATION") {
                $fatal = $Matches[0]
            }
            elseif ($tail.Contains("[NATIVE-IPC-RING3] OK")) {
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
    Get-Content $Log -Tail 160
    throw "Fatal detecte pendant le probe: $fatal"
}

if (-not $success) {
    if (Test-Path $Log) {
        Get-Content $Log -Tail 160
    }
    throw "Le marqueur [NATIVE-IPC-RING3] OK n'est pas apparu en $TimeoutSeconds secondes"
}

& python $Scanner $Log
if ($LASTEXITCODE -ne 0) {
    throw "Le scanner reliability a refuse le journal"
}

$required = @(
    "[NATIVE-IPC-RING3] ABI=1.0",
    "[NATIVE-IPC-RING3] CHANNEL_OK",
    "[NATIVE-IPC-RING3] HANDLE_TRANSFER_OK",
    "[NATIVE-IPC-RING3] EVENT_WAITSET_OK",
    "[NATIVE-IPC-RING3] RIGHTS_OK",
    "[NATIVE-IPC-RING3] SHM_OK",
    "[NATIVE-IPC-RING3] OK"
)

$whole = Get-Content $Log -Raw
foreach ($marker in $required) {
    if (-not $whole.Contains($marker)) {
        throw "Marqueur obligatoire absent: $marker"
    }
}

Select-String -Path $Log -SimpleMatch -Pattern "[NATIVE-IPC-RING3]" |
    ForEach-Object { $_.Line }

Write-Host ""
Write-Host "NATIVE_IPC_RING3_OK"
