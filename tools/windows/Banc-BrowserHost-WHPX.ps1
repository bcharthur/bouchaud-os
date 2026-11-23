#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ReferenceCheckout,
    [Parameter(Mandatory)][string]$CandidateCheckout,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{40}$')][string]$ReferenceSha,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{40}$')][string]$CandidateSha,
    [Parameter(Mandatory)][string]$BootImage,
    [Parameter(Mandatory)][string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

$roots = @{
    A = (Resolve-Path -LiteralPath $ReferenceCheckout).ProviderPath
    B = (Resolve-Path -LiteralPath $CandidateCheckout).ProviderPath
}
$heads = @{A = $ReferenceSha; B = $CandidateSha}
$runs = @{A = '45009297744'; B = '45044774669'}
$bootHash = '4006FFD6EB9B11B33F4C1BBAE43661CE73D767EE656AC6726B130C161CB68246'
$boot = (Resolve-Path -LiteralPath $BootImage).ProviderPath
$launcher = Join-Path $roots.B 'run.ps1'
$launcherHash = Get-Sha256 $launcher
$qemu = 'C:\Program Files\qemu\qemu-system-x86_64.exe'
$qemuHash = Get-Sha256 $qemu
$fixtureHash = Get-Sha256 (Join-Path $PSScriptRoot '..\health\ladybird-whpx-p0.html')
$query = 'ladybird-whpx-p0.html?phases=1&auto-start=1&phase-markers=1'
$url = "http://10.0.2.2:18082/$query"
$profile = 'whpx,irqchip=off,smp=1,ram_mib=8192,surface=simple,audio=none,cpu-renderer'
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'

function Confirm-Inputs([string]$Arm) {
    $root = $roots[$Arm]
    $head = git -C $root rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $head -ne $heads[$Arm]) { throw "SHA inattendu pour $Arm." }
    $changes = @(git -C $root status --porcelain --untracked-files=no)
    if ($LASTEXITCODE -ne 0 -or $changes.Count -ne 0) { throw "Checkout modifie pour $Arm." }
    if ((Get-Sha256 $boot) -ne $bootHash -or (Get-Sha256 $qemu) -ne $qemuHash -or
        (Get-Sha256 $launcher) -ne $launcherHash) { throw 'Bootimage, QEMU ou lanceur modifie.' }
    $client = New-Object System.Net.WebClient
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $body = $client.DownloadData("http://127.0.0.1:18082/$query")
        $servedHash = [BitConverter]::ToString($hasher.ComputeHash($body)).Replace('-', '')
        if ($servedHash -ne $fixtureHash) { throw 'Fixture HTTP incompatible.' }
    }
    finally {
        $client.Dispose()
        $hasher.Dispose()
    }
}

