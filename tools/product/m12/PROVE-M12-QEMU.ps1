# BOUCHAUD_M12_QEMU_HOTFIX_V1
# Uses hashtable splatting; named switches MUST NOT be passed as positional strings.
param(
    [string]$Repo = 'C:\Users\Arthur\RustroverProjects\bouchaud-os-p13-current',
    [switch]$Run,
    [switch]$PersistLab,
    [switch]$DebugExit,
    [ValidateSet('tcg','whpx')][string]$Accel = 'tcg',
    [switch]$RebuildImage,
    [switch]$ACPIProof
)
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path -LiteralPath $Repo -ErrorAction Stop).Path
$Runner = Join-Path $Repo 'tools\reference\run-trigkey-single-usb-qemu.ps1'
$Image = Join-Path $Repo 'target\reference\bouchaud-trigkey-stage2-ladybird.img'
$Builder = Join-Path $Repo 'tools\reference\IMAGE-TRIGKEY.ps1'
if (-not (Test-Path -LiteralPath $Runner -PathType Leaf)) { throw "M12_RUNNER_MISSING $Runner" }
$runnerScript = Get-Content -LiteralPath $Runner -Raw
if ($DebugExit -and $runnerScript -notmatch '\[switch\]\$DebugExit') { throw 'M12_RUNNER_NO_DEBUG_EXIT' }
if ($ACPIProof -and $DebugExit) { throw 'M12_ACPI_PROOF_MUST_DISABLE_DEBUG_EXIT' }
if ($ACPIProof -and $runnerScript -notmatch '\[switch\]\$ExitOnShutdown') { throw 'M12_RUNNER_NO_EXIT_ON_SHUTDOWN' }
if ($PersistLab -and $runnerScript -notmatch '\[switch\]\$PersistLab') { throw 'M12_RUNNER_NO_PERSIST_LAB' }
# Intentional build is never triggered by the default dry-run.
if ($RebuildImage) {
    if (-not (Test-Path -LiteralPath $Builder -PathType Leaf)) { throw 'M12_IMAGE_BUILDER_MISSING' }
    Write-Host 'M12_REBUILD_IMAGE_REQUESTED source_dirty_allowed=1'
    & $Builder -QuandMeme
    if (-not $?) { throw 'M12_REBUILD_IMAGE_FAILED' }
    if ($LASTEXITCODE -ne 0) { throw "M12_REBUILD_IMAGE_EXIT=$LASTEXITCODE" }
}
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) { throw 'M12_IMAGE_MISSING_REBUILD_REQUIRED' }
$ImageItem = Get-Item -LiteralPath $Image
$SourceFiles = @(
    'src\kernel\process\thread\diagnostic_stall.rs',
    'src\gui\window_manager.rs',
    'src\kernel\power.rs',
    'src\kernel\time\timer.rs'
)
$NewerSources = @()
foreach ($Rel in $SourceFiles) {
    $File = Join-Path $Repo $Rel
    if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { throw "M12_SOURCE_MISSING $Rel" }
    $Item = Get-Item -LiteralPath $File
    if ($Item.LastWriteTimeUtc -gt $ImageItem.LastWriteTimeUtc.AddSeconds(2)) {
        $NewerSources += $Rel
    }
}
if ($NewerSources.Count -gt 0) {
    Write-Host 'M12_IMAGE_STALE: M1.2 Rust source is newer than the image.'
    $NewerSources | ForEach-Object { Write-Host "  newer_source=$_" }
    if ($Run) { throw 'M12_IMAGE_STALE_REBUILD_WITH_IMAGE_TRIGKEY_FIRST' }
    Write-Host 'M12_TEST_DRY_RUN_UNQUALIFIED stale_image=1'
}
$sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $Image).Hash
Write-Host "M12_TEST_IMAGE_SHA256=$sha"
Write-Host "M12_TEST_PLAN accel=$Accel PersistLab=$([bool]$PersistLab) DebugExit=$([bool]$DebugExit) ACPIProof=$([bool]$ACPIProof) Run=$([bool]$Run)"
if (-not $Run) {
    Write-Host 'M12_TEST_DRY_RUN_OK (no QEMU launched, no runtime qualified)'
    return
}
# Hashtable splatting is ESSENTIAL for named PowerShell parameters.
# Array splatting turned the literal '-SkipBuild' into RamMiB's positional value.
$RunnerParams = @{
    SkipBuild = $true
    Accel = $Accel
}
if ($PersistLab) { $RunnerParams['PersistLab'] = $true }
if ($DebugExit) { $RunnerParams['DebugExit'] = $true }
if ($ACPIProof) { $RunnerParams['ExitOnShutdown'] = $true }
Write-Host 'M12_QEMU_START new_session_required=1'
$global:LASTEXITCODE = 0
& $Runner @RunnerParams
$rc = $global:LASTEXITCODE
if (-not $?) { throw 'M12_QEMU_RUNNER_FAILED' }
Write-Host "M12_QEMU_RETURNED shell_exit_code=$rc"
Write-Host 'M12_QEMU_TRUTH: DebugExit proves only emulator exit; ACPI S5 uses -ACPIProof without DebugExit, with automatic QEMU shutdown enabled.'
Write-Host 'M12_QEMU_TRUTH: check newest serial log before declaring boot/performance success.'
