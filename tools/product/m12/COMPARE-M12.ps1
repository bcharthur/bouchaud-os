# BOUCHAUD_M12_COMPARE_HOTFIX_V1
param(
    [string]$Repo = 'C:\Users\Arthur\RustroverProjects\bouchaud-os-p13-current',
    [string]$Baseline,
    [string]$Current
)
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path -LiteralPath $Repo -ErrorAction Stop).Path
$ReportScript = Join-Path $Repo 'tools\product\m12\m12_runtime_report.py'
if (-not (Test-Path -LiteralPath $ReportScript -PathType Leaf)) { throw 'M12_REPORT_SCRIPT_MISSING' }
$LogDir = Join-Path $Repo 'target\reference\qemu-logs'
if (-not $Current) {
    $Latest = Get-ChildItem -LiteralPath $LogDir -Filter '*.log' -ErrorAction Stop |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if (-not $Latest) { throw 'M12_NO_QEMU_LOG' }
    $Current = $Latest.FullName
}
if (-not (Test-Path -LiteralPath $Current -PathType Leaf)) { throw 'M12_CURRENT_LOG_MISSING' }
$Current = (Resolve-Path -LiteralPath $Current).Path
if ($Baseline) {
    if (-not (Test-Path -LiteralPath $Baseline -PathType Leaf)) { throw 'M12_BASELINE_LOG_MISSING' }
    $Baseline = (Resolve-Path -LiteralPath $Baseline).Path
    if ([string]::Equals($Baseline, $Current, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'M12_IDENTICAL_LOGS: same path; run a NEW QEMU session after rebuilding image.'
    }
    $BeforeSHA = (Get-FileHash -Algorithm SHA256 -LiteralPath $Baseline).Hash
    $AfterSHA = (Get-FileHash -Algorithm SHA256 -LiteralPath $Current).Hash
    if ($BeforeSHA -eq $AfterSHA) {
        throw 'M12_IDENTICAL_LOGS: identical SHA256, comparison would be meaningless.'
    }
}
$Out = Join-Path $Repo 'target\m12\m12-report.json'
$ArgsList = @($ReportScript,'--log',$Current,'--json',$Out)
if ($Baseline) { $ArgsList += @('--baseline',$Baseline) }
& py -3 @ArgsList
if ($LASTEXITCODE -ne 0) { throw "M12_REPORT_FAILED rc=$LASTEXITCODE" }
Write-Host "M12_REPORT_FILE=$Out"
Write-Host 'M12_COMPARE_TRUTH: rate comparison is observational; same test actions, duration and QEMU config must be confirmed.'