if ($roots.A -eq $roots.B) { throw 'Deux checkouts distincts sont requis.' }
foreach ($arm in 'A','B') {
    Confirm-Inputs $arm
    foreach ($name in 'run-whpx-ab.ps1','ladybird-browser.img') {
        if (Test-Path -LiteralPath (Join-Path $roots[$arm] $name)) {
            throw "Utiliser des checkouts jetables sans $name preexistant."
        }
    }
}
$output = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$bootCopy = Join-Path $output 'bootimage.bin'
Copy-Item -LiteralPath $boot -Destination $bootCopy
$order = 'A','B','B','A','A','B'
$created = @()
try {
    foreach ($arm in 'A','B') {
        $copy = Join-Path $roots[$arm] 'run-whpx-ab.ps1'
        Copy-Item -LiteralPath $launcher -Destination $copy
        $created += $copy
        if ((Get-Sha256 $copy) -ne $launcherHash) { throw 'Copie du lanceur incompatible.' }
    }

    for ($i = 0; $i -lt $order.Count; $i++) {
        $arm = $order[$i]
        $root = $roots[$arm]
        Confirm-Inputs $arm
        if (Get-Process -Name 'qemu-system-x86_64' -ErrorAction SilentlyContinue) {
            throw 'Une VM QEMU est deja active.'
        }
        $trial = (New-Item -ItemType Directory -Path (Join-Path $output ('{0:D2}-{1}' -f ($i + 1), $arm))).FullName
        $logDir = Join-Path $root 'target\journaux-qemu'
        $previous = @(Get-ChildItem -Path "$logDir\*-serie.log" -ErrorAction SilentlyContinue |
            ForEach-Object FullName)
        $disk = Join-Path $root 'ladybird-browser.img'
        $runner = Join-Path $root 'run-whpx-ab.ps1'
        if (Test-Path -LiteralPath $disk) { throw "Disque deja present : $disk" }
        if ((Get-Sha256 $runner) -ne $launcherHash) { throw 'Lanceur de benchmark modifie.' }

        Write-Host "Passage $($i + 1)/6 ($arm). Apres le scroll automatique, quitter normalement la VM."
        $launchArgs = @('-NoProfile', '-File', $runner, '-BenchmarkBootImage', $bootCopy,
            '-BenchmarkBootSha256', $bootHash, '-Accel', 'whpx', '-WhpxIrqchip', 'off',
            '-CpuCount', '1', '-RamMiB', '8192', '-Surface', 'simple', '-Audio', 'none',
            '-LadybirdRunId', $runs[$arm], '-Gate0Autostart', '-LadybirdUrl', $url)
        & $windowsPowerShell @launchArgs | Tee-Object -FilePath (Join-Path $trial 'launcher.log') | Out-Host
        $status = $LASTEXITCODE
        $logs = @(Get-ChildItem -Path "$logDir\*-serie.log" -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -notin $previous })
        if (Test-Path -LiteralPath $disk) {
            # run.ps1 initializes the 128 MiB persistence area to zero on image creation.
            Move-Item -LiteralPath $disk -Destination (Join-Path $trial 'disk-after-run.img')
        }
        if ($logs.Count -ne 1) { throw "Journal serie unique absent (code du lanceur $status)." }
        Copy-Item -LiteralPath $logs[0].FullName -Destination (Join-Path $trial 'serial.log')
        $qemuLog = $logs[0].FullName -replace '-serie\.log$', '-qemu.log'
        if (Test-Path -LiteralPath $qemuLog) { Copy-Item -LiteralPath $qemuLog -Destination $trial }
        if ($status -ne 0 -or
            -not (Select-String -LiteralPath $logs[0].FullName -Pattern 'BOUCHAUD_BUILD commit=621a84a8b440\b' -Quiet) -or
            -not (Select-String -LiteralPath $logs[0].FullName -Pattern 'BOUCHAUD_CPU_ISA hyperviseur=MicrosoftHv\b' -Quiet) -or
            -not (Select-String -LiteralPath $logs[0].FullName -Pattern '\(js log\).*WHPX_P0_SCROLL ' -Quiet)) {
            throw "Passage non qualifiable (lanceur $status, noyau, WHPX ou scroll). Voir $trial."
        }
        Confirm-Inputs $arm
        $browser = Join-Path $root 'native-browser-m9'
        $manifest = Join-Path $browser 'BOUCHAUD_ARTIFACT_MANIFEST.json'
        Copy-Item -LiteralPath $manifest -Destination $trial
        [ordered]@{
            arm = $arm
            checkout = $heads[$arm]
            run_id = $runs[$arm]
            boot_sha256 = Get-Sha256 $bootCopy
            fixture_sha256 = $fixtureHash
            qemu_sha256 = $qemuHash
            launcher_sha256 = $launcherHash
            manifest_sha256 = Get-Sha256 $manifest
            browserhost_sha256 = Get-Sha256 (Join-Path $browser 'BouchaudBrowserHost')
            webcontent_sha256 = Get-Sha256 (Join-Path $browser 'WebContent')
            profile = $profile
            url = $url
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $trial 'evidence.json') -Encoding UTF8
    }
}
finally {
    foreach ($copy in $created) { Remove-Item -LiteralPath $copy }
}
