param(
    [string]$HostName = "169.254.178.21",
    [double]$Timeout = 8.0,
    [int]$Events = 1024,
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $Repo

if (-not $env:BOUCHAUD_DEBUG_TOKEN) {
    throw "BOUCHAUD_DEBUG_TOKEN est absent. Le bundle BRDP exige le jeton de l'image de laboratoire."
}
if ($Events -lt 1 -or $Events -gt 1024) {
    throw "Events doit etre entre 1 et 1024."
}
if ([string]::IsNullOrWhiteSpace($Out)) {
    $Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $Out = Join-Path $Repo "target\ladybird-debug-$Stamp"
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path

$Head = git rev-parse HEAD
$Branch = git branch --show-current
@(
    "head=$Head"
    "branch=$Branch"
    "host=$HostName"
    "captured=$(Get-Date -Format o)"
) | Set-Content -Encoding ascii (Join-Path $Out "context.txt")

git status --short | Set-Content -Encoding utf8 (Join-Path $Out "git-status.txt")

$Failures = 0
function Invoke-Probe {
    param([string]$Name, [string[]]$CommandArgs)
    Write-Host "=== $Name ==="
    $Log = Join-Path $Out "$Name.txt"
    $ProbeOutput = & python @CommandArgs 2>&1
    $ExitCode = $LASTEXITCODE
    $ProbeOutput | Tee-Object -FilePath $Log
    if ($ExitCode -ne 0) {
        $script:Failures++
        Write-Warning "$Name a echoue (exit=$ExitCode), collecte suivante poursuivie."
    }
}

Invoke-Probe "doctor" @(
    ".\tools\remote\bouchaud-control.py",
    "--host", $HostName,
    "--timeout", "$Timeout",
    "doctor"
)

Invoke-Probe "services-all" @(
    ".\tools\remote\bouchaud-lab.py",
    "services-all",
    "--host", $HostName,
    "--timeout", "15",
    "--out", (Join-Path $Out "services-detail.json")
)

Invoke-Probe "serial-capture" @(
    ".\tools\remote\bouchaud-lab.py",
    "serial-capture",
    "--host", $HostName,
    "--timeout", "15",
    "--bytes", "1048576",
    "--out", (Join-Path $Out "serial-live.log")
)

Invoke-Probe "events" @(
    ".\tools\remote\bouchaud-lab.py",
    "events",
    "--host", $HostName,
    "--timeout", "$Timeout",
    "--tail", "$Events"
)

# Le dump standard garde en plus status/net/processes/memory/blackbox dans son propre sous-dossier.
Invoke-Probe "dump" @(
    ".\tools\remote\bouchaud-lab.py",
    "dump",
    "--host", $HostName,
    "--timeout", "15",
    "--events", "$Events",
    "--out", $Out
)

$Zip = "$Out.zip"
if (Test-Path $Zip) { Remove-Item -LiteralPath $Zip -Force }
Compress-Archive -Path (Join-Path $Out "*") -DestinationPath $Zip -CompressionLevel Optimal
$Hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Zip).Hash
Write-Host "LADYBIRD_DEBUG_BUNDLE=$Zip"
Write-Host "SHA256=$Hash"
Write-Host "PROBE_FAILURES=$Failures"
if ($Failures -gt 0) {
    Write-Warning "Le bundle est exploitable mais $Failures sonde(s) ont echoue ; leurs logs sont inclus."
}
